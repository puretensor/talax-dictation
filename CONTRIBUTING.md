# Contributing to TalaX

TalaX is early-stage source-available software. Small, focused pull requests are preferred.

Before opening a PR, run:

```bash
cargo fmt --all -- --check
cargo test -p talax-engine
cd ui && npm ci && npm run check && npm run build
```

Do not include voice recordings, correction databases, downloaded model files, private configuration, credentials, or machine-specific paths in commits.
