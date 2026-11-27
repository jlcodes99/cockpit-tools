# Bounded opening progress for large native requests

Community change based on upstream `f74f80f1d052694b16cbdcf04e7515e36a4f7fdd` (1.5.2). This is a source contribution, not an official release or a promise of uninterrupted upstream service.

## Problem and intended behavior

A long conversation can produce a large HTTP input even when the new user message is short. The existing opening deadline combines preparation, connection, request upload and response-header waiting. A receiver that is still accepting a large body can exhaust that deadline before the request finishes. Account retries then resend the same large input. This is separate from the already fixed stale task-local preparation deadline and from actual account quota/authentication errors.

The change preserves the existing bounded network recovery episode and gives eligible large native request bodies a bounded opening allowance based on observed local body consumption. It does not delete, resize, summarize, deduplicate or replace historical images. It does not make body polling into proof that the remote peer accepted the complete request, nor permission to replay an operation of unknown execution status.

The second change removes a full plaintext allocation during zstd request preparation. The existing body is moved into the bounded blocking worker, compressed by borrowing its bytes, then retained for the existing safe identity fallback. Compression level, duplicate-inline-image exclusion, headers and encoding-rejection policy remain unchanged.

## Existing fixes retained

- PR #3284's continuous transport recovery window and managed client retry policy remain in place.
- The current preparation scope from the #3320 follow-up remains in place.
- Account eligibility, quota/cooling, identity/history reconstruction and downstream-output replay restrictions are not relaxed.
- Production frontend packaging guard and the same-executable WebView verification remain separate release requirements.

## Why not enable client stream retries

An isolated test of the installed Codex CLI found that request/stream settings `0/1` recovered a synthetic dropped stream by sending a new request, but also retried terminal HTTP and SSE `network_recovery_exhausted` responses. `1/0` still retried terminal HTTP errors without recovering that dropped stream. `0/0` is retained; mixed values do not implement selective safe continuation.

The dropped-stream fixture is not an exactly-once recovery proof: its second POST had no previous-response reference. It must not be used to promise that real image generation or tool side effects cannot repeat. A compatible client-side terminal-error protocol would be a separate change.

## Opening policy and compatibility

Only native Codex HTTP bodies at least **8 MiB after optional compression** receive progress observation. The existing bytes are exposed in 64 KiB slices. The first local body poll gives no credit; subsequent polls before expiry contribute the elapsed interval, capped at one extra configured opening window. Connection time and an unpolled stall earn no new time. A late poll cannot revive an expired opening. Small requests, existing streaming bodies, other providers and official-plan routes retain the original body and timeout behavior.

Total/overload deadlines and the existing request-wide network recovery deadline still win. A validated compression rejection and its plain fallback share one opening allowance. Healthy response streams are not limited by this opening timer. If a peer sends headers before it finishes reading the request, the body continues normal delivery after opening expiry, without earning more credit.

With a 120-second opening value, a progressing initial upload can take up to roughly 240 seconds before its first transport failure, followed by the existing 120-second continuous recovery episode. That particular failure sequence can therefore take roughly **360 seconds instead of 240 seconds**. This is a deliberate completion-versus-wait tradeoff, not a universal request-duration limit and not a claim that every failure returns sooner. Fast small requests receive no such allowance.

Wrapping a reusable body makes it non-cloneable to reqwest. The implementation explicitly preserves the locked reqwest 0.12.28 default H2 protocol-NACK recovery for wrapped requests: at most two additional sends using the same client and original request, only for a typed remote `REFUSED_STREAM` or remote `GOAWAY(NO_ERROR)`. The source-chain check matches reqwest's classifier. Small/unwrapped requests keep reqwest's original policy and do not receive an extra adapter budget. Explicit request timeouts decrease across those attempts; opening credit and outer recovery state are never renewed. Arbitrary TCP errors, local resets, HTTP failures, partial output and unknown execution outcomes do not qualify for this adapter.

The native client redirect policy, fixed Content-Length, Content-Encoding, body bytes, request extensions and safe encoding-rejection rules remain in force. No account policy, quota, identity, history or replay rule is relaxed.

## Validation and reproduction

The new correctness suite contains 26 tests: 5 upload unit tests, 10 real loopback HTTP tests, 4 H2 NACK tests and 7 compression tests. Six additional ignored tests are opt-in process-memory benchmarks and are counted separately. Final receipts and adjacent-regression results are recorded in [validation.md](validation.md); the memory methodology and measurements are in [compression-memory.md](compression-memory.md).

From a normal Rust/platform development environment, with the repository's dependencies available:

```powershell
pwsh -NoProfile -File scripts/tests/run-large-request-opening.ps1 -Offline -IncludeAdjacent
```

Omit `-Offline` when Cargo must fetch locked dependencies. The runner builds the real library tests, selects the exact Cargo JSON artifact, saves a compilation-input manifest, isolates test data and environment, and refuses empty test selections. It does not start the production application or use accounts. Its `-RunOnly` mode requires the artifact hash and saved source manifest; it cannot silently use a stale executable. See the script's parameters for dedicated target/output directories. Memory tests require the separately documented Windows runner.

The real HTTP/1 and HTTP/2 comparison sends a 24 MiB body to the same throttled receiver. A zero-credit control fails at the fixed 1.5-second opening deadline; earned, bounded credit completes the identical body from one business send. The control uses the same new body wrapper with credit disabled; it is not a separately compiled pristine-upstream binary. An additional native-gateway test exercises production routing, request canonicalization, retry wiring, response completion and permit release.

Other cases cover an unread receiver, exhausted hard cap, slow headers after a fast upload, total/network caps, cancellation, successful streams beyond opening expiry, early headers followed by continued upload, explicit request timeout, non-followed redirects, safe 415 fallback and refusal to replay generic 400/SSE output. H2 peers compare baseline reusable reqwest bodies to wrapped requests for refusal/GOAWAY success, retry exhaustion and disallowed failures. Compression tests verify 5/30/110 MiB roundtrips, exact fallback contents, unchanged headers, incompressible data, duplicate-image exclusion and cancelled-worker capacity.

Initial verification exposed two fixture defects (canonical input-shape comparison and returning an HTTP/1 redirect before draining the request); both were corrected and the final suite rebuilt and rerun. Independent review also caught and corrected loss of safe NACK retries and a potential early-header upload stall before this contribution was packaged. Passing initial tests alone was not treated as release acceptance.

## Incident boundaries

Historical diagnostics showed large waiting-to-send request bodies and opening timeouts, but did not record the full payload or packet acknowledgements. The same approximate size also succeeded. These observations motivate the reproducible slow-receiver test; they do not prove that all bytes were images or that every historical failure had this cause.

A separate incident during development interrupted an already-started response for a request smaller than 0.2 MB. Related maintenance connections failed around the same period; the application stayed running and the same account subsequently succeeded. That evidence supports a transient transport-path problem but does not locate it uniquely in the local proxy, network or upstream service. This large-upload patch is **not** a fix for arbitrary mid-response TLS disconnects. Automatically resampling such an operation is not necessarily safe.

The client retry matrix, rationale for retaining 0/0, and its limits are documented in [client-retry-policy.md](client-retry-policy.md). No real-account fault injection, quota exhaustion, account revocation, historical-message rewrite or business-task replay was used for these tests.

## Preview release acceptance

The community preview must be built through the Tauri CLI with the actual `repo/dist` frontend validated by the existing guard. A size or `assets/index-` string check alone is insufficient. Run `scripts/release/verify-embedded-frontend.ps1` against the **actual executable extracted from the finished installer**, with its exact SHA256: it verifies real WebView page completion, mounted/rendered UI, loaded resources, stable snapshots and exit. This CLI temporarily patches the bundle marker and then restores the build-path file; that different file's smoke receipt cannot authorize the packaged one. See [validation.md](validation.md) for the verified three-byte marker distinction and both hashes.

The distributable is an unsigned, explicitly labelled community preview. Its executable and repository resources must match the unpacked installer, and its archive must exclude account data, configuration, diagnostics, caches and local backups. It uses the original product identity and data profile, not a second isolated installation. Official future updates may replace the preview. Preserve upstream attribution and the repository's CC BY-NC-SA 4.0 terms; do not describe this package as author-signed or commercially redistributable.

The optional preview overlay and commands are in [build-preview.md](build-preview.md). They are separate from the author's official release configuration and must not silently replace it.
