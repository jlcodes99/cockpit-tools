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

### 2026-05-27 - Codex active chats require rollout files under sessions
- **What:** Editing `projectless-thread-ids` or sqlite `archived` flags alone does not restore Codex Chats. Active threads must have `threads.rollout_path` pointing under `.codex\sessions\YYYY\MM\DD`, and the rollout file must physically live there.
- **Why it matters:** Leaving rollout files in `.codex\archived_sessions` while marking rows active makes the sidebar show empty or inconsistent Chats.
- **Fix / workaround:** Follow upstream `openai/codex` `unarchive_thread.rs`: move rollout from `archived_sessions` to dated `sessions`, clear sqlite `archived`/`archived_at`, keep `projectless-thread-ids` to active IDs only, and avoid restoring old full `project-order` snapshots unless the user wants Projects unarchived.

### 2026-05-27 - CPTools switch/start must not sync Codex histories
- **What:** CPTools was slow on account switch/start because it ran Codex thread sync and session visibility repair, rebuilding/syncing hundreds of archived histories across instances.
- **Why it matters:** This made launch/switch feel broken and caused archived histories to appear again after pressing switch.
- **Fix / workaround:** Keep automatic thread sync and provider-change session repair disabled for normal switch/start. If histories get expanded again, restore the pre-sync `session_index.jsonl`, `.codex-global-state.json`, and `rollouts` backups for root `.codex` and managed instances, then fully relaunch CPTools/Codex.
