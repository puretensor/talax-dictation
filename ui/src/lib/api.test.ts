import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  createProfile,
  getAppConfig,
  getAvailableModels,
  getPatterns,
  getProfiles,
  getRecordingStatus,
  getRuntimeDiagnostics,
  getSession,
  getSessions,
  getStats,
  isModelReady,
} from "./api";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("api failure fallbacks", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("returns safe read defaults when IPC queries fail", async () => {
    mockInvoke.mockRejectedValue(new Error("ipc unavailable"));

    await expect(getProfiles()).resolves.toEqual(["default"]);
    await expect(getStats()).resolves.toEqual({
      session_count: 0,
      pattern_count: 0,
      auto_apply_count: 0,
    });
    await expect(getPatterns()).resolves.toEqual([]);
    await expect(getSessions()).resolves.toEqual([]);
    await expect(getSession("missing")).resolves.toBeNull();
    await expect(getAvailableModels()).resolves.toEqual([]);
    await expect(getRecordingStatus()).resolves.toBe("idle");
    await expect(isModelReady()).resolves.toBe(false);
  });

  it("returns complete fallback config and diagnostics", async () => {
    mockInvoke.mockRejectedValue(new Error("ipc unavailable"));

    await expect(getAppConfig()).resolves.toEqual({
      hotkey: "Ctrl+Shift+Space",
      model: "small.en-q5_1",
      review_mode: "auto_inject",
      injection_strategy: "clipboard",
      active_profile: "default",
      vad_enabled: true,
      pre_roll_ms: 300,
      silence_stop_ms: 700,
    });

    await expect(getRuntimeDiagnostics()).resolves.toEqual({
      platform: "unknown",
      session_type: null,
      microphone_ready: false,
      hotkey_ready: false,
      injection_ready: false,
      injection_mode_effective: "clipboard_only",
      model_downloaded: false,
      model_loaded: false,
      warnings: ["Runtime diagnostics unavailable."],
    });
  });

  it("does not swallow mutating command failures", async () => {
    mockInvoke.mockRejectedValue(new Error("write failed"));

    await expect(createProfile("demo")).rejects.toThrow("write failed");
  });
});
