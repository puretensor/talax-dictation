<script lang="ts">
  let { oncomplete }: { oncomplete: () => void } = $props();
</script>

<div class="onboarding">
  <div class="onboarding-card">
    <div class="ready-icon">✓</div>
    <h2>Welcome to TalaX</h2>
    <p class="step-desc">
      TalaX is ready for local dictation. Download a Whisper model in Settings,
      confirm microphone and hotkey readiness, then start dictating.
    </p>

    <div class="capabilities">
      <div class="capability">
        <strong>What works now</strong>
        <span>Push-to-talk dictation, review/save learning, per-profile corrections</span>
      </div>
      <div class="capability">
        <strong>Recommended setup</strong>
        <span>Clipboard Paste on most platforms; Clipboard Only on restrictive desktop sessions</span>
      </div>
    </div>

    <div class="ready-actions">
      <button class="btn primary large" onclick={oncomplete}>
        Open Dictation
      </button>
    </div>
  </div>
</div>

<style>
  .onboarding {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 24px;
    background: var(--bg, #0d1117);
  }

  .onboarding-card {
    background: var(--bg-secondary, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 16px;
    padding: 40px;
    max-width: 560px;
    width: 100%;
    text-align: center;
  }

  .ready-icon {
    width: 56px;
    height: 56px;
    margin: 0 auto 20px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: rgba(63, 185, 80, 0.12);
    color: var(--green, #3fb950);
    font-size: 28px;
    font-weight: 700;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 28px;
    color: var(--text, #e6edf3);
  }

  .step-desc {
    margin: 0 auto 24px;
    max-width: 460px;
    color: var(--text-muted, #8b949e);
    line-height: 1.6;
  }

  .capabilities {
    display: grid;
    gap: 12px;
    margin-bottom: 28px;
    text-align: left;
  }

  .capability {
    padding: 14px 16px;
    border-radius: 10px;
    background: #0d1117;
    border: 1px solid var(--border, #30363d);
    display: grid;
    gap: 6px;
  }

  .capability strong {
    color: var(--text, #c9d1d9);
    font-size: 14px;
  }

  .capability span {
    color: var(--text-muted, #8b949e);
    font-size: 13px;
  }

  .ready-actions {
    display: flex;
    justify-content: center;
  }

  .btn {
    background: #21262d;
    border: 1px solid var(--border, #30363d);
    color: var(--text, #c9d1d9);
    padding: 10px 18px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 14px;
  }

  .btn.primary {
    background: var(--accent-dark, #1f6feb);
    border-color: var(--accent-dark, #1f6feb);
    color: white;
  }

  .btn.large {
    min-width: 180px;
  }
</style>
