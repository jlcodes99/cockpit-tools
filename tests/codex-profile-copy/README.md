# Codex profile copy regression tests

This harness compiles the production `codex_profile_copy` module without the
Tauri application or Go sidecar. Every test uses synthetic files and SQLite
databases in a temporary directory; no installed Codex profile is read or changed.

```sh
cargo test --manifest-path tests/codex-profile-copy/Cargo.toml --locked
cargo clippy --manifest-path tests/codex-profile-copy/Cargo.toml --locked --all-targets -- -D warnings
```

The main regression starts with a paginated thread whose database points at a
source-home rollout. After copying, the database must point at the corresponding
target-home rollout, and appending to the source must not change the copied
thread. The rollout bytes and offsets must remain unchanged.

Other cases cover projects without folders, legacy JSON project assignments,
host-scoped project/pin mappings, committed WAL data, old database schemas,
multiple history segments and archived ancestors, directory aliases, conflicting
target directories, and rejecting incomplete history or projection snapshots
before publishing the target directory.

These tests exercise profile initialization. They do not validate a desktop UI,
perform a model request, repair existing profiles, or cover the separate manual
session-copy and all-instance synchronization commands.
