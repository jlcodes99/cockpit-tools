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

## Final upstream-integration checks

Rebased the independent feature commit onto the upstream squash merge of #3301 (`d4b7de66`), without repeating the original repair commit.

- Final WorkBuddy Rust regression run: **166 passed, 0 failed, 2 ignored**. Explicitly excluded only the already identified installed-process path probe, which needs process inspection outside the outer sandbox; ignored live checks were not invoked.
- Final selected frontend run, including the production-modal tests and an all-locales coverage regression: **51 passed, 0 failed**.
- Added all 28 new bridge/text-only-entry strings to all 18 locales. The locale key/reference/value-reuse checker passed. The coverage regression also checks `page.t()` references, which the original static checker did not catch.
- Full TypeScript typecheck, production frontend build and feature-only `git diff --check`: passed.
- Final exact-token audit of all **59 feature files**: no WorkBuddy access-token matches.
- No additional real inference was used for integration or final regression.

The broader pre-integration failure disclosures above remain applicable; they were not silently converted into passing results.

## Managed-key security follow-up on v1.5.2

The initial integration results above describe the earlier #3301-based build. This follow-up merged upstream v1.5.2 (`b418133c`) into the published feature branch without rewriting its history. The single UI conflict was resolved by retaining the upstream icon action and the separate native tool-bridge action.

- The new named-command regression was first executed before the backend fix. It failed because the original command succeeded instead of rejecting the managed-key reset; this was not a compile-only or source-grep test.
- Both backend rotation entry points now reject a typed managed key before modifying or persisting its material. Six command-level tests cover enabled/disabled targets, the legacy first-enabled/first-item fallback, preservation of disk/runtime/marker state, later revocation and user-session preservation, and ordinary-key compatibility.
- The disabled fixtures model a real previously enabled grant followed by a persisted disable. The first broad follow-up run caught an invalid fixture assumption about a first write of an already-disabled key; the fixture was corrected through real storage writes without weakening assertions or changing production revocation behavior.
- Final WorkBuddy Rust run: **179 passed, 0 failed, 2 ignored**. The same installed-process path probe remains explicitly excluded; ignored real-client/inference checks were not invoked.
- Final selected frontend run: **87 passed, 0 failed**, including **16 new production View/controller/shared-Modal regressions**. Both generic named-key controls and the legacy settings-modal action are protected and recheck current state after asynchronous confirmation.
- All 18 locales contain the new explanation. Locale validation passed with **9,232 keys per locale**, no missing static references and no unintended shared-English translation values. Full TypeScript typecheck and diff checks passed.
- Broader Local Access suite: **1,103 passed, 0 failed, 8 ignored**. Its first run hit the outer sandbox's process-inspection restriction and subsequent shared-lock poisoning; the exact suite passed after an explicitly approved retry outside that outer restriction. No production permission or Codex sandbox setting was changed.
- Additional Native API run using the same compiled test binary: **1,438 passed, 1 failed, 8 ignored**. The image-open-budget test `exhausted_image_stream_open_budget_can_retry_an_alternate_before_output` received HTTP 503 instead of 200; its fixture has a 30 ms timeout. Three isolated reruns passed, but the complete suite is **not** counted as green, and no passing pristine-base reproduction is claimed. The image test file is unchanged from the upstream v1.5.2 base; no unrelated image/WebSocket behavior was modified to hide this result.
- Production frontend build passed (**2,765 modules**). Existing mixed-import and bundle-size warnings remain; new test-file formatting and feature diff checks passed.
- Independent read-only review found no additional P1/P2 issue in this change. This is not a security proof or a substitute for test execution.
- No additional real WorkBuddy inference was performed. SHA-256, profile ownership comparisons and monotonic revocation checks were deliberately retained; alert #579 was not dismissed or declared resolved. See the [security review](workbuddy-codex-native-security.md) for the hash purposes, unavailable alert trace and remaining boundaries.

## CI repair after latest upstream integration (2026-11-27)

The latest release page still identified v1.5.2. The feature branch already contained that release; subsequent upstream main `f74f80f1` was merged non-rewritingly as `358e4c3ce`, with 42 upstream files integrated without conflicts.

The real full `cockpit-core` CI package selection was reproduced on the merged baseline: compilation failed with eight errors in the shared OAuth tests, exposing missing scope response/model behavior in Core. The correction synchronizes the existing desktop scope semantics, keeps the shared tests, maps scope in all four refresh-import paths, and persists scope-only updates. Two additional source-parity tests constrain future drift without replacing behavioral coverage.

Independent review caught scope-only persistence using re-import semantics or stale whole-account snapshots. The final metadata-only paths reload current account state, retain disabled/error/notes fields, avoid overwriting a newer credential generation, and pass the selected token through the actual wakeup return path. Regressions call production helpers and verify real temporary-file saves/reloads; old access-token refresh and persistence-error policies remain intentionally unchanged.

- Final post-review full Core CI selection: **251 unit tests passed, 0 failed, 1 ignored**, plus **2 source-parity integration tests passed**. The final suite includes all metadata-only state preservation, credential-generation and actual wakeup-token-return corrections.
- Final desktop regression selections: **WorkBuddy 179 passed / 2 ignored**, **OAuth 12 passed**, **context rebuild 87 passed**, **quota sync 21 passed**, and **T3 privacy 5 passed**; no selected test failed. The same installed-process path probe is explicitly excluded from the WorkBuddy selection; ignored live checks were not invoked. These are filtered selections, not a claim that all 5,780 desktop tests ran.
- Core, WorkBuddy, OAuth and context rebuild ran through Cargo. After another worktree acquired the shared build lock, the queued wrapper was stopped and quota/T3 selections ran from a private copy-on-write snapshot of that same freshly built desktop test binary. No source, feature or test implementation was substituted, and no other worktree process was interrupted.
- Merged frontend regression selection: **99 passed, 0 failed**. All 18 locales validate with **9,243 keys each**; full TypeScript typecheck passes.
- Merged production frontend build: passed, **2,767 modules**. Existing mixed-import/bundle-size warnings remain.
- The older broader Native API failure disclosures above remain historical disclosures, not silently converted into a full-suite pass.
- No additional real WorkBuddy inference was used. Both CodeQL language analyses and all ownership/revocation checks remain enabled. A missing historical Go baseline and the unavailable complete hashing-alert trace require maintainer evidence; compiling Core does not resolve either security gate.

## Claims deliberately not made

- No Windows installed-build, Codex desktop injection, different WorkBuddy CLI version, arbitrary model, malicious CLI containment or process-restart recovery claim.
- No image, hosted search, grammar-constrained custom tool, compact, WebSocket or forced-tool-choice support claim.
- ACP notification-only usage is not fabricated as Responses token usage. The observed CLI reports usage in notifications; the initial adapter can omit Responses usage.
- A private working directory and denied builtin tools are not an operating-system sandbox against a malicious installed CLI. Codex's local execution/approval controls remain authoritative.
