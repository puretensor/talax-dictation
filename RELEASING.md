# TalaX Release Checklist

Use this checklist before cutting or packaging a public TalaX release from this repository.

## 1. Version and branch hygiene

- Confirm release branch is based on `main` and has no unrelated changes.
- Bump all version locations in the same commit:
  - `Cargo.toml` workspace version
  - `Cargo.lock` package entries
  - `crates/talax-app/tauri.conf.json`
  - `ui/package.json` and `ui/package-lock.json`
- Commit message must start with the version tag, for example `v0.3.0: Harden release checks`.

## 2. Required automated checks

Run these from the repository root unless noted:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check advisories bans sources
cargo test --workspace
npm --prefix ui ci
npm --prefix ui audit --audit-level=high
npm --prefix ui run check
npm --prefix ui test
npm --prefix ui run build
```

All checks must pass before packaging.

## 3. Tauri packaging prerequisites

Linux build hosts need the same system libraries used in CI:

```bash
sudo apt-get update
sudo apt-get install -y \
  cmake \
  libasound2-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libwebkit2gtk-4.1-dev \
  libx11-dev \
  libxtst-dev \
  pkg-config
```

Then build the desktop bundle:

```bash
cargo tauri build
```

## 4. Manual desktop smoke test

Before publishing binaries, install or run the packaged app on the target platform and verify:

- App starts without console panics.
- Default profile is created under the platform Tauri app data directory.
- Settings load and save; invalid values are rejected rather than persisted.
- Selected Whisper model download completes and checksum validation succeeds.
- Model load succeeds after download.
- Push-to-talk recording starts and stops.
- A short dictation creates a session and segment.
- Correction review saves and updates learned patterns.
- Profile clone preserves sessions and patterns.
- Auto-inject, clipboard-only, and manual inject paths behave as expected for the platform.
- System tray and global hotkey behavior are checked on the target desktop session.

## 5. Publish gate

- Confirm GitHub Actions passed on the release PR.
- Confirm no generated `crates/talax-app/gen/`, `ui/dist/`, model files, databases, secrets, or local configs are staged.
- Tag only after the release PR is merged and the package artifact has passed smoke testing.
