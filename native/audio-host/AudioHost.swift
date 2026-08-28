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

private final class VoiceController {
    private enum Mode { case wake, pushToTalk }

    private let queue = DispatchQueue(label: "desktop-ai.audio-host.voice", qos: .userInteractive)
    private let requestLock = NSLock()
    private let locale = Locale(identifier: "zh-CN")
    private let wakeWord = "爱丽丝"
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
    func cancel() { queue.async { self.stop(cancelled: true) } }

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
        nextRequest.contextualStrings = [wakeWord, "天童爱丽丝"]
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
            if mode == .wake && !wakeDetected, latestText.localizedCaseInsensitiveContains(wakeWord) {
                wakeDetected = true
                emit(["type": "wake.detected", "capture_id": captureID, "generation": generation, "wake_word": wakeWord])
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
        guard mode == .wake, let range = text.range(of: wakeWord, options: [.caseInsensitive]) else { return text }
        return String(text[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters))
    }

    private func scheduleEndpoint() {
        endpointWork?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.finishRecognition() }
        endpointWork = work
        queue.asyncAfter(deadline: .now() + 1.1, execute: work)
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

    init(voiceController: VoiceController) {
        self.voiceController = voiceController
        super.init()
        synthesizer.delegate = self
    }

    func speak(turnID: String, generation: Int, text: String) {
        stop()
        self.turnID = turnID
        self.generation = generation
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
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        guard utterance === currentUtterance else { return }
        currentUtterance = nil
        emit(["type": "tts.finished", "turn_id": turnID, "generation": generation, "status": "completed"])
        voiceController.resumeAfterPlayback()
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        guard utterance === currentUtterance else { return }
        currentUtterance = nil
        emit(["type": "tts.finished", "turn_id": turnID, "generation": generation, "status": "cancelled"])
        voiceController.resumeAfterPlayback()
    }
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
        case "permissions.request":
            DispatchQueue.main.async { requestPermissions {} }
        case "permissions.status":
            emitPermissionStatus()
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
