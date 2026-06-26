>  **Public distribution repo.** This is the public, BSL-1.1 release cut of **TalaX**.
>  Development happens in the private `puretensor/TalaX` repo; reviewed releases land here as
>  squashed snapshots. Same product — see TalaX/RELEASING.md for the flow. (88→30 consolidation.)

# TalaX

**Adaptive dictation for developers.** Local Whisper transcription with a 3-layer correction pipeline that improves as you use it.

Built by [PureTensor Inc](https://puretensor.ai).

*Tala* -- Icelandic for "to speak."

- **Learns your vocabulary** -- corrections are persisted to SQLite; after 3 consistent fixes for the same word, corrections auto-apply
- **3-layer correction pipeline** -- dictionary substitution (<1ms), n-gram context scoring (<50ms), heuristic expansion (<10ms)
- **Fully local** -- whisper.cpp via whisper-rs, no audio leaves your machine, no subscription
- **Voice profiles** -- separate correction databases per context (work, personal, project-specific)

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
- Platform-specific packaging and distribution

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
4. TalaX transcribes locally, runs corrections, and injects text into your active app

When you review and correct a transcription, the diff is extracted at the word level and stored as correction patterns. Patterns that recur 3+ times with high confidence are promoted to auto-apply. The n-gram model retrains on your reviewed corpus each time the pipeline reloads, improving context-aware corrections over time.

## Correction Pipeline

**Layer 1 -- Dictionary:** Exact regex substitutions from learned patterns. Longest-match-first ordering, automatic case preservation.

**Layer 2 -- N-gram:** Interpolated trigram model (0.6 tri + 0.3 bi + 0.1 uni) trained on your reviewed correction history. Flags low-probability words given their context. Starts inert on a fresh profile and activates as you build up reviewed transcriptions.

**Layer 3 -- Heuristic Expander:** Catches what the first two layers miss:
- Fuzzy matching (Levenshtein distance <= 2)
- Phonetic matching (Double Metaphone)
- Compound word detection (split/join)
- Acronym restoration (lowercase to uppercase)
- Number normalization ("v two" to "v2", "node zero" to "node0")

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

- Rust (stable, edition 2024)
- Node.js 24+ recommended
- Linux system libraries: `libasound2-dev`, `libx11-dev`, `libxtst-dev`, `cmake`, `pkg-config`

### Build and Run

```bash
git clone https://github.com/puretensor/talax-dictation.git
cd talax-dictation
cd ui && npm install && cd ..

# Development mode
cargo tauri dev

# Engine tests
cargo test -p talax-engine

# Type-check frontend
cd ui && npx svelte-check
```

### Test Coverage

The engine test suite currently includes 137 unit tests, 36 integration tests, and doctests. Coverage focuses on:

| Area | Covers |
|------|--------|
| Dictionary corrector | Word boundaries, case preservation, longest-match behavior |
| N-gram corrector | Training, save/load, scoring, vocabulary behavior |
| Heuristic expander | Levenshtein, Metaphone, compounds, acronyms, numbers |
| Database | Corrections, patterns, auto-apply, domain context |
| Profiles | CRUD, clone independence, reset behavior |
| Audio | Energy detection, VAD state transitions, ring buffer behavior |
| Whisper | Params, conversion, model metadata, serialization |
| Hotkey | Parse, key mapping, validation, serialization |
| Inject | Config, serialization, mode handling |
| Integration | Full pipeline, multi-layer correction, reload behavior |

## Release Validation

- Generated Tauri schema output under `crates/talax-app/gen/` is treated as disposable generated output and is not committed.
- Before packaging a release, run `cargo test -p talax-engine`, `cd ui && npm run check`, and a manual desktop smoke test on the target platform.

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

Business Source License 1.1. Copyright 2026 PureTensor Inc. Converts to Apache 2.0 on 2030-03-28. See [LICENSE](LICENSE).
