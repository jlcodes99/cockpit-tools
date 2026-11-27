# Validation record

Executed on Windows x64 on 2026-11-27. Base: upstream `f74f80f1d052694b16cbdcf04e7515e36a4f7fdd`. All runtime inputs and peers were synthetic. Production accounts, configuration, conversations and services were not modified or used as a fault-injection test.

## Runtime correctness

The final library-test build completed with `cargo test --locked -p cockpit-tools --lib --no-run --message-format=json --offline`. The 1,489 Rust/Cargo/Tauri input files had the same manifest before and after compilation. Warnings were retained; this was not a warning-free build.

- Compilation-input manifest SHA256: `17659226f902de09be0e1596676fa1bac07360f4bc41f90cbc8a0e54111f943e`.
- Exact library-test executable SHA256: `1F5D42083BCDA8D3E428EDCB33A9514AFA853C6D5E04A437B7E4EDA4D6A180B4`.
- **94 distinct correctness tests passed**, 0 failed, across 103 executions. Repeated coverage is not counted twice.
- Six ignored memory tests were intentionally excluded from the ordinary pass count and run separately as described in [compression-memory.md](compression-memory.md).

| Selected suite under `modules::codex_native_api` | Passed executions |
|---|---:|
| `request_upload::tests` | 5 |
| `tests::request_upload_http_tests` | 10 |
| `tests::request_upload_nack_tests` | 4 |
| `http_compression::large_compression_tests` | 7 |
| `network_recovery::tests` | 7 |
| `tests::compression_upstream_tests` | 3 |
| `http_compression::duplicate_image_tests` | 2 |
| Full `http_compression` correctness subset (overlaps above) | 9 |
| `bootstrap::tests` including network/overload HTTP regressions | 42 |
| `http::preparation_timeout_tests` | 10 |
| `blocking::tests` | 1 |
| `execution::layout_tests` | 1 |
| `tests::reasoning_policy_transport_tests` | 2 |

Counts were checked against `--list --skip ::compression_memory_` on the exact executable, not just parsed from interleaved progress lines. This matters because a fixture printing within a test can split its `test ... ok` line. The distinct listed test names and concise suite results are in [runtime-results.json](runtime-results.json).

The submitted portable correctness runner was also executed in `-RunOnly` mode against that hash and manifest. Its independent source check, test selection, isolated environment and all 26 new tests passed. This repeats coverage and is not added to the 94 distinct count. The public Windows memory runner's six normal-path executions passed separately; see the memory report.

The final public client retry-policy runner was exercised against all nine scenarios and matched the documented matrix, with no scenario timeout or remaining registered process/listener. Two terminal SSE cases required precisely identified test-descendant cleanup; they are not described as natural process-tree exit. Earlier wrapper failures are retained in the client report. The public PowerShell correctness runner's later output-drain hardening was separately checked with a normal-output fixture and an inherited-pipe fixture: the latter failed explicitly in 5.531 seconds instead of waiting indefinitely. That harness-only change did not alter Rust compilation inputs.

This is targeted regression coverage, not a claim that every test in the repository or every operating system was run. The independent CLI retry-policy scenarios are a separate client experiment, not additional Rust test passes.

## Frontend and release gates

The existing frontend guard passed all 6 tests, including absent/empty assets, development HTML and merged Tauri configuration. The production frontend build produced 172 files totaling 19,327,727 bytes at the configured `dist` location, with all 10 local index references validated. No protected source/config/lock file was changed by that frontend build.

- Frontend manifest tree SHA256: `25009725827490A59281FA3F092D4A50A068FFC57D35E3AD8C978E457CCF78F7`.
- `index.html` SHA256: `BEEA0AADBFCE81103FAD86004AB8575E8CC9754F754B1CC2350B59049DB7F631`.

The optimized Tauri/NSIS build completed successfully. The actual installer was extracted without executing it. Its **packaged executable**, not just the compiler output, passed the real isolated WebView verifier:

| Artifact/check | Verified result |
|---|---|
| NSIS installer | 40,851,087 bytes; SHA256 `F078E2BEEBD8AAD3D5017638EA8C5528CB35B68EC6FEE4BE0FD3B4451D3B3137` |
| Packaged EXE | 170,802,176 bytes (162.89 MiB); SHA256 `6EE688D6E2D6CA8759CE83D2268709505075044453F1170CCE4EEF217D4058DE` |
| Embedded frontend marker | `assets/index-` found |
| Actual WebView/DOM | Passed; 3 consecutive valid snapshots, stable for 2001 ms, visible application/main DOM, index loaded, 10 stylesheets |
| Resource/fatal errors | 0 / 0 |
| Owned frontend process/WebView cleanup | Exit 0; no scoped remaining processes |
| Bundled resources | All 19 matched source size and SHA256; two license notices retained |
| Signature | NotSigned; explicit community build |
| Installer version string | `1.5.2+preview.20261127.1`; Windows numeric installer version `1.5.2.0` |

Tauri CLI 2.12.6 temporarily patches `__TAURI_BUNDLE_TYPE_VAR_UNK` to `NSS` while constructing NSIS and then restores the original build-path file. Independent comparison found exactly three differing bytes (offsets 138592256–138592258), with all other bytes equal. Thus `target/release/cockpit-tools.exe` has SHA256 `D5954E8B62729FF2075E3CA3F5A2C7F2F476091934C3E65829C4C9E9A633385C`; it is not the hash of the file recipients install. Both were smoke-tested, but the **NSS packaged file's receipt is authoritative**. No manual binary patching or arbitrary hash exception was used.

The application EXE's Windows ProductVersion remains `1.5.2`; the installer carries the full preview label. The Tauri generated PackageInfo takes the supplied config version, but the About page was not separately inspected, so no claim is made that its rendered label was manually observed. The package filename and accompanying note explicitly identify it as a community preview.

Release warnings were retained (846 library warnings and one linker warning); successful build does not mean warning-free code. The installed production application was not replaced or restarted by this verification, and the installer was not run against the user's active installation.

## Limitations

- The slow-receiver red/green case is a same-build, zero-credit comparison, not a separate pristine-upstream build.
- 24 MiB was used for real throttled HTTP/1 and HTTP/2 transfer; 5/30/110 MiB were used for compression fidelity and fresh-process memory tests. These are different test scopes.
- The current native production client uses connect timeout/keepalive and reqwest's default protocol-NACK retry policy. The adapter preserves that configuration and explicit request timeout. It cannot inspect reqwest's private client-wide timeout or arbitrary third-party custom retry policies and does not promise equivalence for hypothetical custom Host clients with those policies.
- No real account exhaustion, revoked-account switching, arbitrary proxy outage or exactly-once recovery after image/tool execution is claimed.
- The preview is not an official author release. No full installation on a separate physical machine or clean Windows VM is implied by extracting its installer or testing an isolated WebView.
