import AVFoundation
import Dispatch
import Foundation
import Speech

// MARK: - Apple SpeechAnalyzer bridge (macOS 26+)
//
// Compiles with a macOS 11 deployment target: every entry point is guarded by
// `@available(macOS 26.0, *)` and Speech is weakly linked, so older systems
// still launch the app and simply report the engine as unavailable.

public typealias ResponsePointer = UnsafeMutablePointer<AppleSpeechResponse>

private enum SpeechBridgeError: LocalizedError {
    case unsupportedOS
    case localeUnsupported(String)
    case audioBufferAllocation

    var errorDescription: String? {
        switch self {
        case .unsupportedOS:
            return "Apple Speech requires macOS 26 or newer"
        case .localeUnsupported(let identifier):
            return "SpeechTranscriber does not support locale '\(identifier)'"
        case .audioBufferAllocation:
            return "Failed to allocate an audio buffer for SpeechAnalyzer"
        }
    }
}

private func duplicateCString(_ text: String) -> UnsafeMutablePointer<CChar>? {
    text.withCString { strdup($0) }
}

private func makeResponse(text: String?, error: String?) -> ResponsePointer {
    let pointer = ResponsePointer.allocate(capacity: 1)
    pointer.pointee = AppleSpeechResponse(
        text: text.flatMap(duplicateCString),
        success: error == nil ? 1 : 0,
        error_message: error.flatMap(duplicateCString)
    )
    return pointer
}

@_cdecl("apple_speech_is_available")
public func appleSpeechIsAvailable() -> Int32 {
    guard #available(macOS 26.0, *) else { return 0 }
    // SpeechTranscriber.isAvailable reflects hardware support, not just OS.
    return SpeechTranscriber.isAvailable ? 1 : 0
}

@_cdecl("apple_speech_supported_languages")
public func appleSpeechSupportedLanguages() -> UnsafeMutablePointer<CChar>? {
    guard #available(macOS 26.0, *) else {
        return duplicateCString("[]")
    }
    // `supportedLocales` is async; bridge with a short timeout.
    final class Box: @unchecked Sendable {
        var value: [String] = []
    }
    let box = Box()
    let semaphore = DispatchSemaphore(value: 0)
    Task.detached {
        let locales = await SpeechTranscriber.supportedLocales
        box.value = locales.map { $0.identifier(.bcp47) }
        semaphore.signal()
    }
    guard semaphore.wait(timeout: .now() + 10) == .success else {
        return duplicateCString("[]")
    }
    let json = (try? JSONEncoder().encode(box.value))
        .flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    return duplicateCString(json)
}

@_cdecl("free_apple_speech_string")
public func appleSpeechFreeString(_ string: UnsafeMutablePointer<CChar>?) {
    guard let string else { return }
    free(string)
}

@_cdecl("free_apple_speech_response")
public func appleSpeechFreeResponse(_ response: ResponsePointer?) {
    guard let response else { return }
    if let text = response.pointee.text {
        free(text)
    }
    if let message = response.pointee.error_message {
        free(message)
    }
    response.deallocate()
}

@_cdecl("apple_speech_transcribe")
public func appleSpeechTranscribe(
    _ samples: UnsafePointer<Float>?,
    _ sampleCount: Int32,
    _ sampleRate: Int32,
    _ localeIdentifier: UnsafePointer<CChar>?
) -> ResponsePointer? {
    guard #available(macOS 26.0, *) else {
        return makeResponse(text: nil, error: SpeechBridgeError.unsupportedOS.errorDescription)
    }

    let locale = localeIdentifier.map { String(cString: $0) } ?? ""
    guard let samples, sampleCount > 0, sampleRate > 0 else {
        // Empty audio is not an error: nothing to transcribe.
        return makeResponse(text: "", error: nil)
    }

    let pcm = Array(UnsafeBufferPointer(start: samples, count: Int(sampleCount)))
    let rate = Double(sampleRate)

    // @_cdecl entry points are synchronous; bridge to the async Speech API
    // with a semaphore. The box is only touched across the semaphore
    // happens-before edge.
    final class ResultBox: @unchecked Sendable {
        var value: Result<String, Error>?
    }
    let box = ResultBox()
    let semaphore = DispatchSemaphore(value: 0)

    Task.detached(priority: .userInitiated) {
        do {
            box.value = .success(try await transcribePCM(pcm, sampleRate: rate, localeIdentifier: locale))
        } catch {
            box.value = .failure(error)
        }
        semaphore.signal()
    }

    // Generous ceiling: dictation is minutes, not tens of minutes, and the
    // caller runs on a blocking thread.
    if semaphore.wait(timeout: .now() + 300) == .timedOut {
        return makeResponse(text: nil, error: "Speech transcription timed out")
    }

    switch box.value {
    case .success(let text):
        return makeResponse(text: text, error: nil)
    case .failure(let error):
        return makeResponse(text: nil, error: error.localizedDescription)
    case .none:
        return makeResponse(text: nil, error: "Speech transcription produced no result")
    }
}

@available(macOS 26.0, *)
private func transcribePCM(
    _ samples: [Float],
    sampleRate: Double,
    localeIdentifier: String
) async throws -> String {
    // Fallback chain: requested locale -> system locale -> first supported.
    // The caller normally passes an explicit language; Locale.current is only
    // a last resort because a Tauri bundle without CFBundleLocalizations
    // reports the development region (English) rather than the user's system.
    let requestedLocale = localeIdentifier.isEmpty
        ? Locale.current
        : Locale(identifier: localeIdentifier)
    var locale = await SpeechTranscriber.supportedLocale(equivalentTo: requestedLocale)
    if locale == nil, localeIdentifier.isEmpty == false {
        locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current)
    }
    if locale == nil {
        locale = await SpeechTranscriber.supportedLocales.first
    }
    guard let locale else {
        throw SpeechBridgeError.localeUnsupported(localeIdentifier)
    }

    let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)

    // Install on-device assets if the system has none for this locale yet.
    if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
        try await request.downloadAndInstall()
    }

    // SpeechAnalyzer consumes files; write the PCM buffer to a temporary CAF.
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("parler-speech-\(UUID().uuidString).caf")
    defer { try? FileManager.default.removeItem(at: url) }

    guard let format = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: sampleRate,
        channels: 1,
        interleaved: false
    ) else {
        throw SpeechBridgeError.audioBufferAllocation
    }
    guard let buffer = AVAudioPCMBuffer(
        pcmFormat: format,
        frameCapacity: AVAudioFrameCount(samples.count)
    ) else {
        throw SpeechBridgeError.audioBufferAllocation
    }
    buffer.frameLength = AVAudioFrameCount(samples.count)
    samples.withUnsafeBufferPointer { source in
        guard let destination = buffer.floatChannelData?[0], let base = source.baseAddress else {
            return
        }
        destination.update(from: base, count: samples.count)
    }

    let audioFile = try AVAudioFile(
        forWriting: url,
        settings: format.settings,
        commonFormat: .pcmFormatFloat32,
        interleaved: false
    )
    try audioFile.write(from: buffer)

    let readFile = try AVAudioFile(forReading: url)
    let analyzer = SpeechAnalyzer(modules: [transcriber])

    let analysisTask = Task {
        if let lastSampleTime = try await analyzer.analyzeSequence(from: readFile) {
            try await analyzer.finalizeAndFinish(through: lastSampleTime)
        } else {
            try await analyzer.finalizeAndFinishThroughEndOfInput()
        }
    }

    var collected = ""
    for try await result in transcriber.results {
        collected += String(result.text.characters)
    }
    try await analysisTask.value

    return collected.trimmingCharacters(in: .whitespacesAndNewlines)
}
