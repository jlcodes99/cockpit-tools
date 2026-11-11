# Separate follow-up: preparation and session-selection timeouts

This follow-up was reported while preparing this PR. It is included so maintainers can distinguish a second observed failure from the custom-result replay defect fixed here. It is **not claimed to be fixed by this PR**. Times below are UTC+08:00, spanning 2026-11-11 and 2026-11-12.

The user reported that the maintenance conversation itself stopped advancing, and that they closed and reopened both the desktop client and Cockpit. The installed executable still had the previously deployed custom-result patch; the restart was user performed, not an automated recovery or a new deployment during PR preparation.

## Directly observed evidence

| Time | Evidence |
|---|---|
| Nov 11, 23:43:56–23:49:59 | Main-conversation request finished with HTTP 504, `upstream_connect_timeout`, failure phase `dial`, and no first-response measurement. |
| 23:54–Nov 12, 00:13 | Client recorded repeated roughly 120-second HTTP 504 failures and one HTTP 503, without the hours-long `Retry-After` seen in the original incident. |
| 23:58:05 | Client error text: `Session account selection timed out` (503). |
| Nov 12, 00:08:25 | Client error text: `Upstream preparation exceeded the opening time limit` (504). |
| 00:15:15 | Current Cockpit process start, consistent with the user-reported restart. Its executable hash matched the deployed patched build. |
| From 00:16 | The same main-conversation owner returned successful 200 responses again. No account rebinding was performed for this audit. |

The latest failure window (from 23:43) had no recorded 401 or 429 attempts. In the wider 12-hour main-conversation sample there were 264 final requests: 187×200, 41×499, 25×502, 11×504. Intermediate attempts included three earlier 429 responses (approximately 19:50, 20:28, 22:43). Two `token_revoked` attempts belonged to related session-only child scopes at approximately 19:21 and 19:41. Those are not evidence of a revoked main owner in the latest failure window. All 187 successful main requests requested `priority`, but reported `default` / `standard`; no normal-mode control experiment or causal speed/quota measurement was performed.

Request diagnostics can omit failures before upstream dispatch. The direct client errors are therefore material evidence even when no matching final request row exists. Related child scopes were grouped separately from the exact main thread; their account selection is not evidence of a main-owner change. No raw conversation, headers, credentials, account identifiers, private paths, or actual image data are published.

## Source-level interpretation and remaining gap

In `codex_local_access_native_routes.rs`, `session_selection_busy` is produced when acquisition of the preparation `OwnedMutex` times out. Its key is caller plus session when a session exists; otherwise the existing fallback identity key is used. The acquired guard is held through `native_select_route_once`, including account metadata loading, concurrency-permit acquisition, credential refresh, profile preparation, and HTTP-client preparation. Main and child requests sharing that key can wait behind each other.

`codex_native_api/execution.rs` bounds route preparation by the smaller remaining request/opening budget. The configured opening budget in this incident was 120,000 ms. Its timeout text matches the observed preparation error. Account concurrency-permit timeout was also configured as 120,000 ms, but that path reports HTTP 429 `account_busy`; the similar duration alone does not establish that it caused these 503/504 errors.

Normal future cancellation drops the local owned mutex guard. No forgotten guard or guard moved into a detached task was found in the scoped review, so **a leaked session mutex is not established**. Existing blocking account I/O uses a global four-slot semaphore whose permit remains with the real worker. Cancelling its await does not stop a running `spawn_blocking` worker. Prolonged workers could delay subsequent preparation, but the old process's stacks and worker lifetimes were not captured; this remains a hypothesis, not a root-cause finding.

The new replay helper executes in identity preparation, before account routing and outside that session-selection guard. It does not change the mutex, account-I/O semaphore, request/opening budgets, credential refresh, or service-tier selection. Source placement does not prove that its runtime cost is irrelevant under every workload, but no direct connection from the new helper, requested priority, or a newly revoked account to this later timeout was established.

Restart recovery proves that a new process and the same account could proceed; it cannot identify which former worker or state was stuck. The original process was already closed before this audit, and no attempt was made to recreate a production hang, discard cooldowns, raise concurrency, or modify account credentials. Maintainer reproduction should inspect cancellation/worker lifetimes and lock holding in a synthetic stalled-preparation fixture. This PR remains a focused fix for safely replayable contiguous custom-tool results, not a speculative timeout/credential fix.
