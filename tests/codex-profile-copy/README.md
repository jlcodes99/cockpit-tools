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

On Unix, staging is created with mode `0700` before copying any profile data.
An existing empty target's permission bits are restored immediately before
publication; a new target keeps `0700`. Regression cases cover existing targets
with modes `0700`, `0750`, and `0500`, observe staging while a SQLite lock holds
the copy in progress, and check cleanup after validation and publication failures.
This preserves Unix permission bits, not ownership, group identity, or ACLs.

These tests exercise profile initialization. They do not validate a desktop UI,
perform a model request, repair existing profiles, or cover the separate manual
session-copy and all-instance synchronization commands.

## Optional macOS runtime acceptance

Use an installed desktop Codex runtime that supports paginated history:

```sh
cargo build --manifest-path tests/codex-profile-copy/Cargo.toml --locked --example copy_profile
python3 tests/codex-profile-copy/runtime_acceptance.py \
  --codex /Applications/ChatGPT.app/Contents/Resources/codex \
  --copy-helper tests/codex-profile-copy/target/debug/examples/copy_profile \
  --work-dir /tmp/cockpit-profile-copy-acceptance
```

The work directory must be new or empty. The script creates a genuine runtime
schema, a folderless project, and synthetic parent/forked histories. Turns are
interrupted with networking disabled by macOS Seatbelt; no real model response
is requested from an accessible provider. The runtime is launched with the
`Codex Desktop` originator, a fake
home, and a disposable `CODEX_HOME`. Reads and writes to the user's home are
blocked except for the test directory and compiled helper directory. Control
probes verify file read/write denial and network denial against a working local
HTTP server.

It checks byte-preserving copying, legacy project membership, paginated reads,
resume and restart with the source unavailable, and isolation from later source
turns. A raw directory-copy control reports its runtime result. A deliberately
truncated ancestor reproduces `cutoff byte offset is past the source rollout`,
and the production copy helper must reject it without publishing the target.
Results and runtime stderr logs remain in the disposable work directory.

A separate Tauri-library integration test in `codex_instance.rs` calls the real
`create_instance` default-copy entry point and checks instance registration,
history relocation, and folderless project membership. Run it with:

```sh
cargo test -p cockpit-tools --lib codex_profile_copy --locked
```

On upstream `d4f1dbf`, the macOS test build needs two unrelated Windows test-cfg
corrections in `process_path_resolution.rs` before that command can compile.
Local validation applied those corrections temporarily and restored the file;
they are not part of this change. The standalone harness has no such dependency.
