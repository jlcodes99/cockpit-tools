# Embedded frontend release verification

These checks supplement the application's Rust tests. They do not install a
build, change account/configuration files, or start the business API.

## Packaging guard tests

From the repository root, with a Rust toolchain installed:

```sh
cargo test --locked --manifest-path scripts/release/frontend-guard-tests/Cargo.toml
```

The independent workspace imports the **actual** `src-tauri/build_frontend.rs`.
It collects its six tests through Cargo, rather than relying on tests inside a
build script being collected by `cargo test`. Direct dependency versions match
the application lockfile. Its own lockfile records the standalone dependency
resolution. It does not run the application build script or need `dist`.

The cases cover the Windows drive-path-to-URL parsing bug, Tauri platform and
environment merge order, valid local bundles, missing directories/index/assets,
empty assets, and development HTML. Normal `cargo test -p cockpit-tools` still
runs the business recovery and takeover regressions separately.

## Exact executable WebView2 test (Windows)

Build with the Tauri CLI first, for example `npm run tauri build`. Prepare an
existing directory for test evidence, then run PowerShell with absolute paths:

```powershell
pwsh -NoProfile -File .\scripts\release\verify-embedded-frontend.ps1 `
  -Executable 'C:\build\cockpit-tools.exe' `
  -ExpectedSha256 '<64-character SHA256 from your build artifact>' `
  -EvidenceRoot 'C:\build\verification'
```

Supply an EXE built from this source with `--frontend-smoke-test` support. The
runner checks its expected SHA256 and actual `assets/index-` binary marker,
creates a fresh child evidence directory, and runs that **same executable** in
its hidden, isolated WebView2 mode. WebView2 Runtime must be installed.

Passing requires exit code 0 **and** a new matching receipt: exact PID/EXE/hash,
page load completion, real frontend startup and React mount, stable rendered
DOM, a loaded entry script and CSS, no resource/fatal JavaScript errors, and
event-loop exit. `App::run` exits the process rather than returning, so code
after that call cannot serve as an acceptance gate. Early exit or an absent,
incomplete, stale, or failing evidence file is rejected by this runner.

The output includes EXE byte size/MiB, artifact hash, and a hashed
`final-acceptance.json` pointing to `webview-evidence.json`. Size is diagnostic,
not a hard-coded threshold. The hidden WebView supplies real loading/DOM
evidence; this is not a screenshot or a test of production account/API behavior.

The app registers no production handlers/plugins in this mode and uses the
new evidence directory for WebView data. The runner has no installation-path
defaults, does not examine existing Cockpit processes, and cleans up only the
exact test host PID it started (after checking start time and executable path).
It rejects path links/reparse points and retains evidence on failure. It does
not claim to block every possible browser network resource or to validate
normal production startup and account switching.
