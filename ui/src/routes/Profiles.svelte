<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    createProfile,
    switchProfile,
    cloneProfile,
    deleteProfile,
    resetProfile,
  } from "../lib/api";
  import { formatError } from "../lib/errors";

  let {
    profiles,
    activeProfile,
    loading,
    onprofilechange,
    onprofileschanged,
    onmutationpending = () => undefined,
    blocked = false,
  }: {
    profiles: string[];
    activeProfile: string;
    loading: boolean;
    onprofilechange: (activeProfile: string) => void;
    onprofileschanged: () => Promise<void>;
    onmutationpending?: (pending: boolean) => void;
    blocked?: boolean;
  } = $props();

  // Modal state
  let showCreateModal = $state(false);
  let showCloneModal = $state(false);
  let showConfirmModal = $state(false);
  let newName = $state("");
  let cloneSource = $state("");
  let cloneName = $state("");
  let confirmAction = $state<{
    label: string;
    failureLabel: string;
    successMessage: string;
    action: () => Promise<void>;
  } | null>(null);
  let actionMessage = $state("");
  let actionError = $state(false);
  let mutationPending = $state(false);
  let controlsBlocked = $derived(blocked || mutationPending);
  let actionMessageTimer: ReturnType<typeof setTimeout> | null = null;
  let destroyed = false;

  async function handleCreate(): Promise<void> {
    const name = newName.trim();
    if (!name || !beginMutation()) return;
    clearMessage();
    try {
      await createProfile(name);
      if (!destroyed) {
        newName = "";
        showCreateModal = false;
      }
      try {
        await onprofileschanged();
        showMessage("Profile created");
      } catch (error) {
        showError(`Profile created, but refresh failed: ${formatError(error)}`);
      }
    } catch (error) {
      showError(`Create profile failed: ${formatError(error)}`);
    } finally {
      endMutation();
    }
  }

  async function handleSwitch(name: string): Promise<void> {
    if (name === activeProfile || !beginMutation()) return;
    clearMessage();
    try {
      await switchProfile(name);
      onprofilechange(name);
      showMessage(`Switched to "${name}"`);
    } catch (error) {
      showError(`Switch profile failed: ${formatError(error)}`);
    } finally {
      endMutation();
    }
  }

  function openClone(source: string): void {
    if (controlsBlocked) return;
    clearMessage();
    cloneSource = source;
    cloneName = source + "-copy";
    showCloneModal = true;
  }

  async function handleClone(): Promise<void> {
    const target = cloneName.trim();
    if (!target || !beginMutation()) return;
    clearMessage();
    try {
      await cloneProfile(cloneSource, target);
      if (!destroyed) {
        cloneName = "";
        showCloneModal = false;
      }
      try {
        await onprofileschanged();
        showMessage("Profile cloned");
      } catch (error) {
        showError(`Profile cloned, but refresh failed: ${formatError(error)}`);
      }
    } catch (error) {
      showError(`Clone profile failed: ${formatError(error)}`);
    } finally {
      endMutation();
    }
  }

  function confirmDelete(name: string): void {
    if (controlsBlocked) return;
    clearMessage();
    confirmAction = {
      label: `Delete profile "${name}"? This cannot be undone.`,
      failureLabel: "Delete profile",
      successMessage: "Profile deleted",
      action: () => deleteProfile(name),
    };
    showConfirmModal = true;
  }

  function confirmReset(name: string): void {
    if (controlsBlocked) return;
    clearMessage();
    confirmAction = {
      label: `Reset profile "${name}"? All learned patterns and settings will be cleared.`,
      failureLabel: "Reset profile",
      successMessage: "Profile reset",
      action: () => resetProfile(name),
    };
    showConfirmModal = true;
  }

  async function executeConfirm(): Promise<void> {
    if (!confirmAction || !beginMutation()) return;
    const pendingAction = confirmAction;
    clearMessage();
    try {
      await pendingAction.action();
      if (!destroyed) {
        showConfirmModal = false;
        confirmAction = null;
      }
      try {
        await onprofileschanged();
        showMessage(pendingAction.successMessage);
      } catch (error) {
        showError(
          `${pendingAction.successMessage}, but refresh failed: ${formatError(error)}`
        );
      }
    } catch (error) {
      showError(`${pendingAction.failureLabel} failed: ${formatError(error)}`);
    } finally {
      endMutation();
    }
  }

  function cancelConfirm(): void {
    if (mutationPending) return;
    showConfirmModal = false;
    confirmAction = null;
  }

  function openCreate(): void {
    if (controlsBlocked) return;
    clearMessage();
    showCreateModal = true;
  }

  function closeCreate(): void {
    if (!mutationPending) showCreateModal = false;
  }

  function closeClone(): void {
    if (!mutationPending) showCloneModal = false;
  }

  function beginMutation(): boolean {
    if (blocked || mutationPending || destroyed) return false;
    mutationPending = true;
    onmutationpending(true);
    return true;
  }

  function endMutation(): void {
    if (!mutationPending) return;
    mutationPending = false;
    onmutationpending(false);
  }

  function clearMessageTimer(): void {
    if (actionMessageTimer !== null) {
      clearTimeout(actionMessageTimer);
      actionMessageTimer = null;
    }
  }

  function showMessage(msg: string): void {
    clearMessageTimer();
    if (destroyed) return;
    actionMessage = msg;
    actionError = false;
    actionMessageTimer = setTimeout(() => {
      actionMessage = "";
      actionMessageTimer = null;
    }, 2000);
  }

  function showError(msg: string): void {
    clearMessageTimer();
    if (destroyed) return;
    actionMessage = msg;
    actionError = true;
  }

  function clearMessage(): void {
    clearMessageTimer();
    actionMessage = "";
    actionError = false;
  }

  function handleCreateKeydown(e: KeyboardEvent): void {
    if (e.key === "Enter") void handleCreate();
    else if (e.key === "Escape") closeCreate();
  }

  function handleCloneKeydown(e: KeyboardEvent): void {
    if (e.key === "Enter") void handleClone();
    else if (e.key === "Escape") closeClone();
  }

  onDestroy(() => {
    destroyed = true;
    clearMessageTimer();
  });
</script>

<div class="profiles-view">
  <div class="header-row">
    <h2>Voice Profiles</h2>
    <div class="header-actions">
      {#if actionMessage && !showCreateModal && !showCloneModal && !showConfirmModal}
        <span
          class="action-msg"
          class:error={actionError}
          role={actionError ? "alert" : undefined}
        >{actionMessage}</span>
      {/if}
      <button class="btn primary" onclick={openCreate} disabled={controlsBlocked}>
        Create New
      </button>
    </div>
  </div>

  {#if loading}
    <div class="empty-state">Loading profiles...</div>
  {:else if profiles.length === 0}
    <div class="empty-state">
      <p>No profiles found. Create one to get started.</p>
    </div>
  {:else}
    <div class="profile-list">
      {#each profiles as name}
        <div class="profile-card" class:active={name === activeProfile}>
          <div class="profile-info">
            <div class="profile-name">
              {name}
              {#if name === activeProfile}
                <span class="active-badge">Active</span>
              {/if}
            </div>
          </div>
          <div class="profile-actions">
            {#if name !== activeProfile}
              <button
                class="btn small"
                onclick={() => handleSwitch(name)}
                disabled={controlsBlocked}
              >
                Switch
              </button>
            {/if}
            <button
              class="btn small"
              onclick={() => openClone(name)}
              disabled={controlsBlocked}
            >
              Clone
            </button>
            <button
              class="btn small"
              onclick={() => confirmReset(name)}
              disabled={controlsBlocked}
            >
              Reset
            </button>
            {#if name !== activeProfile && name !== "default"}
              <button
                class="btn small danger"
                onclick={() => confirmDelete(name)}
                disabled={controlsBlocked}
              >
                Delete
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- Create Modal -->
{#if showCreateModal}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="modal-overlay" onclick={closeCreate} onkeydown={(e) => { if (e.key === 'Escape') closeCreate(); }} role="presentation">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Create new profile">
      <h3>Create New Profile</h3>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="modal-input"
        type="text"
        placeholder="Profile name"
        bind:value={newName}
        onkeydown={handleCreateKeydown}
        autofocus
      />
      {#if actionError}
        <p class="modal-error" role="alert">{actionMessage}</p>
      {/if}
      <div class="modal-actions">
        <button class="btn" onclick={closeCreate} disabled={mutationPending}>Cancel</button>
        <button class="btn primary" onclick={handleCreate} disabled={!newName.trim() || controlsBlocked}>
          Create
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Clone Modal -->
{#if showCloneModal}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="modal-overlay" onclick={closeClone} onkeydown={(e) => { if (e.key === 'Escape') closeClone(); }} role="presentation">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Clone profile">
      <h3>Clone Profile "{cloneSource}"</h3>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="modal-input"
        type="text"
        placeholder="New profile name"
        bind:value={cloneName}
        onkeydown={handleCloneKeydown}
        autofocus
      />
      {#if actionError}
        <p class="modal-error" role="alert">{actionMessage}</p>
      {/if}
      <div class="modal-actions">
        <button class="btn" onclick={closeClone} disabled={mutationPending}>Cancel</button>
        <button class="btn primary" onclick={handleClone} disabled={!cloneName.trim() || controlsBlocked}>
          Clone
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Confirm Modal -->
{#if showConfirmModal && confirmAction}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="modal-overlay" onclick={cancelConfirm} onkeydown={(e) => { if (e.key === 'Escape') cancelConfirm(); }} role="presentation">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Confirm action">
      <h3>Confirm</h3>
      <p class="confirm-text">{confirmAction.label}</p>
      {#if actionError}
        <p class="modal-error" role="alert">{actionMessage}</p>
      {/if}
      <div class="modal-actions">
        <button class="btn" onclick={cancelConfirm} disabled={mutationPending}>Cancel</button>
        <button class="btn danger" onclick={executeConfirm} disabled={controlsBlocked}>
          Confirm
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .profiles-view {
    max-width: 700px;
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

  .action-msg {
    font-size: 13px;
    color: var(--green, #3fb950);
  }

  .action-msg.error {
    color: var(--red, #f85149);
  }

  .empty-state {
    text-align: center;
    padding: 48px 24px;
    color: var(--text-muted, #8b949e);
  }

  .profile-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .profile-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 8px;
    padding: 14px 16px;
  }

  .profile-card.active {
    border-color: var(--accent, #58a6ff);
  }

  .profile-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .profile-name {
    font-weight: 500;
    font-size: 15px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .active-badge {
    font-size: 11px;
    background: rgba(88, 166, 255, 0.15);
    color: var(--accent, #58a6ff);
    padding: 2px 8px;
    border-radius: 10px;
    font-weight: 600;
  }

  .profile-actions {
    display: flex;
    gap: 6px;
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

  .btn:hover {
    background: #30363d;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn.primary {
    background: var(--accent-dark, #1f6feb);
    border-color: var(--accent-dark, #1f6feb);
    color: #fff;
  }

  .btn.primary:hover:not(:disabled) {
    opacity: 0.9;
  }

  .btn.danger {
    color: var(--red, #f85149);
    border-color: var(--red, #f85149);
    background: transparent;
  }

  .btn.danger:hover:not(:disabled) {
    background: rgba(248, 81, 73, 0.1);
  }

  .btn.small {
    padding: 4px 10px;
    font-size: 12px;
  }

  /* Modals */
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .modal {
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 12px;
    padding: 24px;
    min-width: 360px;
    max-width: 480px;
  }

  .modal h3 {
    margin: 0 0 16px;
    font-size: 18px;
    color: var(--text, #e6edf3);
  }

  .modal-input {
    width: 100%;
    background: var(--bg, #0d1117);
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text, #c9d1d9);
    font-size: 14px;
    outline: none;
    margin-bottom: 16px;
  }

  .modal-input:focus {
    border-color: var(--accent, #58a6ff);
  }

  .confirm-text {
    color: var(--text, #c9d1d9);
    font-size: 14px;
    margin: 0 0 16px;
    line-height: 1.5;
  }

  .modal-error {
    color: var(--red, #f85149);
    font-size: 13px;
    margin: 0 0 16px;
  }

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
