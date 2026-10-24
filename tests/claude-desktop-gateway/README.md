# Claude Desktop Gateway regression tests

This standalone crate compiles the production configuration writer, account model,
local HTTP gateway and atomic file writer without building the Tauri GUI. Only the
GUI's Windows error decoration and warning logger are replaced in the harness.

From the repository root:

```sh
cargo test --manifest-path tests/claude-desktop-gateway/Cargo.toml --locked --lib
```

Tests use temporary profiles and a loopback mock upstream. No installed Claude
profile, Cockpit account store or external API is accessed. They cover preservation
of policy and model metadata, direct/local mapping transitions, credential rotation,
ownership-scoped cleanup, invalid input and rollback after actual filesystem errors.
Unix-only permission checks in the production atomic-write tests run on Unix.

The build matrix runs this crate on Windows and Linux. A full Tauri build and manual
Claude Desktop validation remain separate checks.
