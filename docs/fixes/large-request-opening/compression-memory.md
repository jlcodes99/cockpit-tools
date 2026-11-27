# Large-request compression memory measurement

The change removes a temporary plaintext-sized copy from request compression. It moves the existing request body into the bounded worker, borrows its bytes during zstd encoding, and retains the original body for the existing safe fallback. It does not change compression level, compressed representation, duplicate-inline-image exclusions, or upstream replay rules.

## Reproduce on Windows

Use Windows and PowerShell 7. First build the Rust **library test executable** with the regression runner described in [README.md](README.md). Obtain the exact executable path and SHA256 from that build's receipt. Keep Cargo's original `antigravity_cockpit_tools_lib-<hash>.exe` filename: the script rejects desktop or renamed executables before launching anything. Do not select an arbitrary old artifact by filename glob.

From the repository root:

```powershell
# Set this to the exact library test artifact from the saved build receipt.
$testExe = 'PATH_FROM_BUILD_RECEIPT/antigravity_cockpit_tools_lib-HASH.exe'
pwsh -NoProfile -File scripts/tests/measure-large-request-memory.ps1 `
  -TestExecutable $testExe `
  -ArtifactsDir ./target/compression-memory
```

The measurement script does not compile or prove that an arbitrary supplied executable matches the current source. Preserve the original build receipt and its source manifest alongside the generated measurement summary. The script records the supplied executable's SHA256, copies it to a new unique run directory, checks the copy, and checks both hashes again at completion.

Exactly six isolated processes run the ignored fixtures: historical-copy baseline and owned-body implementation, each at 5, 30 and 110 MiB. Each uses an exact test filter, `--ignored --nocapture --test-threads=1`, and must report exactly one passing test plus the expected size/mode verification marker. An absent or mismatched fixture cannot pass as an empty successful test selection.

Every process has fresh home, application-data, test-data and temporary directories. Credential/provider, proxy and LIVE opt-in environment variables are removed from the **child environment**; the calling shell's environment is not modified. These known fixtures only compress and verify synthetic bytes and do not open network connections or load production configuration. Environment isolation is not an OS security sandbox for an arbitrary executable supplied by the caller.

The script samples Windows `GetProcessMemoryInfo` every 20 ms, retaining the exact process handle until a final query after exit. The reported `PeakWorkingSetSize` and `PeakPagefileUsage` are OS high-water counters; `maximumSampledWorkingSetBytes` is a separate sampled value and must not be substituted for the peak. A passing measurement requires zero sample errors, a successful final query, exit code zero, the expected fixture output, and confirmed termination of the owned child. A timeout fails the case; cleanup only terminates the exact process started for that case. A cleanup failure stops further cases and fails the run. It never kills processes by application name or deletes existing artifacts.

Each unique run directory contains the copied executable and manifest, per-case stdout/stderr and `measurement.json`, plus `summary.json`. Failures remain available for inspection. Raw local receipts naturally contain the caller's local paths and process IDs; review/redact those before public sharing. No environment values or account data are recorded by the measurement script.

## Recorded measurement

One complete pass was measured on Windows on 2026-11-27 using a debug library test executable with SHA256:

```text
BF3787C610F61E1E17F7A740A3DFC789809B42AD39081EDBD58DFE24919C73F5
```

These values come from the original isolated measurement runner on which the portable script is based. They are not a claim that the subsequently packaged portable wrapper or a final release executable was rerun for these numbers.

| Synthetic input | Historical-copy peak working set | Owned-body peak working set | Reduction |
|---|---:|---:|---:|
| 5 MiB | 30.770 MiB | 23.895 MiB | 6.875 MiB |
| 30 MiB | 105.418 MiB | 75.398 MiB | 30.020 MiB |
| 110 MiB | 360.988 MiB | 250.957 MiB | 110.031 MiB |

All six cases exited zero. All final post-exit counter queries succeeded, with zero sampling errors. Independent verification confirmed matching source/copied executable hashes and no remaining process at the exact copied-runner path.

Both modes produced the same compressed length per size: 3,933,507; 23,598,111; and 86,524,827 bytes. Each fixture verified **every decoded byte**, matching plaintext SHA256 and preservation of the original fallback allocation. Decompression verification used fixed 64 KiB scratch space, avoiding another full plaintext allocation that could hide the compression change in process peak counters.

The baseline fixture reproduces the previous plaintext `to_vec()` compression algorithm inside the same executable. It is not an independently installed old application. Both fixtures include their own synthetic-input construction and byte verification, so these are whole **test-process** peaks, not an allocator-only measure of the encoder or the memory footprint of the desktop application.

## Portable runner normal-path acceptance

The portable script was subsequently run once on 2026-11-27 against the second debug test build. This separate run validates the published wrapper's successful execution path; it does not replace the original measurements above. The compression implementation and fixture source hashes were unchanged between the two builds.

```text
Portable script SHA256:
F62018345DC10FA1B53D9FAEF2E454820871EA152DE7FCA7F5E526C98E9EE531
Second library test executable SHA256:
1F5D42083BCDA8D3E428EDCB33A9514AFA853C6D5E04A437B7E4EDA4D6A180B4
```

All six cases passed and the wrapper exited zero. All six post-exit memory queries succeeded, with zero sampling failures, confirmed child exit and cleanup, and matching source/copied executable hashes. An independent exact-runner-path process check found zero leftovers. Both modes again produced matching compressed lengths and plaintext hashes at all three sizes.

| Synthetic input | Historical-copy peak working set | Owned-body peak working set | Reduction |
|---|---:|---:|---:|
| 5 MiB | 30.797 MiB | 23.938 MiB | 6.859 MiB |
| 30 MiB | 104.992 MiB | 74.980 MiB | 30.012 MiB |
| 110 MiB | 361.000 MiB | 250.988 MiB | 110.012 MiB |

These values differ slightly from the initial pass, as expected for process-memory measurements. A release compilation was running separately; available physical memory was approximately 7.1 GiB before and 6.0 GiB after the wrapper check. Treat this run as wrapper acceptance and a repeated observation of the copy reduction, not as a controlled speed benchmark. No production application or configuration was changed, and the ordinary Rust functional suites were not rerun as part of this wrapper check.

## Limits and interpretation

The 30 and 110 MiB differences support removal of one plaintext-sized temporary allocation. The result does not imply that all application memory scales down by that amount, that the wire body becomes smaller, or that previously failing production requests now succeed.

This is one fresh-process pair per size, not a statistical benchmark. Encoder times were 76 to 95 ms, 580 to 511 ms, and 2,428 to 2,078 ms, respectively. The smallest case was slower in this pass. Do not claim universal speedup, extrapolate debug-test timing to release builds, or attribute long network stalls to compression from these figures alone. OS working-set behavior, allocator state, synthetic data distribution and machine load affect results.

Functional regression and package/frontend validation remain separate requirements. This measurement does not verify actual account switching, provider quotas, proxy health, interrupted image delivery, or safe replay after downstream output has already been sent.
