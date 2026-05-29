# Lessons

Hard-won notes for **this project folder**. Add entries when something surprised you. No API keys or tokens here - use Bitwarden session (`$bitwarden-session`).

## Format

```markdown
### YYYY-MM-DD - short title
- **What:** ...
- **Why it matters:** ...
- **Fix / workaround:** ...
```

## Entries

### 2026-05-29 - Codex tests must not touch real Cockpit home
- **What:** Full Rust tests exposed/left fixture Codex account data (`demo@example.com`, `acc-current`) in `~/.antigravity_cockpit/codex_accounts.json`, which made `codex-lb-routing.ps1 sync-cockpit` report `no-cockpit-account-mapping`.
- **Why it matters:** On Windows, `dirs::home_dir()` follows `USERPROFILE`; setting only `HOME` in tests is not enough. A polluted Cockpit index makes LB look broken even when `127.0.0.1:2455` is up.
- **Fix / workaround:** Test guards must set and restore `USERPROFILE`, `HOME`, and `CODEX_HOME`. If pollution already happened, restore/rebuild `codex_accounts.json` from real `codex_accounts/*.json` detail files, set `current_account_id` to the matching account, and rerun provider repair for usage-based accounts.

### 2026-05-29 - Codex history repairs must validate rollout JSONL shape
- **What:** A thread can exist in `state_5.sqlite`, be active, have `session_index.jsonl`, and still not show in Codex Desktop if its rollout JSONL starts with merged/forked parent history or duplicate `session_meta` records.
- **Why it matters:** Sidebar visibility is not only sqlite state. For hidden threads, inspect the rollout itself: line 1 should be the target `session_meta`, and early event rows should belong to that same thread/turn. In one Budget v2 repair, the fix was keeping line 1 plus the first native v2 event tail; in one CPTools handoff repair, removing a duplicate `session_meta` made the thread usable again.
- **Fix / workaround:** Before editing, back up `state_5.sqlite`, `.codex-global-state.json`, `session_index.jsonl`, and the rollout. Repair narrowly: set sqlite active fields, ensure rollout lives under `.codex\sessions\YYYY\MM\DD`, update `session_index`, and split/dedup rollout JSONL only when the early events clearly belong to another archived parent. Fully close Codex before replacing sqlite files so WAL/SHM do not override the restore.

### 2026-05-29 - Do not install raw Tauri cargo build output
- **What:** Copying `target\release\cockpit-tools.exe` from a plain `cargo build --release` into the installed app made Cockpit Tools open a localhost/dev-server error page.
- **Why it matters:** Raw cargo output can expect the Vite dev server. Packaged Tauri output embeds frontend assets.
- **Fix / workaround:** Use `npm run tauri -- build` / Tauri bundle output when replacing the installed app, or use the existing installer/update flow. If testing a launch-path source patch, keep app binary backups and verify the UI opens before using it to launch Codex.

### 2026-05-28 - Usage-based Codex accounts should route through codex-lb after import
- **What:** `SELF_SERVE_BUSINESS_USAGE_BASED` initially failed with `503 Rate limit exceeded` on `http://127.0.0.1:2455/backend-api/codex/responses` because the usage-based account was missing from `~/.codex-lb/store.db` or the pin pointed at exhausted Team rows.
- **Why it matters:** Direct `openai` bypass can make chat work, but it loses LB routing/cost tracking and fights the intended Cockpit/LB integration. Per-thread `threads.model_provider` in `state_5.sqlite` can still hit the wrong route even when top-level `config.toml` changes.
- **Fix / workaround:** Keep OAuth Codex accounts on `model_provider = "codex-lb"`. On OAuth save/switch, import the auth bundle into LB (`POST /api/accounts/import`), run `codex-lb-routing.ps1 -Mode sync-cockpit`, pin the Cockpit-selected usage-based account, and retag active threads `openai` -> `codex-lb` only as part of that intentional provider repair. If 503 returns, first verify LB pool import and pin before considering direct `openai` as a temporary fallback.

### 2026-05-27 - Codex LB must still require ChatGPT auth
- **What:** Desktop Plugins and `/fast` stayed grey even with `[features].plugins = true` because active provider `codex-lb` had `requires_openai_auth = false`.
- **Why it matters:** Desktop gates Plugins, MCP UI, and fast/cloud affordances on auth method, not just feature flags. `codex-lb` can point at `http://127.0.0.1:2455/backend-api/codex`, but it must still advertise ChatGPT auth so Desktop sees `authMethod = chatgpt`. A later bug showed Cockpit instance configs with an existing bad `[model_providers.codex-lb]` block could skip repair if the top-level provider was already `codex-lb`.
- **Fix / workaround:** Keep canonical `codex-lb` provider blocks as `requires_openai_auth = true`; keep `freemodel`/API-key providers as `false`. `start-codex-lb.ps1` now uses `Get-CodexLbProviderBlock` / `Get-FreemodelProviderBlock` and repairs existing bad `codex-lb` blocks. Verify with `codex doctor`: auth configured, stored auth mode `chatgpt`, reachability mode `ChatGPT auth`. If UI still looks disabled after this, check `.codex-global-state.json` `electron-persisted-atom-state.codexCloudAccess` for stale `"disabled"` and fully relaunch Desktop.

### 2026-05-29 — Budget v2 hidden when rollout forks archived parent
- **What:** Budget project showed only v1; v2 row existed in sqlite (`title=v2`, `has_user_event=1`) but not in Desktop sidebar.
- **Why it matters:** v2 rollout `session_meta` had `forked_from_id` pointing at archived thread `019e6ef0-…`. Desktop treats forked children of archived parents as non-listable. `has_user_event` and `session_index` alone are not enough.
- **Fix / workaround:** Remove `forked_from_id` from the v2 rollout first line (backup `.bak-fork-strip` first), set sqlite `title`/`preview`/`has_user_event`, keep parent archived. Pair with short `thread_name` in `session_index.jsonl` for v1/v2 and projectless chats. Fully quit/relaunch Codex Desktop after edits.

### 2026-05-27 - Codex active chats require rollout files under sessions
- **What:** Editing `projectless-thread-ids` or sqlite `archived` flags alone does not restore Codex Chats. Active threads must have `threads.rollout_path` pointing under `.codex\sessions\YYYY\MM\DD`, and the rollout file must physically live there.
- **Why it matters:** Leaving rollout files in `.codex\archived_sessions` while marking rows active makes the sidebar show empty or inconsistent Chats.
- **Fix / workaround:** Follow upstream `openai/codex` `unarchive_thread.rs`: move rollout from `archived_sessions` to dated `sessions`, clear sqlite `archived`/`archived_at`, keep `projectless-thread-ids` to active IDs only, and avoid restoring old full `project-order` snapshots unless the user wants Projects unarchived.

### 2026-05-27 - CPTools switch/start must not sync Codex histories
- **What:** CPTools was slow on account switch/start because it ran Codex thread sync and session visibility repair, rebuilding/syncing hundreds of archived histories across instances.
- **Why it matters:** This made launch/switch feel broken and caused archived histories to appear again after pressing switch.
- **Fix / workaround:** Keep automatic thread sync and provider-change session repair disabled for normal switch/start. If histories get expanded again, restore the pre-sync `session_index.jsonl`, `.codex-global-state.json`, and `rollouts` backups for root `.codex` and managed instances, then fully relaunch CPTools/Codex.
