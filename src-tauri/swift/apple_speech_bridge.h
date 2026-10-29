#ifndef APPLE_SPEECH_BRIDGE_H
#define APPLE_SPEECH_BRIDGE_H

#include <stdint.h>

typedef struct {
    char *text;
    int32_t success;
    char *error_message;
} AppleSpeechResponse;

/// 1 when SpeechAnalyzer/SpeechTranscriber are usable on this machine
/// (macOS 26+ on supported hardware), 0 otherwise.
int32_t apple_speech_is_available(void);

/// JSON array of BCP-47 language identifiers supported by SpeechTranscriber.
/// Caller frees with free_apple_speech_string.
char *apple_speech_supported_languages(void);

/// Transcribes mono float32 PCM samples. Blocks until finished.
/// Returns NULL only on allocation failure; check `success` on the response.
/// Caller frees with free_apple_speech_response.
AppleSpeechResponse *apple_speech_transcribe(const float *samples,
                                             int32_t sample_count,
                                             int32_t sample_rate,
                                             const char *locale_identifier);

void free_apple_speech_response(AppleSpeechResponse *response);
void free_apple_speech_string(char *string);

#endif
