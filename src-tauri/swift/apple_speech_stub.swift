import Foundation

// Stub implementation when the build SDK predates SpeechAnalyzer (macOS 26).
// The app still launches and reports the Apple Speech engine as unavailable.

public typealias ResponsePointer = UnsafeMutablePointer<AppleSpeechResponse>

@_cdecl("apple_speech_is_available")
public func appleSpeechIsAvailable() -> Int32 {
    return 0
}

@_cdecl("apple_speech_supported_languages")
public func appleSpeechSupportedLanguages() -> UnsafeMutablePointer<CChar>? {
    return strdup("[]")
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
) -> UnsafeMutablePointer<AppleSpeechResponse>? {
    let pointer = ResponsePointer.allocate(capacity: 1)
    pointer.pointee = AppleSpeechResponse(
        text: nil,
        success: 0,
        error_message: strdup("Apple Speech requires macOS 26 (SpeechAnalyzer)")
    )
    return pointer
}
