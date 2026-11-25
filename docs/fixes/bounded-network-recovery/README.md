# Technical evidence: bounded network recovery and verified frontend packaging

This is a sanitized technical evidence record for the upstream contribution. It describes the locally implemented and validated repair. It is not a claim that these checks have already run against a subsequently rebased PR HEAD. Only source, tests, and documentation are intended for upstream delivery; no installer, user configuration, account data, raw production logs, or screenshots are required.

## 1. Scope and provenance

The implementation started from official v1.4.2, upstream commit `586d409df963610a51a48da24256ffd30b924493`. The local archival baseline is not upstream history. The network repair was recorded locally at commit `9ab8e71adfe38f3bb2975bb0afa393111c8b1df3`; the final frontend repair additionally includes working-tree changes. Do not use that commit alone as the identity of the final installed artifact.

The delivered changes have three related parts:

1. Stop repeated transport failures from leaving an ordinary native API request waiting for many minutes across successive account attempts.
2. Keep the managed Codex provider from multiplying that server-side recovery with its own request and stream retries.
3. Reject a frontend-less release at build time and support a real, isolated WebView check of the exact release executable.

The frontend packaging accident was introduced by our local custom build procedure. It was not an identified defect in the author's official v1.4.2 binary. The defensive checks are included because that local mistake exposed a gap in our own release validation.

## 2. Observed failure and what it establishes

Two minimal greeting requests remained in a loading state. One eventually succeeded after about 10 minutes 34 seconds. Another gateway request lasted approximately 939 seconds, made 12 attempts across 6 accounts, and encountered network 502/504 failures on its first 11 attempts; it ended with 499 when stopped. A subsequent minimal greeting succeeded in about 27 seconds.

The evidence establishes retry amplification after transport failures. It does not establish that adding accounts, fast mode, loss of authorization, exhaustion of the entire account pool, a particular proxy, or an OpenAI-wide outage was the unique initiating cause. A recorded `dial` phase includes connection establishment, request upload, and waiting for response headers; it is not sufficient to attribute the failure specifically to TCP.

Before this repair:

- An ordinary request had no shared deadline for its transport-recovery episode.
- `maxRetryCredentials = 0` did not bound how many candidate accounts could be attempted.
- Switching accounts could start another opening wait.
- The outer Codex client could retry the complete request or failed stream, multiplying an already long server-side recovery.

The repair addresses the amplification and termination semantics. It does not create quota, bypass genuine upstream 429 responses, or guarantee that an unavailable network will recover.

## 3. Server-side behavior

### Shared transport-recovery episode

`RequestTracker.network_recovery` holds a shared `Budget` backed by `watch::Sender<State>`. It is activated only after the first typed transport failure.

For ordinary requests, recovery terminates at the earlier of:

- 120 seconds from the first transport failure; or
- 4 consecutive transport failures.

The initial healthy attempt retains its existing opening timeout; it is not retroactively placed under a 120-second whole-request deadline. Same-account refresh, account switching, backoff, and waiting for replacement routing share one recovery budget and do not reset it.

Valid HTTP response headers call `recovered()`, ending the consecutive network-failure episode. Authentication, quota, and upstream HTTP service responses such as 401, 429, and 503 do not independently arm this transport budget. They also do not inherit a stale timer from an earlier network episode.

A watcher can interrupt waiting for response headers, route replacement, refresh, and retry delay. Once response headers are received, the watcher leaves the active response body alone. A stale timer that wakes in the same scheduling tick as valid headers rechecks shared state before deciding to terminate.

### Images, streaming, cancellation, and account state

- Healthy long-lived streams do not acquire a new overall deadline.
- `ExecutionPolicy::for_images()` sets its recovery window to `max(network_recovery_timeout, image_open_timeout)`, preserving the longer image-opening allowance.
- Existing request cancellation cleans up tracker/global/account ownership. Already-started `spawn_blocking` work remains subject to the existing worker-capacity protection; this is not a claim that synchronous work can be forcibly killed immediately.
- Exhausting a local network-recovery budget does not cool down an account, remove it from the pool, or clear its soft affinity. Existing real-quota/429 handling remains separate.

### Terminal response and diagnostics

The terminal result is HTTP 504 with code `network_recovery_exhausted`:

- error details include `retryable: false`, `transport_failures`, and `last_error_code`;
- the response includes `x-should-retry: false`;
- `credential_scope = false`;
- attempt diagnostics retain the original transport failure;
- the final `RequestReport` records the terminal 504 and terminal code rather than accidentally preserving an earlier 502.

English and Simplified Chinese messages distinguish network-recovery exhaustion from account quota exhaustion.

## 4. Managed client retry policy

For the selected provider managed by Cockpit, `codex_local_access`, transport configuration now writes:

```toml
request_max_retries = 0
stream_max_retries = 0
```

This is wired into transport takeover and maintenance of the existing managed profile. Other providers are outside this change. Cockpit still performs its own bounded recovery.

On takeover cleanup, only fields still equal to Cockpit's managed value of zero are restored from the backup. A field absent before takeover is removed. Later nonzero user edits and deletions are preserved. This preservation statement describes cleanup/restore behavior; it must not be read as a promise that managed-profile maintenance never repairs selected provider settings.

Tradeoff: after a local-provider stream has emitted partial output and then fails, the client does not automatically reconnect that stream. The user can retry manually. Healthy long streams are unaffected by this policy.

### Why the response retry hint alone was insufficient

An isolated harness used the actually installed `codex-cli 0.171.0-alpha.13.2`, with isolated home/configuration, an ephemeral loopback server, and no real account authentication or upstream calls:

| Synthetic response | Client retries | Observed POST count | Observed result |
| --- | --- | ---: | --- |
| HTTP 504, terminal code, `x-should-retry: false` | request 0 / stream 0 | 1 | `turn.failed`, exit 1, 402 ms |
| Same HTTP response | request 1 / stream 1 | 4 | failure, 1033 ms |
| SSE HTTP 200 with `response.failed`, terminal code, `retryable: false` | request 0 / stream 0 | 1 | failure, 452 ms |
| Same SSE response | request 1 / stream 1 | 2 | failure, 583 ms |

These four final harness results show that the paired zero-retry policy avoids the observed client-side multiplication for this client version. Mixed settings (0/1 and 1/0) were not independently tested. An earlier harness iteration hung while waiting for stdio closure; it was corrected and the final four cases were rerun. The earlier incomplete run is not evidence.

The desktop application was subsequently updated. We did not perform a per-request runtime readback of the new desktop's retry policy. The four cases are CLI behavior evidence, not a claim of desktop hot-reload verification for every desktop version.

## 5. Frontend packaging failure and correction

### Verified local build mistake

The custom build override placed a Windows absolute path in `TAURI_CONFIG.frontendDist`. With the exact dependency version `tauri-utils 2.8.3`, its untagged frontend-path enum tries URL parsing before directory parsing. A Rust probe confirmed:

- `C:\\...\` and `C:/...` parse as URLs with scheme `c`;
- `../dist` parses as a directory.

In `tauri-codegen 2.5.5`, the URL branch does not embed the local frontend files. A nonexistent directory variant would instead fail compilation. Therefore the precise cause was the override being parsed as a URL, not the claim that Cargo inherently cannot embed frontend assets. The absence of `repo/dist` alone did not fully explain the successful but broken build.

The bad local binary was 155,799,040 bytes (148.58 MiB), SHA-256:

`BBF719D6E90F33704CC0A8517C922D0B66DE1EE78C8E911506ABC4A3A0C4F42F`

Its process, tray, and backend could run while its main WebView showed `ERR_FILE_NOT_FOUND`. The old deployment fixtures checked installation/configuration safety and had no actual frontend-rendering check. Our earlier conclusion that this binary contained the real frontend was wrong and has been withdrawn. That binary is not suitable for redistribution or installation.

### Corrected build and fail-fast guard

The corrected release used Tauri CLI 2.10.1. A controlled before-build step copied the 158 production frontend files into `repo/dist`, and the effective configuration used `frontendDist: "../dist"`. The procedure checked source, frontend, and build-tool fingerprints before and after the build. It avoided the original preparation step that would terminate a debug process.

`src-tauri/build_frontend.rs`, called from `build.rs`, validates the effective platform configuration and `TAURI_CONFIG` merge using the same configuration representation as Tauri. In non-development builds it rejects:

- a URL or absolute-directory frontend distribution;
- a missing distribution directory or index;
- a development/Vite entry page instead of built output;
- missing or empty referenced local JavaScript/CSS assets.

The guard resolves and checks the actual local references in `index.html`. The validated release had 11 checked entries: the index and 10 referenced assets. Binary static inspection separately found `assets/index-` and the 10 HTML entry-resource names.

Binary size is a useful anomaly signal, not a universal pass/fail rule. String matches do not prove that the interface renders. Both are supplemented by the real WebView check below.

### Same-executable isolated WebView check

The explicit `--frontend-smoke-test <absolute-isolated-directory>` mode branches before production logging and worker startup. It uses an independent identifier/WebView data directory and does not register production plugins or business invoke handlers.

The dedicated directory must contain the marker `frontend-smoke.allowed` with the exact value `cockpit-frontend-smoke-v1`; linked paths are rejected. The hidden WebView loads the real embedded index and explicitly rejects business invokes.

Acceptance requires actual `script_loaded` and `react_mounted` observations, completion of entry-resource loading, a populated and visible root/app/main layout, applied CSS, and zero fatal or resource errors. At least two consecutive snapshots must remain healthy for two seconds. Timeout is 45 seconds, with a failure exit path.

The external runner checks both process termination and a Passed evidence receipt. It must not treat exit code 0 alone as acceptance: `App::run` does not return, so post-run checks in the current implementation are not reachable. The fetch/WebSocket shims are testing aids; this mode is not advertised as a complete network sandbox.

Local deployment gates bind the exact executable, source and frontend fingerprints, and actual WebView evidence/receipt hashes. They explicitly reject the known bad executable. These deployment scripts are local delivery tools, not automatically part of the upstream source fix.

## 6. Verification matrix

### Business regression tests

On 2026-11-24, 14 selected suites passed: **190 passed, 0 failed, 0 ignored**. The repair introduced 22 cases: 7 budget cases, 11 HTTP cases, 3 managed-client-policy cases, and 1 affinity regression case.

| Test filter / suite | Passed |
| --- | ---: |
| `network_recovery` | 7 |
| `bootstrap::tests` | 39 |
| `retry::cooldown_tests` | 7 |
| `lifecycle::tests` | 26 |
| `identity::continuity` | 17 |
| `images::tests` | 14 |
| `images::http_tests` | 49 |
| `shutdown_cancels_stalled_sse_and_releases_account_permit` | 1 |
| `bounded_network_client_policy_` | 3 |
| `passive_takeover_` | 13 |
| `explicit_websocket_sync_updates_only_the_owned_oauth_profile_transport` | 1 |
| `explicit_routing_sync_repairs_owned_oauth_default_without_claiming_other_profiles` | 1 |
| `takeover_cleanup_` | 3 |
| `native_soft_affinity_regression_tests` | 9 |
| **Total** | **190** |

The 11 new HTTP integration cases cover:

1. 45 candidate accounts stop at one shared transport-failure cap.
2. The deadline interrupts a retry waiting for headers.
3. The same deadline bounds stalled replacement routing.
4. Recovery can succeed before budget exhaustion.
5. The recovery window is not a healthy-first-attempt deadline.
6. Successful long streams are not truncated.
7. Cancellation releases global and account slots.
8. A new request gets a fresh budget.
9. Authentication/quota responses do not arm a transport budget.
10. A valid authentication response ends the previous network episode.
11. A quota response clears the timer before a slow healthy route.

This is selected synthetic/local HTTP regression coverage, not a full-repository, multi-OS, real-account-exhaustion, or live production-inference run. The network modules were not changed during the frontend rework, and the 190 cases were not rerun after that rework. The submission keeps those 24 source files unchanged. New submission checks are separately labelled below; a future adaptation or merge needs its own validation.

Historical business-test source fingerprint:

`E49ECD9015950A63C0564D6DEB9F22C8BB018B3064E7B49CF227DAD5642DD4AB`

Historical test executable SHA-256:

`30345641EB338808B956CB50EBDE889959083AB7106A1AC8AD45D0FC899F05C4`

The older regression summary also contains a reference to the rejected release binary. Only its test results belong in this section; that stale artifact reference must not be used as final-release evidence.

### Build and delivery checks

| Check | Result | Scope |
| --- | --- | --- |
| TypeScript + Vite production build | Passed; 158 files, 18,521,917 bytes | Frontend production output |
| Build-guard standalone fixtures | 6 passed | Local assets accepted; Windows URL, merge order, missing output/assets, development page checked |
| Actual-WebView acceptance-gate fixtures | 7 passed | Reject missing/mismatched/not-ready/nonexited/tampered evidence; accept valid fixture |
| Deployment/configuration fixture checks | 23 passed | Backup/rollback, injected failures, path/hash/junction defenses, later edits, TOML semantics and provider isolation |
| Final executable static frontend inspection | Passed | Asset-name evidence, separate from rendering |
| Same-executable real isolated WebView | Passed | Actual embedded frontend and React rendering |
| Post-install normal production launch | Passed | Real startup events and installed-file identity; no new live inference test |

The 7 acceptance-gate fixtures are not 7 real WebView launches. The 23 delivery checks are not application-startup tests. Three older installation-coordinator cases exist separately and are not added to these totals to avoid double counting.

## 7. Final local artifact and installed-state acceptance

The accepted local executable is **164,302,848 bytes (156.69 MiB)**, SHA-256:

`2E50122655E30479401BF5679575E63C249049540EF33EBE0273A433D88557F0`

Associated identities:

| Item | SHA-256 / fingerprint |
| --- | --- |
| Final source fingerprint | `929D8F9ED757FF3C244D0AB58F1B6856B81A1816AA89B21E841D594079DD7096` |
| Frontend fingerprint | `41827C34E53327558DC1F414717895726B8F5789D6EA20D3E0EAE6B51A3A7DF4` |
| Build-tool fingerprint | `709AFD424B4AFC020D886338846E0C59E59069B90149DF746E0E8DEF0E13A721` |

The 2026-11-24 real WebView check recorded Started and Finished for `http://tauri.localhost/`, rendered the real React/dashboard interface, observed one root with visible app/main content and 10 stylesheets, and accepted four consecutive snapshots over 2,997 ms. Fatal errors and resource errors were both zero. The process exited with code 0 and its owned WebView processes did not remain running.

The accepted version was installed on 2026-11-24. After the user opened it on 2026-11-25, all 20 deployed files again matched their expected sizes and hashes, and the running executable matched the accepted SHA-256 above. Normal production startup recorded `script_loaded`, `react_mount_start`, and `react_mounted` with no frontend startup error. The local API listener responded to OPTIONS with HTTP 204. Both managed client retry values were confirmed as zero.

These facts establish that the corrected installed application contains and loads its frontend. They do not constitute a new real model-inference or real account-rotation test. No active production request, app restart, or screen-control session was needed for this submission preparation.

## 8. Source-completeness map

The local repaired source consists of these **24 changed or added files**. The two previously untracked files must be explicitly included when constructing a source patch.

| Files | Purpose |
| --- | --- |
| `Cargo.lock`, `src-tauri/Cargo.toml` | Build-time dependencies/configuration needed by the frontend validation |
| `src-tauri/build.rs`, **`src-tauri/build_frontend.rs` (new)** | Effective frontend configuration and asset guard |
| `src-tauri/src/lib.rs`, **`src-tauri/src/frontend_smoke.rs` (new)** | Early isolated actual-WebView acceptance mode |
| `src-tauri/src/modules/codex_native_api/mod.rs`, `network_recovery.rs` under the same module | Register and implement shared recovery budget |
| `src-tauri/src/modules/codex_native_api/contract.rs` | Execution policy and terminal semantics |
| `src-tauri/src/modules/codex_native_api/retry.rs` | Failure classification and bounded retries |
| `src-tauri/src/modules/codex_native_api/lifecycle.rs` | Shared tracker budget, response/recovery transitions, final diagnostics |
| `src-tauri/src/modules/codex_native_api/server.rs` | Server response integration |
| `src-tauri/src/modules/codex_native_api/diagnostic_error.rs` | Terminal diagnostic classification |
| `src-tauri/src/modules/codex_native_api/bootstrap_tests.rs`, `network_recovery_http_tests.rs` under the same module | HTTP integration regression coverage |
| `src-tauri/src/modules/codex_local_access_native.rs`, `codex_local_access_native_scheduler.rs` under the same module | Credential-neutral terminal failure and affinity behavior |
| `src-tauri/src/modules/codex_local_access_profile_takeover.rs`, `codex_local_access_takeover_maintenance.rs`, `codex_local_access_sidecar_runtime.rs` under the same module | Managed provider retry configuration, refresh/maintenance and restore integration |
| `src-tauri/src/modules/codex_local_access_tests_takeover.rs` | Managed-provider configuration and cleanup regression tests |
| `src-tauri/src/modules/native_soft_affinity_regression_tests.rs` | Preserve account affinity on local network-budget exhaustion |
| `src/locales/en.json`, `src/locales/zh-CN.json` | Distinguish network-recovery failure from quota failure |

The companion `coverage-map.json` enumerates each path separately and maps requirements to evidence and limitations. Submission preparation verified all 24 files byte-for-byte against the accepted local source. See the submission verification below for the new checks; historical results are not relabelled as a fresh full test run.

## 9. Maintainer-facing limits and delivery boundaries

- The patch bounds transport-failure recovery; it does not promise that every request completes or bypass genuine quota/rate restrictions.
- The 120-second/four-failure values are current policy, not empirical guarantees for all networks.
- The managed client retry policy has an intentional manual-retry tradeoff after a partial-output failure.
- The smoke mode is explicit and isolated from normal startup, but is not a comprehensive security/network sandbox.
- Final local artifact acceptance does not transfer automatically to a different compiler, dependency lockfile, branch, or rebased PR HEAD.
- Executable size and string matches are supplementary checks. Actual WebView rendering is required for the local release acceptance claim.
- Local deployment/rollback tools contain machine-specific delivery responsibilities. They should not be inserted into the application runtime or published with raw paths, configurations, private logs, or backups.
- Source and a reproducible patch are the upstream deliverables. The accepted binary hash documents provenance; it is not a request to redistribute an installer.

## 10. Submission verification on 2026-11-25

The 24 application/source/test/locale files listed above are byte-for-byte
identical to the latest accepted local source, including the two frontend
files that had not yet been committed in the local development branch. The
application source was not rebased or rewritten for submission. The official
v1.4.2 base tree and the local archival baseline tree were independently
compared and both are `06d628afd55d71823c2a4d09f392d6edecd41d66`.
The contribution uses the real upstream v1.4.2 commit as its parent.

Additional repository files make the frontend checks reproducible:

- `scripts/release/frontend-guard-tests/` is an independent Cargo workspace
  that imports the actual build guard. All six tests passed with its locked
  dependencies on 2026-11-25. Ordinary application tests do not collect tests
  inside a build script; this explicit entry point closes that gap.
- `scripts/release/verify-embedded-frontend.ps1` validates a caller-selected
  release EXE and its expected hash, creates a fresh isolated evidence folder,
  and verifies the actual WebView receipt as well as the process exit. Script
  parsing, a wrong-hash refusal before launch, and the real binary asset-marker
  check passed.
- That new runner was then executed against the exact accepted EXE above on
  2026-11-25. It passed again: real embedded page finished, React mounted,
  entry JS and CSS loaded, stable visible DOM, no resource/fatal errors,
  process exit 0. This was a new isolated verification, not a restart of the
  running production application.

`submission-verification.json` records sanitized source hashes, exact test
filters and counts, and the new WebView measurements. `coverage-map.json`
maps the requirements to code, tests and evidence. `<WORK>` references in that
map are names of privately retained local evidence, not missing source files
or publicly accessible URLs; public results are summarized here and in the
verification JSON. Raw logs, configurations and deployment backups are
deliberately excluded.

### Reproduce the checks

Install the repository's documented Node/Rust/platform prerequisites, then
from the repository root:

```sh
npm ci
cargo test --locked --manifest-path scripts/release/frontend-guard-tests/Cargo.toml
```

To rerun the selected business suites in PowerShell (the 190 historical cases
are listed separately in the JSON; counts can change after future upstream
edits):

```powershell
$verification = Get-Content ./docs/fixes/bounded-network-recovery/submission-verification.json -Raw | ConvertFrom-Json
foreach ($suite in $verification.historicalBusinessTests.suites) {
  cargo test --locked --package cockpit-tools --lib $suite.filter -- --test-threads=1
  if ($LASTEXITCODE -ne 0) { throw "Failed suite: $($suite.filter)" }
}
```

Use the normal Tauri build (`npm run tauri build`), retain the relative
`frontendDist: "../dist"`, then run the Windows exact-EXE verifier as described
in [the release-check README](../../../scripts/release/README.md). Do not put a
Windows drive path into `TAURI_CONFIG.frontendDist`. Compilation or a zero
exit code alone is not release acceptance. The expected EXE hash should come
from the new build; no newly rebuilt binary is expected to reproduce our
local hash byte-for-byte.

The local machine-specific deploy/rollback helpers are not installed into the
application and are not required for these source changes. They backed up the
files they replaced, preserved the prior presence/values of only the two
managed retry keys, gated changes on application exit, verified exact file
hashes, and retained resumable rollback receipts. Their fixture results are
reported in section 6, separately from real UI acceptance. They are omitted
from the public source package to avoid distributing machine-specific paths,
credentials/configuration material, backup data or a production installer.

The selected 190 business cases are still the accurately labelled 2026-11-24
results; they were not rerun merely to repackage unchanged source. A full
repository test run, macOS/Linux GUI run, fault injection into live production,
real account-exhaustion test, and a rebuilt post-merge release are outside the
claims of this submission.

## 11. Upstream CI feedback and locale completion

The first published revision (33 files) exposed a real omission in this
contribution: `common.networkErrors.recoveryExhausted` had only been added to
English and Simplified Chinese. The upstream locale parity gate correctly
rejected it. The follow-up adds exactly that message, translated, to the other
16 existing locales; none of the original 24 repaired files changes. The final
contribution therefore contains 40 source/test/locale files plus 6 release-check
files and 3 evidence documents (49 files total). The installed local artifact
identity above still describes the original validated artifact; these extra
translations have not been deployed or included in a new local release build.

The complete repository locale checker is rerun for this follow-up. The
machine-readable result and each additional locale hash are recorded under
`localeCompletion` in `submission-verification.json`.

The initial GitHub regression job also failed in an unrelated updater test:
`install and relaunch failures show the error before recovering the API listener`
raised `ReferenceError: resolveUpdateApplyFailureMessage is not defined`.
The same test and exact error already failed on upstream main
`714471e528747b4ee028cbc89ed80cc63a5abd73` before this PR:

- [Upstream main failure](https://github.com/jlcodes99/cockpit-tools/actions/runs/45442208675/job/138616144350)
- [Initial PR failure](https://github.com/jlcodes99/cockpit-tools/actions/runs/45547761769/job/138981370768)

This PR does not change that updater implementation or its test. Its existing
failure is disclosed, not counted as a passing check or silently attributed to
the network repair. The new revision's GitHub checks must be read from the PR;
local historical success is not a substitute for a green merge build.
