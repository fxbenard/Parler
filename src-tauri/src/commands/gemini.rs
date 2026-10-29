use tauri::AppHandle;

#[tauri::command]
#[specta::specta]
pub fn change_gemini_api_key_setting(app: AppHandle, api_key: String) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    let api_key = api_key.trim().to_string();

    // Keychain first when available; empty value deletes the entry.
    // On fallback, the plaintext stays in the settings file.
    if crate::settings::try_store_secret(crate::secrets::GEMINI_API_KEY, &api_key) {
        settings.gemini_api_key = None;
    } else {
        settings.gemini_api_key = if api_key.is_empty() {
            None
        } else {
            Some(api_key)
        };
    }

    crate::settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_gemini_model_setting(app: AppHandle, model: String) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.gemini_model = model;
    crate::settings::write_settings(&app, settings);
    Ok(())
}
