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

## P2 — transcribe-rs 0.2.5 → 0.3.x upgrade (unlocks Canary 1B v2 + Cohere Transcribe)

- **What:** Absorb the 0.3.0 breaking change (`SpeechModel` trait) to unlock Canary + Cohere engines via the existing `onnx` feature (same infra as Parakeet).
- **Why:** Cohere Transcribe (2B, Apache 2.0, 14 langs incl. FR, claimed 5.42% WER #1 leaderboard, shipped by Superwhisper for local dictation) = best downloadable FR quality. Canary 1B v2 (1B, 25 langs, beats Whisper-large-v3 EN at 10x speed, built-in AST could replace `translate_to_english`) = solid middle ground. One upgrade unlocks both for comparison.
- **Pros:** No new dependency class; real A/B/C against Parakeet v3; Cohere is prod-proven for dictation.
- **Cons:** Breaking trait migration across `managers/transcription.rs` + `managers/model.rs`; 2 GB (Cohere) / 1 GB (Canary) int8 downloads; InferenceBench (2026-10) couldn't validate Cohere's 5.42% claim on real audio — test before switching defaults.
- **Context:** transcribe-rs is cjpais's crate (Handy upstream author) — upgrading keeps Parler close to upstream. Canary AST = candidate to replace the whisper `translate_to_english` path.
- **Effort:** M human / S-M CC
- **Reactivation trigger:** after T1–T5 ship; pairs naturally with the Apple SpeechAnalyzer work (engine selector UI touched by both).

---

## Killed (explicitly out of scope, do not resurrect without new evidence)

- **Wayland uinput injection** — macOS user; Linux "for others" is out of personal scope (upstream Handy covers it).
- **SmartMic (phone-as-mic)** — niche use I don't have.
- **CLI config import for mass deployment** — enterprise feature; `export_settings`/`import_settings` commands already cover single-machine needs.
- **Parakeet boost tree** — existing fuzzy Levenshtein custom-words correction (`audio_toolkit/text.rs`, tested) does the job.
- **Porting Murmure's ONNX/ort engine** — our transcribe-rs multi-engine support (whisper/parakeet/moonshine/sense_voice) is architecturally superior.
