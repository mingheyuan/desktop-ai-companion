import CoreGraphics
import Darwin
import Foundation
import ImageIO
import Vision

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

guard let inputIndex = CommandLine.arguments.firstIndex(of: "--input"),
      inputIndex + 1 < CommandLine.arguments.count else {
    fail("missing --input")
}

let inputURL = URL(fileURLWithPath: CommandLine.arguments[inputIndex + 1])
guard let imageSource = CGImageSourceCreateWithURL(inputURL as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil) else {
    fail("unable to decode input image")
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
request.recognitionLanguages = ["zh-Hans", "en-US", "ja-JP"]

do {
    let handler = VNImageRequestHandler(cgImage: image, options: [:])
    try handler.perform([request])
} catch {
    fail("vision request failed: \(error.localizedDescription)")
}

let lines = (request.results ?? []).compactMap { observation in
    observation.topCandidates(1).first?.string.trimmingCharacters(in: .whitespacesAndNewlines)
}.filter { !$0.isEmpty }
let payload: [String: Any] = ["text": lines.joined(separator: "\n")]
guard let data = try? JSONSerialization.data(withJSONObject: payload) else {
    fail("unable to encode OCR result")
}
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write(Data([0x0A]))
