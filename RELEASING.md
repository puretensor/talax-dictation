# Releasing TalaX

`puretensor/talax-dictation` is the canonical source repository for TalaX. Releases are
prepared through pull requests against `main` and published from the merge commit; there is
no separate snapshot-export step.

## Relationship to `puretensor/TalaX`

`puretensor/TalaX` (private) was the canonical development repo until 2026-08-24. It is now
**frozen and non-canonical**: do not develop there, do not release from it, and do not treat
its version line as authoritative. The two repos had re-diverged in both directions, so the
switch is recorded here rather than left implicit.

One back-port is still outstanding at the time of writing — the private repo's UI layer is
ahead on user-visible error handling (`ui/src/lib/errors.ts`), a stale-response guard for
concurrent detail fetches (`ui/src/lib/latest-request.ts`), and four Vitest suites
(`App.test.ts`, `routes/Editor.test.ts`, `routes/Profiles.test.ts`,
`lib/latest-request.test.ts`). Until that lands, `Editor.svelte` here can render a stale
session detail if two expands race, and a failed `saveCorrections` surfaces no message.
Tracked separately; do not delete `puretensor/TalaX` until it is done.

## 1. Version and branch hygiene

- Start from an up-to-date `origin/main` with no unrelated changes.
- Bump every version location in the same commit:
  - `Cargo.toml` `[workspace.package] version`
  - `crates/talax-app/Cargo.toml` `talax-engine` path dependency version
  - `Cargo.lock` workspace package entries (`cargo update -w`)
  - `crates/talax-app/tauri.conf.json`
  - `ui/package.json` and `ui/package-lock.json`
- Lead the commit message with the version, for example `v1.5.2: Short description`.
- Open a pull request against `main`; do not tag or publish until it is reviewed and merged.

## 2. Required automated checks

Run the same gates as `.github/workflows/ci.yml` from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo audit
cargo deny check advisories bans sources
npm --prefix ui ci
npm --prefix ui audit --audit-level=high
npm --prefix ui run check
npm --prefix ui test
npm --prefix ui run build
```

All checks must pass on the release commit.

## 3. Build and smoke-test

After the version pull request is merged, check out its merge commit and build the desktop
bundle:

```bash
git switch main
git pull --ff-only origin main
cd crates/talax-app
cargo tauri build
```

For the currently supported Linux packages, install each generated format on a clean test
system and verify:

1. TalaX starts and creates its platform-managed config and data directories.
2. A model can be downloaded and passes its integrity check.
3. Push-to-talk records, transcribes, and returns to the idle state.
4. Review-first delivery copies corrected text without injecting it automatically.
5. A correction persists after restart and the active profile can be switched.
6. Uninstalling the package does not remove user profiles or downloaded models.

Do not record release artifacts in this repository.

## 4. Tag and publish

Once CI and the smoke test are green:

1. Tag the verified `main` commit as `vX.Y.Z` and push that tag to `origin`.
2. Publish a GitHub release in `puretensor/talax-dictation` for that tag.
3. Attach the Linux bundles and a `SHA256SUMS.txt` covering every artifact.
4. Verify the release page, download links, and checksums from a clean machine.

Follow SemVer. Pre-releases use the `-rc.N` suffix and must not replace the latest stable
release until they have passed the same gates.
