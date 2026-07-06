>  **Public distribution repo.** This is the public, BSL-1.1 release cut of **TalaX**.
>  Development happens in the private `puretensor/TalaX` repo; reviewed releases land here as
>  snapshot commits. Same product — one version line across both repos.

# TalaX

[![Latest release](https://img.shields.io/github/v/release/puretensor/talax-dictation)](https://github.com/puretensor/talax-dictation/releases/latest)
[![License: BSL 1.1](https://img.shields.io/badge/license-BSL%201.1-blue)](LICENSE)

**Adaptive dictation for developers.** Local Whisper transcription with a 3-layer correction pipeline that improves as you use it.

Built by [PureTensor](https://puretensor.ai) · [talax.puretensor.ai](https://talax.puretensor.ai)

*Tala* -- Icelandic for "to speak."

- **Learns your vocabulary** -- corrections are persisted to SQLite; after 3 consistent fixes for the same word, corrections auto-apply
- **3-layer correction pipeline** -- dictionary substitution (<1ms), n-gram context scoring (<50ms), heuristic expansion (<10ms)
- **Fully local** -- whisper.cpp via whisper-rs, no audio leaves your machine, no subscription
- **Voice profiles** -- separate correction databases per context (work, personal, project-specific)

## Install

Prebuilt packages for **Linux x86_64** (deb, rpm, AppImage) are on the
[releases page](https://github.com/puretensor/talax-dictation/releases/latest), alongside
`SHA256SUMS.txt` for integrity verification.

```bash
# Verify checksums (optional)
sha256sum -c SHA256SUMS.txt --ignore-missing

# Debian / Ubuntu
sudo apt install ./TalaX_*_amd64.deb

# Fedora / RHEL
sudo dnf install ./TalaX-*.x86_64.rpm

# AppImage (any distro)
chmod +x TalaX_*_amd64.AppImage && ./TalaX_*_amd64.AppImage
```

macOS and Windows are source-build only for now -- see [Development](#development).

## Development Status

TalaX is functional but early-stage. Here is what works today and what is still in progress.

**Working:**
- Local Whisper transcription (whisper.cpp via whisper-rs, CPU-only)
- Push-to-talk recording with energy-based VAD and pre-roll buffer
- L1 Dictionary correction -- regex substitution from learned patterns, auto-apply after 3 consistent fixes
- L3 Heuristic correction -- Levenshtein fuzzy matching, Double Metaphone phonetic matching, compound word detection, acronym restoration, number normalization
- SQLite persistence -- sessions, segments, and correction patterns survive app restarts
- Voice profiles with independent correction databases
- Text injection via clipboard paste or keystroke simulation
- Model download with progress tracking and integrity verification
- Tauri v2 desktop app with Svelte 5 frontend (7 views)

**Working but cold-start dependent:**
- L2 N-gram correction -- the interpolated trigram model is fully implemented (training, scoring, save/load, apply), but it only activates after you have reviewed and corrected enough transcriptions to build a meaningful language model. On a fresh profile, L2 is inert.
- The correction feedback loop is wired end-to-end: frontend edit -> `save_corrections` IPC -> word-level diff -> pattern extraction -> auto-apply flag rebuild -> pipeline reload (including n-gram retrain). This works within and across app sessions. However, effectiveness scales with usage -- a fresh install has no training data.

**Planned / not yet implemented:**
- Cross-profile pattern sharing
- Audio excerpt extraction for fine-tuning
- Batch correction review workflows
- macOS and Windows packaging (Linux deb/rpm/AppImage ship since v1.2.0; other platforms build from source)

## Architecture

```
Tauri v2 (Svelte 5 frontend, Rust backend)
  |
  +-- whisper.cpp (whisper-rs) -- local STT, CPU-only
  |     tiny.en bundled, small.en-q5_1 recommended
  |
  +-- 3-Layer Correction Pipeline
  |     L1: Dictionary    -- regex substitution from learned patterns
  |     L2: N-gram        -- interpolated trigram model (0.6/0.3/0.1)
  |     L3: Heuristic     -- Levenshtein, Double Metaphone, compounds
  |
  +-- SQLite (WAL) -- corrections, sessions, learning loop
  |
  +-- System Integration
        Global hotkey (rdev) -- push-to-talk
        Text injection (arboard + enigo) -- clipboard or keystroke
        Audio capture (cpal) -- 16kHz mono with energy VAD
        System tray -- recording indicator
```

## How It Works

1. Hold the hotkey (default: `Ctrl+Shift+Space`)
2. Speak naturally
3. Release
4. TalaX transcribes locally, runs corrections, and places the result on your clipboard to review and paste

By default TalaX uses **review-first / clipboard-only** delivery: the corrected text is copied to your clipboard and you paste it yourself. Auto-inject (simulated paste) and keystroke type-out are opt-in. They inject into whatever window holds focus at that moment, so if focus moves during transcription the text can land in the wrong application -- only enable them once you trust your workflow.

When you review and correct a transcription, the diff is extracted at the word level and stored as correction patterns. Patterns that recur 3+ times with high confidence are promoted to auto-apply. The n-gram model retrains on your reviewed corpus each time the pipeline reloads, improving context-aware corrections over time.

## Correction Pipeline

**Layer 1 -- Dictionary:** Exact regex substitutions from learned patterns. Longest-match-first ordering, automatic case preservation.

**Layer 2 -- N-gram:** Interpolated trigram model (0.6 tri + 0.3 bi + 0.1 uni) trained on your reviewed correction history. Flags low-probability words given their context. Starts inert on a fresh profile and activates as you build up reviewed transcriptions.

**Layer 3 -- Heuristic Expander:** Catches what the first two layers miss:
- Fuzzy matching (Levenshtein distance <= 2)
- Phonetic matching (Double Metaphone)
- Compound word detection (split/join)
- Acronym restoration (lowercase to uppercase)
- Number normalization ("v two" to "v2", "cpu dash node zero" to "cpu-node-0")

## Voice Profiles

Separate correction databases per context. Switch freely between profiles.
Config, profiles, and models now live under the platform-managed Tauri config/data
directories rather than a hard-coded path.

```
<app-config-dir>/
    config.toml

<app-data-dir>/
    profiles/
        default/
            corrections.db
            ngram.json
            domain_context.json
            profile.toml
        work-devops/
            ...
    models/
        ggml-small.en-q5_1.bin
        ...
```

## Whisper Models

Downloaded from HuggingFace on first use with progress tracking and integrity verification.

| Model | Size | Speed | Accuracy |
|-------|------|-------|----------|
| tiny.en | 75 MB | Fastest | Basic |
| base.en | 142 MB | Fast | Moderate |
| small.en-q5_1 | 181 MB | Balanced | Good (recommended) |
| medium.en-q5_0 | 515 MB | Slower | High |
| large-v3-turbo-q5_0 | 574 MB | Slowest | Highest |

## Development

### Prerequisites

- Rust (stable, edition 2024) with `cargo install tauri-cli`
- Node.js 24+ recommended
- Linux system libraries: `libasound2-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`,
  `libwebkit2gtk-4.1-dev`, `libx11-dev`, `libxtst-dev`, `cmake`, `pkg-config`

### Build and Run

```bash
git clone https://github.com/puretensor/talax-dictation.git
cd talax-dictation
npm --prefix ui install

# Development mode (run from the app crate)
cd crates/talax-app && cargo tauri dev

# Release bundle (deb/rpm/AppImage on Linux)
cd crates/talax-app && cargo tauri build

# Engine tests (from the repo root)
cargo test --workspace

# Type-check + unit-test frontend
npm --prefix ui run check && npm --prefix ui test
```

### Test Coverage

The suite currently includes 138 engine unit tests, 38 engine integration tests, 6 app unit
tests (including config validation), frontend unit tests (vitest), and doctests. Coverage focuses on:

| Area | Covers |
|------|--------|
| Dictionary corrector | Word boundaries, case preservation, longest-match, contextual rules |
| N-gram corrector | Training, JSON save/load, scoring |
| Heuristic expander | Levenshtein, Metaphone, compounds, acronyms, numbers |
| Database | Corrections, patterns, auto-apply, domain context, fallible reads |
| Profiles | CRUD, clone independence + atomic clone, reset |
| Audio (VAD + buffer) | Energy detection, state transitions, ring buffer |
| Whisper | Params, conversion, serialization |
| Hotkey | Parse, key mapping, validation |
| Inject | Config, serialization, mode handling, safe defaults |
| App commands | Config validation (hotkey/modes/ranges/profile name) |
| Integration | Full pipeline, multi-layer, reload, end-to-end learning loop |
| Frontend | IPC failure fallbacks (`ui/src/lib/api.test.ts`) |

## Release Validation

- Cross-platform smoke runbook: `tasks/platform_smoke_runbook.md`
- Smoke report template: `tasks/platform_smoke_report_template.md`
- Generated Tauri schema output under `crates/talax-app/gen/` is treated as
  disposable generated output and is not committed.

## Tech Stack

| Component | Role |
|-----------|------|
| Tauri v2 | Desktop app shell |
| Svelte 5 | Frontend UI (7 views: Dictate, Editor, Profiles, Patterns, Stats, Settings, Onboarding) |
| whisper-rs (whisper.cpp) | Local speech-to-text |
| rusqlite | Correction database |
| rdev | Global hotkey detection |
| cpal | Audio capture with VAD |
| arboard + enigo | Text injection (clipboard paste / keystroke simulation) |

## Project Structure

```
talax/
    Cargo.toml                   # Workspace: talax-engine + talax-app
    crates/
        talax-engine/         # Core library (no UI dependency)
            src/
                audio/           # cpal capture, VAD, ring buffer
                db/              # SQLite schema, corrections, sessions
                hotkey/          # Global hotkey detection
                inject/          # Text injection
                pipeline/        # dict_corrector, ngram, heuristic expander
                profile/         # Voice profile management
                whisper/         # Transcriber + model manager
        talax-app/            # Tauri v2 application
            src/
                commands.rs      # 21 IPC command handlers
                recording.rs     # Recording state machine
                tray.rs          # System tray
    ui/                          # Svelte 5 frontend
```

## License

Business Source License 1.1. Copyright 2026 PureTensor, Inc. Converts to Apache 2.0 on 2030-03-28. See [LICENSE](LICENSE).
