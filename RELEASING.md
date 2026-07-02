# Releasing TalaX

TalaX is the **canonical, private development repo** for the adaptive-dictation desktop app.
The public, BSL-1.1-licensed distribution lives in a separate repo: **`puretensor/talax-dictation`**.

This split is deliberate (88→30 portfolio consolidation, Phase 2): development history and any
internal references stay here; only reviewed release snapshots are published downstream.
Do **not** merge the two repos — the licence boundary (private dev ↔ public BSL-1.1) depends on the separation.

## 1. Version and branch hygiene

- Release from `master` with no unrelated changes.
- Bump all version locations **in the same commit**:
  - `Cargo.toml` `[workspace.package] version`
  - `crates/talax-app/Cargo.toml` `talax-engine` path-dep `version` pin
  - `Cargo.lock` package entries (run `cargo update -w`)
  - `crates/talax-app/tauri.conf.json`
  - `ui/package.json` and `ui/package-lock.json` (run `npm --prefix ui install` after editing)
- Commit message leads with the version tag, e.g. `v1.2.0: Short description`. Tag `vX.Y.Z`.

## 2. Required automated checks (release gates)

Run from the repository root. All must pass before export:

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

These are the same gates CI enforces (`.github/workflows/ci.yml`, shared by both repos).

## 3. Export snapshot to the public repo

Each release lands on `talax-dictation:main` as **one snapshot commit on top of the existing
public history** (linear, additive — never force-push). Internal-only paths are excluded.

```bash
# from a clean checkout of this repo at the release tag
V=X.Y.Z
git clone --depth 1 git@github.com:puretensor/talax-dictation.git /tmp/talax-pub
find /tmp/talax-pub -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
rsync -a \
  --exclude='.git' --exclude='tasks/' --exclude='.simplify/' --exclude='.claude/' \
  --exclude='target/' --exclude='ui/node_modules/' --exclude='ui/dist/' \
  ./ /tmp/talax-pub/
# public README carries a distribution-repo banner on top of the shared body
{ cat <<'BANNER'
>  **Public distribution repo.** This is the public, BSL-1.1 release cut of **TalaX**.
>  Development happens in the private `puretensor/TalaX` repo; reviewed releases land here as
>  snapshot commits. Same product — one version line across both repos.

BANNER
  cat /tmp/talax-pub/README.md; } > /tmp/talax-pub/README.new && mv /tmp/talax-pub/README.new /tmp/talax-pub/README.md
cd /tmp/talax-pub && git add -A && git commit -m "v${V}: release snapshot from TalaX" && git push origin main
git tag "v${V}" && git push origin "v${V}"
```

(`tasks/` holds internal audit notes, `.simplify/` internal tooling — excluded from the public cut.)

## 4. Verify and publish

1. Confirm talax-dictation CI is green on the snapshot commit.
2. Build the desktop bundle from the tagged snapshot: `cargo tauri build` (see prerequisites in CONTRIBUTING.md / ci.yml).
3. Run the manual smoke test (`tasks/platform_smoke_runbook.md` in this repo).
4. Publish a GitHub release on `talax-dictation` at tag `vX.Y.Z` with the bundle artifacts.
5. Mirror both repos to sovereign Gitea (org policy: everything mirrored GitHub ↔ Gitea).

## Versioning

One product, one version line: the public repo adopts the TalaX workspace version from v1.2.0
onward (the public 0.x tags predate the unification). Follow SemVer; pre-releases use `-rc.N`.

## Why

The market-facing product is `talax-dictation` (per-seat, sovereign/local-first dictation). TalaX is
where it's built. One product, two repos with a clear licence/visibility boundary — not a duplication.
