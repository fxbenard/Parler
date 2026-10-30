use crate::audio_toolkit::{
    finalize_transcription, ConversionHint, ConversionLanguage,
};
use crate::managers::model::{EngineType, ModelManager};
use crate::settings::{get_settings, AppSettings, ModelUnloadTimeout};
use anyhow::Result;
use log::{debug, error, info, warn};
use serde::Serialize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter};
use transcribe_rs::{
    onnx::{
        canary::{CanaryModel, CanaryParams},
        cohere::{CohereModel, CohereParams},
        moonshine::{
            MoonshineModel, MoonshineParams, MoonshineStreamingParams, MoonshineVariant,
            StreamingModel,
        },
        parakeet::{ParakeetModel, ParakeetParams, TimestampGranularity},
        sense_voice::{SenseVoiceModel, SenseVoiceParams},
        Quantization,
    },
    whisper_cpp::{WhisperEngine, WhisperInferenceParams},
};

/// Picks the ONNX quantization from the model's directory name (the model id
/// and the filename can differ, e.g. id `parakeet-tdt-0.6b-v3` vs directory
/// `parakeet-tdt-0.6b-v3-int8`).
///
/// Int8 is the safe default: `resolve_model_path` falls back to the
/// unquantized file when no int8 variant exists, but it never upgrades a
/// plain request to a quantized file.
fn quantization_for(model_filename: &str) -> Quantization {
    if model_filename.contains("int4") {
        Quantization::Int4
    } else if model_filename.contains("fp16") {
        Quantization::FP16
    } else {
        Quantization::Int8
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelStateEvent {
    pub event_type: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
}

/// Maps the pinned transcription language to a conversion hint.
/// Returns `None` for "auto" (caller falls back to on-text detection).
/// Only French and English are convertible today — everything else skips.
fn map_selected_language(lang: &str) -> Option<ConversionHint> {
    match lang {
        "auto" => None,
        l if l.starts_with("fr") => Some(ConversionHint::Known(ConversionLanguage::French)),
        l if l.starts_with("en") => Some(ConversionHint::Known(ConversionLanguage::English)),
        _ => Some(ConversionHint::Skip),
    }
}

/// Conversion hint from explicit settings signals only (no engine knowledge).
/// Used by Gemini and SenseVoice paths. `translate_to_english` is deliberately
/// NOT handled here — it only applies to the Whisper path.
fn conversion_hint_from_settings(settings: &AppSettings) -> ConversionHint {
    map_selected_language(&settings.selected_language).unwrap_or(ConversionHint::Detect)
}

enum LoadedEngine {
    Whisper(WhisperEngine),
    Parakeet(ParakeetModel),
    Moonshine(MoonshineModel),
    MoonshineStreaming(StreamingModel),
    SenseVoice(SenseVoiceModel),
    Cohere(CohereModel),
    Canary(CanaryModel),
    GeminiApi,
    AppleSpeech,
}

#[derive(Clone)]
pub struct TranscriptionManager {
    engine: Arc<Mutex<Option<LoadedEngine>>>,
    model_manager: Arc<ModelManager>,
    app_handle: AppHandle,
    current_model_id: Arc<Mutex<Option<String>>>,
    last_activity: Arc<AtomicU64>,
    shutdown_signal: Arc<AtomicBool>,
    watcher_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
}

impl TranscriptionManager {
    pub fn new(app_handle: &AppHandle, model_manager: Arc<ModelManager>) -> Result<Self> {
        let manager = Self {
            engine: Arc::new(Mutex::new(None)),
            model_manager,
            app_handle: app_handle.clone(),
            current_model_id: Arc::new(Mutex::new(None)),
            last_activity: Arc::new(AtomicU64::new(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            )),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            watcher_handle: Arc::new(Mutex::new(None)),
            is_loading: Arc::new(Mutex::new(false)),
            loading_condvar: Arc::new(Condvar::new()),
        };

        // Start the idle watcher
        {
            let app_handle_cloned = app_handle.clone();
            let manager_cloned = manager.clone();
            let shutdown_signal = manager.shutdown_signal.clone();
            let handle = thread::spawn(move || {
                while !shutdown_signal.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_secs(10)); // Check every 10 seconds

                    // Check shutdown signal again after sleep
                    if shutdown_signal.load(Ordering::Relaxed) {
                        break;
                    }

                    let settings = get_settings(&app_handle_cloned);
                    let timeout_seconds = settings.model_unload_timeout.to_seconds();

                    if let Some(limit_seconds) = timeout_seconds {
                        // Skip polling-based unloading for immediate timeout since it's handled directly in transcribe()
                        if settings.model_unload_timeout == ModelUnloadTimeout::Immediately {
                            continue;
                        }

                        let last = manager_cloned.last_activity.load(Ordering::Relaxed);
                        let now_ms = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;

                        if now_ms.saturating_sub(last) > limit_seconds * 1000 {
                            // idle -> unload
                            if manager_cloned.is_model_loaded() {
                                let unload_start = std::time::Instant::now();
                                debug!("Starting to unload model due to inactivity");

                                if let Ok(()) = manager_cloned.unload_model() {
                                    let _ = app_handle_cloned.emit(
                                        "model-state-changed",
                                        ModelStateEvent {
                                            event_type: "unloaded".to_string(),
                                            model_id: None,
                                            model_name: None,
                                            error: None,
                                        },
                                    );
                                    let unload_duration = unload_start.elapsed();
                                    debug!(
                                        "Model unloaded due to inactivity (took {}ms)",
                                        unload_duration.as_millis()
                                    );
                                }
                            }
                        }
                    }
                }
                debug!("Idle watcher thread shutting down gracefully");
            });
            *manager.watcher_handle.lock().unwrap() = Some(handle);
        }

        Ok(manager)
    }

    /// Lock the engine mutex, recovering from poison if a previous transcription panicked.
    fn lock_engine(&self) -> MutexGuard<'_, Option<LoadedEngine>> {
        self.engine.lock().unwrap_or_else(|poisoned| {
            warn!("Engine mutex was poisoned by a previous panic, recovering");
            poisoned.into_inner()
        })
    }

    pub fn is_model_loaded(&self) -> bool {
        let engine = self.lock_engine();
        engine.is_some()
    }

    pub fn unload_model(&self) -> Result<()> {
        let unload_start = std::time::Instant::now();
        debug!("Starting to unload model");

        {
            // 0.3 models have no unload() method: dropping the engine frees
            // the ONNX sessions / whisper context.
            let mut engine = self.lock_engine();
            *engine = None;
        }
        {
            let mut current_model = self.current_model_id.lock().unwrap();
            *current_model = None;
        }

        // Emit unloaded event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "unloaded".to_string(),
                model_id: None,
                model_name: None,
                error: None,
            },
        );

        let unload_duration = unload_start.elapsed();
        debug!(
            "Model unloaded manually (took {}ms)",
            unload_duration.as_millis()
        );
        Ok(())
    }

    /// Unloads the model immediately if the setting is enabled and the model is loaded
    pub fn maybe_unload_immediately(&self, context: &str) {
        let settings = get_settings(&self.app_handle);
        if settings.model_unload_timeout == ModelUnloadTimeout::Immediately
            && self.is_model_loaded()
        {
            info!("Immediately unloading model after {}", context);
            if let Err(e) = self.unload_model() {
                warn!("Failed to immediately unload model: {}", e);
            }
        }
    }

    pub fn load_model(&self, model_id: &str) -> Result<()> {
        let load_start = std::time::Instant::now();
        debug!("Starting to load model: {}", model_id);

        // Emit loading started event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_started".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: None,
                error: None,
            },
        );

        let model_info = self
            .model_manager
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        if !model_info.is_downloaded {
            let error_msg = "Model not downloaded";
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_failed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: Some(error_msg.to_string()),
                },
            );
            return Err(anyhow::anyhow!(error_msg));
        }

        let model_path = if matches!(
            model_info.engine_type,
            EngineType::GeminiApi | EngineType::AppleSpeech
        ) {
            std::path::PathBuf::new()
        } else {
            self.model_manager.get_model_path(model_id)?
        };

        // Create appropriate engine based on model type
        let loaded_engine = match model_info.engine_type {
            EngineType::Whisper => {
                let engine = WhisperEngine::load(&model_path).map_err(|e| {
                    let error_msg = format!("Failed to load whisper model {}: {}", model_id, e);
                    let _ = self.app_handle.emit(
                        "model-state-changed",
                        ModelStateEvent {
                            event_type: "loading_failed".to_string(),
                            model_id: Some(model_id.to_string()),
                            model_name: Some(model_info.name.clone()),
                            error: Some(error_msg.clone()),
                        },
                    );
                    anyhow::anyhow!(error_msg)
                })?;
                LoadedEngine::Whisper(engine)
            }
            EngineType::Parakeet => {
                let engine = ParakeetModel::load(&model_path, &quantization_for(&model_info.filename))
                    .map_err(|e| {
                        let error_msg =
                            format!("Failed to load parakeet model {}: {}", model_id, e);
                        let _ = self.app_handle.emit(
                            "model-state-changed",
                            ModelStateEvent {
                                event_type: "loading_failed".to_string(),
                                model_id: Some(model_id.to_string()),
                                model_name: Some(model_info.name.clone()),
                                error: Some(error_msg.clone()),
                            },
                        );
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::Parakeet(engine)
            }
            EngineType::Moonshine => {
                let engine = MoonshineModel::load(
                    &model_path,
                    MoonshineVariant::Base,
                    &quantization_for(&model_info.filename),
                )
                .map_err(|e| {
                        let error_msg =
                            format!("Failed to load moonshine model {}: {}", model_id, e);
                        let _ = self.app_handle.emit(
                            "model-state-changed",
                            ModelStateEvent {
                                event_type: "loading_failed".to_string(),
                                model_id: Some(model_id.to_string()),
                                model_name: Some(model_info.name.clone()),
                                error: Some(error_msg.clone()),
                            },
                        );
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::Moonshine(engine)
            }
            EngineType::MoonshineStreaming => {
                let threads = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
                    .min(8);
                let engine = StreamingModel::load(
                    &model_path,
                    threads,
                    &quantization_for(&model_info.filename),
                )
                .map_err(|e| {
                        let error_msg = format!(
                            "Failed to load moonshine streaming model {}: {}",
                            model_id, e
                        );
                        let _ = self.app_handle.emit(
                            "model-state-changed",
                            ModelStateEvent {
                                event_type: "loading_failed".to_string(),
                                model_id: Some(model_id.to_string()),
                                model_name: Some(model_info.name.clone()),
                                error: Some(error_msg.clone()),
                            },
                        );
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::MoonshineStreaming(engine)
            }
            EngineType::SenseVoice => {
                let engine = SenseVoiceModel::load(&model_path, &quantization_for(&model_info.filename))
                    .map_err(|e| {
                        let error_msg =
                            format!("Failed to load SenseVoice model {}: {}", model_id, e);
                        let _ = self.app_handle.emit(
                            "model-state-changed",
                            ModelStateEvent {
                                event_type: "loading_failed".to_string(),
                                model_id: Some(model_id.to_string()),
                                model_name: Some(model_info.name.clone()),
                                error: Some(error_msg.clone()),
                            },
                        );
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::SenseVoice(engine)
            }
            EngineType::Cohere => {
                let engine = CohereModel::load(&model_path, &quantization_for(&model_info.filename))
                    .map_err(|e| {
                        let error_msg =
                            format!("Failed to load Cohere model {}: {}", model_id, e);
                        let _ = self.app_handle.emit(
                            "model-state-changed",
                            ModelStateEvent {
                                event_type: "loading_failed".to_string(),
                                model_id: Some(model_id.to_string()),
                                model_name: Some(model_info.name.clone()),
                                error: Some(error_msg.clone()),
                            },
                        );
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::Cohere(engine)
            }
            EngineType::Canary => {
                let engine = CanaryModel::load(&model_path, &quantization_for(&model_info.filename))
                    .map_err(|e| {
                        let error_msg =
                            format!("Failed to load Canary model {}: {}", model_id, e);
                        let _ = self.app_handle.emit(
                            "model-state-changed",
                            ModelStateEvent {
                                event_type: "loading_failed".to_string(),
                                model_id: Some(model_id.to_string()),
                                model_name: Some(model_info.name.clone()),
                                error: Some(error_msg.clone()),
                            },
                        );
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::Canary(engine)
            }
            EngineType::AppleSpeech => {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    LoadedEngine::AppleSpeech
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    return Err(anyhow::anyhow!(
                        "Apple Speech is only available on Apple Silicon macOS"
                    ));
                }
            }
            EngineType::GeminiApi => {
                let settings = get_settings(&self.app_handle);
                if settings.gemini_api_key.is_none()
                    || settings
                        .gemini_api_key
                        .as_ref()
                        .map_or(true, |k| k.is_empty())
                {
                    let error_msg = "Gemini API key not configured";
                    let _ = self.app_handle.emit(
                        "model-state-changed",
                        ModelStateEvent {
                            event_type: "loading_failed".to_string(),
                            model_id: Some(model_id.to_string()),
                            model_name: Some(model_info.name.clone()),
                            error: Some(error_msg.to_string()),
                        },
                    );
                    return Err(anyhow::anyhow!(error_msg));
                }
                LoadedEngine::GeminiApi
            }
        };

        // Update the current engine and model ID
        {
            let mut engine = self.lock_engine();
            *engine = Some(loaded_engine);
        }
        {
            let mut current_model = self.current_model_id.lock().unwrap();
            *current_model = Some(model_id.to_string());
        }

        // Emit loading completed event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_completed".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: Some(model_info.name.clone()),
                error: None,
            },
        );

        let load_duration = load_start.elapsed();
        debug!(
            "Successfully loaded transcription model: {} (took {}ms)",
            model_id,
            load_duration.as_millis()
        );
        Ok(())
    }

    /// Kicks off the model loading in a background thread if it's not already loaded
    pub fn initiate_model_load(&self) {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading || self.is_model_loaded() {
            return;
        }

        *is_loading = true;
        let self_clone = self.clone();
        thread::spawn(move || {
            let settings = get_settings(&self_clone.app_handle);
            if let Err(e) = self_clone.load_model(&settings.selected_model) {
                error!("Failed to load model: {}", e);
            }
            let mut is_loading = self_clone.is_loading.lock().unwrap();
            *is_loading = false;
            self_clone.loading_condvar.notify_all();
        });
    }

    pub fn get_current_model(&self) -> Option<String> {
        let current_model = self.current_model_id.lock().unwrap();
        current_model.clone()
    }

    pub fn get_current_model_name(&self) -> Option<String> {
        let model_id = self.get_current_model()?;
        self.model_manager
            .get_model_info(&model_id)
            .map(|info| info.name)
    }

    /// Engine type of the currently loaded model, if any.
    pub fn current_engine_type(&self) -> Option<EngineType> {
        let model_id = self.get_current_model()?;
        self.model_manager
            .get_model_info(&model_id)
            .map(|info| info.engine_type)
    }

    pub fn transcribe(&self, audio: Vec<f32>) -> Result<String> {
        // Update last activity timestamp
        self.last_activity.store(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            Ordering::Relaxed,
        );

        let st = std::time::Instant::now();

        debug!("Audio vector length: {}", audio.len());

        if audio.is_empty() {
            debug!("Empty audio vector");
            self.maybe_unload_immediately("empty audio");
            return Ok(String::new());
        }

        // Check if model is loaded, if not try to load it
        {
            // If the model is loading, wait for it to complete.
            let mut is_loading = self.is_loading.lock().unwrap();
            while *is_loading {
                is_loading = self.loading_condvar.wait(is_loading).unwrap();
            }

            let engine_guard = self.lock_engine();
            if engine_guard.is_none() {
                return Err(anyhow::anyhow!("Model is not loaded for transcription."));
            }
        }

        // Get current settings for configuration
        let settings = get_settings(&self.app_handle);

        // Handle Gemini API separately (requires async HTTP call)
        {
            let engine_guard = self.lock_engine();
            if let Some(LoadedEngine::GeminiApi) = engine_guard.as_ref() {
                drop(engine_guard);
                let api_key = settings
                    .gemini_api_key
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Gemini API key not configured"))?
                    .clone();
                let gemini_model = settings.gemini_model.clone();

                // Use block_in_place to safely run async code from a tokio worker thread.
                // Handle::block_on() panics if called directly from an async context,
                // so block_in_place tells tokio to move its work off this thread first.
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(
                        crate::gemini_client::transcribe_audio(&api_key, &gemini_model, &audio),
                    )
                })?;

                // Full post-processing chain (custom words → fillers → number conversion).
                // Gemini does not translate, so only the pinned language matters here.
                let final_result = finalize_transcription(
                    &result,
                    &settings,
                    conversion_hint_from_settings(&settings),
                    true,
                );

                let et = std::time::Instant::now();
                info!(
                    "Gemini transcription completed in {}ms",
                    (et - st).as_millis()
                );

                self.maybe_unload_immediately("gemini transcription");
                return Ok(final_result);
            }
        }

        // Conversion hint cascade (explicit signals first, detection as fallback).
        // Declared before the engine block: set per engine arm inside it, consumed
        // by the post-processing chain after it.
        let mut conversion_hint = ConversionHint::Skip;
        // Whisper gets custom words via `initial_prompt`; other engines keep
        // the fuzzy post-correction.
        let mut apply_custom_words_post = true;
        let parakeet_hint = {
            let model_id = self.current_model_id.lock().unwrap();
            match model_id.as_deref() {
                // Parakeet v2 is English-only; v3 auto-detects 25 languages.
                Some(id) if id.contains("v2") => {
                    ConversionHint::Known(ConversionLanguage::English)
                }
                _ => ConversionHint::Detect,
            }
        };

        // Perform transcription with the appropriate engine.
        // We use catch_unwind to prevent engine panics from poisoning the mutex,
        // which would make the app hang indefinitely on subsequent operations.
        let result = {
            let mut engine_guard = self.lock_engine();

            // Take the engine out so we own it during transcription.
            // If the engine panics, we simply don't put it back (effectively unloading it)
            // instead of poisoning the mutex.
            let mut engine = match engine_guard.take() {
                Some(e) => e,
                None => {
                    return Err(anyhow::anyhow!(
                        "Model failed to load after auto-load attempt. Please check your model settings."
                    ));
                }
            };

            // Release the lock before transcribing — no mutex held during the engine call
            drop(engine_guard);

            let transcribe_result = catch_unwind(AssertUnwindSafe(
                || -> Result<transcribe_rs::TranscriptionResult> {
                    match &mut engine {
                        LoadedEngine::Whisper(whisper_engine) => {
                            conversion_hint = if settings.translate_to_english {
                                ConversionHint::Known(ConversionLanguage::English)
                            } else {
                                conversion_hint_from_settings(&settings)
                            };
                            let whisper_language = if settings.selected_language == "auto" {
                                None
                            } else {
                                let normalized = if settings.selected_language == "zh-Hans"
                                    || settings.selected_language == "zh-Hant"
                                {
                                    "zh".to_string()
                                } else {
                                    settings.selected_language.clone()
                                };
                                Some(normalized)
                            };

                            apply_custom_words_post = false;
                            let params = WhisperInferenceParams {
                                language: whisper_language,
                                translate: settings.translate_to_english,
                                initial_prompt: if settings.custom_words.is_empty() {
                                    None
                                } else {
                                    Some(settings.custom_words.join(", "))
                                },
                                ..Default::default()
                            };

                            whisper_engine
                                .transcribe_with(&audio, &params)
                                .map_err(|e| anyhow::anyhow!("Whisper transcription failed: {}", e))
                        }
                        LoadedEngine::Parakeet(parakeet_engine) => {
                            conversion_hint = parakeet_hint;
                            let params = ParakeetParams {
                                language: None,
                                timestamp_granularity: Some(TimestampGranularity::Segment),
                            };
                            parakeet_engine
                                .transcribe_with(&audio, &params)
                                .map_err(|e| {
                                    anyhow::anyhow!("Parakeet transcription failed: {}", e)
                                })
                        }
                        LoadedEngine::Moonshine(moonshine_engine) => {
                            // Moonshine is English-only.
                            conversion_hint = ConversionHint::Known(ConversionLanguage::English);
                            moonshine_engine
                                .transcribe_with(&audio, &MoonshineParams::default())
                                .map_err(|e| anyhow::anyhow!("Moonshine transcription failed: {}", e))
                        }
                        LoadedEngine::MoonshineStreaming(streaming_engine) => {
                            conversion_hint = ConversionHint::Known(ConversionLanguage::English);
                            streaming_engine
                                .transcribe_with(
                                    &audio,
                                    &MoonshineStreamingParams {
                                        language: None,
                                        max_length: None,
                                    },
                                )
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Moonshine streaming transcription failed: {}",
                                        e
                                    )
                                })
                        }
                        LoadedEngine::SenseVoice(sense_voice_engine) => {
                            // zh/ja/ko/yue map to Skip (unsupported), "en" converts,
                            // "auto" falls back to on-text detection.
                            conversion_hint = conversion_hint_from_settings(&settings);
                            let language = match settings.selected_language.as_str() {
                                "zh" | "zh-Hans" | "zh-Hant" => "zh",
                                "en" => "en",
                                "ja" => "ja",
                                "ko" => "ko",
                                "yue" => "yue",
                                _ => "auto",
                            };
                            let params = SenseVoiceParams {
                                language: Some(language.to_string()),
                                use_itn: Some(true),
                            };
                            sense_voice_engine
                                .transcribe_with(&audio, &params)
                                .map_err(|e| {
                                    anyhow::anyhow!("SenseVoice transcription failed: {}", e)
                                })
                        }
                        LoadedEngine::Canary(canary_engine) => {
                            conversion_hint = conversion_hint_from_settings(&settings);
                            // Canary needs an explicit source language; in
                            // "auto" mode use the UI language as the hint.
                            let language = if settings.selected_language == "auto" {
                                settings.app_language.clone()
                            } else {
                                settings.selected_language.clone()
                            };
                            let target = if settings.translate_to_english
                                && !language.starts_with("en")
                            {
                                Some("en".to_string())
                            } else {
                                None
                            };
                            let params = CanaryParams {
                                language: Some(language),
                                target_language: target,
                                ..Default::default()
                            };
                            canary_engine
                                .transcribe_with(&audio, &params)
                                .map_err(|e| {
                                    anyhow::anyhow!("Canary transcription failed: {}", e)
                                })
                        }
                        LoadedEngine::Cohere(cohere_engine) => {
                            conversion_hint = conversion_hint_from_settings(&settings);
                            let params = CohereParams {
                                language: if settings.selected_language == "auto" {
                                    None
                                } else {
                                    Some(settings.selected_language.clone())
                                },
                                translate: settings.translate_to_english,
                                max_new_tokens: None,
                            };
                            cohere_engine
                                .transcribe_with(&audio, &params)
                                .map_err(|e| {
                                    anyhow::anyhow!("Cohere transcription failed: {}", e)
                                })
                        }
                        LoadedEngine::AppleSpeech => {
                            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                            {
                                // Never rely on Locale.current inside the app
                                // bundle: a Tauri app without CFBundleLocalizations
                                // reports the development region (English), so
                                // "auto" must use the app's UI language instead.
                                let locale = if settings.selected_language == "auto" {
                                    settings.app_language.clone()
                                } else {
                                    settings.selected_language.clone()
                                };
                                conversion_hint = map_selected_language(&locale)
                                    .unwrap_or(ConversionHint::Detect);
                                crate::apple_speech::transcribe(
                                    &audio,
                                    crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE,
                                    &locale,
                                )
                                .map(|text| transcribe_rs::TranscriptionResult {
                                    text,
                                    segments: None,
                                })
                                .map_err(|e| {
                                    anyhow::anyhow!("Apple Speech transcription failed: {}", e)
                                })
                            }
                            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                            {
                                Err(anyhow::anyhow!(
                                    "Apple Speech is only available on Apple Silicon macOS"
                                ))
                            }
                        }
                        LoadedEngine::GeminiApi => {
                            unreachable!("GeminiApi handled before catch_unwind")
                        }
                    }
                },
            ));

            match transcribe_result {
                Ok(inner_result) => {
                    // Success or normal error — put the engine back
                    let mut engine_guard = self.lock_engine();
                    *engine_guard = Some(engine);
                    inner_result?
                }
                Err(panic_payload) => {
                    // Engine panicked — do NOT put it back (it's in an unknown state).
                    // The engine is dropped here, effectively unloading it.
                    let panic_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic".to_string()
                    };
                    error!(
                        "Transcription engine panicked: {}. Model has been unloaded.",
                        panic_msg
                    );

                    // Clear the model ID so it will be reloaded on next attempt
                    {
                        let mut current_model = self
                            .current_model_id
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *current_model = None;
                    }

                    let _ = self.app_handle.emit(
                        "model-state-changed",
                        ModelStateEvent {
                            event_type: "unloaded".to_string(),
                            model_id: None,
                            model_name: None,
                            error: Some(format!("Engine panicked: {}", panic_msg)),
                        },
                    );

                    return Err(anyhow::anyhow!(
                        "Transcription engine panicked: {}. The model has been unloaded and will reload on next attempt.",
                        panic_msg
                    ));
                }
            }
        };

        // Full post-processing chain (custom words → fillers → number conversion)
        let filtered_result = finalize_transcription(
            &result.text,
            &settings,
            conversion_hint,
            apply_custom_words_post,
        );

        let et = std::time::Instant::now();
        let translation_note = if settings.translate_to_english {
            " (translated)"
        } else {
            ""
        };
        info!(
            "Transcription completed in {}ms{}",
            (et - st).as_millis(),
            translation_note
        );

        let final_result = filtered_result;

        if final_result.is_empty() {
            info!("Transcription result is empty");
        } else {
            info!("Transcription result: {}", final_result);
        }

        self.maybe_unload_immediately("transcription");

        Ok(final_result)
    }
}

impl Drop for TranscriptionManager {
    fn drop(&mut self) {
        debug!("Shutting down TranscriptionManager");

        // Signal the watcher thread to shutdown
        self.shutdown_signal.store(true, Ordering::Relaxed);

        // Wait for the thread to finish gracefully
        if let Some(handle) = self.watcher_handle.lock().unwrap().take() {
            if let Err(e) = handle.join() {
                warn!("Failed to join idle watcher thread: {:?}", e);
            } else {
                debug!("Idle watcher thread joined successfully");
            }
        }
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    /// Manual smoke test for the Canary ONNX model.
    ///   CANARY_MODEL_DIR=/path/canary-180m-flash \
    ///   CANARY_WAV=/path/mono16k.wav
    #[test]
    fn canary_real_transcription_env() {
        let (Ok(dir), Ok(wav)) = (
            std::env::var("CANARY_MODEL_DIR"),
            std::env::var("CANARY_WAV"),
        ) else {
            return;
        };

        let mut model = CanaryModel::load(std::path::Path::new(&dir), &Quantization::Int8)
            .expect("failed to load canary model");
        let mut reader = hound::WavReader::open(&wav).expect("failed to open wav");
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.expect("wav sample") as f32 / i16::MAX as f32)
            .collect();

        let params = CanaryParams {
            language: Some("fr".to_string()),
            target_language: None,
            ..Default::default()
        };
        let result = model
            .transcribe_with(&samples, &params)
            .expect("canary transcription failed");
        println!("Canary transcript: {}", result.text);
        assert!(!result.text.trim().is_empty(), "transcript must not be empty");
    }

    /// Manual smoke test for the Parakeet ONNX model. Skipped unless both env
    /// vars are set:
    ///   PARAKEET_MODEL_DIR=/path/to/parakeet-tdt-0.6b-v3-int8 \
    ///   PARAKEET_WAV=/path/to/mono16k.wav
    #[test]
    fn parakeet_real_transcription_env() {
        let (Ok(dir), Ok(wav)) = (
            std::env::var("PARAKEET_MODEL_DIR"),
            std::env::var("PARAKEET_WAV"),
        ) else {
            return;
        };

        let mut model = ParakeetModel::load(std::path::Path::new(&dir), &Quantization::Int8)
            .expect("failed to load parakeet model");
        let mut reader = hound::WavReader::open(&wav).expect("failed to open wav");
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.expect("wav sample") as f32 / i16::MAX as f32)
            .collect();

        let params = ParakeetParams {
            language: None,
            timestamp_granularity: Some(TimestampGranularity::Segment),
        };
        let result = model
            .transcribe_with(&samples, &params)
            .expect("parakeet transcription failed");
        println!("Parakeet transcript: {}", result.text);
        assert!(!result.text.trim().is_empty(), "transcript must not be empty");
    }

    /// Manual smoke test for the Cohere ONNX model. Skipped unless both env
    /// vars are set:
    ///   COHERE_MODEL_DIR=/path/to/cohere-transcribe-int4 \
    ///   COHERE_WAV=/path/to/mono16k.wav \
    ///   cargo test cohere_real_transcription_env -- --nocapture
    #[test]
    fn cohere_real_transcription_env() {
        let (Ok(dir), Ok(wav)) = (
            std::env::var("COHERE_MODEL_DIR"),
            std::env::var("COHERE_WAV"),
        ) else {
            return;
        };

        let mut model = CohereModel::load(std::path::Path::new(&dir), &Quantization::Int4)
            .expect("failed to load cohere model");
        let mut reader = hound::WavReader::open(&wav).expect("failed to open wav");
        let spec = reader.spec();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.expect("wav sample") as f32 / i16::MAX as f32)
            .collect();

        let params = CohereParams {
            language: Some("fr".to_string()),
            translate: false,
            max_new_tokens: None,
        };
        let result = model
            .transcribe_with(&samples, &params)
            .expect("cohere transcription failed");
        println!("Cohere transcript: {}", result.text);
        assert!(!result.text.trim().is_empty(), "transcript must not be empty");
        assert_eq!(spec.sample_rate, 16_000);
    }
}
