# Handoff: Codex LB auto-import + usage-based routing

> **2026-05-28 update:** Auto-import experiment **reverted**. Working fix is usage-based **bypass** (`openai`), not `codex_lb_sync`. See `LESSONS.md` entry for usage-based bypass.

**Repo:** `c:\Users\user\Documents\Codex\CPTools\cockpit-tools-codex-lb`  
**Prior chat:** [487cdf82-a3fb-41ab-8353-0461905da5dd](file:///C:/Users/user/.cursor/projects/c-Users-user-Documents-Codex-CPTools-cockpit-tools-codex-lb/agent-transcripts/487cdf82-a3fb-41ab-8353-0461905da5dd/487cdf82-a3fb-41ab-8353-0461905da5dd.jsonl)  
**Date:** 2026-05-28

## Goal

User wanted Cockpit to **auto-register OAuth accounts in codex-lb** when adding or switching—not only write `~/.codex/auth.json`. Earlier pain: usage-based plan (`SELF_SERVE_BUSINESS_USAGE_BASED`, upstream `6ef7c0da-8fca-4fbf-91c5-50d67faaa4b8`) hit **503 rate limit** on `http://127.0.0.1:2455` because the account was **not** in `~/.codex-lb/store.db` and `sync-cockpit` pinned the wrong exhausted Team row by email.

## What was done (uncommitted)

| Area | Change |
|------|--------|
| **New** `src-tauri/src/modules/codex_lb_sync.rs` | `POST http://127.0.0.1:2455/api/accounts/import` (multipart `auth_json`), then `codex-lb-routing.ps1 -Mode sync-cockpit` |
| `codex_account.rs` | On OAuth save → `schedule_sync_oauth_account_to_codex_lb` (background). On switch → `sync_oauth_account_to_codex_lb` before bundle write. Removed usage-based **bypass** to direct `openai`; OAuth again uses `codex-lb`. Switch retags active threads `openai` → `codex-lb`. `build_auth_file_value` is `pub(crate)` for LB import |
| `codex_session_visibility.rs` | `retag_active_threads_model_provider(data_dir, from, to)` for active threads |
| `Cargo.toml` | `reqwest` + `multipart` |
| `LESSONS.md` | Entry updated: auto-import, not bypass |
| **Live (not in repo)** | `~/.codex/codex-lb-routing.ps1` — removed `usage_based` early exit in `Resolve-CockpitPinnedAccount`. `~/.codex/start-codex-lb.ps1` — removed `Test-CockpitCurrentUsageBasedPlan` forcing `openai` |

**Verified manually:** curl import of usage-based Cockpit account returned `accountId` `6ef7c0da-8fca-4fbf-91c5-50d67faaa4b8_319d8ed3`. Unit test `usage_based_oauth_account_bundle_uses_codex_lb` passes.

## Key paths (user machine)

- Cockpit accounts: `~/.antigravity_cockpit/codex_accounts.json`, `codex_accounts/*.json`
- LB DB: `~/.codex-lb/store.db`, routing: `~/.codex-lb/routing-pinned-account.txt`
- Codex: `~/.codex/config.toml`, `~/.codex/state_5.sqlite` (`threads.model_provider`)
- LB API: `http://127.0.0.1:2455/openapi.json`, import `POST /api/accounts/import`

## User still needs to do

1. **Rebuild / restart Cockpit** from this branch (changes are Rust-only, not shipped until build).
2. **Switch** to usage-based account once to run import + pin (existing pool may already have manual import).
3. Ensure **codex-lb** listens on `127.0.0.1:2455` before add/switch (import logs warn on failure; switch continues).
4. **Full quit/relaunch Codex Desktop** after switch if threads still hit wrong provider.

## Open / follow-up

- **No git commit** yet—user did not ask.
- **E2E:** confirm chat works through LB + `request_logs` cost rows for usage-based after Cockpit switch.
- **Race:** background import on *add* vs immediate switch—switch path is synchronous import; add is async only.
- **Dead code:** `write_oauth_direct_provider_to_config_toml` now unused (warns in build); keep as fallback or delete.
- **macOS/Linux:** LB sync only runs PowerShell routing script; no shell equivalent in Rust.
- **One-shot script** `~/.codex/repair-usage-based-provider.ps1` still forces direct `openai`—opposite of new default; document or retire.
- **Untracked in repo:** `.codegraph/`, `.codex/config.toml`—do not commit secrets/config.

## Design tension (if 503 returns)

- **Direct `openai`** = reliable chat, no LB cost rows.
- **LB import + `codex-lb` + correct pin** = cost tracking in `request_logs`; must not rotate onto `quota_exceeded` pool accounts.

## Suggested skills (next agent)

- `codex-lb-cockpit` — routing, pin, watcher, LB pool
- `repo-lessons` — read `LESSONS.md` before more CPTools changes
- `bitwarden-session` — only if touching API keys/secrets

## Quick verify commands (Windows)

```powershell
# LB up?
Test-NetConnection 127.0.0.1 -Port 2455

# Pool has usage-based id?
sqlite3 "$env:USERPROFILE\.codex-lb\store.db" "SELECT id, email, status FROM accounts WHERE id LIKE '6ef7c0da%';"

# Pin
Get-Content "$env:USERPROFILE\.codex-lb\routing-pinned-account.txt"
```
