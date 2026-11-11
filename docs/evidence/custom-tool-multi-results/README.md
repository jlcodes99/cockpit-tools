# Preserve contiguous custom-tool results during account failover

An accepted Codex conversation can contain several `custom_tool_call_output` records for one `custom_tool_call`: the main result, followed by progress notifications emitted by `exec`'s `notify()`. The existing one-result-per-call replay check consumes the pending call on the first result and rejects the next notification. Consequently, a conversation that still works on its current account can lose the ability to rebuild safely on another account when that owner becomes unavailable.

This change creates a narrowly validated replay copy containing one combined result per affected custom call. It preserves every text and image part, while keeping the original request and accepted checkpoint records unchanged. It does not add a blanket exception for orphan results or classify `exec` notifications as independent host results.

This document separates the reproducible software defect, local synthetic tests, and observations from a real deployment. Account **A** denotes the owner at the stalled point; **B** denotes a replacement account in the synthetic tests. These labels are illustrative, not exported account identifiers. No private conversation content, images, credentials, account fingerprints, or runtime IDs are included.

## Incident timeline and evidence limits

Times below are on 2026-11-11 in UTC+08:00. The observations concern one explicitly scoped conversation, not every request sharing its session.

| Time | Observed evidence | What it establishes |
|---|---|---|
| 17:17 | The affected turn began. | Start of the investigated execution window. |
| 17:41 | An earlier request encountered an upstream 429, changed accounts, and completed with HTTP 200. | Account failover already worked for an earlier history shape. |
| 17:42–17:43 | Another request encountered `401/token_revoked`, changed accounts, and ended as `499/client_cancelled`; subsequent requests succeeded on the later owner A. | Authorization failure did cause reselection, but this individual request did not complete successfully. |
| 18:06:43–18:06:45 | The last `exec` call returned `Script completed`; its four command items exited 0 and its two file operations completed. The preceding two image operations also completed. | The stall was after the final tool result, not an outstanding image/tool execution. |
| 18:07:00 | The client recorded local gateway HTTP 503 with `Retry-After: 16730`. | The response instructed a delay of roughly 4 h 39 min, to about 22:45:50. |
| 19:11:48 | A recovery attempt recorded local HTTP 503 with `Retry-After: 12843`. | Another response instructed waiting until approximately the same reset point. |
| 19:52:49 | A subsequent attempt recorded local HTTP 503 with `Retry-After: 10381`. | The decreasing delay again pointed to approximately 22:45:50. |
| 20:20:21.424 | The latest stalled attempt recorded local HTTP 503 with `Retry-After: 8730`. | The instructed earliest retry was 22:45:51.424. |
| Around 22:45 | The previously indicated account-window reset/retry time passed. | The later successes are consistent with natural quota recovery; the reset itself was not observed as a separate upstream event. |
| 22:47:28 and 22:48:59 | Two new requests completed with HTTP 200 on A while the old executable was still installed. | The conversation naturally resumed before this patch was deployed. |
| 22:49 | The active turn was stopped through the supported client interface to prepare the deployment. | A controlled deployment boundary, not a new conversation or database repair. |
| 22:59:53 onward | The new executable was installed and started; the original conversation was resumed once. | Start of the post-deployment observation window described below. |

The 503 response bodies were not captured or inspected. Therefore the exact local error code/body is **not established**. The recorded status and `Retry-After` values support a long local wait instruction; they do not by themselves prove the full routing decision, prove that the client was continuously sleeping, or establish a sole cause for every stalled interval. Requests with zero upstream dispatches and unfinished requests can be absent from the persisted request table, so an empty table interval is not evidence that no client request existed.

Client logs also contained orphan-custom-output markers. A separate rollout inspection found delayed notification outputs around a compaction boundary. Those markers are not treated as proof that arbitrary orphan results are safe to replay, and this change continues to reject genuinely unpaired histories.

## Verified accepted-history structure

The last inserted accepted checkpoint in the active generation at inspection time contained **56 ordered items**, encoded as **56 chunks**. All 56 chunks were decompressed and their SHA256 digests verified before inspecting structure. The decoded size was **22,092,099 bytes** (about 22.1 MB decimal).

| Item type | Count |
|---|---:|
| `additional_tools` | 1 |
| `message` | 14 |
| `reasoning` | 10 |
| `custom_tool_call` | 10 |
| `custom_tool_call_output` | 15 |
| `function_call` | 2 |
| `function_call_output` | 2 |
| `agent_message` | 2 |

Three groups had the relevant shape, using zero-based positions:

| Call | Consecutive results | First result | Later results |
|---:|---|---|---|
| 31 | 32, 33, 34 | Seven content parts | Two distinct 21-character strings |
| 41 | 42, 43, 44 | Seven content parts | Two distinct 9-character strings |
| 50 | 51, 52, 53 | Seven content parts | Two distinct 21-character strings |

For each group, the call was an `exec` custom call; all three results had exactly its call ID. All four item IDs were distinct, nonempty strings. The main result had an array payload; the later results had string payloads and the optional `name: "exec"`. Apart from their IDs, outputs, and that optional name, their envelopes agreed. The three payload values were different, so discarding them as duplicate data would lose information.

The inspected envelopes contained no signature or encrypted-envelope fields. The overall history did contain reasoning ciphertext, but no server-only resource handles and no output lacking a call ID. A read-only reproduction of the deployed `replay_items`/`paired_tools` logic accepted the content shape but rejected pairing at index 33: index 32 had already consumed the pending call.

The checkpoint ended with another custom call at index 55 awaiting a later result. That is a valid checkpoint boundary: the next request may supply the result. The current incoming `previous_response_id` and full request were unavailable during this inspection, so this checkpoint is not claimed to be the exact complete input of every later recovery attempt. The structural reproduction was JavaScript mirroring the deployed Rust logic; the subsequently added Rust tests provide the executable regression evidence.

Stored parameter metadata was a valid object containing `parallel_tool_calls`, `reasoning`, `text`, and `tool_choice`. The absence of stored `tools` or `instructions` alone was not classified as corruption; inherited/request parameter completeness remains a separate check.

## Why the additional results exist

The additional short results were traced to `notify(...)` calls in the corresponding `exec` scripts. They were progress notifications appended as `custom_tool_call_output`, not fixed image-rendering placeholders. The image data belonged to the main array result. Notification text was dynamically produced by the script, so its wording, length, or a `name: "exec"` field is not a trusted classification rule.

The protocol issue can be illustrated with synthetic content:

```text
custom_tool_call(call_id = C, name = exec)
custom_tool_call_output(C, output = [text part, image part, ...])
custom_tool_call_output(C, name = exec, output = "first progress")
custom_tool_call_output(C, name = exec, output = "second progress")
```

The old pairing rule accepts the first result and then rejects the second. Allowing all results with a familiar tool name would also admit malformed histories; merging only a proven consecutive group avoids that broad exception.

## Implementation and retained safeguards

`responses_custom_results::replay` is used consistently by full-input admission, checkpoint completeness checks, replay planning, and the existing identity fallback check.

1. Validate the original items using the existing replay-content checks. If strict pairing already succeeds, return a borrowed slice unchanged.
2. For the new compatibility path, require unique call IDs and a preceding custom call. Only consecutive `custom_tool_call_output` records with the same exact, nonblank call ID may form a group.
3. Require the supported plain call/result envelope fields. The custom call must have a nonempty name and string input; its status must be absent or `completed`. Optional result names must match the original call name. Reject malformed IDs, conflicting reuse of an output item ID within a combined group, unknown envelope extensions, and signed/encrypted call or result envelopes. Repeated identical result records are retained rather than deduplicated.
4. Preserve array parts verbatim and in order. Turn each string into an `input_text` part without trimming, deduplicating, or summarizing it. Preserve empty strings, repeated content, images, and their attributes.
5. Validate the combined copy again, including the unchanged strict one-call/one-result pairing rule. Reject orphan results, nonconsecutive groups, wrong kinds, duplicate calls, or any remaining mismatch.

Only the replay copy combines envelopes. `Plan.source_input` carries the original ordered records into `start_rebuild_generation`, so a successful new checkpoint retains the original sequence and all its item IDs. Existing HTTP, WebSocket, capability, and fallback generation paths use that hook. The client request, prior checkpoint, and opaque payloads are not edited in place. The first result's envelope is retained in the replay copy; later envelope IDs remain available in the source history rather than being reinterpreted as signatures or fabricated calls.

The mergeable call envelope allows only `type`, `call_id`, `name`, `input`, `id`, and `status`. Its result envelopes allow only `type`, `call_id`, `output`, `id`, and `name`. Optional item IDs must be nonblank strings. This whitelist applies to the new combination path; an already strictly paired history retains the existing rules rather than acquiring a new global envelope restriction. Multiple results for a `function_call` are not made eligible by this custom-call change.

Caller/thread/owner checks, parameter inheritance checks, existing authorization/cooldown decisions, cancellation guards, and the requirement for completed output before activation remain in force. This does not repair a revoked credential, manufacture quota, bypass reserve policy, or make an unusable backup account usable. It enables the existing retry/routing machinery when a complete safe history can be represented on a new account.

## Local deployed-tree synthetic validation

The local test build completed with:

```sh
cargo test --locked -p cockpit-tools --lib --no-run --message-format=json-render-diagnostics --offline
```

The Windows test harness was then run in isolated data/temp directories with synthetic loopback upstreams. No real account, quota, or business request was used by these tests.

| Filter/suite | Executions | Result |
|---|---:|---|
| `custom_results_` | 8 | Passed |
| `notify_results_` | 4 | Passed |
| `responses_host_results` | 2 | Passed |
| `codex_wire` | 16 | Passed |
| `identity` | 129 | Passed |
| `context_rebuild_http_tests` | 78 | Passed |
| `retry` | 6 | Passed |
| Total | **243 executions / 238 unique tests** | **0 failed, 0 ignored** |

Five tests were selected by more than one filter. The change adds **12 Rust test functions**: seven content/classification tests, one identity/checkpoint test, and four HTTP/WS integration tests. The four integration functions contain **nine internal scenarios**, not nine additional Rust test functions:

- Actual HTTP **429** and HTTP **401 `token_revoked`**, each with nonstreaming and SSE responses: four scenarios. The same request fails on A and succeeds on B. Assertions cover all ordered text/image parts, unchanged same-owner requests, original old/new checkpoints with six distinct result records, and response-ID-only continuation after a test-server restart.
- A B candidate emits `response.created`, then the client cancels: one scenario. B must not become active or publish completed history.
- A different caller, with shared-continuation capability disabled, attempts to borrow the accepted history: one scenario. It must be rejected before an upstream attempt and must not publish an owner or history.
- Orphan, nonconsecutive, and wrong-kind result groups: three scenarios. They must not unlock B.

The integration fixture establishes history through actual HTTP exchanges: a plain initial request receives a custom call, a second same-owner request supplies the first result group, and a third supplies another group before encountering the synthetic upstream failure. It does not insert accepted history directly into SQLite to make failover pass. History assertions check both legacy/hot storage and compressed manifests; archived chunks are independently decompressed and digest-verified.

All four newly added Rust files passed `rustfmt --check --edition 2021 --config skip_children=true`. Independent review found no definite blocking issue in the classifier, its call sites, or source-history restoration. This is a bounded review result, not a claim of exhaustive correctness.

### Valid red/green comparison

With the same corrected integration fixtures and the previous production logic, the four integration tests produced **two passes and two failures**:

- The successful-failover case returned the original `429 usage_limit_reached` instead of 200.
- The cancellation case received the same 429 before it could reach B's `response.created`.
- Invalid-history rejection and different-caller isolation already passed.

With the patch, all four functions passed. The baseline parameterized success test stopped at its first failing case, **429 with a nonstreaming response**. Therefore the baseline did not independently reach every later 401/SSE scenario; those scenarios are green coverage, not four separately observed red-to-green transitions.

Earlier baseline attempts failed with `QueryReturnedNoRows` because the initial fixture had not established a portable accepted checkpoint; its reader also initially assumed that every checkpoint had a hot-cache row. Those were test-fixture failures, were corrected, and are explicitly **excluded from the defect's red evidence**.

## Independent upstream-main candidate validation

The custom-result change was separately applied to upstream `main` commit `7ca7f0db1f915452681c9bbc366f199e2310761f`. The earlier page-host-result PR [#3074](https://github.com/jlcodes99/cockpit-tools/pull/3074) is not included in this independent candidate. The candidate also consumes the replay `Cow` directly, moving the existing transcript instead of making two avoidable whole-transcript copies; the performance distinction from the deployed version is detailed below.

The final independent build and all five scoped suites completed on 2026-11-12. The build receipt, test artifact, and all suite results share the source fingerprint `E36F83881BC72542A5CA0C29D2691E03298B82755E0492325BF8C10327460317`. The test artifact and copied runner SHA256 are both `BB2CD32156616B16A86C484F84F333AB6D6B06DF07FF8CCC9A68E440DC955CF5`.

| Filter/suite | Executions | Final run |
|---|---:|---|
| `custom_results_` | 8 | Passed |
| `codex_wire` | 14 | Passed |
| `identity` | 129 | Passed |
| `context_rebuild_http_tests` | 75 | Passed |
| `retry` | 6 | Passed |
| Total | **232 executions / 231 unique tests** | **0 failed, 0 ignored; all five suite exits 0** |

The first independent run had one existing retry test fail after its functional assertions, at `remove_dir_all` with Windows error 32 because the test's `Handle` still owned its SQLite connection. The candidate explicitly drops that handle before removing the directory. This is a test-resource-lifetime fix, not a new retry policy, and no assertion or test was skipped. The candidate therefore changes 11 Rust files rather than the original 10-file custom-result diff. A fresh build and the complete five-suite rerun then passed as tabulated above; the earlier cleanup failure is retained as part of the validation history.

The independent suite has seven fewer unique tests than the 238-test deployed-tree validation because the separate page-host-result patch and its seven added tests are not included. The new custom-result tests are retained. The duplicate selection of the custom-result identity test explains 232 executions versus 231 unique tests. Neither total is a full-repository test run, and the independent candidate results remain distinct from the prior deployed-tree results.

The machine-readable evidence is in [clean-main-validation.json](clean-main-validation.json); sanitized baseline, deployed-tree, and independent-candidate outputs are preserved in [test-logs/](test-logs/). The independent candidate was library-test compiled and exercised, but no new Release binary was built or deployed for this PR preparation. The earlier successful Release build and real installation observations belong to the deployed-tree version described below, not to the new move-optimized candidate.

## Authorization warnings and requested fast service

Two accounts shown with authorization warnings in the user's interface were checked against the scoped request metadata. Neither was the active owner at the stalled point, and neither appeared in that conversation's request attempts during the investigation window. Their warnings did not establish the cause of this later stall. Credential files were not opened for that comparison, and the freshness of the displayed warnings was not independently established by that metadata audit.

The earlier real 401 event described above did cause account reselection, but its final status was client cancellation. This distinction matters: final request aggregates can hide intermediate 401/429 attempts, and a change of account is not automatically a completed successful response.

All 95 requests in the inspected pre-recovery sample requested `priority`. For the 86 successful requests, the recorded response tier was `default` and the legacy label was `standard`; the nine cancelled requests recorded `auto`/`auto`. There were 97 attempts in total: 86 HTTP 200, nine HTTP 499, one HTTP 401, and one HTTP 429. There were zero observed priority-tier successes and no normal-tier control sample. Requested fast/priority service therefore does not establish actual priority execution, a measured speedup, or a quota multiplier. The patch does not change the tier request or claim to fix upstream service-tier behavior.

## Deployment observations and attribution

A local Release build was verified against the same Rust inputs as the passing test build, installed with a backup, and its installed hash and service listener were checked. The build used `cargo build --locked --release -p cockpit-tools --bin cockpit-tools --features tauri/custom-protocol --message-format=json-render-diagnostics --offline`; a separate check recomputed the Rust-input, embedded-frontend, and executable digests. Existing compiler/linker warnings were retained; this is not a warning-free or full-repository validation claim. Normal account membership, binding, reserve thresholds, concurrency, and cooldown settings were preserved. No identity/chat database rewriting or permanent account exclusion was used.

Before installation, the old executable had already completed two HTTP 200 requests on A after the approximately 22:45 reset. A clock command and subsequent read-only artifact check completed successfully. Those events are **natural recovery under the old executable**, not production proof of the new patch.

After installation, the original conversation was resumed once. A clock command completed successfully at 23:03:21, and later official client event metadata showed **three completed image generations**, completed commands, and successful saves. No generated image or business content is included here.

The exact main-conversation request window from 22:59:53 through the 23:24:26 snapshot contained:

| Final status | Count | Interpretation |
|---|---:|---|
| 200 | 9 | Recorded successful requests |
| 499 | 2 | Recorded client cancellation |
| 504 | 1 | `upstream_connect_timeout`, failure phase `dial` |

The 504 was followed by further successful requests. A client-side 503 without `Retry-After` was recorded 15 ms after the gateway's 504 completion; this is a time correlation because the client line did not contain the gateway request ID. Existing code can translate a connection failure into a client-facing 503, but the uninspected body prevents claiming that exact branch as directly observed. This event is distinct from the earlier hours-long wait instructions. Ten HTTP 200 requests sharing the session but belonging to other threads were counted separately and are **not** included in the main-conversation totals or used as evidence of its owner changing. The 499 records are cancellation outcomes; the metadata does not identify every cancellation's initiator.

The main conversation continued on A. Consequently, the deployment demonstrates installation integrity and real workflow continuation, **not a real production 401/429 cross-account failover caused by this patch**. That behavior is covered by the synthetic HTTP/SSE regression tests. The observations also do not establish that every network error was eliminated or that the entire creative workflow had finished.

A later, separately reported maintenance-conversation stall exposed preparation/session-selection timeout messages and was followed by a user-performed restart. The evidence and scoped source interpretation are documented in [RESTART-FOLLOWUP.md](RESTART-FOLLOWUP.md). That later episode is not claimed to be caused by the multi-result defect or fixed by this PR; it requires its own targeted reproduction.

## Performance, integration, and maintenance limits

Inside the helper, already strictly paired input returns `Cow::Borrowed` without copying its contents. The **already deployed local version** subsequently calls `into_owned` in `configure_rebuild`, however, so even that ordinary replay-planning path adds a full transcript clone relative to moving the previous transcript. Its combination path also clones the original transcript while building `source_input`.

The **independent PR candidate** instead consumes the `Cow`: `Borrowed` moves the original transcript into the replay body, while `Owned` uses the newly built replay vector and moves the original transcript into `Arc`. This removes those two avoidable copies without changing which items are accepted or the retained source content. It is a source-level allocation improvement, not a measured latency or peak-memory improvement; it is not claimed to have been installed in the user's already deployed executable. The extra replay-content validation pass remains.

The new combination path allocates a replay copy, clones content parts, and retains original source input for checkpoint provenance. `Plan` clones share that retained input through `Arc`, while `start_rebuild_generation` clones it into the candidate context. These operations are linear in the accepted history size; no peak-memory or throughput bound was measured. Large image-bearing histories, including the approximately 22.1 MB decoded example, can increase temporary memory, copying, and serialization cost beyond the raw JSON byte count. The default request-body limit is not a hard memory ceiling. No representative peak-memory, concurrency, or latency benchmark was run; test execution times are not performance benchmarks, and no zero-overhead or speedup claim is made.

The reported 238-test evidence was collected against the maintained local v1.3.81 tree containing earlier compatibility fixes. This patch is scoped to the custom-result change, its tests, and the test-only Windows handle cleanup; it does not republish the earlier page-host-result patch. The earlier upstream PR [#3074](https://github.com/jlcodes99/cockpit-tools/pull/3074) was still open and unmerged at preparation time. Clean upstream-main application, the final library-test build, and the 231-unique-test regression gate completed as described above. The local 238-test results are not relabeled as that independent validation.

Normal future software updates can replace this locally patched executable. Once the upstream change is released, use the supported upstream update path; if upgrading before that, verify whether the replacement contains this fix and re-evaluate compatibility against its source. Do not assume a local binary patch persists through an updater, and do not keep an obsolete binary merely to avoid reviewing the new version.

## Published source newline equivalence

GitHub publication converts the tested Windows Rust source files from CRLF to LF, matching upstream line endings and avoiding whole-file diff noise. No other source character or build input changes in this publication step. The exact per-file tested and published hashes are recorded in [source-equivalence.json](source-equivalence.json). The test fingerprint above remains the actual tested Windows-source fingerprint; no new build or production deployment is claimed for newline normalization. Raw test logs retain their original bytes.
