<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { getSessions, getSession, saveCorrections } from "../lib/api";
  import { formatError } from "../lib/errors";
  import { LatestRequest } from "../lib/latest-request";
  import type { SessionSummary, SessionDetail } from "../lib/api";

  type SaveFeedback = { message: string; error: boolean };
  type SessionLoadResult = "loaded" | "failed" | "stale";

  let sessions: SessionSummary[] = $state([]);
  let expandedId: string | null = $state(null);
  let expandedDetail: SessionDetail | null = $state(null);
  let editedTexts: Map<string, Map<number, string>> = $state(new Map());
  let savingSessions: Set<string> = $state(new Set());
  let saveFeedbacks: Map<string, SaveFeedback> = $state(new Map());
  let loading = $state(true);
  let loadingDetail = $state(false);
  const saveMessageTimers = new Map<string, ReturnType<typeof setTimeout>>();
  let destroyed = false;
  const sessionRequests = new LatestRequest();
  const detailRequests = new LatestRequest();

  async function loadSessions(showLoading = true): Promise<SessionLoadResult> {
    const request = sessionRequests.begin();
    if (showLoading) loading = true;
    try {
      const nextSessions = await getSessions();
      if (!sessionRequests.isCurrent(request)) return "stale";
      sessions = nextSessions;
      return "loaded";
    } catch {
      return sessionRequests.isCurrent(request) ? "failed" : "stale";
    } finally {
      if (sessionRequests.isCurrent(request)) loading = false;
    }
  }

  async function refreshSessionsAndExpanded(
    showLoading = true,
    resetDetail = false
  ): Promise<SessionLoadResult> {
    const detailRequest = detailRequests.begin();
    if (resetDetail) {
      expandedDetail = null;
      loadingDetail = expandedId !== null;
    }

    const result = await loadSessions(showLoading);
    if (!detailRequests.isCurrent(detailRequest)) return "stale";
    if (result !== "loaded") {
      loadingDetail = false;
      return result;
    }

    const id = expandedId;
    if (!id) {
      loadingDetail = false;
      return result;
    }
    if (!sessions.some((session) => session.id === id)) {
      expandedId = null;
      expandedDetail = null;
      loadingDetail = false;
      detailRequests.invalidate();
      return result;
    }

    await loadExpandedDetail(id, false);
    return result;
  }

  async function loadExpandedDetail(
    id: string,
    showLoading = true
  ): Promise<boolean> {
    const request = detailRequests.begin();
    if (showLoading) loadingDetail = true;
    try {
      const detail = await getSession(id);
      if (!detailRequests.isCurrent(request) || expandedId !== id) return false;
      expandedDetail = detail;
      return detail !== null;
    } catch {
      if (!detailRequests.isCurrent(request) || expandedId !== id) return false;
      expandedDetail = null;
      return false;
    } finally {
      if (detailRequests.isCurrent(request) && expandedId === id) {
        loadingDetail = false;
      }
    }
  }

  async function toggleSession(id: string): Promise<void> {
    if (expandedId === id) {
      detailRequests.invalidate();
      expandedId = null;
      expandedDetail = null;
      loadingDetail = false;
    } else {
      expandedId = id;
      expandedDetail = null;
      await loadExpandedDetail(id);
    }
  }

  function getEditedText(sessionId: string, segIndex: number, original: string): string {
    const sessionEdits = editedTexts.get(sessionId);
    if (sessionEdits) {
      const val = sessionEdits.get(segIndex);
      if (val !== undefined) return val;
    }
    return original;
  }

  function updateSegmentText(sessionId: string, segIndex: number, value: string) {
    const newMap = new Map(editedTexts);
    const sessionEdits = new Map(newMap.get(sessionId) ?? []);
    sessionEdits.set(segIndex, value);
    newMap.set(sessionId, sessionEdits);
    editedTexts = newMap;
  }

  function isSaving(sessionId: string): boolean {
    return savingSessions.has(sessionId);
  }

  function setSaving(sessionId: string, saving: boolean): void {
    const next = new Set(savingSessions);
    if (saving) next.add(sessionId);
    else next.delete(sessionId);
    savingSessions = next;
  }

  function clearSaveMessageTimer(sessionId: string): void {
    const timer = saveMessageTimers.get(sessionId);
    if (timer !== undefined) {
      clearTimeout(timer);
      saveMessageTimers.delete(sessionId);
    }
  }

  function clearAllSaveMessageTimers(): void {
    for (const timer of saveMessageTimers.values()) clearTimeout(timer);
    saveMessageTimers.clear();
  }

  function clearSaveFeedback(sessionId: string, preserveError: boolean): void {
    clearSaveMessageTimer(sessionId);
    const feedback = saveFeedbacks.get(sessionId);
    if (!feedback || (preserveError && feedback.error)) return;
    const next = new Map(saveFeedbacks);
    next.delete(sessionId);
    saveFeedbacks = next;
  }

  function showSaveMessage(
    message: string,
    error: boolean,
    sessionId: string
  ): void {
    clearSaveMessageTimer(sessionId);
    if (destroyed) return;
    const next = new Map(saveFeedbacks);
    next.set(sessionId, { message, error });
    saveFeedbacks = next;
    if (!error) {
      const timer = setTimeout(() => {
        if (saveMessageTimers.get(sessionId) !== timer) return;
        const current = new Map(saveFeedbacks);
        current.delete(sessionId);
        saveFeedbacks = current;
        saveMessageTimers.delete(sessionId);
      }, 2000);
      saveMessageTimers.set(sessionId, timer);
    }
  }

  function removeSubmittedEdits(
    sessionId: string,
    submittedEdits: Map<number, string>
  ): void {
    const currentEdits = editedTexts.get(sessionId);
    if (!currentEdits) return;

    const remaining = new Map(currentEdits);
    for (const [segmentIndex, submittedText] of submittedEdits) {
      if (remaining.get(segmentIndex) === submittedText) {
        remaining.delete(segmentIndex);
      }
    }

    const nextEdits = new Map(editedTexts);
    if (remaining.size === 0) nextEdits.delete(sessionId);
    else nextEdits.set(sessionId, remaining);
    editedTexts = nextEdits;
  }

  async function handleSave(sessionId: string): Promise<void> {
    const sessionEdits = editedTexts.get(sessionId);
    if (!sessionEdits || sessionEdits.size === 0 || isSaving(sessionId)) return;

    setSaving(sessionId, true);
    clearSaveFeedback(sessionId, true);
    const submittedEdits = new Map(sessionEdits);
    const corrections = Array.from(submittedEdits.entries()).map(
      ([segment_index, corrected_text]) => ({ segment_index, corrected_text })
    );
    try {
      try {
        await saveCorrections(sessionId, corrections);
      } catch (error) {
        showSaveMessage(`Save failed: ${formatError(error)}`, true, sessionId);
        return;
      }

      removeSubmittedEdits(sessionId, submittedEdits);
      if (destroyed) return;
      await refreshSessionsAndExpanded(false);
      showSaveMessage("Corrections saved", false, sessionId);
    } finally {
      setSaving(sessionId, false);
    }
  }

  function hasEdits(sessionId: string): boolean {
    const edits = editedTexts.get(sessionId);
    return !!edits && edits.size > 0;
  }

  function formatTimestamp(ts: string): string {
    try {
      const d = new Date(ts);
      return d.toLocaleString();
    } catch {
      return ts;
    }
  }

  function formatDuration(secs: number): string {
    const m = Math.floor(secs / 60);
    const s = Math.round(secs % 60);
    return m > 0 ? `${m}m ${s}s` : `${s}s`;
  }

  onMount(() => {
    void loadSessions();
    const unlisten = listen("profile-data-changed", async () => {
      if (destroyed) return;
      await refreshSessionsAndExpanded(true, true);
    });

    return () => {
      destroyed = true;
      sessionRequests.invalidate();
      detailRequests.invalidate();
      clearAllSaveMessageTimers();
      void unlisten.then((fn) => fn());
    };
  });
</script>

<div class="editor-view">
  <h2>Sessions</h2>

  {#each Array.from(saveFeedbacks.entries()) as [sessionId, feedback] (sessionId)}
    <p
      class="save-message editor-feedback"
      class:error={feedback.error}
      role={feedback.error ? "alert" : undefined}
    >
      {feedback.message}{#if feedback.error}
        <span class="save-session"> (Session: {sessionId})</span>
      {/if}
    </p>
  {/each}

  {#if loading}
    <div class="empty-state">Loading sessions...</div>
  {:else if sessions.length === 0}
    <div class="empty-state">
      <p>No dictation sessions yet.</p>
      <p class="muted">Sessions will appear here after you start dictating.</p>
    </div>
  {:else}
    <div class="session-list">
      {#each sessions as session}
        <div class="session-card" class:expanded={expandedId === session.id}>
          <button class="session-header" onclick={() => toggleSession(session.id)}>
            <div class="session-meta">
              <span class="session-time">{formatTimestamp(session.created_at)}</span>
              <span class="session-segments">
                {session.segment_count} segment{session.segment_count !== 1 ? "s" : ""}
                &middot; {formatDuration(session.duration)}
                {#if session.reviewed}<span class="reviewed-badge">Reviewed</span>{/if}
              </span>
            </div>
            <span class="expand-icon">{expandedId === session.id ? "\u25B2" : "\u25BC"}</span>
          </button>

          {#if expandedId === session.id}
            <div class="session-body">
              {#if loadingDetail}
                <div class="empty-state">Loading segments...</div>
              {:else if expandedDetail}
                {#each expandedDetail.segments as segment}
                  <div class="segment">
                    <div class="segment-header">
                      <span class="segment-time">{segment.start_time.toFixed(1)}s - {segment.end_time.toFixed(1)}s</span>
                    </div>
                    <div class="segment-texts">
                      <div class="text-block">
                        <span class="field-label">Original</span>
                        <div class="text-display">{segment.original_text}</div>
                      </div>
                      <div class="text-block">
                        <label class="field-label" for="correction-{session.id}-{segment.segment_index}">Corrected</label>
                        <textarea
                          id="correction-{session.id}-{segment.segment_index}"
                          class="text-input"
                          value={getEditedText(
                            session.id,
                            segment.segment_index,
                            segment.corrected_text || segment.original_text
                          )}
                          oninput={(e) =>
                            updateSegmentText(
                              session.id,
                              segment.segment_index,
                              (e.target as HTMLTextAreaElement).value
                            )
                          }
                          rows="2"
                        ></textarea>
                      </div>
                    </div>
                  </div>
                {/each}

                <div class="session-actions">
                  <button
                    class="save-btn"
                    onclick={() => handleSave(session.id)}
                    disabled={isSaving(session.id) || !hasEdits(session.id)}
                  >
                    {isSaving(session.id) ? "Saving..." : "Save Corrections"}
                  </button>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .editor-view {
    max-width: 800px;
  }

  h2 {
    margin: 0 0 24px;
    font-size: 24px;
    color: var(--text, #e6edf3);
  }

  .empty-state {
    text-align: center;
    padding: 48px 24px;
    color: var(--text-muted, #8b949e);
  }

  .empty-state .muted {
    font-size: 13px;
    margin-top: 8px;
  }

  .session-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .session-card {
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 8px;
    overflow: hidden;
  }

  .session-card.expanded {
    border-color: var(--accent, #58a6ff);
  }

  .session-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: 12px 16px;
    background: none;
    border: none;
    color: var(--text, #c9d1d9);
    cursor: pointer;
    font-size: 14px;
    text-align: left;
  }

  .session-header:hover {
    background: #21262d;
  }

  .session-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .session-time {
    font-weight: 500;
  }

  .session-segments {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
  }

  .expand-icon {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
  }

  .session-body {
    border-top: 1px solid var(--border, #30363d);
    padding: 16px;
  }

  .segment {
    margin-bottom: 16px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border, #30363d);
  }

  .segment:last-child {
    margin-bottom: 0;
    padding-bottom: 0;
    border-bottom: none;
  }

  .segment-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }

  .segment-time {
    font-size: 12px;
    color: var(--text-muted, #8b949e);
  }

  .segment-texts {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .text-block .field-label {
    display: block;
    font-size: 11px;
    color: var(--text-muted, #8b949e);
    text-transform: uppercase;
    margin-bottom: 4px;
  }

  .text-display {
    background: var(--bg, #0d1117);
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 8px 12px;
    font-size: 14px;
    color: var(--text-muted, #8b949e);
  }

  .text-input {
    width: 100%;
    background: var(--bg, #0d1117);
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 8px 12px;
    font-size: 14px;
    color: var(--text, #c9d1d9);
    font-family: inherit;
    resize: vertical;
    outline: none;
  }

  .text-input:focus {
    border-color: var(--accent, #58a6ff);
  }

  .session-actions {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 16px;
  }

  .save-btn {
    background: var(--accent-dark, #1f6feb);
    color: #fff;
    border: none;
    padding: 8px 16px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    transition: opacity 0.15s;
  }

  .save-btn:hover:not(:disabled) {
    opacity: 0.9;
  }

  .save-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .save-message {
    font-size: 13px;
    color: var(--green, #3fb950);
  }

  .editor-feedback {
    margin: -12px 0 20px;
  }

  .save-message.error {
    color: var(--red, #f85149);
  }

  .save-session {
    font-weight: 600;
  }
</style>
