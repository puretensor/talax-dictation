<script lang="ts">
  import {
    getProfiles,
    createProfile,
    switchProfile,
    cloneProfile,
    deleteProfile,
    resetProfile,
    getAppConfig,
  } from "../lib/api";
  let profiles: string[] = $state([]);
  let activeProfile = $state("default");
  let loading = $state(true);

  // Modal state
  let showCreateModal = $state(false);
  let showCloneModal = $state(false);
  let showConfirmModal = $state(false);
  let newName = $state("");
  let cloneSource = $state("");
  let cloneName = $state("");
  let confirmAction = $state<{ label: string; action: () => Promise<void> } | null>(null);
  let actionMessage = $state("");

  async function loadProfiles() {
    loading = true;
    const [p, cfg] = await Promise.all([getProfiles(), getAppConfig()]);
    profiles = p;
    activeProfile = cfg.active_profile;
    loading = false;
  }

  loadProfiles();

  async function handleCreate() {
    if (!newName.trim()) return;
    await createProfile(newName.trim());
    newName = "";
    showCreateModal = false;
    await loadProfiles();
    showMessage("Profile created");
  }

  async function handleSwitch(name: string) {
    await switchProfile(name);
    activeProfile = name;
    showMessage(`Switched to "${name}"`);
  }

  function openClone(source: string) {
    cloneSource = source;
    cloneName = source + "-copy";
    showCloneModal = true;
  }

  async function handleClone() {
    if (!cloneName.trim()) return;
    await cloneProfile(cloneSource, cloneName.trim());
    cloneName = "";
    showCloneModal = false;
    await loadProfiles();
    showMessage("Profile cloned");
  }

  function confirmDelete(name: string) {
    confirmAction = {
      label: `Delete profile "${name}"? This cannot be undone.`,
      action: async () => {
        await deleteProfile(name);
        await loadProfiles();
        showMessage("Profile deleted");
      },
    };
    showConfirmModal = true;
  }

  function confirmReset(name: string) {
    confirmAction = {
      label: `Reset profile "${name}"? All learned patterns and settings will be cleared.`,
      action: async () => {
        await resetProfile(name);
        await loadProfiles();
        showMessage("Profile reset");
      },
    };
    showConfirmModal = true;
  }

  async function executeConfirm() {
    if (confirmAction) {
      await confirmAction.action();
    }
    showConfirmModal = false;
    confirmAction = null;
  }

  function cancelConfirm() {
    showConfirmModal = false;
    confirmAction = null;
  }

  function showMessage(msg: string) {
    actionMessage = msg;
    setTimeout(() => (actionMessage = ""), 2000);
  }

  function handleCreateKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleCreate();
    else if (e.key === "Escape") showCreateModal = false;
  }

  function handleCloneKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleClone();
    else if (e.key === "Escape") showCloneModal = false;
  }
</script>

<div class="profiles-view">
  <div class="header-row">
    <h2>Voice Profiles</h2>
    <div class="header-actions">
      {#if actionMessage}
        <span class="action-msg">{actionMessage}</span>
      {/if}
      <button class="btn primary" onclick={() => (showCreateModal = true)}>
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
              <button class="btn small" onclick={() => handleSwitch(name)}>
                Switch
              </button>
            {/if}
            <button class="btn small" onclick={() => openClone(name)}>
              Clone
            </button>
            <button class="btn small" onclick={() => confirmReset(name)}>
              Reset
            </button>
            {#if name !== activeProfile && name !== "default"}
              <button
                class="btn small danger"
                onclick={() => confirmDelete(name)}
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
  <div class="modal-overlay" onclick={() => (showCreateModal = false)} onkeydown={(e) => { if (e.key === 'Escape') showCreateModal = false; }} role="presentation">
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
      <div class="modal-actions">
        <button class="btn" onclick={() => (showCreateModal = false)}>Cancel</button>
        <button class="btn primary" onclick={handleCreate} disabled={!newName.trim()}>
          Create
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Clone Modal -->
{#if showCloneModal}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="modal-overlay" onclick={() => (showCloneModal = false)} onkeydown={(e) => { if (e.key === 'Escape') showCloneModal = false; }} role="presentation">
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
      <div class="modal-actions">
        <button class="btn" onclick={() => (showCloneModal = false)}>Cancel</button>
        <button class="btn primary" onclick={handleClone} disabled={!cloneName.trim()}>
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
      <div class="modal-actions">
        <button class="btn" onclick={cancelConfirm}>Cancel</button>
        <button class="btn danger" onclick={executeConfirm}>Confirm</button>
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

  .btn.danger:hover {
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

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
