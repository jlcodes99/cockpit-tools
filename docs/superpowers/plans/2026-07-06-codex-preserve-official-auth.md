# Codex Preserve Official Auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the CC Switch setting that preserves Codex official login when switching to third-party API providers.

**Architecture:** Store a default-off Cockpit user setting and expose it through `get_general_config` / `save_general_config`. When an API key Codex account is written and the setting is enabled, keep an existing OAuth-like `auth.json` and only update `config.toml` with the third-party provider token. If no official login exists, fall back to the current API key `auth.json` behavior.

**Tech Stack:** Tauri Rust backend, TOML/JSON auth files, React/TypeScript Settings page.

---

### Task 1: Backend Behavior

**Files:**
- Modify: `src-tauri/src/modules/codex_account.rs`
- Modify: `src-tauri/src/modules/config.rs`
- Modify: `src-tauri/src/commands/system.rs`

- [x] **Step 1: Write failing Rust tests**

Add tests in `src-tauri/src/modules/codex_account.rs` near the existing API key bundle tests:

```rust
#[test]
fn preserve_official_auth_keeps_existing_oauth_auth_file_for_api_key_switch() {
    // Seed an OAuth auth.json, switch to an API key account with preserve enabled,
    // then assert auth.json still contains OAuth tokens while config.toml has the third-party token.
}

#[test]
fn preserve_official_auth_falls_back_to_api_key_auth_without_existing_oauth() {
    // Enable preserve without an existing OAuth auth.json and assert the old API key auth.json is written.
}
```

Run: `cargo test -p cockpit-tools preserve_official_auth`

Observed in this workspace: `cargo` is not installed, so the test could not execute locally.

- [x] **Step 2: Implement minimal backend behavior**

Add `CodexAccountBundleWriteOptions`, OAuth auth detection, and an options-aware writer. Keep `write_account_bundle_to_dir` as the public default wrapper.

- [x] **Step 3: Wire config**

Add `codex_preserve_official_auth_on_provider_switch` to `UserConfig`, `GeneralConfig`, and `save_general_config`, defaulting to `false`.

### Task 2: Settings UI

**Files:**
- Modify: `src/pages/SettingsPage.tsx`

- [x] **Step 1: Add state and invoke payload**

Extend the local `GeneralConfig`, add a boolean state, load it from `get_general_config`, and save it as `codexPreserveOfficialAuthOnProviderSwitch`.

- [x] **Step 2: Add Codex settings row**

Add a switch in the existing Codex settings group with the CCS title and description shown in the user screenshot.

### Task 3: Verification And Handoff

**Files:**
- Modify: `docs/refactor/implementation-handoff.md`

- [x] **Step 1: Run available verification**

Run:

```powershell
npm run typecheck
cargo test -p cockpit-tools preserve_official_auth
go test ./...
```

Record unavailable toolchains and key logs.

- [x] **Step 2: Update handoff**

Update `docs/refactor/implementation-handoff.md` with the new CCS feature, tests, limitations, PR branch, and git status.

- [x] **Step 3: Commit and push**

Run:

```powershell
git add docs/superpowers/plans/2026-07-06-codex-preserve-official-auth.md src-tauri/src/modules/codex_account.rs src-tauri/src/modules/config.rs src-tauri/src/commands/system.rs src/pages/SettingsPage.tsx docs/refactor/implementation-handoff.md
git commit -m "feat: preserve codex official auth on provider switch"
git push fork codex/refactor-ccs-integration
```
