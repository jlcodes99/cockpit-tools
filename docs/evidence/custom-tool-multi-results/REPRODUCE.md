# Reproducing the targeted Windows regression suites

This is a portable restatement of the build and isolated-runner steps used for
the final clean-main candidate, with machine-specific toolchain paths removed.
The commands below have been checked against that runner, but this portable
snippet has **not itself been executed**. The recorded executions and exact
artifact hashes remain in [clean-main-validation.json](clean-main-validation.json)
and [test-logs](test-logs/).

Use a disposable **PowerShell 7** session with the x64 Visual Studio developer
environment initialized. Rust's active host/target must be
`x86_64-pc-windows-msvc`; Cargo, MSVC and the Windows SDK must already be installed
and available in that shell. Cargo's
dependencies must already be cached: the build below is deliberately offline
and stops if dependencies are missing. Prepare missing dependencies separately;
this recipe does not download or install anything. Run from the root of the
checked-out PR source, with `Cargo.lock` intact, and do not edit sources while
building or testing. The library test build uses Tauri's development context;
it does not require a release frontend build or start Cockpit.

Build artifacts, test data, logs and temporary files go under a new directory in this checkout's
`target/custom-tool-multi-results/`. The three Cockpit data overrides and the
temporary directory point there. Live-test opt-ins and proxy variables are
removed from this process; the five fixed filters below select synthetic tests,
including local mock HTTP servers. Do not add `--ignored` or a live-test filter.
These environment changes are process-local; close this disposable shell when
finished. The script neither restarts a service nor changes installed binaries.

```powershell
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repoRoot = [IO.Path]::GetFullPath((Get-Location).Path)

function Assert-NoLinks([string]$Path) {
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band
                [IO.FileAttributes]::ReparsePoint) {
                throw "Refusing a reparse point: $cursor"
            }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Assert-RepoOutput([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($repoRoot.TrimEnd('\') + '\',
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "Output outside this checkout: $full"
    }
    Assert-NoLinks $full
}

Assert-NoLinks $repoRoot
foreach ($relative in @('Cargo.toml', 'Cargo.lock', 'src-tauri\Cargo.toml')) {
    if (-not (Test-Path -LiteralPath (Join-Path $repoRoot $relative))) {
        throw 'Start in the root of the checked-out PR source.'
    }
}
$cargo = (Get-Command cargo.exe -CommandType Application).Source
$runRoot = Join-Path $repoRoot ('target\custom-tool-multi-results\' +
    (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N'))
foreach ($path in @($runRoot, (Join-Path $runRoot 'data'),
        (Join-Path $runRoot 'temp'), (Join-Path $runRoot 'build'))) {
    Assert-RepoOutput $path
    New-Item -ItemType Directory -Path $path -Force | Out-Null
}
$env:COCKPIT_TOOLS_TEST_DATA_DIR = Join-Path $runRoot 'data'
$env:COCKPIT_TEST_DATA_DIR = $env:COCKPIT_TOOLS_TEST_DATA_DIR
$env:COCKPIT_TOOLS_DATA_DIR = $env:COCKPIT_TOOLS_TEST_DATA_DIR
$env:TEMP = Join-Path $runRoot 'temp'
$env:TMP = $env:TEMP
Get-ChildItem Env: | Where-Object { $_.Name -match '^COCKPIT_.*LIVE' } |
    ForEach-Object {
        [Environment]::SetEnvironmentVariable($_.Name, $null, 'Process')
    }
foreach ($name in @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY',
        'http_proxy', 'https_proxy', 'all_proxy')) {
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
$env:NO_PROXY = '*'
$env:CARGO_TARGET_DIR = Join-Path $runRoot 'build'
$env:CARGO_BUILD_JOBS = '4'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_NET_OFFLINE = 'true'

$artifact = $null
$buildLog = Join-Path $runRoot 'build.log'
$buildArgs = @('test', '--locked', '-p', 'cockpit-tools', '--lib',
    '--no-run', '--message-format=json-render-diagnostics', '--offline')
$ErrorActionPreference = 'Continue'
& $cargo @buildArgs 2>&1 | Tee-Object -FilePath $buildLog | ForEach-Object {
    try { $event = [string]$_ | ConvertFrom-Json -ErrorAction Stop }
    catch { $event = $null }
    if ($event.reason -eq 'compiler-artifact' -and $event.executable -and
        $event.profile.test -eq $true -and
        $event.target.name -eq 'antigravity_cockpit_tools_lib') {
        $artifact = [string]$event.executable
    } elseif ($event.reason -eq 'compiler-message') {
        if ($event.message.level -eq 'error') { Write-Output $event.message.rendered }
    } elseif (-not $event) { Write-Output $_ }
}
$buildExit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($buildExit -ne 0) { throw "Cargo failed ($buildExit); see $buildLog" }
if (-not $artifact) { throw 'Cargo did not report the library test executable.' }
Assert-RepoOutput $artifact
if (-not ([IO.Path]::GetFullPath($artifact)).StartsWith(
        $env:CARGO_TARGET_DIR + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Cargo reported an artifact outside this isolated build directory.'
}

$artifactHash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash
$runner = Join-Path $runRoot 'isolated-tests.exe'
Copy-Item -LiteralPath $artifact -Destination $runner
$runnerHash = (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash
if ($runnerHash -ne $artifactHash) { throw 'Copied runner differs from the build.' }
$manifest = Join-Path $runRoot 'common-controls.manifest'
@'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="amd64" name="Cockpit.IsolatedTests" type="win32"/>
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="amd64" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
</assembly>
'@ | Set-Content -LiteralPath $manifest -Encoding utf8
Copy-Item -LiteralPath $manifest -Destination ($runner + '.manifest')
# Sidecar manifest: retain the compiler artifact's bytes, enable Common Controls v6.
$env:PATH = $env:SystemRoot + '\System32;' + $env:SystemRoot

$filters = @(
    'custom_results_',
    'modules::codex_native_api::codex_wire::',
    'modules::codex_native_api::identity::',
    'modules::codex_native_api::context_rebuild_http_tests::',
    'modules::codex_native_api::retry::'
)
$results = @()
foreach ($filter in $filters) {
    $log = Join-Path $runRoot (($filter -replace '[^a-zA-Z0-9_-]', '_') + '.log')
    $ErrorActionPreference = 'Continue'
    $lines = @(& $runner $filter '--test-threads=1' 2>&1)
    $testExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    $lines | Tee-Object -FilePath $log
    $nonemptySuccess = [bool]($lines -match
        'test result: ok\. [1-9][0-9]* passed; 0 failed; 0 ignored;')
    $results += [pscustomobject]@{
        filter = $filter; exitCode = $testExit
        nonemptySuccess = $nonemptySuccess; log = $log
    }
}
[pscustomobject]@{
    originalArtifactHash = $artifactHash; runnerHash = $runnerHash
    buildLog = $buildLog; manifest = $manifest; results = $results
} | ConvertTo-Json -Depth 5 |
    Set-Content -LiteralPath (Join-Path $runRoot 'result.json') -Encoding utf8
if (@($results | Where-Object {
    $_.exitCode -ne 0 -or -not $_.nonemptySuccess
}).Count -gt 0) {
    throw "A suite failed, matched no tests, or ignored tests; inspect $runRoot"
}
Write-Output "All five nonempty suites passed; evidence: $runRoot"
```

At the recorded final candidate, these filters produced **8, 14, 129, 75 and 6**
passes: **232 executions, 231 unique tests**, with zero failures or ignored
tests. One checkpoint/restart test matches two filters. Counts can change after
subsequent source changes; a zero-test exit is never accepted. This is a targeted
regression run, not the complete library suite, a production deployment, or a
real-account failover test. Artifact hashes can differ across toolchains and
build paths; the historical hashes identify the actual recorded build rather
than promising reproducible binaries on every machine.
