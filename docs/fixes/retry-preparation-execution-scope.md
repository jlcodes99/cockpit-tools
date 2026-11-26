# Preserve the active preparation budget across retries

Baseline: main commit `d4b7de6` (version 1.5.1).

After an allowed route refresh or retry, request preparation could still read the previous attempt's expired task-local opening deadline. This change makes five preparation operations use the `Execution` already attached to the current `Prepared` request. It does not grant an additional retry or restart a request's total budget.

## Problem and evidence boundaries

The retry loop and route-refresh helpers can renew `Prepared.execution` for an allowed opening attempt. Tokio task-local state is independent of that struct: leaving a nested `Execution::run` scope restores the caller's previous `CURRENT` value. Updating `Prepared` therefore does not update every later helper that reads `CURRENT`.

Helpers such as reasoning-policy lookup and bounded blocking preparation consult that task-local execution. A later attempt could have a valid opening deadline in `Prepared` while these helpers see the expired deadline from an earlier attempt. Preparation may then fail before the replacement request is dispatched, including with a local state-worker timeout. Increasing account count or changing an account's authorization does not correct that scope mismatch.

The motivating observations included network failures followed by unsuccessful recovery, with request bodies of roughly 100 MB. Comparable requests had also succeeded. Those observations support investigating retry preparation and timing; they do not establish a request-size limit, prove an authorization failure, or uniquely attribute every observed failure to this defect. Large payloads are not required for the scope mismatch and are not a substitute for a controlled regression test.

## Why a client can see 503 while diagnostics record another error

Request diagnostics and the final client response describe different stages. Diagnostics can retain the actual attempted transport failures, such as a locally classified 502 or 504. A later local failure can determine the final response without constituting another upstream attempt.

There is also an existing explicit translation in the retry loop: when an unsent connection failure reaches terminal handling after the permitted recovery paths and there is no earlier upstream rejection to preserve, the final error becomes `503 no_available_account`, retaining the failure explanation. That label does not prove that every account was tried. A local bounded-worker failure may independently produce a 503. Neither result by itself proves that an upstream account returned 503, lost authorization, or was removed from the account pool.

This patch addresses the stale preparation scope. It does not redefine those status mappings, alter diagnostic ownership, or turn the client/diagnostic status difference into proof of a single root cause.

## Five scoped operations

The retry loop clones the current `Prepared.execution` after its existing opening-deadline handling and explicitly scopes these operations:

1. `anthropic_compact::expand_wire`, whose recognized compaction content uses bounded blocking work.
2. `reasoning_policy::check_dispatch_with_parent`, whose host policy lookup uses `execution::within`.
3. `http_compression::request`, whose compression work uses the remaining preparation budget.
4. The existing `execution::within` call around official failure observation.
5. `mapping::retry_capability`, including its preparation and candidate-selection helpers.

The capability branch clones `Prepared.execution` again immediately before use. An earlier official fallback can renew that execution even when it returns no candidate. Reusing the loop's earlier clone at this point would reintroduce an outdated budget.

Adjacent raw-body rewriting, continuation recovery, route refresh, alternate-account selection, and local-quota replacement already establish their own execution scopes. The change does not wrap the entire retry loop in a new opening timeout or replace those helpers' existing scopes.

## Preserved budgets, cancellation, and replay restrictions

- **First attempt:** ordinary initial selection and its first send continue to share their original opening budget. No new `renew_open` call is added. When an existing continuation-recovery path has already updated `Prepared.execution`, subsequent preparation now observes that execution correctly.
- **Total budget:** the existing total deadline and its caps remain in force. `Execution::run` clones an existing execution; it does not create a new total deadline. This change does not add or remove an optional total limit.
- **Network recovery:** the request-local recovery watcher, failure count, and deadline remain unchanged. A preparation scope does not reset that recovery episode.
- **Cancellation:** `Execution::run` uses a task-local scope and a boxed future, without spawning a replacement task. Stop checks and dropping the async operation retain their existing behavior. Already running blocking work remains tracked and retains its capacity permit until its actual completion; this patch does not claim to forcibly cancel such work.
- **Safe replay:** attempt counts, backoff, credential limits, account affinity, pinned-route constraints, history/rebuild validation, and pre-execution fences are unchanged. The fix supplies the proper budget only where the existing policy already permits preparation or retry. It does not authorize replay after downstream output or when upstream execution may have occurred without a safe replay path.

## Frontend packaging remains a separate acceptance gate

Merged PR #3284's frontend packaging guard and isolated frontend smoke verifier are retained. The production guard checks the effective Tauri configuration, a relative local `frontendDist`, and real nonempty production entry assets. The isolated verifier checks the exact executable and its embedded frontend rather than accepting compilation success or executable size alone.

These safeguards do not test account switching or production API behavior. Runtime regression success must not be used as a substitute for verifying the final executable's embedded frontend.

## Validation status

Windows/MSVC, locked repository dependencies, synthetic loopback requests only:

- **Unchanged runtime baseline, final 9 tests:** 3 passed / 6 failed / 0 ignored. The same-account and replacement-account cases each captured one real compressed request and two route selections, then failed with local `blocking_operation_timeout`. The delayed reasoning lookup failed with preparation timeout; capsule expansion and two deadline guards exposed the same premature local failure. The bypass/recovery/cancellation controls are not required to fail on the baseline. See [complete baseline output](retry-scope-evidence/baseline-red.txt).
- **Fixed runtime, identical 9 tests:** 9 passed / 0 failed / 0 ignored. Both main recovery cases captured two real requests and returned HTTP 200 with unchanged input hashes. See [complete fixed output](retry-scope-evidence/fixed-green.txt).
- **Portable Windows runner:** its actual Cargo build, nonempty test inventory, copied-artifact hash check and manifest-activated execution also passed all 9 tests. See [instructions](retry-scope-windows-validation.md). This repeats the 9 tests; it does not add 9 distinct cases.
- **Adjacent suites:** 14 suites / 136 distinct tests passed, including network recovery, bootstrap/replay restrictions, initial preparation budgets, official and capability fallback, compression, reasoning, lifecycle, cooldown, future layout and shutdown/worker-capacity behavior. See [per-suite counts and setup caveats](retry-scope-evidence/adjacent-summary.json). Together with the 9 new cases this is **145 distinct runtime tests**; the full 5,562-test library suite was not run.
- **Frontend guard:** the existing standalone harness passed its 6 tests. Production TypeScript/Vite build also passed; existing chunk-size and mixed-import warnings remain disclosed.

Two initial adjacent-run setup failures were investigated rather than counted as passes: all preparation cases first exceeded their existing 100 ms database-initialization budget before the HTTP listener existed; unchanged baseline and fixed binaries then each passed all 9. Another fixture could not open SQLite under the long Windows harness temp path and passed with the same executable after shortening that path. No product timeout or test assertion was relaxed. The exact transient initialization cause was not measured.

The new HTTP cases directly cover compression, capsule expansion and asynchronous reasoning checks. Official failure observation and capability selection additionally have existing regression coverage and static scope review; this is not a claim of a new expired-deadline HTTP test for every modified branch.

The final Tauri CLI release build passed with the production frontend guard verifying 11 local entry/assets. The exact release executable is 170,326,528 bytes (162.44 MiB), SHA256 `8D4E51508EFBDF7D86322F472136943F61A54AE490B9F6603AF09469C5F5A8A3`; its `assets/index-` marker is present. The same executable passed the existing isolated WebView2 verifier: completed page load, visible application/main DOM, loaded entry script and 10 stylesheets, four consecutive valid snapshots stable for 3,001 ms, zero resource/fatal errors and exit code 0. The test host and WebView processes belonging to its isolated data directory were confirmed exited. See the [sanitized release acceptance](retry-scope-evidence/frontend-release.json).

The release build emitted 846 library warnings, one linker-output warning for the binary, and a future-compatibility warning for `proc-macro-error2` 2.0.1; these are disclosed rather than represented as a warning-free build. The verifier did not register production command handlers or plugins. No installation, production deployment, or production-account acceptance is claimed.
