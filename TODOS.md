# TODOS

Deferred work from /plan-ceo-review (2026-10-23, SCOPE REDUCTION mode — Parler = personal tool first).
Each item has an explicit reactivation trigger. Do not build speculatively.

## P3 — Local HTTP API

- **What:** Localhost HTTP server (axum) to trigger dictation and return transcribed text in the response (Raycast, scripts, external tools).
- **Why:** Murmure has one (`http_api/`, ~25KB incl. prompt_mode). Our SIGUSR1/2 signals + single-instance CLI already cover basic trigger/cancel.
- **Pros:** Rich integrations; text returned over HTTP; prompt_mode enables voice-driven automation.
- **Cons:** New localhost attack surface (needs token auth); no personal automation use case identified yet.
- **Context:** Start from Murmure's `src-tauri/src/http_api/` for shape. Existing signals live in `signal_handle.rs`, CLI in `cli.rs`.
- **Effort:** L human / M CC
- **Reactivation trigger:** I want to trigger dictation or get transcribed text from an external tool (Raycast, Stream Deck, script).

## P3 — Wake word

- **What:** Hands-free recording trigger via keyword detection ("ok Parler").
- **Why:** Murmure ships a 42KB module (`wake_word/`). Real comfort only when dictating away from the keyboard.
- **Pros:** True hands-free experience.
- **Cons:** Heavy (always-on detection model, CPU, false triggers); global shortcut + PTT already cover keyboard-centric use.
- **Context:** Murmure's `src-tauri/src/wake_word/wake_word.rs` is the reference implementation.
- **Effort:** XL human / L CC
- **Reactivation trigger:** I regularly dictate with hands off the keyboard (cooking, injury, walking).

## P3 — Docs site + distribution channels

- **What:** mkdocs site, winget manifest, install.sh — Murmure-style public credibility.
- **Why:** Parler already ships signed public releases; discoverability/installability is the next adoption lever.
- **Pros:** Adoption, credibility, one-command Windows install.
- **Cons:** Does not serve personal use; ongoing maintenance.
- **Context:** Murmure's `docs/` (mkdocs.yml), `install.sh`, `packaging/` as references. Release workflow already exists in `.github/workflows/`.
- **Effort:** M human / S CC
- **Reactivation trigger:** I decide to push Parler adoption publicly.

## P3 — Long-audio chunking

- **What:** Smart chunking of long recordings with sentence-boundary handling (port of Murmure's `audio/chunking.rs`, 23KB).
- **Why:** Current setup uses `long_audio_model_setting` + threshold but no dedicated chunker; boundary artifacts may appear on long dictations.
- **Pros:** Better quality on long recordings (meetings, long notes).
- **Cons:** Zero gain for short dictations (<1-2 min); non-trivial port.
- **Context:** Murmure `src-tauri/src/audio/chunking.rs`. Our pipeline: `managers/transcription.rs` + `audio_toolkit/`.
- **Effort:** L human / M CC
- **Reactivation trigger:** I observe boundary artifacts or quality drops on long dictations.

## P2 — Apple SpeechAnalyzer engine (macOS 26+)

- **What:** New engine using macOS 26 `SpeechAnalyzer`/`SpeechTranscriber` (+`DictationTranscriber` fallback) via a Swift/objc2 bridge. Pattern precedent: `apple_intelligence.rs`.
- **Why:** Best personal engine for macOS+French use: 0 MB download (system-integrated), near-instant on ANE, native French, Apple-maintained, fully private. Evaluated 2026-10-25 vs Canary/Cohere — wins for personal macOS dictation.
- **Pros:** No model download, instant, battery-friendly, partial/volatile results possible, zero maintenance burden on model updates.
- **Cons:** macOS 26+ only (multi-engine stays for Win/Linux); Swift bridge + AssetInventory handling; locale support must be verified against `SpeechTranscriber.supportedLocales` (FR expected, confirm at impl time); language auto-detect TBD (DictationTranscriber covers keyboard-dictation languages).
- **Context:** WWDC25 session 277. `LoadedEngine` enum in `managers/transcription.rs` gains a variant. Check language auto-detection: SpeechTranscriber needs a locale; DictationTranscriber may detect. Engine-agnostic features (text2num, keyring) already designed to work with it.
- **Effort:** L human / M CC
- **Reactivation trigger:** after T1–T5 (text2num + keyring) ship.

## P3 — Canary 1B v2 engine (blocked on packaging)

- **What:** Add NVIDIA Canary 1B v2 as an engine (transcribe-rs 0.3 support already present).
- **Why blocked:** `CanaryModel::load` requires `nemo128.onnx` (FP32 preprocessor), which is absent from every public ONNX export checked (istupakov/canary-1b-v2-onnx, canary-1b-flash-onnx, canary-180m-flash-onnx). Producing it requires exporting the NeMo preprocessor (nemo_toolkit) ourselves.
- **Pros once unblocked:** 25 languages, beats Whisper-large-v3 on English at 10x speed, built-in AST (`target_language`) could replace the whisper-only `translate_to_english`.
- **Context:** transcribe-rs 0.3.12 is already wired (`EngineType::Cohere` path is the template). Add `EngineType::Canary` + a model entry with `hf_repo`, mirroring Cohere. Files needed in the model dir: `nemo128.onnx`, `encoder-model.int8.onnx`, `decoder-model.int8.onnx`, `vocab.txt`.
- **Effort:** M human / S CC (once nemo128.onnx is produced)
- **Reactivation trigger:** a complete ONNX export with the preprocessor appears, or someone exports nemo128.onnx.

## P3 — ONNX Runtime CoreML acceleration

- **What:** Enable `ort-coreml` (`transcribe-rs` feature) so Parakeet/Cohere/Moonshine/SenseVoice run on the Neural Engine/GPU instead of CPU.
- **Why:** Cohere int4 on CPU already runs ~8s for a short phrase; CoreML EP could cut that substantially and reduce battery cost.
- **Pros:** Major speedup on Apple Silicon for all ONNX engines.
- **Cons:** Build-time risk (ORT binaries with CoreML), possible EP fallbacks/instability; needs A/B testing per engine.
- **Context:** `transcribe-rs` features `ort-coreml` / `ort-cuda` / `ort-directml` exist. Enable per-target in Cargo.toml.
- **Effort:** S-M human / S CC
- **Reactivation trigger:** Cohere (or Parakeet) CPU latency becomes annoying in daily use.

---

## Killed (explicitly out of scope, do not resurrect without new evidence)

- **Wayland uinput injection** — macOS user; Linux "for others" is out of personal scope (upstream Handy covers it).
- **SmartMic (phone-as-mic)** — niche use I don't have.
- **CLI config import for mass deployment** — enterprise feature; `export_settings`/`import_settings` commands already cover single-machine needs.
- **Parakeet boost tree** — existing fuzzy Levenshtein custom-words correction (`audio_toolkit/text.rs`, tested) does the job.
- **Porting Murmure's ONNX/ort engine** — our transcribe-rs multi-engine support (whisper/parakeet/moonshine/sense_voice) is architecturally superior.
