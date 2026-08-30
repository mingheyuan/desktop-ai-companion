import AppKit
import AVFoundation
import Darwin
import Foundation
import Speech

private let protocolMajor = 1
private let outputLock = NSLock()

private func argument(named name: String) -> String? {
    guard let index = CommandLine.arguments.firstIndex(of: name), index + 1 < CommandLine.arguments.count else {
        return nil
    }
    return CommandLine.arguments[index + 1]
}

private func emit(_ payload: [String: Any]) {
    guard JSONSerialization.isValidJSONObject(payload),
          let data = try? JSONSerialization.data(withJSONObject: payload) else { return }
    outputLock.lock()
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
    outputLock.unlock()
}

private func fail(_ code: String, _ message: String) -> Never {
    emit(["type": "host.error", "code": code, "message": message])
    exit(1)
}

private struct WakeWordMatch {
    let phrase: String
    let range: Range<String.Index>
}

private enum WakeWordMatcher {
    static let canonical = "爱丽丝"

    // Keep this list intentionally small. These are local recognition variants,
    // not user-configurable phrases, so a false wake-up does not broaden silently.
    static let phrases = ["天童爱丽丝", "爱丽丝", "艾丽丝", "爱丽思", "爱丽斯", "阿丽丝"]

    static func match(in text: String) -> WakeWordMatch? {
        for phrase in phrases {
            if let range = text.range(of: phrase, options: [.caseInsensitive]) {
                return WakeWordMatch(phrase: phrase, range: range)
            }
        }
        return nil
    }

    static func textAfterWakeWord(in text: String) -> String {
        guard let match = match(in: text) else { return text }
        return String(text[match.range.upperBound...])
            .trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters))
    }
}

private func connectControlSocket(at path: String) -> Bool {
    let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
    guard descriptor >= 0 else { return false }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let pathBytes = Array(path.utf8CString)
    let capacity = MemoryLayout.size(ofValue: address.sun_path)
    guard pathBytes.count <= capacity else {
        Darwin.close(descriptor)
        return false
    }
    path.withCString { source in
        withUnsafeMutablePointer(to: &address.sun_path) { pointer in
            pointer.withMemoryRebound(to: CChar.self, capacity: capacity) { destination in
                _ = Darwin.strncpy(destination, source, capacity)
            }
        }
    }
    let length = socklen_t(MemoryLayout<sa_family_t>.size + pathBytes.count)
    let connected = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Darwin.connect(descriptor, $0, length)
        }
    }
    guard connected == 0,
          Darwin.dup2(descriptor, STDIN_FILENO) >= 0,
          Darwin.dup2(descriptor, STDOUT_FILENO) >= 0 else {
        Darwin.close(descriptor)
        return false
    }
    Darwin.close(descriptor)
    return true
}

private func microphoneStatus() -> String {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: return "authorized"
    case .denied: return "denied"
    case .restricted: return "restricted"
    case .notDetermined: return "not_determined"
    @unknown default: return "restricted"
    }
}

private func speechStatus() -> String {
    switch SFSpeechRecognizer.authorizationStatus() {
    case .authorized: return "authorized"
    case .denied: return "denied"
    case .restricted: return "restricted"
    case .notDetermined: return "not_determined"
    @unknown default: return "restricted"
    }
}

private func emitPermissionStatus() {
    emit([
        "type": "permissions.status",
        "microphone": microphoneStatus(),
        "speech_recognition": speechStatus()
    ])
}

private func requestPermissions(completion: @escaping () -> Void) {
    func requestSpeech() {
        if SFSpeechRecognizer.authorizationStatus() == .notDetermined {
            SFSpeechRecognizer.requestAuthorization { _ in
                DispatchQueue.main.async { emitPermissionStatus(); completion() }
            }
        } else {
            emitPermissionStatus()
            completion()
        }
    }
    if AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined {
        AVCaptureDevice.requestAccess(for: .audio) { _ in
            DispatchQueue.main.async { requestSpeech() }
        }
    } else {
        requestSpeech()
    }
}

private func openPrivacySettings() {
    let settingsURL = URL(fileURLWithPath: "/System/Applications/System Settings.app")
    guard FileManager.default.fileExists(atPath: settingsURL.path) else {
        emit(["type": "host.error", "code": "settings_unavailable", "message": "找不到 macOS 系统设置。"])
        return
    }
    let configuration = NSWorkspace.OpenConfiguration()
    NSWorkspace.shared.openApplication(at: settingsURL, configuration: configuration) { _, error in
        if let error {
            emit(["type": "host.error", "code": "settings_unavailable", "message": error.localizedDescription])
        }
    }
}

private final class VoiceController {
    private enum Mode { case wake, pushToTalk }

    private let queue = DispatchQueue(label: "desktop-ai.audio-host.voice", qos: .userInteractive)
    private let requestLock = NSLock()
    private let locale = Locale(identifier: "zh-CN")
    private let wakeWord = WakeWordMatcher.canonical
    private var engine: AVAudioEngine?
    private var input: AVAudioInputNode?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var mode: Mode?
    private var captureID = ""
    private var generation = 0
    private var epoch = 0
    private var revision = 0
    private var latestText = ""
    private var wakeDetected = false
    private var armed = false
    private var finishing = false
    private var wakeErrorCount = 0
    private var endpointWork: DispatchWorkItem?
    private var rolloverWork: DispatchWorkItem?

    func setArmed(_ enabled: Bool) {
        queue.async {
            self.armed = enabled
            if enabled {
                self.start(mode: .wake, captureID: UUID().uuidString, generation: self.generation + 1)
            } else if self.mode == .wake {
                self.stop(cancelled: true)
            }
        }
    }

    func startPushToTalk(captureID: String, generation: Int) {
        queue.async {
            self.start(mode: .pushToTalk, captureID: captureID, generation: generation)
        }
    }

    func finalize() { queue.async { self.finishRecognition() } }
    func cancel() {
        queue.async {
            self.stop(cancelled: true)
            self.restartWakeIfNeeded()
        }
    }

    func pauseForPlayback() {
        queue.async {
            if self.mode == .wake { self.stop(cancelled: true) }
        }
    }

    func resumeAfterPlayback() {
        queue.async {
            if self.armed && self.mode == nil {
                self.start(mode: .wake, captureID: UUID().uuidString, generation: self.generation + 1)
            }
        }
    }

    private func start(mode nextMode: Mode, captureID: String, generation: Int) {
        stop(cancelled: true)
        guard microphoneStatus() == "authorized", speechStatus() == "authorized" else {
            emit(["type": "voice.error", "code": "permission_denied", "message": "麦克风或语音识别权限未授权。"])
            return
        }
        guard let recognizer = SFSpeechRecognizer(locale: locale), recognizer.isAvailable else {
            emit(["type": "voice.error", "code": "asr_unavailable", "message": "Apple Speech 当前不可用。"])
            return
        }
        guard nextMode != .wake || recognizer.supportsOnDeviceRecognition else {
            armed = false
            emit([
                "type": "voice.error",
                "code": "on_device_wake_unavailable",
                "message": "此设备不支持 Apple Speech 设备端识别，已停止持续监听以保护环境音隐私。"
            ])
            return
        }

        let nextRequest = SFSpeechAudioBufferRecognitionRequest()
        nextRequest.shouldReportPartialResults = true
        nextRequest.taskHint = .dictation
        nextRequest.contextualStrings = WakeWordMatcher.phrases
        if #available(macOS 13.0, *) { nextRequest.addsPunctuation = true }
        if recognizer.supportsOnDeviceRecognition { nextRequest.requiresOnDeviceRecognition = true }

        let nextEngine = AVAudioEngine()
        let nextInput = nextEngine.inputNode
        let format = nextInput.outputFormat(forBus: 0)
        nextInput.installTap(onBus: 0, bufferSize: 512, format: format) { [weak self] buffer, _ in
            self?.requestLock.lock()
            self?.request?.append(buffer)
            self?.requestLock.unlock()
        }

        self.mode = nextMode
        self.captureID = captureID
        self.generation = generation
        self.epoch += 1
        let activeEpoch = self.epoch
        self.revision = 0
        self.latestText = ""
        self.wakeDetected = nextMode == .pushToTalk
        self.finishing = false
        self.request = nextRequest
        self.engine = nextEngine
        self.input = nextInput
        self.task = recognizer.recognitionTask(with: nextRequest) { [weak self] result, error in
            self?.queue.async { self?.handle(result: result, error: error, epoch: activeEpoch) }
        }
        do {
            nextEngine.prepare()
            try nextEngine.start()
            emit(["type": "capture.started", "capture_id": captureID, "generation": generation, "mode": nextMode == .wake ? "wake" : "push_to_talk", "on_device": recognizer.supportsOnDeviceRecognition])
            scheduleRollover(epoch: activeEpoch)
        } catch {
            emit(["type": "voice.error", "code": "capture_failed", "message": error.localizedDescription])
            stop(cancelled: true)
        }
    }

    private func handle(result: SFSpeechRecognitionResult?, error: Error?, epoch activeEpoch: Int) {
        guard activeEpoch == epoch, mode != nil else { return }
        if let result {
            wakeErrorCount = 0
            latestText = result.bestTranscription.formattedString.trimmingCharacters(in: .whitespacesAndNewlines)
            if mode == .wake && !wakeDetected, WakeWordMatcher.match(in: latestText) != nil {
                wakeDetected = true
                emit(["type": "wake.detected", "capture_id": captureID, "generation": generation, "wake_word": wakeWord])
                scheduleEndpoint(after: 5)
            }
            if wakeDetected {
                let visibleText = textAfterWakeWord(latestText)
                revision += 1
                emit(["type": "asr.partial", "capture_id": captureID, "generation": generation, "recognition_epoch": activeEpoch, "revision": revision, "text": visibleText])
                if mode == .wake && !visibleText.isEmpty { scheduleEndpoint() }
            }
            if result.isFinal {
                finishWithText(textAfterWakeWord(latestText))
                return
            }
        }
        if let error, !finishing {
            let wasWake = mode == .wake
            stop(cancelled: true)
            if wasWake {
                wakeErrorCount += 1
                if wakeErrorCount >= 3 {
                    armed = false
                    emit([
                        "type": "voice.error",
                        "code": "wake_recognition_unavailable",
                        "message": "设备端持续识别连续失败，已关闭持续监听：\(error.localizedDescription)"
                    ])
                    return
                }
            }
            emit(["type": "voice.error", "code": "recognition_failed", "message": error.localizedDescription])
            restartWakeIfNeeded()
        }
    }

    private func textAfterWakeWord(_ text: String) -> String {
        guard mode == .wake else { return text }
        return WakeWordMatcher.textAfterWakeWord(in: text)
    }

    private func scheduleEndpoint(after delay: TimeInterval = 1.1) {
        endpointWork?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.finishRecognition() }
        endpointWork = work
        queue.asyncAfter(deadline: .now() + delay, execute: work)
    }

    private func scheduleRollover(epoch activeEpoch: Int) {
        rolloverWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.epoch == activeEpoch else { return }
            if self.mode == .wake && !self.wakeDetected {
                self.start(mode: .wake, captureID: UUID().uuidString, generation: self.generation + 1)
            } else {
                self.finishRecognition()
            }
        }
        rolloverWork = work
        queue.asyncAfter(deadline: .now() + 45, execute: work)
    }

    private func finishRecognition() {
        guard mode != nil, !finishing else { return }
        finishing = true
        endpointWork?.cancel()
        engine?.stop()
        input?.removeTap(onBus: 0)
        requestLock.lock()
        request?.endAudio()
        requestLock.unlock()
        let activeEpoch = epoch
        queue.asyncAfter(deadline: .now() + 2.5) { [weak self] in
            guard let self, self.epoch == activeEpoch, self.mode != nil else { return }
            self.finishWithText(self.textAfterWakeWord(self.latestText))
        }
    }

    private func finishWithText(_ text: String) {
        guard mode != nil else { return }
        let shouldEmit = mode == .pushToTalk || wakeDetected
        let finalCaptureID = captureID
        let finalGeneration = generation
        stop(cancelled: false)
        if shouldEmit {
            emit(["type": "asr.final", "capture_id": finalCaptureID, "generation": finalGeneration, "text": text])
        }
        restartWakeIfNeeded()
    }

    private func restartWakeIfNeeded() {
        if armed {
            queue.asyncAfter(deadline: .now() + 0.25) {
                if self.armed && self.mode == nil {
                    self.start(mode: .wake, captureID: UUID().uuidString, generation: self.generation + 1)
                }
            }
        }
    }

    private func stop(cancelled: Bool) {
        endpointWork?.cancel()
        rolloverWork?.cancel()
        engine?.stop()
        input?.removeTap(onBus: 0)
        requestLock.lock()
        request?.endAudio()
        request = nil
        requestLock.unlock()
        task?.cancel()
        task = nil
        engine = nil
        input = nil
        mode = nil
        finishing = false
        epoch += 1
        if cancelled { latestText = "" }
    }
}

private final class SpeechController: NSObject, @unchecked Sendable, AVSpeechSynthesizerDelegate {
    private let synthesizer = AVSpeechSynthesizer()
    private let voiceController: VoiceController
    private var turnID = ""
    private var generation = 0
    private var currentUtterance: AVSpeechUtterance?
    private var lastLevelEmittedAt = 0.0

    init(voiceController: VoiceController) {
        self.voiceController = voiceController
        super.init()
        synthesizer.delegate = self
    }

    func speak(turnID: String, generation: Int, text: String) {
        stop()
        self.turnID = turnID
        self.generation = generation
        lastLevelEmittedAt = 0
        voiceController.pauseForPlayback()
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = AVSpeechSynthesisVoice(language: "zh-CN")
        utterance.rate = 0.48
        currentUtterance = utterance
        synthesizer.speak(utterance)
    }

    func stop() {
        if synthesizer.isSpeaking { synthesizer.stopSpeaking(at: .immediate) }
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) {
        guard utterance === currentUtterance else { return }
        emit(["type": "tts.started", "turn_id": turnID, "generation": generation])
        emitLevel(0.12, for: utterance)
    }

    func speechSynthesizer(
        _ synthesizer: AVSpeechSynthesizer,
        willSpeakRangeOfSpeechString characterRange: NSRange,
        utterance: AVSpeechUtterance
    ) {
        guard utterance === currentUtterance else { return }
        let pulse = 0.48 + Double(NSMaxRange(characterRange) % 4) * 0.11
        emitLevel(pulse, for: utterance)
        let identity = ObjectIdentifier(utterance)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.09) { [weak self] in
            guard let self,
                  let current = self.currentUtterance,
                  ObjectIdentifier(current) == identity else { return }
            self.emitLevel(0.08, for: current)
        }
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        guard utterance === currentUtterance else { return }
        emitLevel(0, for: utterance, force: true)
        currentUtterance = nil
        emit(["type": "tts.finished", "turn_id": turnID, "generation": generation, "status": "completed"])
        voiceController.resumeAfterPlayback()
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        guard utterance === currentUtterance else { return }
        emitLevel(0, for: utterance, force: true)
        currentUtterance = nil
        emit(["type": "tts.finished", "turn_id": turnID, "generation": generation, "status": "cancelled"])
        voiceController.resumeAfterPlayback()
    }

    private func emitLevel(_ level: Double, for utterance: AVSpeechUtterance, force: Bool = false) {
        guard utterance === currentUtterance else { return }
        let now = ProcessInfo.processInfo.systemUptime
        guard force || now - lastLevelEmittedAt >= 0.05 else { return }
        lastLevelEmittedAt = now
        emit(["type": "tts.level", "turn_id": turnID, "generation": generation, "level": level])
    }
}

private func runWakeWordSelfTest() -> Never {
    let cases: [(Bool, String)] = [
        (WakeWordMatcher.match(in: "爱丽丝你好")?.phrase == "爱丽丝", "canonical phrase"),
        (WakeWordMatcher.match(in: "天童爱丽丝，打开浏览器")?.phrase == "天童爱丽丝", "full name"),
        (WakeWordMatcher.match(in: "艾丽丝提醒我开会")?.phrase == "艾丽丝", "homophone"),
        (WakeWordMatcher.match(in: "爱丽思，今天天气怎么样")?.phrase == "爱丽思", "recognition variant"),
        (WakeWordMatcher.textAfterWakeWord(in: "天童爱丽丝，打开浏览器") == "打开浏览器", "command extraction"),
        (WakeWordMatcher.textAfterWakeWord(in: "阿丽丝   设一个提醒") == "设一个提醒", "trimmed command extraction"),
        (WakeWordMatcher.match(in: "今天想聊聊") == nil, "non-wake speech"),
        (WakeWordMatcher.textAfterWakeWord(in: "今天想聊聊") == "今天想聊聊", "unchanged non-wake text")
    ]
    if let failure = cases.first(where: { !$0.0 }) {
        fail("wake_self_test_failed", "唤醒词自测失败：\(failure.1)")
    }
    print("WakeWordMatcher tests passed")
    exit(0)
}

if CommandLine.arguments.contains("--self-test-wake") {
    runWakeWordSelfTest()
}

guard let socketPath = argument(named: "--socket"),
      let tokenPath = argument(named: "--token-file"),
      let launchNonce = argument(named: "--launch-nonce") else {
    fail("invalid_arguments", "缺少 AudioHost 启动参数。")
}
guard let token = try? String(contentsOfFile: tokenPath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines),
      !token.isEmpty else {
    fail("invalid_token", "无法读取握手 token。")
}
try? FileManager.default.removeItem(atPath: tokenPath)
guard connectControlSocket(at: socketPath) else { exit(2) }

NSApplication.shared.setActivationPolicy(.accessory)
private let voiceController = VoiceController()
private let speechController = SpeechController(voiceController: voiceController)

emit([
    "type": "host.hello",
    "protocol": ["major": protocolMajor, "minor": 0],
    "launch_nonce": launchNonce,
    "token": token,
    "host_version": "0.1.0",
    "pid": ProcessInfo.processInfo.processIdentifier,
    "capabilities": ["apple_asr", "apple_tts", "apple_on_device_wake"]
])

DispatchQueue.global(qos: .userInitiated).async {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let message = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = message["type"] as? String else {
            emit(["type": "host.error", "code": "protocol_mismatch", "message": "无法解析控制消息。"])
            continue
        }
        switch type {
        case "host.ack":
            emitPermissionStatus()
            emit(["type": "host.ready"])
        case "core.ping":
            emit(["type": "host.pong", "seq": message["seq"] as? Int ?? 0])
        case "permissions.request":
            DispatchQueue.main.async { requestPermissions {} }
        case "permissions.status":
            emitPermissionStatus()
        case "settings.open_privacy":
            DispatchQueue.main.async { openPrivacySettings() }
        case "listening.set":
            voiceController.setArmed((message["enabled"] as? Bool) == true)
        case "asr.start":
            voiceController.startPushToTalk(
                captureID: message["capture_id"] as? String ?? UUID().uuidString,
                generation: message["generation"] as? Int ?? 0
            )
        case "asr.finalize": voiceController.finalize()
        case "asr.stop": voiceController.cancel()
        case "tts.speak":
            DispatchQueue.main.async {
                speechController.speak(
                    turnID: message["turn_id"] as? String ?? "",
                    generation: message["generation"] as? Int ?? 0,
                    text: message["text"] as? String ?? ""
                )
            }
        case "tts.stop": DispatchQueue.main.async { speechController.stop() }
        case "shutdown":
            voiceController.cancel()
            DispatchQueue.main.async {
                speechController.stop()
                emit(["type": "host.stopped"])
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { exit(0) }
            }
            return
        default:
            emit(["type": "host.error", "code": "unsupported_message", "message": "不支持的控制消息：\(type)"])
        }
    }
    voiceController.cancel()
    DispatchQueue.main.async {
        speechController.stop()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { exit(0) }
    }
}

RunLoop.main.run()
