<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { getPatterns } from "../lib/api";
  import type { CorrectionPattern } from "../lib/api";

  let patterns: CorrectionPattern[] = $state([]);
  let loading = $state(true);
  let searchQuery = $state("");
  let sortColumn: keyof CorrectionPattern = $state("frequency");
  let sortAsc = $state(false);

  async function loadPatterns() {
    loading = true;
    patterns = await getPatterns();
    loading = false;
  }

  let filtered = $derived(
    patterns.filter((p) => {
      if (!searchQuery) return true;
      const q = searchQuery.toLowerCase();
      return (
        p.original.toLowerCase().includes(q) ||
        p.corrected.toLowerCase().includes(q)
      );
    })
  );

  let sorted = $derived(
    [...filtered].sort((a, b) => {
      const aVal = a[sortColumn];
      const bVal = b[sortColumn];
      if (typeof aVal === "string" && typeof bVal === "string") {
        return sortAsc ? aVal.localeCompare(bVal) : bVal.localeCompare(aVal);
      }
      if (typeof aVal === "number" && typeof bVal === "number") {
        return sortAsc ? aVal - bVal : bVal - aVal;
      }
      if (typeof aVal === "boolean" && typeof bVal === "boolean") {
        return sortAsc
          ? Number(aVal) - Number(bVal)
          : Number(bVal) - Number(aVal);
      }
      return 0;
    })
  );

  function toggleSort(col: keyof CorrectionPattern) {
    if (sortColumn === col) {
      sortAsc = !sortAsc;
    } else {
      sortColumn = col;
      sortAsc = false;
    }
  }

  function sortIndicator(col: keyof CorrectionPattern): string {
    if (sortColumn !== col) return "";
    return sortAsc ? " \u25B2" : " \u25BC";
  }

  onMount(() => {
    loadPatterns();
    const unlisten = listen("profile-data-changed", loadPatterns);
    return () => {
      unlisten.then((fn) => fn());
    };
  });
</script>

<div class="patterns-view">
  <h2>Learned Patterns</h2>

  <div class="toolbar">
    <input
      class="search-input"
      type="text"
      placeholder="Search patterns..."
      bind:value={searchQuery}
    />
    <span class="count">{sorted.length} pattern{sorted.length !== 1 ? "s" : ""}</span>
  </div>

  {#if loading}
    <div class="empty-state">Loading patterns...</div>
  {:else if patterns.length === 0}
    <div class="empty-state">
      <p>No correction patterns yet.</p>
      <p class="muted">Patterns are learned automatically as you correct transcriptions.</p>
    </div>
  {:else if sorted.length === 0}
    <div class="empty-state">
      <p>No patterns match "{searchQuery}"</p>
    </div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th class="sortable" onclick={() => toggleSort("original")}>
              Original{sortIndicator("original")}
            </th>
            <th class="sortable" onclick={() => toggleSort("corrected")}>
              Corrected{sortIndicator("corrected")}
            </th>
            <th class="sortable num" onclick={() => toggleSort("frequency")}>
              Freq{sortIndicator("frequency")}
            </th>
            <th class="sortable num" onclick={() => toggleSort("confidence")}>
              Conf{sortIndicator("confidence")}
            </th>
            <th class="center">Auto</th>
          </tr>
        </thead>
        <tbody>
          {#each sorted as pattern}
            <tr>
              <td class="mono">{pattern.original}</td>
              <td class="mono corrected">{pattern.corrected}</td>
              <td class="num">{pattern.frequency}</td>
              <td class="num">{(pattern.confidence * 100).toFixed(0)}%</td>
              <td class="center">
                <span class="badge" class:active={pattern.confidence >= 0.75 && pattern.frequency >= 3}>
                  {pattern.confidence >= 0.75 && pattern.frequency >= 3 ? "Yes" : "No"}
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .patterns-view {
    max-width: 900px;
  }

  h2 {
    margin: 0 0 24px;
    font-size: 24px;
    color: var(--text, #e6edf3);
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 16px;
  }

  .search-input {
    flex: 1;
    max-width: 360px;
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text, #c9d1d9);
    font-size: 14px;
    outline: none;
  }

  .search-input:focus {
    border-color: var(--accent, #58a6ff);
  }

  .search-input::placeholder {
    color: var(--text-muted, #8b949e);
  }

  .count {
    font-size: 13px;
    color: var(--text-muted, #8b949e);
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

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 14px;
  }

  thead {
    border-bottom: 2px solid var(--border, #30363d);
  }

  th {
    text-align: left;
    padding: 10px 12px;
    color: var(--text-muted, #8b949e);
    font-weight: 600;
    font-size: 12px;
    text-transform: uppercase;
    white-space: nowrap;
    user-select: none;
  }

  th.sortable {
    cursor: pointer;
  }

  th.sortable:hover {
    color: var(--text, #c9d1d9);
  }

  th.num,
  td.num {
    text-align: right;
  }

  th.center,
  td.center {
    text-align: center;
  }

  td {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border, #30363d);
    color: var(--text, #c9d1d9);
  }

  tr:hover td {
    background: rgba(88, 166, 255, 0.04);
  }

  .mono {
    font-family: "SF Mono", "Fira Code", monospace;
    font-size: 13px;
  }

  .corrected {
    color: var(--green, #3fb950);
  }

  .badge {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 10px;
    font-size: 12px;
    background: #21262d;
    color: var(--text-muted, #8b949e);
  }

  .badge.active {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green, #3fb950);
  }
</style>
