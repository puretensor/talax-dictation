<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { getStats } from "../lib/api";
  import type { Stats } from "../lib/api";

  let stats: Stats = $state({
    session_count: 0,
    pattern_count: 0,
    auto_apply_count: 0,
  });
  let loading = $state(true);

  async function loadStats() {
    loading = true;
    stats = await getStats();
    loading = false;
  }

  onMount(() => {
    loadStats();
    const unlisten = listen("profile-data-changed", loadStats);
    return () => {
      unlisten.then((fn) => fn());
    };
  });
</script>

<div class="stats-view">
  <h2>Statistics</h2>

  {#if loading}
    <div class="empty-state">Loading statistics...</div>
  {:else}
    <div class="cards">
      <div class="stat-card">
        <div class="stat-value">{stats.session_count}</div>
        <div class="stat-label">Total Sessions</div>
      </div>

      <div class="stat-card">
        <div class="stat-value">{stats.pattern_count}</div>
        <div class="stat-label">Learned Patterns</div>
      </div>

      <div class="stat-card">
        <div class="stat-value">{stats.auto_apply_count}</div>
        <div class="stat-label">Auto-apply Patterns</div>
      </div>

    </div>

    {#if stats.session_count === 0}
      <div class="hint">
        <p>No data yet. Start dictating to see your statistics.</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .stats-view {
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

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 12px;
  }

  .stat-card {
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 10px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }


  .stat-value {
    font-size: 32px;
    font-weight: 700;
    color: var(--text, #e6edf3);
    line-height: 1;
  }


  .stat-label {
    font-size: 13px;
    color: var(--text-muted, #8b949e);
    font-weight: 500;
  }

  .hint {
    text-align: center;
    padding: 32px 24px;
    color: var(--text-muted, #8b949e);
    font-size: 14px;
  }
</style>
