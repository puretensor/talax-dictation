//! Tauri IPC command handlers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use talax_engine::db::{Database, SessionDetail, SessionSummary, Stats};
use talax_engine::hotkey::{HotkeyEvent, HotkeyHandle, HotkeyListener, parse_hotkey};
use talax_engine::inject::InjectionMode;
use talax_engine::pipeline::CorrectionPipeline;
use talax_engine::profile::{ProfileManager, is_valid_profile_name};
use talax_engine::whisper::model_manager::ModelManager;
use talax_engine::whisper::transcriber::TranscribeParams;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::recording::{RecordingEvent, RecordingOrchestrator, RecordingState, TranscriptionEvent};

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Application configuration persisted to the Tauri-managed config directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey: String,
    pub model: String,
    pub review_mode: String,
    pub injection_strategy: String,
    pub active_profile: String,
    pub vad_enabled: bool,
    pub pre_roll_ms: u32,
    pub silence_stop_ms: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Shift+Space".to_string(),
            model: "small.en-q5_1".to_string(),
            review_mode: "auto_inject".to_string(),
            injection_strategy: "clipboard".to_string(),
            active_profile: "default".to_string(),
            vad_enabled: true,
            pre_roll_ms: 300,
            silence_stop_ms: 700,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyAppConfig {
    hotkey: Option<String>,
    model: Option<String>,
    mode: Option<String>,
    active_profile: Option<String>,
}

/// Input for saving a single segment correction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionInput {
    pub segment_index: usize,
    pub corrected_text: String,
}

/// Stub model information (whisper model_manager may not exist yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_mb: u64,
    pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDiagnostics {
    pub platform: String,
    pub session_type: Option<String>,
    pub microphone_ready: bool,
    pub hotkey_ready: bool,
    pub injection_ready: bool,
    pub injection_mode_effective: String,
    pub model_downloaded: bool,
    pub model_loaded: bool,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub profile_mgr: ProfileManager,
    pub pipeline: CorrectionPipeline,
    pub active_profile: String,
    pub db: Option<Database>,
    pub app_dir: PathBuf,
    pub config_dir: PathBuf,
    pub config: AppConfig,
    pub model_mgr: ModelManager,
    pub recording: RecordingOrchestrator,
    pub hotkey_handle: Option<HotkeyHandle>,
}

fn lock_state(state: &Mutex<AppState>) -> MutexGuard<'_, AppState> {
    state.lock().unwrap_or_else(|poisoned| {
        tracing::error!("application state mutex was poisoned; recovering inner state");
        poisoned.into_inner()
    })
}

impl AppState {
    pub fn new(
        profile_mgr: ProfileManager,
        app_dir: PathBuf,
        config_dir: PathBuf,
        config: AppConfig,
    ) -> Self {
        let active_profile = config.active_profile.clone();
        let models_dir = app_dir.join("models");
        let model_mgr = ModelManager::new(models_dir).unwrap_or_else(|_| {
            // Fallback: create with a temp dir (should never happen)
            let fallback = app_dir.join("models_fallback");
            ModelManager::new(fallback).expect("failed to create model manager")
        });

        let mut recording = RecordingOrchestrator::new();
        recording.set_injection_mode(injection_mode_from_config(&config));
        recording.set_capture_preferences(
            config.vad_enabled,
            config.pre_roll_ms,
            config.silence_stop_ms,
        );

        // Set model path if downloaded
        if let Some(path) = model_mgr.get_model_path(&config.model)
            && path.exists()
        {
            recording.set_model_path(path);
        }

        Self {
            profile_mgr,
            pipeline: CorrectionPipeline::new(),
            active_profile,
            db: None,
            app_dir,
            config_dir,
            config,
            model_mgr,
            recording,
            hotkey_handle: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Config helpers (used by lib.rs setup)
// ---------------------------------------------------------------------------

fn legacy_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".talax").join("config.toml")
}

pub fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("config.toml")
}

fn parse_config(contents: &str) -> Option<AppConfig> {
    if let Ok(config) = toml::from_str::<AppConfig>(contents) {
        return Some(config);
    }
    if let Ok(legacy) = toml::from_str::<LegacyAppConfig>(contents) {
        return Some(migrate_legacy_config(legacy));
    }
    None
}

/// Load config from disk, or create a default one if missing.
pub fn load_or_create_config(config_dir: &Path) -> AppConfig {
    let path = config_path(config_dir);
    if let Ok(contents) = std::fs::read_to_string(&path)
        && let Some(config) = parse_config(&contents)
    {
        let _ = save_config_to_disk(config_dir, &config);
        return config;
    }

    let legacy_path = legacy_config_path();
    if legacy_path != path
        && let Ok(contents) = std::fs::read_to_string(&legacy_path)
        && let Some(config) = parse_config(&contents)
    {
        let _ = save_config_to_disk(config_dir, &config);
        return config;
    }

    let config = AppConfig::default();
    let _ = save_config_to_disk(config_dir, &config);
    config
}

/// Persist config to disk.
pub fn save_config_to_disk(config_dir: &Path, config: &AppConfig) -> Result<(), String> {
    let path = config_path(config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let contents = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, contents).map_err(|e| e.to_string())?;
    Ok(())
}

fn migrate_legacy_config(legacy: LegacyAppConfig) -> AppConfig {
    let mut config = AppConfig::default();

    if let Some(hotkey) = legacy.hotkey {
        config.hotkey = hotkey;
    }
    if let Some(model) = legacy.model {
        config.model = model;
    }
    if let Some(active_profile) = legacy.active_profile {
        config.active_profile = active_profile;
    }

    match legacy.mode.as_deref() {
        Some("auto-inject") => {
            config.review_mode = "auto_inject".to_string();
            config.injection_strategy = "clipboard".to_string();
        }
        Some("type-out") => {
            config.review_mode = "review_first".to_string();
            config.injection_strategy = "type_out".to_string();
        }
        Some("clipboard-only") => {
            config.review_mode = "review_first".to_string();
            config.injection_strategy = "clipboard_only".to_string();
        }
        Some("review") => {
            config.review_mode = "review_first".to_string();
            config.injection_strategy = "clipboard".to_string();
        }
        _ => {}
    }

    config
}

fn injection_mode_from_config(config: &AppConfig) -> InjectionMode {
    match config.injection_strategy.as_str() {
        "type_out" => InjectionMode::TypeOut,
        "clipboard_only" => InjectionMode::ClipboardOnly,
        _ => InjectionMode::Clipboard,
    }
}

fn auto_inject_enabled(config: &AppConfig) -> bool {
    config.review_mode == "auto_inject"
}

fn validate_config_values(config: &AppConfig) -> Result<(), String> {
    parse_hotkey(&config.hotkey).map_err(|e| format!("invalid hotkey: {e}"))?;

    if !matches!(config.review_mode.as_str(), "auto_inject" | "review_first") {
        return Err(format!("invalid review_mode: {}", config.review_mode));
    }

    if !matches!(
        config.injection_strategy.as_str(),
        "clipboard" | "clipboard_only" | "type_out"
    ) {
        return Err(format!(
            "invalid injection_strategy: {}",
            config.injection_strategy
        ));
    }

    if !is_valid_profile_name(&config.active_profile) {
        return Err(format!("invalid active_profile: {}", config.active_profile));
    }

    if config.pre_roll_ms > 2_000 {
        return Err("pre_roll_ms must be between 0 and 2000".to_string());
    }

    if config.silence_stop_ms > 3_000 {
        return Err("silence_stop_ms must be between 0 and 3000".to_string());
    }

    Ok(())
}

fn platform_name() -> String {
    if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "windows") {
        "windows".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

fn session_type() -> Option<String> {
    if let Ok(value) = std::env::var("XDG_SESSION_TYPE") {
        let trimmed = value.trim().to_lowercase();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Some("wayland".to_string());
    }
    if std::env::var_os("DISPLAY").is_some() {
        return Some("x11".to_string());
    }
    None
}

fn effective_injection_mode(config: &AppConfig, session_type: Option<&str>) -> (String, bool) {
    if cfg!(target_os = "linux")
        && session_type == Some("wayland")
        && config.injection_strategy == "type_out"
    {
        return ("clipboard_only".to_string(), false);
    }

    let ready = matches!(
        config.injection_strategy.as_str(),
        "clipboard" | "clipboard_only" | "type_out"
    );

    (config.injection_strategy.clone(), ready)
}

fn next_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sess-{millis}-{counter}")
}

fn emit_recording_state(app: &AppHandle, state: RecordingState, message: Option<String>) {
    let _ = app.emit("recording-state", RecordingEvent { state, message });
}

pub(crate) fn configure_pipeline_for_profile(
    state: &mut AppState,
    profile_name: &str,
) -> Result<Database, String> {
    let mut db = state.profile_mgr.open_db(profile_name)?;
    let domain_context_path = state
        .profile_mgr
        .profile_path(profile_name)
        .join("domain_context.json");
    db.set_domain_context_path(domain_context_path);
    let ngram_path = state
        .profile_mgr
        .profile_path(profile_name)
        .join("ngram.json");
    state.pipeline.set_ngram_model_path(ngram_path);
    state.pipeline.try_reload(&db).map_err(|e| e.to_string())?;
    Ok(db)
}

fn persist_transcription_session(
    state: &mut AppState,
    raw_result: &talax_engine::whisper::transcriber::TranscribeResult,
    duration_sec: f64,
) -> Result<String, String> {
    let db = state.db.as_ref().ok_or("No active profile")?;
    let session_id = next_session_id();
    db.create_session_with_model(&session_id, "", duration_sec, &state.config.model)
        .map_err(|e| e.to_string())?;

    let aggregate_text = raw_result.full_text.trim().to_string();
    let segment_text = if aggregate_text.is_empty() {
        raw_result.full_text.clone()
    } else {
        aggregate_text
    };
    let segments = vec![(0.0, duration_sec, segment_text.as_str())];
    db.add_segments(&session_id, &segments)
        .map_err(|e| e.to_string())?;

    Ok(session_id)
}

pub(crate) fn install_hotkey_listener(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<Mutex<AppState>>();

    let hotkey = {
        let state = lock_state(&state);
        state.config.hotkey.clone()
    };
    let config = parse_hotkey(&hotkey).map_err(|e| e.to_string())?;

    let previous_handle = {
        let mut state = lock_state(&state);
        state.hotkey_handle.take()
    };

    if let Some(handle) = previous_handle {
        handle.stop();
    }

    let app_handle = app.clone();
    let handle = HotkeyListener::new(config)
        .start(move |event| match event {
            HotkeyEvent::Pressed => {
                let app = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = start_recording_impl(&app);
                });
            }
            HotkeyEvent::Released => {
                let app = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = stop_recording_impl(app).await;
                });
            }
        })
        .map_err(|e| e.to_string())?;

    let mut state = lock_state(&state);
    state.hotkey_handle = Some(handle);
    Ok(())
}

fn start_recording_impl(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<Mutex<AppState>>();
    let mut s = lock_state(&state);
    if !s.recording.is_model_loaded() {
        drop(s);
        emit_recording_state(
            app,
            RecordingState::Error,
            Some("Whisper model not loaded".to_string()),
        );
        return Err("whisper model not loaded".to_string());
    }

    s.recording.start_recording()?;
    drop(s);

    emit_recording_state(app, RecordingState::Recording, None);

    Ok(())
}

async fn stop_recording_impl(app: AppHandle) -> Result<(), String> {
    let state = app.state::<Mutex<AppState>>();

    // Stop recording and get samples (short lock)
    let samples = {
        let mut s = lock_state(&state);
        s.recording.stop_recording()?
    };

    emit_recording_state(
        &app,
        RecordingState::Processing,
        Some("Transcribing...".to_string()),
    );

    if samples.is_empty() {
        let mut s = lock_state(&state);
        s.recording.set_idle();
        emit_recording_state(
            &app,
            RecordingState::Idle,
            Some("No audio captured".to_string()),
        );
        return Ok(());
    }

    let duration_sec = samples.len() as f64 / 16_000.0;

    let transcriber_result = {
        let s = lock_state(&state);
        s.recording.transcriber_handle()
    };
    let transcriber = match transcriber_result {
        Ok(handle) => handle,
        Err(e) => {
            let mut s = lock_state(&state);
            s.recording.set_idle();
            drop(s);
            emit_recording_state(&app, RecordingState::Error, Some(e.clone()));
            return Err(e);
        }
    };

    let raw_result = tauri::async_runtime::spawn_blocking(move || {
        let guard = transcriber.lock().map_err(|e| e.to_string())?;
        let params = TranscribeParams {
            language: Some("en".to_string()),
            ..Default::default()
        };
        let result = guard
            .transcribe_from_i16(&samples, &params)
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| e.to_string())?;

    let raw_result = match raw_result {
        Ok(r) => r,
        Err(e) => {
            let mut s = lock_state(&state);
            s.recording.set_idle();
            drop(s);
            emit_recording_state(&app, RecordingState::Error, Some(e.clone()));
            return Err(e);
        }
    };

    let processing_time_ms = raw_result.processing_time_ms;

    let correction_result = (|| {
        let mut s = lock_state(&state);
        let session_id = persist_transcription_session(&mut s, &raw_result, duration_sec)?;
        let corrected = s.pipeline.process(&raw_result.full_text);
        s.db.as_ref()
            .ok_or("No active profile")?
            .stage_corrections(&session_id, &[(0, corrected.corrected.as_str())])
            .map_err(|e| e.to_string())?;
        let auto_inject = auto_inject_enabled(&s.config);
        Ok::<_, String>((corrected, auto_inject, session_id))
    })();
    let (corrected, auto_inject, session_id) = match correction_result {
        Ok(values) => values,
        Err(e) => {
            let mut s = lock_state(&state);
            s.recording.set_idle();
            drop(s);
            emit_recording_state(&app, RecordingState::Error, Some(e.clone()));
            return Err(e);
        }
    };

    let _ = app.emit("profile-data-changed", ());

    let _ = app.emit(
        "transcription-complete",
        TranscriptionEvent {
            session_id,
            raw: raw_result,
            corrected: corrected.clone(),
            processing_time_ms,
        },
    );

    if auto_inject {
        emit_recording_state(&app, RecordingState::Injecting, None);

        let inject_result = {
            let s = lock_state(&state);
            s.recording.inject_text(&corrected.corrected)
        };

        if let Err(e) = inject_result {
            tracing::warn!("text injection failed: {e}");
        }
    }

    {
        let mut s = lock_state(&state);
        s.recording.set_idle();
    }

    emit_recording_state(&app, RecordingState::Idle, None);

    Ok(())
}

// ---------------------------------------------------------------------------
// Profile commands (existing)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_profiles(state: State<'_, Mutex<AppState>>) -> Vec<String> {
    let state = lock_state(&state);
    state.profile_mgr.list_profiles()
}

#[tauri::command]
pub fn create_profile(state: State<'_, Mutex<AppState>>, name: String) -> Result<String, String> {
    let state = lock_state(&state);
    state
        .profile_mgr
        .create_profile(&name)
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn switch_profile(state: State<'_, Mutex<AppState>>, name: String) -> Result<(), String> {
    let mut state = lock_state(&state);
    let db = configure_pipeline_for_profile(&mut state, &name)?;
    state.profile_mgr.set_active(&name);
    state.active_profile = name.clone();
    state.db = Some(db);
    // Persist the active profile to config
    state.config.active_profile = name;
    save_config_to_disk(&state.config_dir, &state.config).ok();
    Ok(())
}

#[tauri::command]
pub fn delete_profile(state: State<'_, Mutex<AppState>>, name: String) -> Result<(), String> {
    let state = lock_state(&state);
    if name == state.active_profile {
        return Err("Cannot delete the active profile".to_string());
    }
    state.profile_mgr.delete_profile(&name)
}

#[tauri::command]
pub fn reset_profile(state: State<'_, Mutex<AppState>>, name: String) -> Result<(), String> {
    let mut state = lock_state(&state);
    state.profile_mgr.reset_profile(&name).map(|_| ())?;
    // If this was the active profile, reopen its database and reload pipeline
    if name == state.active_profile {
        let db = configure_pipeline_for_profile(&mut state, &name)?;
        state.profile_mgr.set_active(&name);
        state.db = Some(db);
    }
    Ok(())
}

#[tauri::command]
pub fn clone_profile(
    state: State<'_, Mutex<AppState>>,
    source: String,
    target: String,
) -> Result<(), String> {
    let state = lock_state(&state);
    state
        .profile_mgr
        .clone_profile(&source, &target)
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// Stats / patterns / correction commands (existing)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_stats(state: State<'_, Mutex<AppState>>) -> Result<Stats, String> {
    let state = lock_state(&state);
    match &state.db {
        Some(db) => db.get_stats().map_err(|e| e.to_string()),
        None => Err("No active profile".to_string()),
    }
}

#[tauri::command]
pub fn get_patterns(
    state: State<'_, Mutex<AppState>>,
) -> Result<Vec<talax_engine::db::CorrectionPattern>, String> {
    let state = lock_state(&state);
    match &state.db {
        Some(db) => db.get_all_patterns().map_err(|e| e.to_string()),
        None => Err("No active profile".to_string()),
    }
}

#[tauri::command]
pub fn correct_text(
    state: State<'_, Mutex<AppState>>,
    text: String,
) -> talax_engine::pipeline::PipelineResult {
    let state = lock_state(&state);
    state.pipeline.process(&text)
}

// ---------------------------------------------------------------------------
// Session commands (new)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_sessions(
    state: State<'_, Mutex<AppState>>,
    limit: usize,
) -> Result<Vec<SessionSummary>, String> {
    let state = lock_state(&state);
    match &state.db {
        Some(db) => db.list_sessions(limit).map_err(|e| e.to_string()),
        None => Err("No active profile".to_string()),
    }
}

#[tauri::command]
pub fn get_session(
    state: State<'_, Mutex<AppState>>,
    session_id: String,
) -> Result<SessionDetail, String> {
    let state = lock_state(&state);
    match &state.db {
        Some(db) => db
            .get_session_detail(&session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Session '{session_id}' not found")),
        None => Err("No active profile".to_string()),
    }
}

#[tauri::command]
pub fn save_corrections(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    session_id: String,
    corrections: Vec<CorrectionInput>,
) -> Result<(), String> {
    let mut state = lock_state(&state);

    // Convert to the format the engine expects
    let correction_refs: Vec<(usize, String)> = corrections
        .into_iter()
        .map(|c| (c.segment_index, c.corrected_text))
        .collect();
    let correction_slices: Vec<(usize, &str)> = correction_refs
        .iter()
        .map(|(idx, text)| (*idx, text.as_str()))
        .collect();

    // Use a helper to split the borrow: access db and pipeline as disjoint fields.
    let AppState {
        ref db,
        ref mut pipeline,
        ..
    } = *state;

    let db = db.as_ref().ok_or("No active profile")?;
    db.save_corrections(&session_id, &correction_slices)
        .map_err(|e| e.to_string())?;

    // Reload the pipeline with updated patterns
    pipeline.try_reload(db).map_err(|e| e.to_string())?;
    let _ = app.emit("profile-data-changed", ());

    Ok(())
}

// ---------------------------------------------------------------------------
// Model commands (real model manager integration)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_available_models(state: State<'_, Mutex<AppState>>) -> Vec<ModelInfo> {
    let state = lock_state(&state);
    state
        .model_mgr
        .list_available()
        .into_iter()
        .map(|m| ModelInfo {
            id: m.name,
            name: m.description,
            size_mb: m.size_bytes / (1024 * 1024),
            downloaded: m.downloaded,
        })
        .collect()
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    model_id: String,
) -> Result<(), String> {
    let models_dir = {
        let s = lock_state(&state);
        s.app_dir.join("models")
    };

    let mgr = ModelManager::new(models_dir).map_err(|e| e.to_string())?;

    // Run download in a blocking task so we don't block the main thread
    let mid = model_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        mgr.download(&mid, |downloaded, total| {
            if total > 0 {
                let pct = downloaded as f64 / total as f64 * 100.0;
                tracing::debug!("download progress for {}: {:.1}%", mid, pct);
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // Update recording orchestrator with the new model path
    let mut s = lock_state(&state);
    if s.config.model == model_id
        && let Some(path) = s.model_mgr.get_model_path(&model_id)
    {
        s.recording.set_model_path(path);
    }

    // Emit event so frontend can refresh model list
    let _ = app.emit("model-downloaded", &model_id);

    Ok(())
}

// ---------------------------------------------------------------------------
// Recording commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_recording_status(state: State<'_, Mutex<AppState>>) -> RecordingState {
    let state = lock_state(&state);
    state.recording.state()
}

#[tauri::command]
pub fn is_model_ready(state: State<'_, Mutex<AppState>>) -> bool {
    let state = lock_state(&state);
    state.recording.is_model_loaded()
}

#[tauri::command]
pub async fn load_whisper_model(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
) -> Result<(), String> {
    // Check if already loaded
    {
        let s = lock_state(&state);
        if s.recording.is_model_loaded() {
            return Ok(());
        }
    }

    // Load model in a blocking task
    // We need to extract the model path, load outside the lock, then store
    let model_path = {
        let s = lock_state(&state);
        let model_id = &s.config.model;
        s.model_mgr
            .get_model_path(model_id)
            .ok_or_else(|| format!("unknown model: {}", model_id))?
    };

    if !model_path.exists() {
        return Err(format!("model not downloaded: {}", model_path.display()));
    }

    let path = model_path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        talax_engine::whisper::transcriber::Transcriber::new(&path)
    })
    .await
    .map_err(|e| e.to_string())?;

    let transcriber = result.map_err(|e| e.to_string())?;

    // Store the already-loaded model inside the lock.
    {
        let mut s = lock_state(&state);
        s.recording.set_model_path(model_path);
        s.recording.set_loaded_transcriber(transcriber);
    }

    let _ = app.emit("model-loaded", ());
    Ok(())
}

#[tauri::command]
pub fn start_recording(app: AppHandle, _state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    start_recording_impl(&app)
}

#[tauri::command]
pub async fn stop_recording(
    app: AppHandle,
    _state: State<'_, Mutex<AppState>>,
) -> Result<(), String> {
    stop_recording_impl(app).await
}

#[tauri::command]
pub fn inject_review_text(state: State<'_, Mutex<AppState>>, text: String) -> Result<(), String> {
    let state = lock_state(&state);
    state.recording.inject_text(&text)
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_app_config(state: State<'_, Mutex<AppState>>) -> AppConfig {
    let state = lock_state(&state);
    state.config.clone()
}

#[tauri::command]
pub fn get_runtime_diagnostics(state: State<'_, Mutex<AppState>>) -> RuntimeDiagnostics {
    let state = lock_state(&state);
    let session = session_type();
    let (injection_mode_effective, injection_ready) =
        effective_injection_mode(&state.config, session.as_deref());
    let model_path = state.model_mgr.get_model_path(&state.config.model);
    let model_downloaded = model_path.as_ref().is_some_and(|path| path.exists());
    let microphone_ready = talax_engine::audio::capture::probe_default_input_device().is_ok();
    let hotkey_ready = state.hotkey_handle.is_some();

    let mut warnings = Vec::new();
    if !microphone_ready {
        warnings.push("No usable microphone input device detected.".to_string());
    }
    if !hotkey_ready {
        warnings.push("Global hotkey listener is not active.".to_string());
    }
    if !model_downloaded {
        warnings.push("Selected Whisper model is not downloaded.".to_string());
    } else if !state.recording.is_model_loaded() {
        warnings.push("Selected Whisper model is downloaded but not loaded.".to_string());
    }
    if cfg!(target_os = "linux") && session.as_deref() == Some("wayland") {
        warnings.push(
            "Wayland may block global hotkeys and synthetic typing; clipboard delivery is the safest fallback."
                .to_string(),
        );
    }
    if cfg!(target_os = "macos") {
        warnings.push(
            "macOS may require Microphone, Accessibility, and Input Monitoring permissions."
                .to_string(),
        );
    }
    if !injection_ready {
        warnings
            .push("Requested injection strategy is degraded on this platform/session.".to_string());
    }

    RuntimeDiagnostics {
        platform: platform_name(),
        session_type: session,
        microphone_ready,
        hotkey_ready,
        injection_ready,
        injection_mode_effective,
        model_downloaded,
        model_loaded: state.recording.is_model_loaded(),
        warnings,
    }
}

#[tauri::command]
pub fn save_app_config(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    config: AppConfig,
) -> Result<(), String> {
    validate_config_values(&config)?;

    let config_dir = {
        let state = lock_state(&state);
        if state.model_mgr.get_model_path(&config.model).is_none() {
            return Err(format!("unknown model: {}", config.model));
        }
        if config.active_profile != state.active_profile {
            return Err("active_profile must be changed with switch_profile".to_string());
        }
        state.config_dir.clone()
    };
    save_config_to_disk(&config_dir, &config)?;

    let restart_hotkey;
    {
        let mut state = lock_state(&state);
        restart_hotkey = config.hotkey != state.config.hotkey;

        state
            .recording
            .set_injection_mode(injection_mode_from_config(&config));
        state.recording.set_capture_preferences(
            config.vad_enabled,
            config.pre_roll_ms,
            config.silence_stop_ms,
        );

        if config.model != state.config.model {
            state.recording.clear_model();
            if let Some(path) = state.model_mgr.get_model_path(&config.model) {
                state.recording.set_model_path(path);
            }
        }

        state.config = config.clone();
    }

    if restart_hotkey {
        install_hotkey_listener(&app)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validation_accepts_default_config() {
        validate_config_values(&AppConfig::default()).unwrap();
    }

    #[test]
    fn config_validation_rejects_invalid_modes_and_ranges() {
        let mut config = AppConfig {
            review_mode: "surprise".to_string(),
            ..Default::default()
        };
        assert!(validate_config_values(&config).is_err());

        config = AppConfig {
            injection_strategy: "shell".to_string(),
            ..Default::default()
        };
        assert!(validate_config_values(&config).is_err());

        config = AppConfig {
            pre_roll_ms: 2_001,
            ..Default::default()
        };
        assert!(validate_config_values(&config).is_err());

        config = AppConfig {
            silence_stop_ms: 3_001,
            ..Default::default()
        };
        assert!(validate_config_values(&config).is_err());
    }

    #[test]
    fn config_validation_rejects_invalid_hotkey_and_profile() {
        let mut config = AppConfig {
            hotkey: "Ctrl+DefinitelyNotAKey".to_string(),
            ..Default::default()
        };
        assert!(validate_config_values(&config).is_err());

        config = AppConfig {
            active_profile: "../escape".to_string(),
            ..Default::default()
        };
        assert!(validate_config_values(&config).is_err());
    }
}
