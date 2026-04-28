<script lang="ts">
  import { getAppConfig, getProfiles, switchProfile } from "./lib/api";
  import Dictate from "./routes/Dictate.svelte";
  import Editor from "./routes/Editor.svelte";
  import Patterns from "./routes/Patterns.svelte";
  import Profiles from "./routes/Profiles.svelte";
  import Stats from "./routes/Stats.svelte";
  import Settings from "./routes/Settings.svelte";
  import Onboarding from "./routes/Onboarding.svelte";

  let currentView = $state("dictate");
  let profiles: string[] = $state([]);
  let activeProfile = $state("default");
  let showOnboarding = $state(false);

  async function loadProfiles() {
    const [availableProfiles, config] = await Promise.all([
      getProfiles(),
      getAppConfig(),
    ]);
    profiles = availableProfiles;
    if (profiles.length > 0) {
      activeProfile = profiles.includes(config.active_profile)
        ? config.active_profile
        : profiles[0];
    }
  }

  async function handleProfileChange(nextProfile: string) {
    if (!nextProfile || nextProfile === activeProfile) return;
    await switchProfile(nextProfile);
    activeProfile = nextProfile;
  }

  function completeOnboarding() {
    showOnboarding = false;
    loadProfiles();
  }

  loadProfiles();
</script>

{#if showOnboarding}
  <Onboarding oncomplete={completeOnboarding} />
{:else}
  <div class="app">
    <nav class="sidebar">
      <div class="logo">
        <h1>TalaX</h1>
        <span class="subtitle">Adaptive Dictation</span>
      </div>

      <ul class="nav-items">
        <li class:active={currentView === "dictate"}>
          <button onclick={() => (currentView = "dictate")}>Dictate</button>
        </li>
        <li class:active={currentView === "editor"}>
          <button onclick={() => (currentView = "editor")}>Sessions</button>
        </li>
        <li class:active={currentView === "patterns"}>
          <button onclick={() => (currentView = "patterns")}>Patterns</button>
        </li>
        <li class:active={currentView === "profiles"}>
          <button onclick={() => (currentView = "profiles")}>Profiles</button>
        </li>
        <li class:active={currentView === "stats"}>
          <button onclick={() => (currentView = "stats")}>Stats</button>
        </li>
        <li class:active={currentView === "settings"}>
          <button onclick={() => (currentView = "settings")}>Settings</button>
        </li>
      </ul>

      <div class="profile-select">
        <label for="profile-selector">Profile</label>
        <select
          id="profile-selector"
          value={activeProfile}
          onchange={(e) =>
            handleProfileChange((e.target as HTMLSelectElement).value)}
        >
          {#each profiles as p}
            <option value={p}>{p}</option>
          {/each}
        </select>
      </div>
    </nav>

    <main class="content">
      {#key `${currentView}:${activeProfile}`}
        {#if currentView === "dictate"}
          <Dictate />
        {:else if currentView === "editor"}
          <Editor />
        {:else if currentView === "patterns"}
          <Patterns />
        {:else if currentView === "profiles"}
          <Profiles />
        {:else if currentView === "stats"}
          <Stats />
        {:else if currentView === "settings"}
          <Settings />
        {/if}
      {/key}
    </main>
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #0d1117;
    color: #c9d1d9;
  }

  .app {
    display: flex;
    height: 100vh;
  }

  .sidebar {
    width: 200px;
    background: #161b22;
    border-right: 1px solid #30363d;
    display: flex;
    flex-direction: column;
    padding: 16px 0;
  }

  .logo {
    padding: 0 16px 16px;
    border-bottom: 1px solid #30363d;
  }

  .logo h1 {
    margin: 0;
    font-size: 18px;
    color: #58a6ff;
  }

  .subtitle {
    font-size: 12px;
    color: #8b949e;
  }

  .nav-items {
    list-style: none;
    padding: 8px 0;
    margin: 0;
    flex: 1;
  }

  .nav-items li button {
    display: block;
    width: 100%;
    padding: 8px 16px;
    background: none;
    border: none;
    color: #8b949e;
    font-size: 14px;
    text-align: left;
    cursor: pointer;
  }

  .nav-items li button:hover {
    color: #c9d1d9;
    background: #21262d;
  }

  .nav-items li.active button {
    color: #58a6ff;
    background: #1f2937;
    border-left: 3px solid #58a6ff;
  }

  .profile-select {
    padding: 12px 16px;
    border-top: 1px solid #30363d;
  }

  .profile-select label {
    display: block;
    font-size: 11px;
    color: #8b949e;
    margin-bottom: 4px;
    text-transform: uppercase;
  }

  .profile-select select {
    width: 100%;
    background: #21262d;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 4px 8px;
    font-size: 13px;
  }

  .content {
    flex: 1;
    padding: 24px;
    overflow-y: auto;
  }
</style>
