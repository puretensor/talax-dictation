mod commands;
mod recording;
mod tray;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_dir = app.path().app_data_dir().unwrap_or_default();
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| app_dir.clone());

            // Load config from the Tauri-managed config dir (migrates legacy config.toml if present)
            let config = commands::load_or_create_config(&config_dir);

            // Initialize app directories
            let profiles_dir = app_dir.join("profiles");
            std::fs::create_dir_all(&profiles_dir).ok();

            let profile_mgr = talax_engine::profile::ProfileManager::new(profiles_dir);

            // Create default profile if none exist
            if profile_mgr.list_profiles().is_empty() {
                profile_mgr.create_profile("default").ok();
            }

            let mut state =
                commands::AppState::new(profile_mgr, app_dir, config_dir, config.clone());

            // Auto-switch to the last active profile on startup
            let target_profile = &config.active_profile;
            let available = state.profile_mgr.list_profiles();
            let profile_to_load = if available.contains(target_profile) {
                target_profile.clone()
            } else if available.contains(&"default".to_string()) {
                "default".to_string()
            } else if let Some(first) = available.first() {
                first.clone()
            } else {
                "default".to_string()
            };

            if let Ok(db) = commands::configure_pipeline_for_profile(&mut state, &profile_to_load) {
                state.profile_mgr.set_active(&profile_to_load);
                state.active_profile = profile_to_load;
                state.db = Some(db);
            }

            app.manage(std::sync::Mutex::new(state));
            if let Err(err) = commands::install_hotkey_listener(app.handle()) {
                tracing::warn!("failed to install hotkey listener: {err}");
            }

            // Set up system tray
            tray::setup_tray(app.handle()).ok();

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Profile management
            commands::get_profiles,
            commands::create_profile,
            commands::switch_profile,
            commands::delete_profile,
            commands::reset_profile,
            commands::clone_profile,
            // Stats & patterns
            commands::get_stats,
            commands::get_patterns,
            commands::correct_text,
            // Sessions
            commands::get_sessions,
            commands::get_session,
            commands::save_corrections,
            // Models
            commands::get_available_models,
            commands::download_model,
            // Recording
            commands::get_recording_status,
            commands::is_model_ready,
            commands::load_whisper_model,
            commands::start_recording,
            commands::stop_recording,
            commands::inject_review_text,
            // Config
            commands::get_app_config,
            commands::get_runtime_diagnostics,
            commands::save_app_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
