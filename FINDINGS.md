# Documentation truth sweep — talax-dictation

Sweep date: 2026-08-31. Baseline commit: `30f9f42`. Scope: `README.md` (no `docs/` directory). Verification used source reads and read-only local commands (`cargo test --workspace`, `npm test` after `npm ci`).

| Claim | Status | Evidence / change |
|-------|--------|-------------------|
| GitHub release badge URL (`puretensor/talax-dictation`) | OK | Repo remote matches; badge target is external. |
| License badge BSL 1.1 → `LICENSE` | OK | `LICENSE` is Business Source License 1.1; `Cargo.toml` `license = "BSL-1.1"`. |
| Corrections auto-apply after 3 consistent fixes | OK | `crates/talax-engine/src/db/mod.rs:346-347` — `frequency >= 3 AND confidence >= 0.75`. |
| 3-layer pipeline (dictionary, n-gram, heuristic) | OK | `crates/talax-engine/src/pipeline/mod.rs` — `dict_corrector`, `ngram_corrector`, `heuristic`. |
| Local Whisper via whisper-rs | OK | `crates/talax-engine/Cargo.toml:16` — `whisper-rs = "0.16"`. |
| Linux prebuilt deb/rpm/AppImage install examples | OK | `crates/talax-app/tauri.conf.json` — `productName: "TalaX"`, `bundle.active: true`, `targets: "all"`. |
| `SHA256SUMS.txt` on releases page | OK | Referenced in `RELEASING.md:81`; external release artifact (not in tree). |
| macOS/Windows source-build only | OK | `.github/workflows/ci.yml` — `ubuntu-latest` only; no macOS/Windows packaging workflow. |
| CPU-only Whisper transcription | OK | No GPU/CUDA/Metal flags in `crates/talax-engine/src/whisper/transcriber.rs`; thread-count only. |
| Push-to-talk with energy VAD and pre-roll | OK | `crates/talax-engine/src/audio/vad.rs`, `ring_buffer.rs`; `AppConfig.pre_roll_ms` default 300 in `commands.rs:50`. |
| L1 dictionary regex substitution, auto-apply threshold | OK | `crates/talax-engine/src/pipeline/dict_corrector.rs`; auto-apply in `db/mod.rs:346-347`. |
| L3 heuristic: Levenshtein, Double Metaphone, compounds, acronyms, numbers | OK | `crates/talax-engine/src/pipeline/heuristic.rs` — `levenshtein`, `double_metaphone`, compound/number helpers. |
| Levenshtein distance ≤ 2 | OK | `heuristic.rs:240-241` — `max_dist` is 1 for words ≤4 chars, else 2. |
| SQLite persistence (sessions, segments, patterns) | OK | `crates/talax-engine/src/db/mod.rs` schema + APIs. |
| SQLite WAL mode | OK | `db/mod.rs:107` — `PRAGMA journal_mode=WAL`. |
| Voice profiles with independent correction DBs | OK | `crates/talax-engine/src/profile/mod.rs` — per-profile `corrections.db`. |
| Text injection: clipboard paste or keystroke simulation | OK | `crates/talax-engine/src/inject/mod.rs` — `Clipboard`, `TypeOut`, `ClipboardOnly` modes. |
| Default review-first / clipboard-only delivery | OK | `commands.rs:42-47` — `review_mode: "review_first"`, `injection_strategy: "clipboard_only"`; `InjectionMode` default `ClipboardOnly` in `inject/mod.rs:55-56`. |
| Model download with progress and integrity verification | OK | `crates/talax-engine/src/whisper/model_manager.rs` — size + SHA-256 checks. |
| Tauri v2 desktop app | OK | `crates/talax-app/Cargo.toml:21` — `tauri = { version = "2" }`. |
| Svelte 5 frontend | OK | `ui/package.json:22` — `"svelte": "^5.56.8"`. |
| Frontend has 7 views including Onboarding | CODE-GAP | `ui/src/routes/Onboarding.svelte` exists but is not imported or routed in `ui/src/App.svelte:6-11,114-131` (6 nav items only). |
| L2 n-gram cold-start dependent | OK | `ngram_corrector.rs:209` weights; pipeline reload in `commands.rs:373-374`. |
| `save_corrections` IPC → learning loop | OK | `commands.rs:735` handler; `ui/src/lib/api.ts:205-209` invokes `save_corrections`. |
| Planned: cross-profile pattern sharing | OK | No implementation outside README planned list. |
| Planned: audio excerpt extraction | OK | `audio_excerpts` table in `db/mod.rs:60-69` only; no extraction logic elsewhere. |
| Planned: batch correction review workflows | OK | No batch-review workflow in UI or backend. |
| Linux packaging since v1.2.0 | OK | Local tag `v1.2.0` present in repo. |
| N-gram weights 0.6 / 0.3 / 0.1 | OK | `ngram_corrector.rs:209`. |
| Default hotkey `Ctrl+Shift+Space` | OK | `commands.rs:40`; `hotkey/mod.rs:483-485`. |
| Profile layout: `config.toml` under app-config-dir | OK | `commands.rs:172-187`, `lib.rs:12-18`. |
| Profile layout: `profiles/<name>/corrections.db`, `ngram.json`, `domain_context.json`, `profile.toml` | OK | `profile/mod.rs:4-7,103-118`; `ngram.json` path set in `commands.rs:369-373` (created on train/save, not at profile creation). |
| Models under `<app-data-dir>/models/` | OK | `ModelManager` uses app data dir via `commands.rs` / `lib.rs:21`. |
| Whisper models table (names and approximate sizes) | OK | `model_manager.rs:54-96` — five listed models match README rows; `small.en` also exists in code but is omitted from README table (not claimed). |
| Models downloaded from HuggingFace | OK | `model_manager.rs:44` — `HF_BASE` URL. |
| `tiny.en` bundled with the app | FIXED | No `.bin` models in tree; `tauri.conf.json` has no bundled resources. README architecture line now says models are downloaded on first use. |
| Rust edition 2024 | OK | `Cargo.toml:7`. |
| `cargo install tauri-cli` prerequisite | OK | Standard Tauri v2 dev workflow; `cargo tauri dev` / `build` used in README and CI-adjacent docs. |
| Node.js 24+ recommended | OK | `.github/workflows/ci.yml:34` — `node-version: '24'`. |
| Linux system library prerequisites | OK | Matches `.github/workflows/ci.yml:19-27` package list. |
| `npm --prefix ui install` | OK | `ui/package.json` exists; script name valid. |
| `cd crates/talax-app && cargo tauri dev` | OK | `tauri.conf.json:9` — `beforeDevCommand` runs UI dev server. |
| `cd crates/talax-app && cargo tauri build` | OK | `tauri.conf.json:10` — `beforeBuildCommand`; `RELEASING.md:60`. |
| `cargo test --workspace` from repo root | OK | Executed successfully (140 + 38 + 8 + doctests). |
| `npm --prefix ui run check && npm --prefix ui test` | OK | `ui/package.json` scripts `check` and `test`; 35 vitest tests passed after `npm ci`. |
| 138 engine unit tests | FIXED | `cargo test -p talax-engine` lib crate: **140** passed. README updated to 140. |
| 38 engine integration tests | OK | `cargo test -p talax-engine --test integration`: **38** passed. |
| 6 app unit tests | FIXED | `cargo test -p talax-app`: **8** passed (`commands.rs` 4 + `recording.rs` 4). README updated to 8. |
| Frontend vitest suites | OK | 35 tests in 5 files (`npm test`). |
| Doctests present | OK | Workspace doctest run: 1 passed, 1 ignored. |
| Test coverage area table (modules/paths) | OK | All listed areas map to existing source files under `crates/talax-engine` and `ui/src/lib/api.test.ts`. |
| `RELEASING.md` linked and exists | OK | File at repo root; CI gate commands match `.github/workflows/ci.yml`. |
| `crates/talax-app/gen/` disposable, not committed | OK | Directory absent from working tree; `.gitignore` / release docs treat as generated. |
| Tech stack dependency crates (rusqlite, rdev, cpal, arboard, enigo) | OK | `crates/talax-engine/Cargo.toml:10-19`. |
| Project structure paths | OK | All paths in README tree exist (`Cargo.toml`, `crates/talax-engine/src/{audio,db,hotkey,inject,pipeline,profile,whisper}`, `crates/talax-app/src/{commands,recording,tray}.rs`, `ui/`). |
| 21 IPC command handlers | FIXED | `lib.rs:63-93` registers **23** handlers. README updated to 23. |
| License converts to Apache 2.0 on 2030-03-28 | OK | `LICENSE:9-10` — `Change Date: 2030-03-28`, `Change License: Apache License, Version 2.0`. |
| Workspace version 1.6.1 | OK | `Cargo.toml:6`, `ui/package.json:4`, `tauri.conf.json:4`. |
| `CONTRIBUTING.md` commands (`cargo fmt`, `cargo test -p talax-engine`, `npm ci`) | OK | Valid scripts; not modified (out of primary sweep scope but spot-checked). |
| `SECURITY.md` exists | OK | File at repo root. |

## CODE-GAP summary

1. **Onboarding view not wired** — `ui/src/routes/Onboarding.svelte` is implemented but never imported in `ui/src/App.svelte`; the live UI exposes six sidebar routes (Dictate, Sessions, Patterns, Profiles, Stats, Settings).
