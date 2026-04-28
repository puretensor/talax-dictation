<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import {
    startRecording,
    stopRecording,
    isModelReady,
    loadWhisperModel,
    getAppConfig,
    saveCorrections,
    injectReviewText,
    type RecordingState,
    type RecordingEvent,
    type TranscriptionEvent,
    type PipelineChange,
  } from "../lib/api";

  let status = $state<RecordingState>("idle");
  let statusMessage = $state("");
  let transcription = $state("");
  let correctedText = $state("");
  let changes: PipelineChange[] = $state([]);
  let modelReady = $state(false);
  let modelLoading = $state(false);
  let hotkey = $state("Ctrl+Shift+Space");
  let reviewMode = $state("auto_inject");
  let editingIndex: number | null = $state(null);
  let editValue = $state("");
  let copied = $state(false);
  let sessionId = $state<string | null>(null);
  let saving = $state(false);
  let saveMessage = $state("");
  let reviewDirty = $state(false);
  let injectingReview = $state(false);

  let words = $derived(correctedText ? correctedText.split(" ") : []);

  let statusLabel = $derived(
    status === "idle"
      ? modelReady
        ? "Ready"
        : "Model Not Loaded"
      : status === "recording"
        ? "Recording..."
        : status === "processing"
          ? statusMessage || "Processing..."
          : status === "injecting"
            ? "Injecting..."
            : status === "error"
              ? "Error"
              : "Ready"
  );

  let statusClass = $derived(
    status === "recording"
      ? "recording"
      : status === "processing"
        ? "processing"
        : status === "injecting"
          ? "done"
          : status === "error"
            ? "error"
            : "idle"
  );

  async function checkModel() {
    modelReady = await isModelReady();
    if (!modelReady) {
      modelLoading = true;
      try {
        await loadWhisperModel();
        modelReady = true;
      } catch (e) {
        statusMessage = `Model load failed: ${e}`;
      } finally {
        modelLoading = false;
      }
    }
  }

  async function toggleRecording() {
    if (status === "idle" || status === "error") {
      if (!modelReady) {
        statusMessage = "Whisper model not loaded";
        return;
      }
      try {
        await startRecording();
      } catch (e) {
        statusMessage = `${e}`;
        status = "error";
      }
    } else if (status === "recording") {
      try {
        await stopRecording();
      } catch (e) {
        statusMessage = `${e}`;
        status = "error";
      }
    }
  }

  function startWordEdit(index: number) {
    editingIndex = index;
    editValue = words[index];
  }

  function commitWordEdit() {
    if (editingIndex !== null) {
      const updated = [...words];
      updated[editingIndex] = editValue;
      correctedText = updated.join(" ");
      reviewDirty = true;
      editingIndex = null;
      editValue = "";
    }
  }

  function cancelWordEdit() {
    editingIndex = null;
    editValue = "";
  }

  function handleEditKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") commitWordEdit();
    else if (e.key === "Escape") cancelWordEdit();
  }

  async function copyToClipboard() {
    try {
      await navigator.clipboard.writeText(correctedText);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {}
  }

  async function saveCurrentReview() {
    if (!sessionId || !correctedText.trim()) return;
    saving = true;
    try {
      await saveCorrections(sessionId, [
        { segment_index: 0, corrected_text: correctedText },
      ]);
      reviewDirty = false;
      saveMessage = "Saved to profile";
      setTimeout(() => (saveMessage = ""), 2000);
    } catch (e) {
      saveMessage = `Save failed: ${e}`;
    } finally {
      saving = false;
    }
  }

  async function injectCurrentReview() {
    if (!correctedText.trim()) return;
    injectingReview = true;
    try {
      await injectReviewText(correctedText);
      saveMessage = "Injected";
      setTimeout(() => (saveMessage = ""), 2000);
    } catch (e) {
      saveMessage = `Inject failed: ${e}`;
    } finally {
      injectingReview = false;
    }
  }

  onMount(() => {
    getAppConfig().then((cfg) => {
      hotkey = cfg.hotkey;
      reviewMode = cfg.review_mode;
    });
    checkModel();

    // Listen for recording state events from backend
    const unlistenState = listen<RecordingEvent>("recording-state", (event) => {
      status = event.payload.state;
      statusMessage = event.payload.message || "";
    });

    // Listen for transcription results
    const unlistenTranscription = listen<TranscriptionEvent>(
      "transcription-complete",
      (event) => {
        sessionId = event.payload.session_id;
        transcription = event.payload.raw.full_text;
        correctedText = event.payload.corrected.corrected;
        changes = event.payload.corrected.changes;
        reviewDirty = false;
        saveMessage = "";
      }
    );

    // Listen for model loaded events
    const unlistenModel = listen("model-loaded", () => {
      modelReady = true;
      modelLoading = false;
    });

    return () => {
      unlistenState.then((fn) => fn());
      unlistenTranscription.then((fn) => fn());
      unlistenModel.then((fn) => fn());
    };
  });
</script>

<div class="dictate-view">
  <h2>Dictate</h2>

  <div class="status-area">
    <button
      class="status-ring {statusClass}"
      onclick={toggleRecording}
      disabled={modelLoading ||
        status === "processing" ||
        status === "injecting"}
    >
      <div class="status-inner">
        <span class="status-label">{statusLabel}</span>
        {#if status === "recording"}
          <span class="status-hint">Click or press hotkey to stop</span>
        {:else if modelLoading}
          <span class="status-hint">Loading whisper model...</span>
        {:else if !modelReady}
          <span class="status-hint">Download model in Settings</span>
        {:else}
          <span class="status-hint">Click or press hotkey to start</span>
        {/if}
      </div>
    </button>
  </div>

  <p class="hotkey-hint">
    Hotkey: <kbd>{hotkey}</kbd>
  </p>

  {#if changes.length > 0}
    <div class="changes-bar">
      {changes.length} correction{changes.length !== 1 ? "s" : ""} applied
      ({[...new Set(changes.map((c) => c.layer))].join(", ")})
    </div>
  {/if}

  {#if correctedText}
    <div class="output-section">
      <div class="output-header">
        <h3>Corrected Output</h3>
        <div class="output-actions">
          <button class="copy-btn" onclick={copyToClipboard}>
            {copied ? "Copied!" : "Copy"}
          </button>
          {#if reviewMode === "review_first"}
            <button
              class="copy-btn"
              onclick={saveCurrentReview}
              disabled={saving || !reviewDirty || !sessionId}
            >
              {saving ? "Saving..." : "Save to Profile"}
            </button>
            <button
              class="copy-btn"
              onclick={injectCurrentReview}
              disabled={injectingReview}
            >
              {injectingReview ? "Injecting..." : "Inject"}
            </button>
          {/if}
        </div>
      </div>
      <div class="output-text">
        {#each words as word, i}
          {#if editingIndex === i}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="word-edit"
              bind:value={editValue}
              onkeydown={handleEditKeydown}
              onblur={commitWordEdit}
              autofocus
            />
          {:else}
            <button class="word" onclick={() => startWordEdit(i)}>
              {word}
            </button>
          {/if}
          {" "}
        {/each}
      </div>
      <p class="correction-hint">Click any word to correct it</p>
      {#if saveMessage}
        <p class="correction-hint accent">{saveMessage}</p>
      {/if}
    </div>
  {/if}

  {#if transcription && transcription !== correctedText}
    <div class="output-section original">
      <h3>Original Transcription</h3>
      <div class="output-text muted">{transcription}</div>
    </div>
  {/if}

  {#if statusMessage && status === "error"}
    <div class="error-bar">{statusMessage}</div>
  {/if}
</div>

<style>
  .dictate-view {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: 32px;
  }

  h2 {
    margin: 0 0 32px;
    font-size: 24px;
    color: var(--text, #e6edf3);
    align-self: flex-start;
  }

  .status-area {
    display: flex;
    justify-content: center;
    margin-bottom: 24px;
  }

  .status-ring {
    width: 200px;
    height: 200px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.3s ease;
    cursor: pointer;
    background: none;
    font-family: inherit;
  }

  .status-ring:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .status-ring.idle {
    border: 3px solid var(--accent, #58a6ff);
    background: rgba(88, 166, 255, 0.05);
  }

  .status-ring.recording {
    border: 3px solid var(--red, #f85149);
    background: rgba(248, 81, 73, 0.08);
    animation: pulse 1.5s ease-in-out infinite;
  }

  .status-ring.processing {
    border: 3px solid var(--yellow, #d29922);
    background: rgba(210, 153, 34, 0.08);
    animation: spin-border 2s linear infinite;
  }

  .status-ring.done {
    border: 3px solid var(--green, #3fb950);
    background: rgba(63, 185, 80, 0.08);
  }

  .status-ring.error {
    border: 3px solid var(--red, #f85149);
    background: rgba(248, 81, 73, 0.05);
  }

  @keyframes pulse {
    0%,
    100% {
      transform: scale(1);
    }
    50% {
      transform: scale(1.05);
    }
  }

  @keyframes spin-border {
    0% {
      box-shadow: 0 0 0 0 rgba(210, 153, 34, 0.4);
    }
    50% {
      box-shadow: 0 0 20px 4px rgba(210, 153, 34, 0.2);
    }
    100% {
      box-shadow: 0 0 0 0 rgba(210, 153, 34, 0.4);
    }
  }

  .status-inner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }

  .status-label {
    font-size: 20px;
    font-weight: 600;
    color: var(--text, #e6edf3);
  }

  .status-hint {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
  }

  .hotkey-hint {
    color: var(--text-muted, #8b949e);
    font-size: 14px;
    margin-bottom: 32px;
  }

  kbd {
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    border-radius: 4px;
    padding: 2px 6px;
    font-family: monospace;
    font-size: 13px;
    color: var(--text, #c9d1d9);
  }

  .changes-bar {
    background: rgba(63, 185, 80, 0.1);
    border: 1px solid rgba(63, 185, 80, 0.3);
    border-radius: 6px;
    padding: 8px 16px;
    font-size: 13px;
    color: var(--green, #3fb950);
    margin-bottom: 16px;
    width: 100%;
    max-width: 640px;
    text-align: center;
  }

  .error-bar {
    background: rgba(248, 81, 73, 0.1);
    border: 1px solid rgba(248, 81, 73, 0.3);
    border-radius: 6px;
    padding: 8px 16px;
    font-size: 13px;
    color: var(--red, #f85149);
    margin-top: 16px;
    width: 100%;
    max-width: 640px;
    text-align: center;
  }

  .output-section {
    width: 100%;
    max-width: 640px;
    margin-bottom: 16px;
  }

  .output-section.original {
    opacity: 0.6;
  }

  .output-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 12px;
  }

  .output-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .output-header h3 {
    margin: 0;
    font-size: 16px;
    color: var(--text, #e6edf3);
  }

  .output-section.original h3 {
    font-size: 14px;
  }

  .copy-btn {
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    color: var(--text, #c9d1d9);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.15s;
  }

  .copy-btn:hover {
    background: #30363d;
  }

  .copy-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .output-text {
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 8px;
    padding: 16px;
    line-height: 1.8;
    min-height: 80px;
  }

  .output-text.muted {
    color: var(--text-muted, #8b949e);
    font-size: 14px;
    min-height: auto;
  }

  .word {
    background: none;
    border: none;
    color: var(--text, #c9d1d9);
    font-size: 15px;
    padding: 1px 2px;
    border-radius: 3px;
    cursor: pointer;
    font-family: inherit;
    line-height: inherit;
  }

  .word:hover {
    background: rgba(88, 166, 255, 0.15);
    color: var(--accent, #58a6ff);
  }

  .word-edit {
    background: #21262d;
    border: 1px solid var(--accent, #58a6ff);
    color: var(--text, #c9d1d9);
    font-size: 15px;
    padding: 1px 4px;
    border-radius: 3px;
    font-family: inherit;
    outline: none;
    width: auto;
  }

  .correction-hint {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
    margin-top: 8px;
    text-align: center;
  }

  .correction-hint.accent {
    color: var(--accent, #58a6ff);
  }
</style>
