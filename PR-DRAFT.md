# PR Draft: fix(codex-core): align token refresh gating with src-tauri

**Branch:** `fix/codex-core-id-token-refresh`  
**Base:** `upstream/main` (v1.3.28)  
**Status:** Local commit ready; NOT pushed; PR NOT created (awaiting human confirm)

## Summary

- Fix `cockpit-core` Codex token refresh to match the already-correct `src-tauri` behavior from v1.3.25.
- **Finding 1:** `managed_account_tokens_need_refresh` no longer triggers OAuth refresh solely because `id_token` is due/unparsable while `access_token` is still valid.
- **Finding 2:** `resolve_refreshed_id_token` no longer fails the entire refresh when the OAuth response omits `id_token` or returns an expired one; it preserves the current value or returns empty so rotated `access_token`/`refresh_token` can still be persisted.

## Why

`cockpit-core` and `src-tauri` diverged in v1.3.25: the desktop app fixed these paths, but the shared `cockpit-core` crate (used by `cockpit-cli`) still had the stricter logic. That caused unnecessary `refresh_token` rotation and could fail refresh entirely while `access_token` remained healthy — matching the CHANGELOG note about quota refresh and the inline comments in `src-tauri/src/modules/codex_account.rs`.

## Test plan

- [x] `cargo test -p cockpit-core id_token` (10 tests, all pass)
- [x] New: `expired_id_token_does_not_force_refresh_when_access_token_is_fresh`
- [x] New: `id_token_within_refresh_lead_does_not_force_refresh_when_access_token_is_fresh`
- [x] New: `unparsable_id_token_does_not_force_refresh_when_access_token_is_fresh`
- [x] Updated oauth tests for lenient `resolve_refreshed_id_token` fallback behavior

## Files changed

- `crates/cockpit-core/src/modules/codex_account.rs`
- `crates/cockpit-core/src/modules/codex_oauth.rs`

## Isolation from Kimi PR

- Branch created from `upstream/main` in a separate worktree (`cockpit-tools-codex-fix`).
- No Kimi files touched.
- Unrelated to `fix/kimi-code-official-config`.

## Push / create PR (after human confirm)

```bash
cd E:\Save\Temp\cockpit-tools-codex-fix
git push -u origin fix/codex-core-id-token-refresh
gh pr create --base main --title "fix(codex-core): align token refresh gating with src-tauri" --body-file PR-DRAFT.md
```
