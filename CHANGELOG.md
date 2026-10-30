# Changelog

## [0.7.23] - 2026-10-29

### Added

- **Apple Speech Analyzer engine** (macOS 26+, Apple Silicon): fully on-device system model, zero download, near-instant, French and many languages
- **Cohere Transcribe engine**: top multilingual accuracy (14 languages incl. French), downloaded file-by-file from Hugging Face
- **DeepSeek** and **OpenCode Go** post-processing providers
- Editable **Base URL** for the Custom LLM provider
- **Number-to-digits conversion** (French/English, opt-in toggle). Verified threshold keeps articles: "un café" stays untouched
- **API keys stored in the OS keychain** (macOS Keychain / Windows Credential Manager / Secret Service), with a visible fallback indicator when unavailable

### Changed

- **transcribe-rs 0.2 → 0.3**: new speech-model API, per-target GPU features (Metal on macOS, Vulkan on Windows/Linux)
- Tray icons now use Parler branding instead of the inherited Handy artwork
- Linux builds are produced on Ubuntu 24.04: the ONNX Runtime prebuilt requires **glibc 2.39+** (Ubuntu 24.04 / Debian 13 or newer)
- macOS 11.0 is now the minimum (whisper.cpp's use of std::filesystem requires 10.15+)
- **macOS Intel builds are discontinued**: ONNX Runtime no longer ships prebuilt binaries for x86_64 macOS

### Fixed

- Parakeet v3 and other int8 ONNX models failed to load after the transcribe-rs upgrade (quantization is now detected from the model directory)
- Apple Speech transcribed with an English decoder in "auto" mode: the Tauri bundle reports English as development region, so the app language is now passed explicitly
- Long-audio model switch is skipped for engines that handle long recordings natively (Apple Speech, Parakeet, Cohere)
- Pre-existing `llm_client` test build failures

## [0.3.0] - 2025-07-11

### Added

- **Translate to English** setting: Added automatic translation of speech to English
- Settings refactored into React hooks for better state management
- Audio device switching capability
- Hysteresis to VAD (Voice Activity Detection) for more stable recording

### Changed

- Major audio backend refactor for improved performance and reliability
- Moved audio toolkit into src-tauri directory for better permissions handling
- Model files no longer need to be downloaded separately for releases
- Updated settings components and transcription logic

### Fixed

- Audio toolkit permissions issues
- Various stability improvements

## [0.2.3] - 2025-07-03

### Fixed

- Keycode bug that was causing input issues
- Whisper model optimization: switched to unquantized Whisper Turbo, updated Whisper Medium quantization to 4_1

## [0.2.2] - 2025-07-02

### Fixed

- Removed 50ms delay feature flag for Windows (now applies to all platforms for consistency)

## [0.2.1] - 2025-07-01

### Added

- Ctrl+Space key binding for Windows platform

### Fixed

- Windows crash issue
- Model loading on startup when available
- Windows paste functionality bug

## [0.2.0] - 2025-06-30

### Added

- **Microphone activation on demand**: More efficient resource usage
- Less permissive VAD settings for better accuracy

### Changed

- Improved microphone management and activation system

## [0.1.6] - 2025-06-30

### Added

- **Multiple models support**: Users can now select from different transcription models
- Model selection onboarding flow
- Cleanup and refactoring of model management

### Changed

- Enhanced user experience with model selection interface
- Better language and UI tweaks

## [0.1.5] - 2025-06-27

### Added

- **Different start and stop recording sounds**: Enhanced audio feedback
- Recording sound samples for better user experience

## [0.1.4] - 2025-06-27

### Fixed

- Build issues
- Auto-update functionality improvements

## [0.1.3] - 2025-06-26

### Fixed

- Paste functionality using enigo library for better cross-platform compatibility

## [0.1.2] - 2025-06-26

### Added

- **Auto-update functionality**: Application can now automatically update itself
- Footer displaying current version
- Improved menu system

### Changed

- Better user interface for version management
- Enhanced update workflow

## [0.1.1] - 2025-06-25

### Added

- **Comprehensive build system**: Support for Windows, macOS, and Linux
- Windows code signing for trusted installation
- Ubuntu/Linux build support with Vulkan
- Model file download and packaging for releases
- GitHub Actions CI/CD workflow

### Changed

- Improved build process and release workflow
- Better cross-platform compatibility

### Fixed

- Various build-related issues across platforms

## [0.1.0] - 2025-05-16

### Added

- **Initial release** of Handy
- Basic speech-to-text transcription functionality
- Voice Activity Detection (VAD) for automatic recording
- Cross-platform support (macOS, Windows, Linux)
- **Tauri-based desktop application** with React frontend
- **Global keyboard shortcuts** for activation
- **Clipboard integration** for automatic text insertion
- **LLM integration** for enhanced transcription processing
- **Configurable settings** including:
  - Custom key bindings
  - Audio device selection
  - Microphone settings
  - Push-to-talk functionality
- **System tray integration** with recording indicators
- **Accessibility permissions** handling for macOS
- **Settings persistence** with unified settings store
- **Background operation** capability
- **Multiple audio format support** with on-the-fly resampling
- **Whisper model integration** for high-quality transcription
- **MIT License** for open-source distribution

### Technical Implementation

- Built with Tauri (Rust backend) and React (TypeScript frontend)
- Audio processing with cpal and whisper-rs
- Real-time transcription with performance optimizations
- Cross-platform keyboard event handling
- Modular architecture with managers for audio, models, and transcription
