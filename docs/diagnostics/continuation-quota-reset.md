# Preserve known quota-reset waits for pinned continuations

## Functional defect

In native 1.3.75 (`2cfe5e51314bfc0bf70bdda1ba08aed82c7dc8bf`),
`native_select_route_once` filters accounts through `native_model_quota_allows`
before collecting cooldown delays. An exhausted quota snapshot can have an
explicit, imminent reset time, but the original account and its reset time are
discarded. If another account is ready and the continuation must stay with its
owner, the result is immediately `409 account_binding_unavailable`, without a
retry delay. The existing bounded wait in `native_select_route` cannot run.
`native_retry_cooldown_wait` loses the same information.

This is a behavior defect, not just a missing diagnostic. The regression gives
the original account a reset two seconds ahead and a three-second wait budget:
the unmodified routing code immediately returns 409 instead of waiting.

## Fix

A shared quota eligibility helper distinguishes three cases:

- eligible now: zero delay;
- blocked only by a known future quota reset: that delay;
- no predictable eligibility time (unknown reset, reserve/entitlement denial):
  no delay promise and no dispatch.

Routing keeps scope, account eligibility, model, pause and capability checks.
It then combines the known quota delay with any existing in-memory cooldown by
taking the longer delay for each account. A hard continuation pin uses only its
original owner; an unpinned pool uses the earliest eligible candidate. The retry
helper uses the same quota decision.

The existing bounded wait can now select the original account after the quota
reset. A reset beyond the wait budget produces `503 account_cooling` with a
retry delay, without sending early or selecting another owner. Unknown reset
and non-quota policy rejection are not turned into guessed recovery times.
Usable-credit, `gpt-reserve`, API-key and official-subscription exceptions retain
their existing semantics. Disabling ordinary cooling does not disable the
account's real quota snapshot.

No history is deleted, merged, decrypted or reassigned. The strict tool-result
pairing and cross-account replay rules are unchanged. Independent logging-only
changes from the first PR revision have been removed from the final diff.

## Incident evidence and its limits

The motivating incident involved one long conversation on Windows 1.3.75,
returning the exact local binding error on two dates. It later resumed on its
original account; the client recovery turn also contained context compaction.
No production binary, configuration, account or chat database was changed during
that diagnosis. The precise failure-time eligibility reason was not retained.

Four date-specific API/app log checks found no corresponding failure record.
The last successful checkpoint had 371 intact items, including 28 additional
results for existing call IDs. It is not a capture of the failed request.
Multiple results can prevent a safe account-switch rebuild, making same-owner
recovery important; this patch does not relax that guard.

[Sanitized incident metadata](continuation-preflight-409-evidence.json) preserves
the observations and unknowns. There are no raw conversations, images, ciphertext,
account identifiers, credentials, databases or local filesystem paths.

The isolated regression establishes the quota-reset defect and its correction;
it does **not** establish the unique cause of the historical incident, guarantee
all 409 errors are fixed, or prove arbitrary encrypted cross-account replay.
Long-lived quota exhaustion still requires waiting for the real quota window.
There is no deployment to the user's running software.

Related: #2844, without claiming to close its distinct report. This change is
independent of #2899's host-result and checkpoint-format compatibility fixes.

See [before/after validation](continuation-quota-reset-validation.md) and
[actual test output](continuation-quota-reset-tests.txt).
