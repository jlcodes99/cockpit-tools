# Codex page host results: fix and verification evidence

Codex Desktop can insert page-context observations as `function_call_output` items without a model-issued call. Cockpit Tools v1.3.81 did not recognize two `agent` host-result names. These observations could consequently fail tool-pair validation, prevent a safe history-rebuild plan, or consume an unrelated pending tool ID during wire normalization. A conversation could remain pinned to an unavailable account despite other accounts being eligible.

The fix extends the shared host-result classifier. It does not change quota policy, bypass upstream limits, discard conversation history, or permit arbitrary unpaired tool results.

## Evidence provenance and scope

This report consolidates one maintenance conversation and three follow-up repair conversations. The labels below identify stages, not public chat identifiers. All observations were collected on 2026-11-10; times are Asia/Shanghai (UTC+08:00).

| Stage | Contribution | Evidence status |
| --- | --- | --- |
| Original maintenance | Initial service/account investigation and the conversation that later failed with `usage_limit_reached` | Initial diagnostic observations; its later failure supplied the target for the follow-up investigation |
| Investigation branch | Correlated the exact failed request, inspected checkpoint structure, identified the unrecognized page host results, and implemented the narrow classifier fix with seven regressions | Baseline failures followed by passing regressions |
| Review/build branch | Independently reviewed the patch, expanded regression coverage, and prepared the release build | One Windows test-cleanup failure remained unresolved at that stage |
| Deployment/verification branch | Fixed that test lifetime issue, reran all targeted suites, completed the release build, deployed with rollback protection, and verified three original conversations | 226 passing tests plus completed live command-and-response checks |

An earlier authorization-revocation incident was a separate precursor: a real account returned `401/token_revoked`, subsequently received new OAuth authorization, and later resumed after its residual cooldown expired. That recovery does not establish the cause of the later `429/usage_limit_reached` failure, and this patch is not a token-repair or reauthorization fix.

The implementation and historical results below are based on official **v1.3.81**, tag commit `890d7c9034de4297f7fc644aff3e68c4f62a9e35`. If this patch is ported to a newer upstream base, its verification should be reported separately from these historical measurements.

The submission base is `main` commit `7ca7f0db1f915452681c9bbc366f199e2310761f`, whose only change from the v1.3.81 tag is `Casks/cockpit-tools.rb`. All Rust sources and Cargo/build inputs in this submission have the exact same aggregate SHA-256 as the tested release (`23B36DB7FAEB74F02ED2A76420DD53861BED5301E9771DC920B09B76B4CC6E88`); no production logic was changed during packaging.

Included evidence: [baseline seven tests](baseline-seven.txt), [fixed seven tests](fixed-seven.txt), [final 226 tests](final-226.txt), and [Windows cleanup failure before the lifetime fix](windows-cleanup-before.txt). These logs contain only synthetic-test output; process identifiers were removed and trailing blank lines normalized.

## Failure evidence and causal boundary

The affected maintenance conversation received an upstream `429/usage_limit_reached`. A later continuation received a local `503` with a `Retry-After` of 12,567 seconds, consistent with the owner's remaining cooldown. The exact local 503 body was not retained, so its error code is not claimed as a captured fact.

Inspection of the latest accepted checkpoint found **206 items**. At zero-based indices **188** and **191**, the transcript contained standalone items with the following structural shapes:

```json
{"type":"function_call_output","namespace":"agent","name":"codex_apps_open_page","output":"<observation omitted>"}
{"type":"function_call_output","namespace":"agent","name":"codex_apps_selected_page_part_1_of_1","output":"<observation omitted>"}
```

Neither item had a `call_id`. The replay-content checks supported these item shapes, but the shared classifier recognized only the existing `codex_app` / `codex_tui` thread-management results. Tool pairing therefore rejected these host observations; wire normalization could also assign them IDs belonging to ordinary pending tools.

Checkpoint item **205** was a legitimate pending custom-tool call. The checkpoint alone was therefore **not** proof of a complete, replayable continuation. A regression explicitly supplies the next request's matching result-only delta, reconstructs the full history from the checkpoint, and verifies that the completed history can rebuild on another account. It also checks that all prior items and the result remain in the new checkpoint.

The red/green tests demonstrate this classifier defect and its effect on rebuild and wire behavior. They do not establish that every past quota, authentication, transport, or continuation failure had the same cause.

## Change and preserved constraints

The production change is confined to `responses_host_results.rs`:

- Recognize exactly `agent/codex_apps_open_page`.
- Recognize `agent/codex_apps_selected_page_part_N_of_M` only for canonical positive ASCII integer indices, with checked `u32` parsing and `N <= M`.
- Reject zero, leading zeros, signs, whitespace, Unicode digits, overflow, extra suffixes, unknown names, wrong namespaces, and wrong item types.

The existing standalone-result rule still accepts only an absent or empty-string `call_id`. Explicit null, numeric, stale, or whitespace IDs do not become valid standalone observations. Normal tool pairing, duplicate/order checks, payload validation, resource-reference restrictions, opaque history handling, and caller/owner checks remain in place. The wire layer preserves recognized host observations instead of borrowing or inventing IDs for them.

The implementation uses the existing shared classifier in both rebuild validation and wire normalization; it introduces no account-exclusion configuration or new scheduling policy.

## Regression results

The same seven new regressions were first run against the unmodified production classifier and then against the fix:

| Coverage | Baseline | Fixed |
| --- | --- | --- |
| Exact host names and page-part boundaries | Failed | Passed |
| Ordinary tool pairing and payload rejection remain strict | Passed | Passed |
| Host observations do not consume pending tool IDs on HTTP/WebSocket wire | Failed | Passed |
| Malformed host IDs remain unchanged for identity validation | Failed | Passed |
| Rebuild from account A to B after synthetic HTTP and SSE quota exhaustion | Failed | Passed |
| Complete history retains a rebuild plan when A returns a synthetic local cooldown | Failed | Passed |
| Persisted checkpoint plus pending custom-tool result-only delta rebuilds on B | Failed | Passed |
| **Total** | **1 passed, 6 failed** | **7 passed, 0 failed** |

The baseline failures included a final 429 instead of recovery to 200, absence of a safe rebuild plan, absence of the expected checkpoint, and host observations receiving unrelated tool IDs. The tests use synthetic account metadata, synthetic opaque content, synthetic observations, and loopback upstream services; no private browser contents or real credentials are fixtures.

The final expanded run included those seven tests:

| Test filter | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `modules::codex_native_api::responses_host_results::` | 2 | 0 | 0 |
| `modules::codex_native_api::codex_wire::` | 16 | 0 | 0 |
| `modules::codex_native_api::identity::` | 128 | 0 | 0 |
| `modules::codex_native_api::context_rebuild_http_tests::` | 74 | 0 | 0 |
| `modules::codex_native_api::retry::` | 6 | 0 | 0 |
| **Unique total** | **226** | **0** | **0** |

An initial expanded run had 225 passes and one failure. The existing `http_transport_failover_excludes_only_this_request_but_real_status_marks_account` test failed at `remove_dir_all`, after its HTTP and routing assertions had passed. It reproduced with Windows sharing violation 32 on a targeted retry. `Handle::shutdown(&mut self)` drains requests but leaves the handle's `Arc<IdentityStore>` alive, retaining the SQLite connection. The test now calls `drop(handle)` before removing its directory. All behavior assertions and the strict cleanup error check remain intact. The final 226-test run includes that fix.

## Reproduction

Run from the repository root with the project's Rust/Tauri and platform build prerequisites installed. Use a dedicated test-data directory; set `COCKPIT_TOOLS_TEST_DATA_DIR`, `COCKPIT_TEST_DATA_DIR`, and `COCKPIT_TOOLS_DATA_DIR` to it. The recorded runs cleared all inherited `COCKPIT_*LIVE*` opt-in variables, used isolated temporary files, disabled proxy forwarding for the loopback tests, and ran serially. Do not enable live-account tests or use the production data directory.

```sh
cargo test --locked -p cockpit-tools --lib agent_browser -- --test-threads=1
cargo test --locked -p cockpit-tools --lib modules::codex_native_api::responses_host_results:: -- --test-threads=1
cargo test --locked -p cockpit-tools --lib modules::codex_native_api::codex_wire:: -- --test-threads=1
cargo test --locked -p cockpit-tools --lib modules::codex_native_api::identity:: -- --test-threads=1
cargo test --locked -p cockpit-tools --lib modules::codex_native_api::context_rebuild_http_tests:: -- --test-threads=1
cargo test --locked -p cockpit-tools --lib modules::codex_native_api::retry:: -- --test-threads=1
```

The first command selects the seven new regressions; the remaining five cover the 226-test set, so running all six commands repeats those seven. For the baseline experiment, retain the new tests but restore the original v1.3.81 production classifier before compiling. Restore the fixed classifier for the green run.

The recorded Windows run built the library harness with `cargo test --locked -p cockpit-tools --lib --no-run`, verified its hash, and ran a byte-identical copy with an adjacent Common Controls v6 manifest. Dependencies were already cached, so compilation additionally used `--offline`. The manifest was a harness-launch requirement on that machine; it did not change test code or executable bytes.

After a successful frontend build, the production binary was compiled with:

```sh
cargo build --locked --release -p cockpit-tools --bin cockpit-tools --features tauri/custom-protocol
```

The recorded release also used cached dependencies and `--offline`. The explicit custom-protocol feature embedded the built frontend. Existing compiler warnings remained; the work did not attempt an unrelated warning cleanup.

The classifier and new test module pass `rustfmt --check`. A check of all five Rust files reports 13 formatting blocks in two existing test files; running the same check on their unmodified upstream versions reports the same 13 blocks, outside the additions. Existing formatting was left intact to keep the change focused. `git diff --check` passes.

## Build and live verification

The test and release receipts recorded the same source fingerprint. These hashes identify the historical local artifacts; they are not a claim that arbitrary machines will produce byte-identical binaries:

| Artifact | SHA-256 |
| --- | --- |
| Official v1.3.81 source ZIP | `84C7D6EC04A113FB79F490501DDAE9EDD538E43585BF64470E2D77749CFECF8E` |
| Final test harness | `FA1F98B5C5BA1C233905A6E45C6976895D81BED1D6BA17E927DF6C4BE7313BC9` |
| Final deployed release executable | `6B83D738DE27CE8E103C8AFC3A02E5E93DF3AFE30F0CA21F3AA85844AFFAA4CC` |

The release completed at 21:18:44 and was successfully deployed at 21:23. The running executable hash matched the release receipt; the backup hash, exact process identity, and listener ownership were independently checked. These checks establish the installed artifact and service availability, not conversational correctness by themselves.

The first deployment attempt failed because PowerShell converted the third argument of `File.Replace(..., $null)` into an empty string. The independent helper restored and restarted the original executable. After replacing that argument with `[NullString]::Value`, an isolated temporary-file test reproduced the old failure and confirmed the corrected replacement; the next deployment succeeded. This was a local deployment-script issue, separate from the application patch. The machine-specific script, paths, and backups are not included in the PR.

With explicit user authorization, one bounded verification message was sent to each of three original conversations. All three executed a command, returned a final response, and reached completed/idle state. Their new upstream requests ultimately returned HTTP 200, and their recorded owners changed to eligible accounts without manually editing identity state.

- The original maintenance conversation executed its time-read command at **21:25:42** and completed at **21:27:17**, before the earlier **21:29:10** retry time. This distinguishes recovery on another account from simply waiting for the original owner's projected reset.
- The first repair conversation returned the time at **21:28:31** and completed at **21:29:05**. It also performed extra read-only inspection of maintenance state and deployment receipts; its first combined inspection command exited 1, and the later command exited 0. It performed no observed file write or second deployment. It is therefore not described as a perfectly single-command check.
- The second repair conversation executed its time-read command at **21:27:21** and completed at **21:28:06**. Its first verification request recorded **attempt 1: account X, 401/token_revoked; attempt 2: account Y, 200**. Its next request also succeeded on Y. This is real same-request failover evidence for an authentication error; it is not relabeled as a real 429 experiment.

The pool's 35 members and ordering, OAuth binding, model-rule summary, quota-reservation settings, concurrency, cooldown behavior, API-key count, and service settings were unchanged across deployment and verification. No account credentials, chat database, or identity database were manually modified to produce these results.

## Limits and disclosure

- Real quota exhaustion followed by same-request 429 failover was not deliberately induced; HTTP/SSE 429 and long-cooldown coverage comes from the synthetic regressions.
- The real pool was not exhausted account by account. The fix does not provide additional upstream quota or override genuine cooldown, authentication, or capability restrictions.
- This is a targeted native-library verification, not a full-repository, all-platform, or full upstream test run. Historical production results do not automatically validate a later rebased patch.
- Future client host-result formats may require additional explicit compatibility work. Unknown or malformed envelopes remain rejected.
- Public evidence omits credentials, account identifiers and fingerprints, conversation/request IDs, local usernames and paths, private transcript or browser text, and raw production payloads. Anonymous stages and account labels preserve the relevant relationships. Full local receipts remain with the maintenance project; no raw production database or conversation export is part of the PR.
