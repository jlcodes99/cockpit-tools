# Running the retry-scope regressions on Windows

Use PowerShell 7 and the project's normal Rust/MSVC/Windows SDK build environment:

```powershell
pwsh -File scripts/tests/run-native-retry-scope.ps1
# Or select Cargo explicitly; Rust/MSVC/SDK must still be available to it:
pwsh -File scripts/tests/run-native-retry-scope.ps1 -Cargo 'C:\path\to\cargo.exe'
# Optionally reuse an existing build target dedicated to this checkout:
pwsh -File scripts/tests/run-native-retry-scope.ps1 -TargetDirectory 'C:\work\cockpit-test-target'
```

The helper always performs a normal locked `cockpit-tools --lib --no-run` test build. The default build target is `target/native-retry-scope/build`; `-TargetDirectory` accepts an absolute, plain directory dedicated to this checkout, including a sibling build cache. Source or feature changes still cause Cargo to rebuild normally; there is no `SkipBuild` path. It reads Cargo's JSON `compiler-artifact` message instead of guessing an executable filename, requires the artifact to be inside that exact build target, copies it into a new checkout-local run directory, and verifies its SHA256.

Some Windows environments cannot start this library test host without the Common Controls v6 activation context normally provided by the GUI app. This was observed as exit code `-1073741511` before any tests ran. The helper places an application manifest beside the **copy**; it does not patch the executable, build outputs, system configuration, or installed Cockpit. An exit code alone is never a test pass: it verifies that a nonempty test list was selected and that every listed test passed with no failures or ignored cases.

Only the fixed `retry_scope_http_tests` module runs, serially. Its HTTP/TLS servers, credentials, accounts, payloads and databases are synthetic; the apparent ChatGPT hostname resolves to the fixture's loopback listener. Test child processes receive fresh data/temp paths and have inherited Cockpit live/probe opt-ins and proxy variables removed. The script does not change the caller's environment, start the desktop app, modify production data, kill processes, or enable ignored/live tests. Proxy removal is not a network sandbox; the fixture's explicit loopback transport supplies that test boundary. Cargo may download normal build dependencies.

Build outputs remain in the explicitly selected target. Complete compiler/test stdout and stderr, the copied executable, and evidence always remain under the checkout's `target/native-retry-scope/runs/`. Each new `<timestamp>-<id>/receipt.json` records build/list/test exit codes, the source and copied artifact hash, manifest hash, and the nonzero passed count. On loader failure, build failure, or zero selected tests it fails and keeps the evidence. It does not clean prior runs or claim a release GUI/startup check.

The helper requires plain checkout/output paths and rejects reparse points, filesystem-root build targets, and the checkout itself as a build target. Build and test children omit inherited `TAURI_CONFIG` and `TAURI_ENV_*` so an earlier release configuration cannot change the test build; necessary Rust/MSVC/SDK variables and `CARGO_HOME` remain available. It intentionally uses no machine-specific toolchain path and installs no dependencies or global settings. Read the logs before retrying; do not count a loader failure as a regression result.
