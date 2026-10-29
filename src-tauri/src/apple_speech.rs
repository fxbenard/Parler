//! Apple SpeechAnalyzer engine (macOS 26+, Apple Silicon).
//!
//! Fully on-device transcription through the system Speech framework: no
//! model download, near-instant on the Neural Engine, assets managed by the
//! OS. The Swift bridge lives in `swift/apple_speech.swift`; older SDKs link
//! a stub that reports the engine as unavailable.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

#[repr(C)]
pub struct AppleSpeechResponse {
    pub text: *mut c_char,
    pub success: c_int,
    pub error_message: *mut c_char,
}

extern "C" {
    fn apple_speech_is_available() -> c_int;
    fn apple_speech_supported_languages() -> *mut c_char;
    fn apple_speech_transcribe(
        samples: *const f32,
        sample_count: i32,
        sample_rate: i32,
        locale_identifier: *const c_char,
    ) -> *mut AppleSpeechResponse;
    fn free_apple_speech_response(response: *mut AppleSpeechResponse);
    fn free_apple_speech_string(string: *mut c_char);
}

/// True when the OS, SDK and hardware support SpeechAnalyzer.
pub fn is_available() -> bool {
    unsafe { apple_speech_is_available() == 1 }
}

/// BCP-47 identifiers of the languages SpeechTranscriber can handle on this
/// machine. Empty when unavailable.
pub fn supported_languages() -> Vec<String> {
    let raw = unsafe { apple_speech_supported_languages() };
    if raw.is_null() {
        return Vec::new();
    }
    let json = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { free_apple_speech_string(raw) };
    serde_json::from_str(&json).unwrap_or_default()
}

/// Transcribes mono f32 PCM at `sample_rate`. Blocks until done.
/// An empty sample slice returns an empty transcript.
pub fn transcribe(
    samples: &[f32],
    sample_rate: u32,
    locale: &str,
) -> Result<String, String> {
    let locale_c =
        CString::new(locale).map_err(|e| format!("Invalid locale string: {e}"))?;

    let response_ptr = unsafe {
        apple_speech_transcribe(
            samples.as_ptr(),
            samples.len() as i32,
            sample_rate as i32,
            locale_c.as_ptr(),
        )
    };
    if response_ptr.is_null() {
        return Err("Apple Speech returned a null response".to_string());
    }

    let response = unsafe { &*response_ptr };
    let result = if response.success == 1 {
        if response.text.is_null() {
            Ok(String::new())
        } else {
            Ok(unsafe { CStr::from_ptr(response.text) }
                .to_string_lossy()
                .into_owned())
        }
    } else if response.error_message.is_null() {
        Err("Apple Speech failed without an error message".to_string())
    } else {
        Err(unsafe { CStr::from_ptr(response.error_message) }
            .to_string_lossy()
            .into_owned())
    };

    unsafe { free_apple_speech_response(response_ptr) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_probe_does_not_crash() {
        // On CI/older macOS this is false; on the developer's machine true.
        let available = is_available();
        println!("Apple Speech available: {}", available);
    }

    #[test]
    fn empty_audio_returns_empty_text() {
        if !is_available() {
            return;
        }
        let text = transcribe(&[], 16_000, "fr").expect("empty audio must succeed");
        assert_eq!(text, "");
    }

    /// Manual smoke test: set APPLE_SPEECH_WAV to a mono 16 kHz WAV file.
    /// Skipped when the variable is unset so CI never needs audio fixtures.
    #[test]
    fn real_transcription_from_wav_env() {
        let Ok(path) = std::env::var("APPLE_SPEECH_WAV") else {
            return;
        };
        if !is_available() {
            return;
        }
        let mut reader = hound::WavReader::open(&path).expect("failed to open wav");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "fixture must be mono");
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.expect("wav sample") as f32 / i16::MAX as f32)
            .collect();
        let text = transcribe(&samples, spec.sample_rate, "fr").expect("transcription failed");
        println!("Apple Speech transcript: {text}");
        assert!(!text.trim().is_empty(), "transcript must not be empty");
    }
}
