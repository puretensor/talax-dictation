import { invoke } from "@tauri-apps/api/core";

// Type definitions matching Rust backend structs

export interface SessionSummary {
  id: string;
  created_at: string;
  duration: number;
  segment_count: number;
  reviewed: boolean;
}

export interface SegmentDetail {
  segment_index: number;
  original_text: string;
  corrected_text: string | null;
  start_time: number;
  end_time: number;
  reviewed: boolean;
}

export interface SessionDetail {
  id: string;
  created_at: string;
  duration: number;
  whisper_model: string;
  segments: SegmentDetail[];
}

export interface CorrectionPattern {
  original: string;
  corrected: string;
  frequency: number;
  confidence: number;
  context_before: string | null;
  context_after: string | null;
}

export interface Stats {
  session_count: number;
  pattern_count: number;
  auto_apply_count: number;
}

export interface ModelInfo {
  id: string;
  name: string;
  size_mb: number;
  downloaded: boolean;
}

export interface AppConfig {
  hotkey: string;
  model: string;
  review_mode: string;
  injection_strategy: string;
  active_profile: string;
  vad_enabled: boolean;
  pre_roll_ms: number;
  silence_stop_ms: number;
}

export interface RuntimeDiagnostics {
  platform: string;
  session_type: string | null;
  microphone_ready: boolean;
  hotkey_ready: boolean;
  injection_ready: boolean;
  injection_mode_effective: string;
  model_downloaded: boolean;
  model_loaded: boolean;
  warnings: string[];
}

export type RecordingState =
  | "idle"
  | "recording"
  | "processing"
  | "injecting"
  | "error";

export interface RecordingEvent {
  state: RecordingState;
  message?: string;
}

export interface TranscriptionSegment {
  start_time: number;
  end_time: number;
  text: string;
}

export interface TranscriptionResult {
  segments: TranscriptionSegment[];
  full_text: string;
  processing_time_ms: number;
}

export interface PipelineResult {
  corrected: string;
  changes: PipelineChange[];
  layers_used: string[];
}

export interface PipelineChange {
  layer: string;
  position?: number;
  original: string;
  corrected: string;
  rule_freq?: number;
  original_score?: number;
  corrected_score?: number;
}

export interface TranscriptionEvent {
  session_id: string;
  raw: TranscriptionResult;
  corrected: PipelineResult;
  processing_time_ms: number;
}

// ---------------------------------------------------------------------------
// Profile commands
// ---------------------------------------------------------------------------

export async function getProfiles(): Promise<string[]> {
  try {
    return await invoke<string[]>("get_profiles");
  } catch {
    return ["default"];
  }
}

export async function createProfile(name: string): Promise<void> {
  await invoke("create_profile", { name });
}

export async function switchProfile(name: string): Promise<void> {
  await invoke("switch_profile", { name });
}

export async function deleteProfile(name: string): Promise<void> {
  await invoke("delete_profile", { name });
}

export async function resetProfile(name: string): Promise<void> {
  await invoke("reset_profile", { name });
}

export async function cloneProfile(
  source: string,
  target: string
): Promise<void> {
  await invoke("clone_profile", { source, target });
}

// ---------------------------------------------------------------------------
// Stats & patterns
// ---------------------------------------------------------------------------

export async function getStats(): Promise<Stats> {
  try {
    return await invoke<Stats>("get_stats");
  } catch {
    return { session_count: 0, pattern_count: 0, auto_apply_count: 0 };
  }
}

export async function getPatterns(): Promise<CorrectionPattern[]> {
  try {
    return await invoke<CorrectionPattern[]>("get_patterns");
  } catch {
    return [];
  }
}

export async function correctText(text: string): Promise<PipelineResult> {
  return await invoke<PipelineResult>("correct_text", { text });
}

// ---------------------------------------------------------------------------
// Session commands
// ---------------------------------------------------------------------------

export async function getSessions(
  limit: number = 50
): Promise<SessionSummary[]> {
  try {
    return await invoke<SessionSummary[]>("get_sessions", { limit });
  } catch {
    return [];
  }
}

export async function getSession(
  sessionId: string
): Promise<SessionDetail | null> {
  try {
    return await invoke<SessionDetail>("get_session", { sessionId });
  } catch {
    return null;
  }
}

export async function saveCorrections(
  sessionId: string,
  corrections: { segment_index: number; corrected_text: string }[]
): Promise<void> {
  await invoke("save_corrections", { sessionId, corrections });
}

// ---------------------------------------------------------------------------
// Model commands
// ---------------------------------------------------------------------------

export async function getAvailableModels(): Promise<ModelInfo[]> {
  try {
    return await invoke<ModelInfo[]>("get_available_models");
  } catch {
    return [];
  }
}

export async function downloadModel(modelId: string): Promise<void> {
  await invoke("download_model", { modelId });
}

// ---------------------------------------------------------------------------
// Recording commands
// ---------------------------------------------------------------------------

export async function getRecordingStatus(): Promise<RecordingState> {
  try {
    return await invoke<RecordingState>("get_recording_status");
  } catch {
    return "idle";
  }
}

export async function isModelReady(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_model_ready");
  } catch {
    return false;
  }
}

export async function loadWhisperModel(): Promise<void> {
  await invoke("load_whisper_model");
}

export async function startRecording(): Promise<void> {
  await invoke("start_recording");
}

export async function stopRecording(): Promise<void> {
  await invoke("stop_recording");
}

export async function injectReviewText(text: string): Promise<void> {
  await invoke("inject_review_text", { text });
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

export async function getAppConfig(): Promise<AppConfig> {
  try {
    return await invoke<AppConfig>("get_app_config");
  } catch {
    return {
      hotkey: "Ctrl+Shift+Space",
      model: "small.en-q5_1",
      review_mode: "auto_inject",
      injection_strategy: "clipboard",
      active_profile: "default",
      vad_enabled: true,
      pre_roll_ms: 300,
      silence_stop_ms: 700,
    };
  }
}

export async function getRuntimeDiagnostics(): Promise<RuntimeDiagnostics> {
  try {
    return await invoke<RuntimeDiagnostics>("get_runtime_diagnostics");
  } catch {
    return {
      platform: "unknown",
      session_type: null,
      microphone_ready: false,
      hotkey_ready: false,
      injection_ready: false,
      injection_mode_effective: "clipboard_only",
      model_downloaded: false,
      model_loaded: false,
      warnings: ["Runtime diagnostics unavailable."],
    };
  }
}

export async function saveAppConfig(config: AppConfig): Promise<void> {
  await invoke("save_app_config", { config });
}
