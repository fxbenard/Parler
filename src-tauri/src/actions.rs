#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::apple_intelligence;
use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{
    get_settings, AppSettings, PostProcessAction, APPLE_INTELLIGENCE_PROVIDER_ID,
    LANG_SIMPLIFIED_CHINESE, LANG_TRADITIONAL_CHINESE,
};
use crate::shortcut;
use crate::tray::{change_tray_icon, TrayIconState};
use crate::utils::{
    self, show_processing_overlay, show_recording_overlay, show_transcribing_overlay,
};
use crate::TranscriptionCoordinator;
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use log::{debug, error, info, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::AppHandle;
use tauri::Manager;

pub struct ActiveActionState(pub Mutex<Option<u8>>);

/// Audio sample rate used by the transcription pipeline.
const SAMPLE_RATE_HZ: f32 = 16_000.0;

/// Stores the bundle identifier of the application that was frontmost when
/// recording started.  Before pasting we re-activate this app so the text
/// ends up in the correct window (important for Electron apps like Claude
/// Desktop that can lose focus during the transcription pipeline).
#[cfg(target_os = "macos")]
static FRONTMOST_APP_BUNDLE_ID: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[cfg(any(target_os = "macos", test))]
fn is_phraser_bundle_id(bundle_id: &str) -> bool {
    matches!(
        bundle_id,
        "com.newblacc.phraser" | "com.newblacc.parler" | "com.melvynx.parler" | "computer.handy"
    )
}

/// Capture the currently frontmost application (macOS only).
#[cfg(target_os = "macos")]
fn save_frontmost_app() {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            r#"tell application "System Events" to get bundle identifier of first process whose frontmost is true"#,
        ])
        .output();

    if let Ok(out) = output {
        let bundle_id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !bundle_id.is_empty() {
            if is_phraser_bundle_id(&bundle_id) {
                debug!(
                    "Skipping frontmost app save because foreground app is Phraser: {}",
                    bundle_id
                );
                return;
            }
            debug!("Saved frontmost app: {}", bundle_id);
            if let Ok(mut guard) = FRONTMOST_APP_BUNDLE_ID.lock() {
                *guard = Some(bundle_id);
            }
        }
    }
}

/// Re-activate the previously frontmost application before pasting (macOS only).
/// This ensures the paste keystroke targets the correct app, even if the overlay
/// or transcription pipeline accidentally brought Phraser to the foreground.
#[cfg(target_os = "macos")]
fn restore_frontmost_app() {
    let bundle_id = FRONTMOST_APP_BUNDLE_ID
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());

    if let Some(bid) = bundle_id {
        if is_phraser_bundle_id(&bid) {
            debug!(
                "Skipping frontmost app restore for Phraser bundle id: {}",
                bid
            );
            return;
        }
        debug!("Restoring frontmost app: {}", bid);
        let script = format!(r#"tell application id "{}" to activate"#, bid);
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output();
        // Give the target app a moment to become frontmost
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Drop guard that notifies the [`TranscriptionCoordinator`] when the
/// transcription pipeline finishes — whether it completes normally or panics.
struct FinishGuard(AppHandle);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        if let Some(c) = self.0.try_state::<TranscriptionCoordinator>() {
            c.notify_processing_finished();
        }
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

// Transcribe Action
struct TranscribeAction {
    post_process: bool,
}

/// Field name for structured output JSON schema
const TRANSCRIPTION_FIELD: &str = "transcription";

/// Result of the text post-processing pipeline.
struct ProcessedTextResult {
    /// The final text to paste (may be post-processed, Chinese-converted, or raw transcription).
    final_text: String,
    /// The post-processed or Chinese-converted text, if any transformation was applied.
    post_processed_text: Option<String>,
    /// The prompt template used for LLM processing, if any.
    post_process_prompt: Option<String>,
}

/// Call Apple Intelligence for text processing.
/// Returns `None` if Apple Intelligence is unavailable, unsupported, or returns an empty result.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn call_apple_intelligence(system_prompt: &str, user_content: &str, model: &str) -> Option<String> {
    if !apple_intelligence::check_apple_intelligence_availability() {
        debug!("Apple Intelligence selected but not currently available on this device");
        return None;
    }
    let token_limit = model.trim().parse::<i32>().unwrap_or(0);
    match apple_intelligence::process_text_with_system_prompt(
        system_prompt,
        user_content,
        token_limit,
    ) {
        Ok(result) if !result.trim().is_empty() => {
            let result = strip_invisible_chars(&result);
            debug!(
                "Apple Intelligence processing succeeded. Output length: {} chars",
                result.len()
            );
            Some(result)
        }
        Ok(_) => {
            debug!("Apple Intelligence returned an empty response");
            None
        }
        Err(err) => {
            error!("Apple Intelligence processing failed: {}", err);
            None
        }
    }
}

/// Strip invisible Unicode characters that some LLMs may insert
fn strip_invisible_chars(s: &str) -> String {
    s.replace(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'], "")
}

/// Build a system prompt from the user's prompt template.
/// Removes `${output}` placeholder since the transcription is sent as the user message.
fn build_system_prompt(prompt_template: &str) -> String {
    prompt_template.replace("${output}", "").trim().to_string()
}

async fn post_process_transcription(settings: &AppSettings, transcription: &str) -> Option<String> {
    let provider = match settings.active_post_process_provider().cloned() {
        Some(provider) => provider,
        None => {
            debug!("Post-processing enabled but no provider is selected");
            return None;
        }
    };

    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    if model.trim().is_empty() {
        debug!(
            "Post-processing skipped because provider '{}' has no model configured",
            provider.id
        );
        return None;
    }

    let selected_prompt_id = match &settings.post_process_selected_prompt_id {
        Some(id) => id.clone(),
        None => {
            debug!("Post-processing skipped because no prompt is selected");
            return None;
        }
    };

    let prompt = match settings
        .post_process_prompts
        .iter()
        .find(|prompt| prompt.id == selected_prompt_id)
    {
        Some(prompt) => prompt.prompt.clone(),
        None => {
            debug!(
                "Post-processing skipped because prompt '{}' was not found",
                selected_prompt_id
            );
            return None;
        }
    };

    if prompt.trim().is_empty() {
        debug!("Post-processing skipped because the selected prompt is empty");
        return None;
    }

    debug!(
        "Starting LLM post-processing with provider '{}' (model: {})",
        provider.id, model
    );

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    if provider.supports_structured_output {
        debug!("Using structured outputs for provider '{}'", provider.id);

        let system_prompt = build_system_prompt(&prompt);
        let user_content = transcription.to_string();

        // Handle Apple Intelligence separately since it uses native Swift APIs
        if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            return call_apple_intelligence(&system_prompt, &user_content, &model);

            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                debug!("Apple Intelligence provider selected on unsupported platform");
                return None;
            }
        }

        // Define JSON schema for transcription output
        let json_schema = serde_json::json!({
            "type": "object",
            "properties": {
                (TRANSCRIPTION_FIELD): {
                    "type": "string",
                    "description": "The cleaned and processed transcription text"
                }
            },
            "required": [TRANSCRIPTION_FIELD],
            "additionalProperties": false
        });

        match crate::llm_client::send_chat_completion_with_schema(
            &provider,
            api_key.clone(),
            &model,
            user_content,
            Some(system_prompt),
            Some(json_schema),
        )
        .await
        {
            Ok(Some(content)) => {
                // Parse the JSON response to extract the transcription field
                match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => {
                        if let Some(transcription_value) =
                            json.get(TRANSCRIPTION_FIELD).and_then(|t| t.as_str())
                        {
                            let result = strip_invisible_chars(transcription_value);
                            debug!(
                                "Structured output post-processing succeeded for provider '{}'. Output length: {} chars",
                                provider.id,
                                result.len()
                            );
                            return Some(result);
                        } else {
                            error!("Structured output response missing 'transcription' field");
                            return Some(strip_invisible_chars(&content));
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse structured output JSON for provider '{}': {}. Falling back to legacy mode.",
                            provider.id, e
                        );
                        // Fall through to legacy mode below
                    }
                }
            }
            Ok(None) => {
                error!("LLM API response has no content");
                return None;
            }
            Err(e) => {
                warn!(
                    "Structured output failed for provider '{}': {}. Falling back to legacy mode.",
                    provider.id, e
                );
                // Fall through to legacy mode below
            }
        }
    }

    // Legacy mode: Replace ${output} variable in the prompt with the actual text
    let processed_prompt = prompt.replace("${output}", transcription);
    debug!("Processed prompt length: {} chars", processed_prompt.len());

    match crate::llm_client::send_chat_completion(&provider, api_key, &model, processed_prompt)
        .await
    {
        Ok(Some(content)) => {
            let content = strip_invisible_chars(&content);
            debug!(
                "LLM post-processing succeeded for provider '{}'. Output length: {} chars",
                provider.id,
                content.len()
            );
            Some(content)
        }
        Ok(None) => {
            error!("LLM API response has no content");
            None
        }
        Err(e) => {
            error!(
                "LLM post-processing failed for provider '{}': {}. Falling back to original transcription.",
                provider.id,
                e
            );
            None
        }
    }
}

async fn process_action(
    settings: &AppSettings,
    transcription: &str,
    prompt: &str,
    action_model: Option<&str>,
    action_provider_id: Option<&str>,
) -> Option<String> {
    let provider = if let Some(pid) = action_provider_id.filter(|p| !p.is_empty()) {
        match settings.post_process_provider(pid).cloned() {
            Some(p) => p,
            None => {
                debug!(
                    "Action provider '{}' not found, falling back to active provider",
                    pid
                );
                settings.active_post_process_provider().cloned()?
            }
        }
    } else {
        match settings.active_post_process_provider().cloned() {
            Some(p) => p,
            None => {
                debug!("Action processing skipped: no provider configured");
                return None;
            }
        }
    };

    let model = action_model
        .filter(|m| !m.trim().is_empty())
        .map(|m| m.to_string())
        .or_else(|| settings.post_process_models.get(&provider.id).cloned())
        .unwrap_or_default();

    let full_prompt = if prompt.contains("${output}") {
        prompt.replace("${output}", transcription)
    } else {
        format!("{}\n\n{}", prompt, transcription)
    };

    debug!(
        "Starting action processing with provider '{}', model '{}', prompt length: {}",
        provider.id,
        model,
        full_prompt.len()
    );

    // Handle Apple Intelligence via native Swift APIs
    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        return call_apple_intelligence(&full_prompt, transcription, &model);

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            debug!("Apple Intelligence provider selected on unsupported platform");
            return None;
        }
    }

    if model.trim().is_empty() {
        debug!(
            "Action processing skipped: no model configured for provider '{}'",
            provider.id
        );
        return None;
    }

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    let system_prompt = "You are a text processing assistant. Output ONLY the final processed text. Do not add any explanation, commentary, preamble, or formatting such as markdown code blocks. Just output the raw result text, nothing else.".to_string();

    match crate::llm_client::send_chat_completion_with_system(
        &provider,
        api_key,
        &model,
        full_prompt,
        system_prompt,
    )
    .await
    {
        Ok(Some(content)) if !content.is_empty() => {
            let result = strip_invisible_chars(&content);
            debug!(
                "Action processing succeeded for provider '{}'. Output length: {} chars",
                provider.id,
                result.len()
            );
            Some(result)
        }
        Ok(_) => {
            debug!("Action processing returned empty result");
            None
        }
        Err(e) => {
            error!(
                "Action processing failed for provider '{}': {}",
                provider.id, e
            );
            None
        }
    }
}

async fn maybe_convert_chinese_variant(
    settings: &AppSettings,
    transcription: &str,
) -> Option<String> {
    // Check if language is set to Simplified or Traditional Chinese
    let is_simplified = settings.selected_language == LANG_SIMPLIFIED_CHINESE;
    let is_traditional = settings.selected_language == LANG_TRADITIONAL_CHINESE;

    if !is_simplified && !is_traditional {
        debug!("selected_language is not Simplified or Traditional Chinese; skipping translation");
        return None;
    }

    debug!(
        "Starting Chinese translation using OpenCC for language: {}",
        settings.selected_language
    );

    // Use OpenCC to convert based on selected language
    let config = if is_simplified {
        // Convert Traditional Chinese to Simplified Chinese
        BuiltinConfig::Tw2sp
    } else {
        // Convert Simplified Chinese to Traditional Chinese
        BuiltinConfig::S2twp
    };

    match OpenCC::from_config(config) {
        Ok(converter) => {
            let converted = converter.convert(transcription);
            debug!(
                "OpenCC translation completed. Input length: {}, Output length: {}",
                transcription.len(),
                converted.len()
            );
            Some(converted)
        }
        Err(e) => {
            error!("Failed to initialize OpenCC converter: {}. Falling back to original transcription.", e);
            None
        }
    }
}

/// Switch to the long-audio model if the recording duration exceeds the configured threshold.
/// Returns `Some(original_model_id)` if the switch succeeded (caller must restore afterward),
/// or `None` if no switch was needed or the switch failed.
fn maybe_switch_model_for_long_audio(
    tm: &TranscriptionManager,
    settings: &AppSettings,
    sample_count: usize,
) -> Option<String> {
    let original_model = tm.get_current_model();
    let duration_seconds = sample_count as f32 / SAMPLE_RATE_HZ;

    let long_model_id = settings.long_audio_model.as_ref()?;

    if duration_seconds <= settings.long_audio_threshold_seconds
        || original_model.as_deref() == Some(long_model_id.as_str())
    {
        return None;
    }

    debug!(
        "Audio duration {:.1}s exceeds threshold {:.1}s, switching to long audio model: {}",
        duration_seconds, settings.long_audio_threshold_seconds, long_model_id
    );
    match tm.load_model(long_model_id) {
        Ok(()) => original_model,
        Err(e) => {
            warn!(
                "Failed to load long audio model '{}': {}, using current model",
                long_model_id, e
            );
            None
        }
    }
}

/// Apply Chinese conversion, action/post-process routing, and collect the prompt used.
async fn build_processed_text(
    settings: &AppSettings,
    transcription: &str,
    selected_action: Option<PostProcessAction>,
    post_process: bool,
) -> ProcessedTextResult {
    let mut final_text = transcription.to_string();
    let mut post_processed_text: Option<String> = None;
    let mut post_process_prompt: Option<String> = None;

    if let Some(converted) = maybe_convert_chinese_variant(settings, transcription).await {
        final_text = converted;
    }

    let processed = if let Some(ref action) = selected_action {
        process_action(
            settings,
            &final_text,
            &action.prompt,
            action.model.as_deref(),
            action.provider_id.as_deref(),
        )
        .await
    } else if post_process {
        post_process_transcription(settings, &final_text).await
    } else {
        None
    };

    if let Some(processed_text) = processed {
        final_text = processed_text.clone();
        post_processed_text = Some(processed_text);
        if let Some(action) = selected_action {
            post_process_prompt = Some(action.prompt);
        } else if let Some(prompt_id) = &settings.post_process_selected_prompt_id {
            if let Some(prompt) = settings
                .post_process_prompts
                .iter()
                .find(|p| &p.id == prompt_id)
            {
                post_process_prompt = Some(prompt.prompt.clone());
            }
        }
    } else if final_text != transcription {
        // Chinese conversion applied but no LLM post-processing
        post_processed_text = Some(final_text.clone());
    }

    ProcessedTextResult {
        final_text,
        post_processed_text,
        post_process_prompt,
    }
}

/// Spawn an async task to persist the transcription entry to history.
/// Fire-and-forget: drop the JoinHandle so history I/O never blocks transcription output.
fn spawn_save_transcription(
    hm: Arc<HistoryManager>,
    samples: Vec<f32>,
    transcription: String,
    post_processed_text: Option<String>,
    post_process_prompt: Option<String>,
    action_key: Option<u8>,
) {
    let _ = tauri::async_runtime::spawn(async move {
        if let Err(e) = hm
            .save_transcription(
                samples,
                transcription,
                post_processed_text,
                post_process_prompt,
                action_key,
            )
            .await
        {
            error!("Failed to save transcription to history: {}", e);
        }
    });
}

/// Paste the final text on the main thread, then hide the overlay and reset the tray icon.
fn paste_transcription_on_main_thread(ah: AppHandle, final_text: String) {
    let ah_clone = ah.clone();
    let paste_time = Instant::now();
    ah.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        restore_frontmost_app();

        match utils::paste(final_text, ah_clone.clone()) {
            Ok(()) => debug!("Text pasted successfully in {:?}", paste_time.elapsed()),
            Err(e) => error!("Failed to paste transcription: {}", e),
        }
        utils::hide_recording_overlay(&ah_clone);
        change_tray_icon(&ah_clone, TrayIconState::Idle);
    })
    .unwrap_or_else(|e| {
        error!("Failed to run paste on main thread: {:?}", e);
        utils::hide_recording_overlay(&ah);
        change_tray_icon(&ah, TrayIconState::Idle);
    });
}

/// Restore the model that was active before long-audio switching.
/// `original_model` is `Some(id)` only when a switch actually occurred; `None` is a no-op.
fn restore_model_after_long_audio(tm: &TranscriptionManager, original_model: Option<String>) {
    if let Some(orig_id) = original_model {
        debug!("Restoring original model: {}", orig_id);
        if let Err(e) = tm.load_model(&orig_id) {
            warn!("Failed to restore original model '{}': {}", orig_id, e);
        }
    }
}

/// Hide the recording overlay and reset the tray icon to idle.
fn reset_transcribe_ui(ah: &AppHandle) {
    utils::hide_recording_overlay(ah);
    change_tray_icon(ah, TrayIconState::Idle);
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        let start_time = Instant::now();
        debug!("TranscribeAction::start called for binding: {}", binding_id);

        // Save the frontmost app so we can re-activate it before pasting.
        // This must happen before any overlay or window operations that could
        // change the frontmost app.
        #[cfg(target_os = "macos")]
        save_frontmost_app();

        // Load model in the background
        let tm = app.state::<Arc<TranscriptionManager>>();
        tm.initiate_model_load();

        let binding_id = binding_id.to_string();
        change_tray_icon(app, TrayIconState::Recording);
        show_recording_overlay(app);

        let rm = app.state::<Arc<AudioRecordingManager>>();

        // Get the microphone mode to determine audio feedback timing
        let settings = get_settings(app);
        let is_always_on = settings.always_on_microphone;
        debug!("Microphone mode - always_on: {}", is_always_on);

        let mut recording_started = false;
        if is_always_on {
            // Always-on mode: Play audio feedback immediately, then apply mute after sound finishes
            debug!("Always-on mode: Playing audio feedback immediately");
            let rm_clone = Arc::clone(&rm);
            let app_clone = app.clone();
            // The blocking helper exits immediately if audio feedback is disabled,
            // so we can always reuse this thread to ensure mute happens right after playback.
            std::thread::spawn(move || {
                play_feedback_sound_blocking(&app_clone, SoundType::Start);
                rm_clone.apply_mute();
            });

            recording_started = rm.try_start_recording(&binding_id);
            debug!("Recording started: {}", recording_started);
        } else {
            // On-demand mode: Start recording first, then play audio feedback, then apply mute
            // This allows the microphone to be activated before playing the sound
            debug!("On-demand mode: Starting recording first, then audio feedback");
            let recording_start_time = Instant::now();
            if rm.try_start_recording(&binding_id) {
                recording_started = true;
                debug!("Recording started in {:?}", recording_start_time.elapsed());
                // Small delay to ensure microphone stream is active
                let app_clone = app.clone();
                let rm_clone = Arc::clone(&rm);
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    debug!("Handling delayed audio feedback/mute sequence");
                    // Helper handles disabled audio feedback by returning early, so we reuse it
                    // to keep mute sequencing consistent in every mode.
                    play_feedback_sound_blocking(&app_clone, SoundType::Start);
                    rm_clone.apply_mute();
                });
            } else {
                debug!("Failed to start recording");
            }
        }

        if recording_started {
            shortcut::register_cancel_shortcut(app);
            shortcut::register_pause_shortcut(app);
            shortcut::register_action_shortcuts(app);
        }

        debug!(
            "TranscribeAction::start completed in {:?}",
            start_time.elapsed()
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        crate::shortcut::handler::reset_cancel_confirmation();
        shortcut::unregister_cancel_shortcut(app);
        shortcut::unregister_pause_shortcut(app);
        shortcut::unregister_action_shortcuts(app);

        let stop_time = Instant::now();
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());

        change_tray_icon(app, TrayIconState::Transcribing);
        show_transcribing_overlay(app);

        // Unmute before playing audio feedback so the stop sound is audible
        rm.remove_mute();
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string();
        let post_process = self.post_process;

        // Read and clear the selected action before spawning the async task
        let selected_action_key =
            app.try_state::<ActiveActionState>()
                .and_then(|s| match s.0.lock() {
                    Ok(mut guard) => guard.take(),
                    Err(poisoned) => {
                        error!("ActiveActionState mutex poisoned, recovering");
                        poisoned.into_inner().take()
                    }
                });

        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard(ah.clone());
            debug!(
                "Starting async transcription task for binding: {}, action: {:?}",
                binding_id, selected_action_key
            );

            let stop_recording_time = Instant::now();
            if let Some(samples) = rm.stop_recording(&binding_id) {
                debug!(
                    "Recording stopped in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                // Single settings snapshot for the entire pipeline — avoids TOCTOU
                // between model selection and text processing.
                let settings = get_settings(&ah);
                let original_model =
                    maybe_switch_model_for_long_audio(&tm, &settings, samples.len());

                // TODO: Change TranscriptionManager::transcribe() to take &[f32] so this
                // clone can be deferred to the success-and-non-empty branch. Currently
                // unavoidable because transcribe() takes ownership of the audio buffer.
                let samples_for_history = samples.clone();
                let transcription_time = Instant::now();
                match tm.transcribe(samples) {
                    Ok(transcription) if !transcription.is_empty() => {
                        debug!(
                            "Transcription completed in {:?}: '{}'",
                            transcription_time.elapsed(),
                            transcription
                        );

                        let selected_action = selected_action_key.and_then(|key| {
                            settings
                                .post_process_actions
                                .iter()
                                .find(|a| a.key == key)
                                .cloned()
                        });

                        if selected_action.is_some() || post_process {
                            show_processing_overlay(&ah);
                        }

                        let result = build_processed_text(
                            &settings,
                            &transcription,
                            selected_action,
                            post_process,
                        )
                        .await;

                        let action_key_for_history = if result.post_processed_text.is_some() {
                            selected_action_key
                        } else {
                            None
                        };
                        spawn_save_transcription(
                            Arc::clone(&hm),
                            samples_for_history,
                            transcription,
                            result.post_processed_text,
                            result.post_process_prompt,
                            action_key_for_history,
                        );
                        paste_transcription_on_main_thread(ah.clone(), result.final_text);
                    }
                    Ok(_) => {
                        debug!("Transcription returned empty result");
                        reset_transcribe_ui(&ah);
                    }
                    Err(err) => {
                        error!("Transcription failed: {}", err);
                        reset_transcribe_ui(&ah);
                    }
                }

                restore_model_after_long_audio(&tm, original_model);
            } else {
                debug!("No samples retrieved from recording stop");
                reset_transcribe_ui(&ah);
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Cancel Action
struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        utils::cancel_current_operation(app);
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

// Test Action
struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Started - {} (App: {})",
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Stopped - {} (App: {})",
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_phraser_bundle_id_recognized() {
        assert!(is_phraser_bundle_id("com.newblacc.phraser"));
    }

    #[test]
    fn legacy_parler_bundle_ids_still_recognized() {
        assert!(is_phraser_bundle_id("com.newblacc.parler"));
        assert!(is_phraser_bundle_id("com.melvynx.parler"));
    }

    #[test]
    fn handy_bundle_id_recognized() {
        assert!(is_phraser_bundle_id("computer.handy"));
    }

    #[test]
    fn unrelated_bundle_id_rejected() {
        assert!(!is_phraser_bundle_id("com.apple.safari"));
        assert!(!is_phraser_bundle_id("com.newblacc.other"));
        assert!(!is_phraser_bundle_id(""));
    }

    #[test]
    fn action_map_has_expected_keys() {
        assert!(ACTION_MAP.contains_key("transcribe"));
        assert!(ACTION_MAP.contains_key("transcribe_with_post_process"));
        assert!(ACTION_MAP.contains_key("cancel"));
        assert!(ACTION_MAP.contains_key("test"));
    }
}

// Static Action Map
pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction {
            post_process: false,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_with_post_process".to_string(),
        Arc::new(TranscribeAction { post_process: true }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});
