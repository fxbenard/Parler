# TODOS

Backlog for Parler (personal tool first). Each item has an explicit reactivation trigger.
Do not build speculatively.

Shipped since the first review: number conversion, OS keychain for API keys, Apple Speech,
transcribe-rs 0.3 + Cohere + Canary, Parler tray icons, DeepSeek/OpenCode Go providers,
Whisper initial_prompt, accessibility recovery. See CHANGELOG for details.

## P2 — Whisper hallucination filter (long repeated words)

- **What:** Drop runs of the same word repeated many times (e.g. "oui oui oui oui…") from transcripts.
- **Why:** Upstream fix `075fb5f7`; Whisper occasionally loops on silence/noise.
- **Pros:** Cheap, removes an obvious failure mode.
- **Cons:** Must not touch legitimate emphasis ("très très bien").
- **Context:** Best placed next to `filter_transcription_output` in `audio_toolkit/text.rs`, after stutter collapsing.
- **Effort:** S human / XS CC
- **Reactivation trigger:** A dictation pastes a repeated-word loop.

## P2 — Checksum verification for model downloads

- **What:** Verify sha256 of downloaded models (tarballs and Hugging Face files) before extraction.
- **Why:** Upstream `58cda3f3` prevents a corrupt partial download from looping forever. Our HF downloader checks size only.
- **Pros:** Integrity + clearer failure; avoids re-downloading garbage.
- **Cons:** Needs per-model hashes maintained (HF API exposes sha256 for LFS files).
- **Context:** `managers/model.rs` — `download_model` (tarball path) and `download_huggingface_model` (HF path).
- **Effort:** S human / S CC
- **Reactivation trigger:** A model fails to load after a download.

## P2 — Overlay follows the focused display

- **What:** Show the recording overlay on the display that currently has focus, not the primary one.
- **Why:** Upstream `aebd4323` + `53b4ad50`; multi-monitor users never see the overlay otherwise.
- **Pros:** Correct behaviour on multi-display setups.
- **Cons:** Needs focus-change detection.
- **Context:** `overlay.rs` + `utils::update_overlay_position`.
- **Effort:** S-M human / S CC
- **Reactivation trigger:** I use Parler on a second display.

## P3 — Local HTTP API

- **What:** Localhost HTTP server (axum) to trigger dictation and return transcribed text (Raycast, scripts).
- **Why:** Murmure has one (`http_api/`). Signals + CLI cover trigger/cancel today.
- **Pros:** Rich integrations, text returned over HTTP.
- **Cons:** New localhost attack surface (needs token auth); no personal use case yet.
- **Context:** Start from Murmure `src-tauri/src/http_api/`. Signals in `signal_handle.rs`, CLI in `cli.rs`.
- **Effort:** L human / M CC
- **Reactivation trigger:** I want external tools to trigger dictation or receive the text.

## P3 — Wake word

- **What:** Hands-free recording via keyword ("ok Parler").
- **Why:** Comfort when dictating away from the keyboard.
- **Pros:** True hands-free.
- **Cons:** Always-on model, CPU, false triggers; shortcut + PTT already cover keyboard use.
- **Context:** Murmure `src-tauri/src/wake_word/wake_word.rs`.
- **Effort:** XL human / L CC
- **Reactivation trigger:** I regularly dictate with hands off the keyboard.

## P3 — ONNX Runtime CoreML acceleration

- **What:** Enable `ort-coreml` so Parakeet/Cohere/Moonshine/SenseVoice run on ANE/GPU.
- **Why:** Cohere int4 takes ~8 s on CPU per short phrase; CoreML could cut that and battery cost.
- **Pros:** Major speedup on Apple Silicon for all ONNX engines.
- **Cons:** Build-time risk (ORT CoreML binaries), per-engine A/B testing needed.
- **Context:** `transcribe-rs` feature `ort-coreml`; enable per-target in `Cargo.toml`.
- **Effort:** S-M human / S CC
- **Reactivation trigger:** Cohere/Parakeet CPU latency becomes annoying in daily use.

## P3 — Intel macOS builds

- **What:** Restore `x86_64-apple-darwin` release artifacts.
- **Why:** ONNX Runtime ≥ 2.0.0-rc.12 ships no prebuilt binary for Intel macOS; the job was dropped in 0.7.23.
- **Options:** compile ORT from source for x86_64, or cfg-gate ONNX engines off on Intel (Whisper-only build).
- **Effort:** L human / M CC
- **Reactivation trigger:** An Intel Mac user asks for a build (or I need one).

## P3 — Docs site + distribution channels

- **What:** mkdocs site, winget manifest, install.sh.
- **Why:** Public discoverability/installability.
- **Pros:** Adoption, credibility, one-command Windows install.
- **Cons:** Does not serve personal use; ongoing maintenance.
- **Context:** Murmure's `docs/`, `install.sh`, `packaging/`. Release workflow exists.
- **Effort:** M human / S CC
- **Reactivation trigger:** I decide to push adoption publicly.

## P3 — Long-audio chunking

- **What:** Sentence-boundary chunking for long recordings (port of Murmure `audio/chunking.rs`).
- **Why:** Native engines handle long audio, but very long recordings may still hit model limits.
- **Pros:** Better long-form quality.
- **Cons:** Zero gain for short dictations; non-trivial port.
- **Context:** `managers/transcription.rs` + `audio_toolkit/`.
- **Effort:** L human / M CC
- **Reactivation trigger:** Long dictations show boundary artifacts.

---

## Killed (do not resurrect without new evidence)

- **Wayland uinput injection** — macOS user; Linux "for others" out of personal scope.
- **SmartMic (phone-as-mic)** — niche use not needed.
- **CLI config mass-import** — `export_settings`/`import_settings` cover single-machine needs.
- **Parakeet boost tree** — fuzzy correction + Whisper initial_prompt cover custom words.
- **Porting Murmure's ort/ONNX engine** — transcribe-rs multi-engine is superior.
- **Merging Melvynx/Parler upstream** — post-processing/settings rework conflicts with keyring + text2num; cherry-pick surgical fixes instead.
