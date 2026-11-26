# Native WorkBuddy → real Codex acceptance evidence

Date: 2026-11-26. This is a **real installed-client acceptance**, not a mocked inference result. See [setup and limitations](workbuddy-codex-native-tools.md).

## Tested environment and route

- macOS; Codex CLI `0.172.0`.
- Installed WorkBuddy bundled native CLI SHA-256: `c4f1e9d05a51e1bc2e7db20cdcfb4b42875676dd5f8f9a701b4c40e8d458f5cf`.
- Real personal WorkBuddy login, exact model `glm-5.3-flash`, effort `low`.
- Production Native API Responses dispatcher, dedicated-key scope selector, embedded private launcher and persistent delegate actor. Only the installed-CLI executable locator was substituted by the per-registry **test-only** seam to insert an external request-budget guard before importing the real CLI.
- The same production profile generator wrote an isolated `CODEX_HOME`: HTTP Responses, WebSockets off, hosted web search disabled, no upstream retry. No acceptance-only model or tool overrides were supplied to Codex.
- The source/link records were memory-only fixtures carrying the actual credential through anonymous stdin. No Cockpit account store or ordinary Codex profile was changed.
- Codex ran with `--sandbox workspace-write -a never`; these controls were not bypassed. The outer agent sandbox initially prevented nested macOS `sandbox-exec`; the successful run used explicit approval to remove only that outer restriction.

## Actual task and observed results

The temporary repository contained a deliberately broken two-line `add(a,b)` implementation and three immutable unit tests. No real project was uploaded.

### First user turn

1. The model requested a declared `exec_command` through native `_codebuddy.ai/delegateTool`.
2. **Codex**, not WorkBuddy, read both Python files and executed `python3 -m unittest -v`.
3. All three tests failed; the actual command exit code was **1**. The resulting file contents, failures and exit status returned to the original pending ACP callback.
4. The model issued another native delegated call. Codex applied the patch and reran the tests. All three passed; the command exit code was **0**.
5. An independent local test rerun also passed. The test file was unchanged.

The complete implementation diff was:

```diff
 def add(a, b):
-    return a - b
+    return a + b
```

### Second user turn

`codex exec resume <same-thread-id>` asked it to recall the repair, rerun tests and show the implementation diff without editing files.

- Codex executed `python3 -m unittest -v && git diff -- calculator.py` locally.
- All three tests passed again; exit code **0**.
- The response correctly identified subtraction in the addition implementation as the bug.
- Both user turns used **one** original native CLI actor/process; no actor was reconstructed between them.

## Independent checks

The driver checked both actual `codex exec --json` command events and the persisted Codex rollout's matched `function_call`/`function_call_output` records. A separate, model-free verifier rechecked all three command results, actual file contents, unchanged tests, one actor, source/model reports and cleanup.

- Generic fallback selector calls: **0**.
- All gateway reports selected the fixed synthetic source identity, upstream `glm-5.3-flash` and `workbuddy-native-delegate` transport.
- WorkBuddy access-token exact-byte scan of the private native profile before shutdown: **no matches**.
- Exact-byte scan of all synthetic project and acceptance files: **no matches**.
- Owned native profiles removed after clean shutdown: **yes**.
- The successful two-turn run consumed **5 real model HTTP requests**. Including earlier rejected/failed acceptance attempts, the approved implementation budget was **10/20 real model requests** and **6 separately counted initialization HTTP requests**. Retries/errors were counted; the ledger was never reset. No more real inference was needed.

Raw synthetic session logs remain local, outside the repository; access credentials, generated local keys, absolute personal paths and full client prompt dumps are not committed.

## Deterministic coverage

The focused Rust suite at the acceptance build: **71 passed, 1 ignored** (the ignored test is the explicit stdin-bootstrap live listener). It covers actual callback shape, real Codex tool declaration schemas, namespace/freeform fidelity, schema/resource rejection, owner isolation, original RPC correlation, duplicate results, full-history/delta continuation, second-turn actor retention, cancellation during a backpressured credential bootstrap, TERM-ignoring descendant cleanup, metadata limits, stale-snapshot revocation and modified-file preservation.

Frontend tests execute the production native-profile modal through the repository's existing hook harness: **7 passed**. Related existing frontend tests: **43 passed**. Complete TypeScript typecheck passed.

Broader pre-integration runs found two existing-area failures, not hidden by the focused result:

- Native API suite: 1419 passed / 1 failed / 8 ignored. The failure is the unchanged legacy archive-maintenance fixture (`QueryReturnedNoRows` while expecting salvage of an old checkpoint); it also fails in isolation. No passing pristine-base reproduction is claimed here.
- All WorkBuddy tests: 161 passed / 1 failed / 2 ignored. The failure is the unchanged installed-process path probe under the outer process-inspection sandbox. It is not a model/quota failure.

Final post-upstream-integration build/test results are recorded in the PR description.

## Claims deliberately not made

- No Windows installed-build, Codex desktop injection, different WorkBuddy CLI version, arbitrary model, malicious CLI containment or process-restart recovery claim.
- No image, hosted search, grammar-constrained custom tool, compact, WebSocket or forced-tool-choice support claim.
- ACP notification-only usage is not fabricated as Responses token usage. The observed CLI reports usage in notifications; the initial adapter can omit Responses usage.
- A private working directory and denied builtin tools are not an operating-system sandbox against a malicious installed CLI. Codex's local execution/approval controls remain authoritative.
