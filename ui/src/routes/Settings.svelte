<script lang="ts">
  import { onMount } from "svelte";
  import {
    getAppConfig,
    getRuntimeDiagnostics,
    saveAppConfig,
    getAvailableModels,
    downloadModel,
    type AppConfig,
    type ModelInfo,
    type RuntimeDiagnostics,
  } from "../lib/api";
  import { listen } from "@tauri-apps/api/event";

  let config: AppConfig = $state({
    hotkey: "Ctrl+Shift+Space",
    model: "small.en-q5_1",
    review_mode: "auto_inject",
    injection_strategy: "clipboard",
    active_profile: "default",
    vad_enabled: true,
    pre_roll_ms: 300,
    silence_stop_ms: 700,
  });
  let models: ModelInfo[] = $state([]);
  let diagnostics: RuntimeDiagnostics = $state({
    platform: "unknown",
    session_type: null,
    microphone_ready: false,
    hotkey_ready: false,
    injection_ready: false,
    injection_mode_effective: "clipboard_only",
    model_downloaded: false,
    model_loaded: false,
    warnings: [],
  });
  let loading = $state(true);
  let saving = $state(false);
  let saveMessage = $state("");
  let downloading = $state("");

  async function loadData() {
    loading = true;
    [config, models, diagnostics] = await Promise.all([
      getAppConfig(),
      getAvailableModels(),
      getRuntimeDiagnostics(),
    ]);
    loading = false;
  }

  onMount(() => {
    loadData();
    const unlisten = listen("model-downloaded", () => {
      loadData();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  async function handleSave() {
    saving = true;
    try {
      await saveAppConfig(config);
      diagnostics = await getRuntimeDiagnostics();
      saveMessage = "Settings saved";
      setTimeout(() => (saveMessage = ""), 2000);
    } catch (e) {
      saveMessage = `Error: ${e}`;
    } finally {
      saving = false;
    }
  }

  async function handleDownload(modelId: string) {
    downloading = modelId;
    try {
      await downloadModel(modelId);
      [models, diagnostics] = await Promise.all([
        getAvailableModels(),
        getRuntimeDiagnostics(),
      ]);
    } catch (e) {
      saveMessage = `Download failed: ${e}`;
    } finally {
      downloading = "";
    }
  }
</script>

<div class="settings-view">
  <div class="header-row">
    <h2>Settings</h2>
    <div class="header-actions">
      {#if saveMessage}
        <span class="save-msg">{saveMessage}</span>
      {/if}
      <button
        class="btn primary"
        onclick={handleSave}
        disabled={saving || loading}
      >
        {saving ? "Saving..." : "Save"}
      </button>
    </div>
  </div>

  {#if loading}
    <div class="empty-state">Loading settings...</div>
  {:else}
    <div class="settings-sections">
      <!-- Hotkey -->
      <div class="setting-group">
        <div class="setting-header">
          <h3>Runtime Diagnostics</h3>
          <span class="setting-desc">Current platform and readiness state</span>
        </div>
        <div class="diagnostics-grid">
          <div class="diag-row">
            <span class="diag-label">Platform</span>
            <span class="diag-value"
              >{diagnostics.platform}
              {#if diagnostics.session_type}
                / {diagnostics.session_type}
              {/if}</span
            >
          </div>
          <div class="diag-row">
            <span class="diag-label">Microphone</span>
            <span
              class="diag-badge"
              class:ready={diagnostics.microphone_ready}
              class:not-ready={!diagnostics.microphone_ready}
            >
              {diagnostics.microphone_ready ? "Ready" : "Unavailable"}
            </span>
          </div>
          <div class="diag-row">
            <span class="diag-label">Hotkey</span>
            <span
              class="diag-badge"
              class:ready={diagnostics.hotkey_ready}
              class:not-ready={!diagnostics.hotkey_ready}
            >
              {diagnostics.hotkey_ready ? "Listening" : "Not active"}
            </span>
          </div>
          <div class="diag-row">
            <span class="diag-label">Injection</span>
            <span
              class="diag-badge"
              class:ready={diagnostics.injection_ready}
              class:not-ready={!diagnostics.injection_ready}
            >
              {diagnostics.injection_mode_effective}
            </span>
          </div>
          <div class="diag-row">
            <span class="diag-label">Model</span>
            <span
              class="diag-badge"
              class:ready={diagnostics.model_downloaded && diagnostics.model_loaded}
              class:not-ready={!diagnostics.model_downloaded || !diagnostics.model_loaded}
            >
              {#if diagnostics.model_downloaded && diagnostics.model_loaded}
                Loaded
              {:else if diagnostics.model_downloaded}
                Downloaded only
              {:else}
                Missing
              {/if}
            </span>
          </div>
        </div>
        {#if diagnostics.warnings.length > 0}
          <ul class="warning-list">
            {#each diagnostics.warnings as warning}
              <li>{warning}</li>
            {/each}
          </ul>
        {/if}
      </div>

      <div class="setting-group">
        <div class="setting-header">
          <h3>Hotkey</h3>
          <span class="setting-desc"
            >Global keyboard shortcut for dictation</span
          >
        </div>
        <div class="setting-control">
          <kbd>{config.hotkey}</kbd>
          <span class="setting-note">Hotkey configuration coming soon</span>
        </div>
      </div>

      <!-- Model -->
      <div class="setting-group">
        <div class="setting-header">
          <h3>Whisper Model</h3>
          <span class="setting-desc"
            >Select the speech recognition model</span
          >
        </div>
        <div class="setting-control">
          <select bind:value={config.model}>
            {#each models as m}
              <option value={m.id}>{m.id} - {m.name} ({m.size_mb} MB)</option>
            {/each}
          </select>
        </div>
        <div class="model-list">
          {#each models as m}
            <div class="model-row">
              <div class="model-info">
                <span class="model-name">{m.id}</span>
                <span class="model-size">{m.size_mb} MB</span>
              </div>
              {#if m.downloaded}
                <span class="model-badge downloaded">Downloaded</span>
              {:else if downloading === m.id}
                <span class="model-badge downloading">Downloading...</span>
              {:else}
                <button
                  class="btn small"
                  onclick={() => handleDownload(m.id)}
                >
                  Download
                </button>
              {/if}
            </div>
          {/each}
        </div>
      </div>

      <div class="setting-group">
        <div class="setting-header">
          <h3>Capture Behavior</h3>
          <span class="setting-desc"
            >Leading/trailing silence trimming for push-to-talk</span
          >
        </div>
        <div class="setting-control">
          <label class="checkbox-row">
            <input type="checkbox" bind:checked={config.vad_enabled} />
            <span>Enable VAD silence trimming</span>
          </label>

          <label class="number-row">
            <span>Pre-roll (ms)</span>
            <input
              type="number"
              min="0"
              max="2000"
              step="50"
              bind:value={config.pre_roll_ms}
            />
          </label>

          <label class="number-row">
            <span>Trailing silence window (ms)</span>
            <input
              type="number"
              min="0"
              max="3000"
              step="50"
              bind:value={config.silence_stop_ms}
            />
          </label>

          <span class="setting-note">
            {config.vad_enabled
              ? "Keeps a short pre-roll and trims leading/trailing silence before transcription."
              : "Raw push-to-talk capture; no silence trimming is applied."}
          </span>
        </div>
      </div>

      <!-- Review Mode -->
      <div class="setting-group">
        <div class="setting-header">
          <h3>Review Mode</h3>
          <span class="setting-desc"
            >When TalaX should inject text automatically</span
          >
        </div>
        <div class="setting-control">
          <div class="toggle-group">
            <button
              class="toggle-btn"
              class:selected={config.review_mode === "auto_inject"}
              onclick={() => (config.review_mode = "auto_inject")}
            >
              Auto-inject
            </button>
            <button
              class="toggle-btn"
              class:selected={config.review_mode === "review_first"}
              onclick={() => (config.review_mode = "review_first")}
            >
              Review first
            </button>
          </div>
          <span class="setting-note">
            {config.review_mode === "auto_inject"
              ? "Text is injected automatically after transcription."
              : "Text stays in TalaX until you save or inject it manually."}
          </span>
        </div>
      </div>

      <!-- Injection Strategy -->
      <div class="setting-group">
        <div class="setting-header">
          <h3>Injection Strategy</h3>
          <span class="setting-desc"
            >How TalaX delivers text to the focused application</span
          >
        </div>
        <div class="setting-control">
          <div class="toggle-group">
            <button
              class="toggle-btn"
              class:selected={config.injection_strategy === "clipboard"}
              onclick={() => (config.injection_strategy = "clipboard")}
            >
              Clipboard Paste
            </button>
            <button
              class="toggle-btn"
              class:selected={config.injection_strategy === "type_out"}
              onclick={() => (config.injection_strategy = "type_out")}
            >
              Type Out
            </button>
            <button
              class="toggle-btn"
              class:selected={config.injection_strategy === "clipboard_only"}
              onclick={() => (config.injection_strategy = "clipboard_only")}
            >
              Clipboard Only
            </button>
          </div>
          <span class="setting-note">
            {config.injection_strategy === "clipboard"
              ? "Fast default: copy to clipboard and paste."
              : config.injection_strategy === "type_out"
                ? "Slower, but can work better in apps that block paste."
                : "Safe mode: copy only, without injecting."}
          </span>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .settings-view {
    max-width: 640px;
  }

  .header-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 24px;
  }

  h2 {
    margin: 0;
    font-size: 24px;
    color: var(--text, #e6edf3);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .save-msg {
    font-size: 13px;
    color: var(--green, #3fb950);
  }

  .btn {
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    color: var(--text, #c9d1d9);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.15s;
  }

  .btn.primary {
    background: var(--accent-dark, #1f6feb);
    border-color: var(--accent-dark, #1f6feb);
    color: #fff;
  }

  .btn.primary:hover:not(:disabled) {
    opacity: 0.9;
  }

  .btn.small {
    padding: 4px 10px;
    font-size: 12px;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .empty-state {
    text-align: center;
    padding: 48px 24px;
    color: var(--text-muted, #8b949e);
  }

  .settings-sections {
    display: flex;
    flex-direction: column;
  }

  .setting-group {
    padding: 20px 0;
    border-bottom: 1px solid var(--border, #30363d);
  }

  .setting-group:last-child {
    border-bottom: none;
  }

  .setting-header {
    margin-bottom: 12px;
  }

  .setting-header h3 {
    margin: 0;
    font-size: 15px;
    color: var(--text, #e6edf3);
    font-weight: 600;
  }

  .setting-desc {
    font-size: 13px;
    color: var(--text-muted, #8b949e);
  }

  .setting-control {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .diagnostics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px;
  }

  .diag-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    background: #161b22;
    border: 1px solid var(--border, #30363d);
    border-radius: 8px;
  }

  .diag-label {
    font-size: 13px;
    color: var(--text-muted, #8b949e);
  }

  .diag-value {
    font-size: 13px;
    color: var(--text, #c9d1d9);
  }

  .diag-badge {
    font-size: 12px;
    border-radius: 999px;
    padding: 4px 10px;
    background: #21262d;
    color: var(--text-muted, #8b949e);
    text-transform: capitalize;
  }

  .diag-badge.ready {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green, #3fb950);
  }

  .diag-badge.not-ready {
    background: rgba(248, 81, 73, 0.12);
    color: #f85149;
  }

  .warning-list {
    margin: 12px 0 0;
    padding-left: 18px;
    color: var(--text-muted, #8b949e);
    font-size: 13px;
  }

  .checkbox-row,
  .number-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    font-size: 13px;
    color: var(--text, #c9d1d9);
  }

  .checkbox-row input,
  .number-row input {
    accent-color: var(--accent, #58a6ff);
  }

  .number-row input {
    width: 110px;
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 6px 8px;
    color: var(--text, #c9d1d9);
  }

  .setting-note {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
  }

  kbd {
    display: inline-block;
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 6px 12px;
    font-family: "SF Mono", "Fira Code", monospace;
    font-size: 14px;
    color: var(--text, #c9d1d9);
    width: fit-content;
  }

  select {
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text, #c9d1d9);
    font-size: 14px;
    outline: none;
    max-width: 400px;
    cursor: pointer;
  }

  select:focus {
    border-color: var(--accent, #58a6ff);
  }

  .model-list {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .model-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 10px 14px;
  }

  .model-info {
    display: flex;
    gap: 12px;
    align-items: center;
  }

  .model-name {
    font-size: 14px;
    color: var(--text, #c9d1d9);
    font-weight: 500;
  }

  .model-size {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
  }

  .model-badge {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 4px;
  }

  .model-badge.downloaded {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green, #3fb950);
  }

  .model-badge.downloading {
    background: rgba(210, 153, 34, 0.15);
    color: var(--yellow, #d29922);
  }

  .toggle-group {
    display: flex;
    width: fit-content;
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    overflow: hidden;
  }

  .toggle-btn {
    background: #21262d;
    border: none;
    color: var(--text-muted, #8b949e);
    padding: 8px 16px;
    font-size: 13px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .toggle-btn:not(:last-child) {
    border-right: 1px solid var(--border, #30363d);
  }

  .toggle-btn.selected {
    background: var(--accent-dark, #1f6feb);
    color: #fff;
  }

  .toggle-btn:hover:not(.selected) {
    background: #30363d;
    color: var(--text, #c9d1d9);
  }
</style>
