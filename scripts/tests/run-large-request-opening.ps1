#requires -Version 7.0
<#
Build and run only the synthetic large-request regressions. Requires the normal
Rust/platform build environment. Never starts the desktop app or uses accounts.
RunOnly requires an exact artifact hash and its saved compilation input manifest.
#>
[CmdletBinding()]
param(
    [string]$Cargo = 'cargo',
    [string]$TargetDirectory = '',
    [string]$ArtifactsDirectory = '',
    [switch]$Offline,
    [switch]$RunOnly,
    [string]$TestExecutable = '',
    [string]$ExpectedArtifactSha256 = '',
    [string]$SourceManifest = '',
    [switch]$IncludeAdjacent
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-PlainPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    for ($cursor = $full; $cursor; $cursor = [IO.Path]::GetDirectoryName($cursor)) {
        if ((Test-Path -LiteralPath $cursor) -and
            ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing link/reparse path: $cursor"
        }
    }
    return $full
}
function Write-Json([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, (ConvertTo-Json -InputObject $Value -Depth 9), [Text.UTF8Encoding]::new($false))
}
function Get-SourceSnapshot {
    $listed = & git -C $repo ls-files -z --cached --others --exclude-standard
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inventory tracked and new compilation inputs.' }
    $paths = @((($listed -join "`n").Split([char]0)) | Where-Object {
        $_ -and ($_ -like '*.rs' -or $_ -match '(^|/)(Cargo\.toml|Cargo\.lock|tauri[^/]*\.json)$')
    } | Select-Object -Unique)
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $manifest = @(foreach ($relative in $paths) {
        $path = Assert-PlainPath (Join-Path $repo $relative)
        if (-not $path.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Compilation input escaped the checkout.'
        }
        [ordered]@{ path = $relative; bytes = (Get-Item -LiteralPath $path).Length
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $serialized = ConvertTo-Json -InputObject $manifest -Compress -Depth 4
    $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($serialized))).ToLowerInvariant()
    return [ordered]@{ count = $manifest.Count; sha256 = $hash; manifest = $manifest }
}
function Assert-SameSource($Expected, $Actual) {
    $expectedFiles = @($Expected.manifest)
    $actualFiles = @($Actual.manifest)
    if ($expectedFiles.Count -eq 0 -or $expectedFiles.Count -ne $actualFiles.Count) { throw 'Compilation input inventory changed or is empty.' }
    $expectedByPath = @{}
    foreach ($entry in $expectedFiles) {
        if ($expectedByPath.ContainsKey($entry.path)) { throw 'Duplicate manifest entry.' }
        $expectedByPath[$entry.path] = $entry
    }
    foreach ($entry in $actualFiles) {
        $saved = $expectedByPath[$entry.path]
        if (-not $saved -or $saved.sha256 -ne $entry.sha256 -or $saved.bytes -ne $entry.bytes) {
            throw "Compilation input differs from the supplied artifact manifest: $($entry.path)"
        }
    }
}
function Invoke-Logged([string]$Program, [string[]]$Arguments, [string]$Name, [hashtable]$Overrides = @{}, [switch]$Test) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Program
    $info.WorkingDirectory = $repo
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $null = $info.ArgumentList.Add($argument) }
    foreach ($key in @($info.Environment.Keys)) {
        if ($key -eq 'TAURI_CONFIG' -or $key -like 'TAURI_ENV_*' -or
            $key -match '^COCKPIT_.*(?:LIVE|PROBE)' -or
            $key -match '(?i)(^|_)(API_?KEY|TOKEN|SECRET|PASSWORD|AUTHORIZATION|ACCESS_KEY|CLIENT_SECRET)($|_)') {
            $null = $info.Environment.Remove($key)
        }
        if ($Test -and $key -match '(?i)^(HTTP|HTTPS|ALL|FTP|NO)_PROXY$') { $null = $info.Environment.Remove($key) }
    }
    if ($Test) { $info.Environment['NO_PROXY'] = 'localhost,127.0.0.1,::1' }
    foreach ($key in $Overrides.Keys) { $info.Environment[$key] = $Overrides[$key] }
    $stdoutPath = Join-Path $run "$Name.stdout.log"
    $stderrPath = Join-Path $run "$Name.stderr.log"
    $stdout = [IO.File]::Create($stdoutPath)
    $stderr = [IO.File]::Create($stderrPath)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $started = [DateTimeOffset]::UtcNow
    $processStarted = $false
    $outputDrained = $false
    $copies = @()
    $copyCancellation = [Threading.CancellationTokenSource]::new()
    try {
        if (-not $process.Start()) { throw "Cannot start $Name" }
        $processStarted = $true
        $outCopy = $process.StandardOutput.BaseStream.CopyToAsync($stdout, 81920, $copyCancellation.Token)
        $errCopy = $process.StandardError.BaseStream.CopyToAsync($stderr, 81920, $copyCancellation.Token)
        $copies = [Threading.Tasks.Task[]]@($outCopy, $errCopy)
        $watchdogMs = if ($Test) { 180000 } else { 3600000 }
        if (-not $process.WaitForExit($watchdogMs)) {
            throw "Owned process watchdog expired for $Name ($watchdogMs ms)."
        }
        # Exiting the root process does not guarantee EOF: an inherited pipe in
        # a child can remain open. Such incomplete capture must fail, not wait
        # forever or be reported as a fully verified test/build result.
        if (-not [Threading.Tasks.Task]::WaitAll($copies, 5000)) {
            throw "Owned output capture did not finish within 5 seconds for $Name."
        }
        $outputDrained = $true
        return [pscustomobject]@{ name = $Name; exitCode = $process.ExitCode
            startedAt = $started.ToString('o'); completedAt = [DateTimeOffset]::UtcNow.ToString('o')
            stdout = $stdoutPath; stderr = $stderrPath }
    } finally {
        # Only the exact process object created by this invocation is owned.
        # Never find/stop existing apps or processes by executable name.
        try {
            if ($processStarted -and -not $process.HasExited) {
                $process.Kill($true)
                if (-not $process.WaitForExit(10000)) { throw "Owned process did not exit: $Name" }
            }
        } finally {
            # Close only the streams created above; do not discover or terminate
            # unrelated descendants after the owned root has already exited.
            if (-not $outputDrained) {
                $copyCancellation.Cancel()
                if ($processStarted) {
                    $process.StandardOutput.Dispose()
                    $process.StandardError.Dispose()
                }
                if ($copies.Count -gt 0) {
                    try { $null = [Threading.Tasks.Task]::WaitAll($copies, 1000) } catch { }
                }
            }
            $copyCancellation.Dispose()
            $stdout.Dispose(); $stderr.Dispose(); $process.Dispose()
        }
    }
}
function Show-ShortTail([string]$Path) {
    $last = [Collections.Generic.Queue[string]]::new()
    foreach ($line in [IO.File]::ReadLines($Path)) {
        $last.Enqueue($(if ($line.Length -gt 500) { $line.Substring(0, 500) + ' [truncated]' } else { $line }))
        if ($last.Count -gt 12) { $null = $last.Dequeue() }
    }
    foreach ($line in $last) { Write-Host $line }
}

$repo = Assert-PlainPath (Join-Path $PSScriptRoot '../..')
if (-not (Test-Path -LiteralPath (Join-Path $repo 'src-tauri/src/modules/codex_native_api/request_upload.rs'))) { throw 'Large-request sources are absent.' }
if (-not $ArtifactsDirectory) { $ArtifactsDirectory = Join-Path $repo 'target/large-request-opening' }
$output = Assert-PlainPath $ArtifactsDirectory
if ($output -eq [IO.Path]::GetPathRoot($output) -or $output -eq $repo) { throw 'Use a dedicated artifacts directory.' }
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $output 'build' }
$buildTarget = Assert-PlainPath $TargetDirectory
$run = Assert-PlainPath (Join-Path $output ('runs/' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)))
$null = New-Item -ItemType Directory -Path $run
$receipt = [ordered]@{ schema = 1; success = $false; source = $null; build = $null; artifact = $null; suites = @(); uniquePassed = 0; error = $null }
Write-Host "Evidence: $run"
try {
    $snapshot = Get-SourceSnapshot
    Write-Json (Join-Path $run 'source-before.json') $snapshot
    $receipt.source = @{ sha256 = $snapshot.sha256; count = $snapshot.count }
    if ($RunOnly) {
        if (-not $TestExecutable -or $ExpectedArtifactSha256 -notmatch '^[0-9a-fA-F]{64}$' -or -not $SourceManifest) {
            throw 'RunOnly requires TestExecutable, ExpectedArtifactSha256 and SourceManifest from that build.'
        }
        $saved = Get-Content -LiteralPath (Assert-PlainPath $SourceManifest) -Raw | ConvertFrom-Json
        Assert-SameSource $saved $snapshot
        $source = Assert-PlainPath $TestExecutable
    } else {
        $cargoPath = (Get-Command $Cargo -CommandType Application -ErrorAction Stop).Source
        $buildArgs = @('test', '--locked', '-p', 'cockpit-tools', '--lib', '--no-run', '--message-format=json', '--target-dir', $buildTarget)
        if ($Offline) { $buildArgs += '--offline' }
        $receipt.build = Invoke-Logged $cargoPath $buildArgs 'build'
        if ($receipt.build.exitCode -ne 0) { Show-ShortTail $receipt.build.stderr; throw 'Cargo build failed.' }
        $artifacts = @(@(foreach ($line in [IO.File]::ReadLines($receipt.build.stdout)) {
            if ([string]::IsNullOrWhiteSpace($line)) { continue }
            $message = $line | ConvertFrom-Json
            if ($message.reason -eq 'compiler-artifact' -and $message.profile.test -and
                $message.target.name -eq 'antigravity_cockpit_tools_lib' -and $message.executable) { $message.executable }
        }) | Select-Object -Unique)
        if ($artifacts.Count -ne 1) { throw 'Cargo must return exactly one library-test executable.' }
        $source = Assert-PlainPath $artifacts[0]
        if (-not $source.StartsWith($buildTarget + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Compiler artifact escaped the selected target.' }
        Assert-SameSource $snapshot (Get-SourceSnapshot)
    }
    $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    if ($RunOnly -and $hash -ne $ExpectedArtifactSha256) { throw 'Test executable hash mismatch.' }
    $testExe = Join-Path $run ([IO.Path]::GetFileName($source))
    Copy-Item -LiteralPath $source -Destination $testExe
    if ((Get-FileHash -LiteralPath $testExe -Algorithm SHA256).Hash -ne $hash) { throw 'Copied executable hash mismatch.' }
    if ($IsWindows) {
        $manifest = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="Cockpit.LargeRequest.Tests" type="win32"/>
  <dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
</assembly>
'@
        [IO.File]::WriteAllText($testExe + '.manifest', $manifest, [Text.UTF8Encoding]::new($false))
    }
    $receipt.artifact = @{ source = $source; sha256 = $hash; bytes = (Get-Item -LiteralPath $source).Length }
    $filters = @(
        'modules::codex_native_api::request_upload::tests::',
        'modules::codex_native_api::tests::request_upload_http_tests::',
        'modules::codex_native_api::tests::request_upload_nack_tests::',
        'modules::codex_native_api::http_compression::large_compression_tests::'
    )
    if ($IncludeAdjacent) { $filters += @(
        'modules::codex_native_api::network_recovery::tests::',
        'modules::codex_native_api::tests::compression_upstream_tests::',
        'modules::codex_native_api::http_compression::duplicate_image_tests::'
    ) }
    $unique = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $index = 0
    foreach ($filter in $filters) {
        $index++
        $data = Join-Path $run "suite-$index/data"
        $temporary = Join-Path $run "suite-$index/tmp"
        $userDirectory = Join-Path $run "suite-$index/home"
        $roaming = Join-Path $userDirectory 'AppData/Roaming'
        $local = Join-Path $userDirectory 'AppData/Local'
        $null = New-Item -ItemType Directory -Path $data, $temporary, $userDirectory, $roaming, $local
        $overrides = @{ COCKPIT_TOOLS_TEST_DATA_DIR = $data; COCKPIT_TEST_DATA_DIR = $data; COCKPIT_TOOLS_DATA_DIR = $data
            TEMP = $temporary; TMP = $temporary; HOME = $userDirectory; USERPROFILE = $userDirectory
            APPDATA = $roaming; LOCALAPPDATA = $local; XDG_CONFIG_HOME = (Join-Path $userDirectory '.config')
            XDG_DATA_HOME = (Join-Path $userDirectory '.local/share'); CODEX_HOME = (Join-Path $userDirectory '.codex') }
        # Six explicit ignored memory benchmarks are separate process measurements,
        # not part of the normal correctness suite or its pass count.
        $selection = @($filter, '--skip', '::compression_memory_')
        $list = Invoke-Logged $testExe ($selection + @('--list', '--format', 'terse')) "suite-$index-list" $overrides -Test
        $names = @([IO.File]::ReadAllLines($list.stdout) | Where-Object { $_ -match ': test$' } | ForEach-Object { $_.Substring(0, $_.Length - 6) })
        if ($list.exitCode -ne 0 -or $names.Count -eq 0) { throw "Empty or failed test selection: $filter" }
        $tests = Invoke-Logged $testExe ($selection + @('--test-threads=1', '--nocapture', '--color=never')) "suite-$index-tests" $overrides -Test
        $summary = $null
        foreach ($line in [IO.File]::ReadLines($tests.stdout)) {
            $match = [regex]::Match($line, '^test result: ok\. (\d+) passed; 0 failed; 0 ignored; 0 measured;')
            if ($match.Success) { $summary = $match }
        }
        $record = [ordered]@{ filter = $filter; listed = $names.Count; list = $list; tests = $tests; passed = 0 }
        $receipt.suites += $record
        if ($tests.exitCode -ne 0 -or -not $summary -or [int]$summary.Groups[1].Value -ne $names.Count) {
            Show-ShortTail $tests.stdout; Show-ShortTail $tests.stderr
            throw "Test suite failed or did not run all selected cases: $filter"
        }
        $record.passed = $names.Count
        foreach ($name in $names) { $null = $unique.Add($name) }
        Write-Host "PASS $($names.Count): $filter"
    }
    $after = Get-SourceSnapshot
    Assert-SameSource $snapshot $after
    Write-Json (Join-Path $run 'source-after.json') $after
    $receipt.uniquePassed = $unique.Count
    $receipt.success = $true
} catch {
    $receipt.error = $_.Exception.Message
    throw
} finally {
    Write-Json (Join-Path $run 'receipt.json') $receipt
}
Write-Host "PASS: $($receipt.uniquePassed) unique tests; executable SHA256 $hash"
