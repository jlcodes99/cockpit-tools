# Quota-reset continuation validation

Base: `2cfe5e51314bfc0bf70bdda1ba08aed82c7dc8bf` (1.3.75).
Platform: Windows x64, Rust 1.99.1.

## Before the functional fix

The first eight `native_quota_snapshot_` tests were compiled and executed with
the original routing and retry implementation (the previous logging changes
were still present, but do not affect routing). Result: **4 passed, 4 failed**.

| Regression | Original result |
| --- | --- |
| Natural reset within the wait budget | Immediate 409, no retry delay |
| Long reset, original owner remains pinned | 409 instead of retryable 503 |
| Two quota windows plus memory cooling | Binding error instead of waiting |
| Cancel while waiting | Returned immediately; no wait to cancel |

The passing controls covered unknown reset, static ineligibility, reserve/usable
credit exceptions, and bound OAuth quota reservation. The principal failure was:

```text
a known near-term quota reset must not become a terminal binding error:
ServiceError { status: 409, code: "account_binding_unavailable",
message: "The connection's account is no longer available; reconnect with full input",
retry_after: None, ... }
test result: FAILED. 4 passed; 4 failed; 0 ignored
```

The account's quota remains zero throughout the natural-reset test. Its reset
timestamp is two seconds ahead; the configured wait budget is three seconds.
The test never flips an `owner_available` flag or increases the quota balance.

## After the functional fix

| Suite filter | Passed | Failed / ignored |
| --- | ---: | ---: |
| `native_quota_snapshot_` (new route and policy boundaries) | 12 | 0 / 0 |
| `native_quota_http_` (new multi-result, same-owner HTTP continuation) | 1 | 0 / 0 |
| `native_host_tests` (includes the 13 above) | 98 | 0 / 0 |
| `multi_result_history_tests` | 2 | 0 / 0 |
| `context_rebuild` (includes 3 already-counted host tests) | 35 | 0 / 0 |
| `native_scheduler_tests` | 4 | 0 / 0 |

**136 distinct tests passed, including 13 new tests** (152 executions with
the explicitly listed overlaps). See [actual test output](continuation-quota-reset-tests.txt).
The full library-test build succeeded, retaining 346 unrelated warnings.
`rustfmt --check` on the two new Rust test files and `git diff --check` passed.

In the new HTTP test, the first successful turn establishes owner A and its
synthetic history. A new exhausted window with a future reset is then installed
once. During the continuation no quota field is changed: elapsed time alone
permits the real routing code to select A. The loopback upstream receives no
request before the reset, B receives no request at all, and every tool result,
call ID, whitespace and synthetic ciphertext survives unchanged. There is no
mock `owner_available` success switch.

Final test artifact SHA256:
`9A4792A09D7C5B247743839BF06E3E72EAEE3ADF9FA0E696CE5A9E6857F75DB0`.
Baseline artifact SHA256:
`CD823A1391C84992419EB45F21CABD7D7CAFEC40F88BAE60836ADF70E9FE3719`.

## Reproduction and scope

```text
cargo test --locked -p cockpit-tools --lib --no-run
cargo test --locked -p cockpit-tools --lib native_quota_snapshot_ -- --test-threads=1
cargo test --locked -p cockpit-tools --lib native_quota_http_ -- --test-threads=1
```

The native route tests use account overrides containing synthetic OAuth metadata,
a test-local profile directory, and no real upstream send. The HTTP continuation
test uses real routing and identity processing with synthetic history and a
loopback upstream; it does not claim that another real account could decrypt or
accept the history.

All data, Cargo cache, target output and temporary files are task-local. Live
test opt-ins are unset. The existing installed Go sidecar is only a build
resource and is neither executed nor deployed. Windows test execution uses a
byte-identical copy of the Cargo artifact with an adjacent Common Controls v6
manifest, without modifying the executable or a system DLL.

The original PR's 33 logging/regression tests are not evidence of this functional
fix. No full-repository, release, Go or real-account acceptance is claimed.
