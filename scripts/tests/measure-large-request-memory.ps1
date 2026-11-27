#requires -Version 7.0
<#
Measure the six ignored synthetic compression fixtures in a supplied Rust library
TEST executable, never the desktop executable. Requires Windows and PowerShell 7.
This script does not build, access production configuration or make API calls.
Each case runs in a fresh process; only that owned child may be terminated.
Outputs stay in a new unique subdirectory of the caller's ArtifactsDir.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$TestExecutable,
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$ArtifactsDir,
    [ValidateRange(30, 600)][int]$CaseTimeoutSeconds = 180
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows is required for process peak-memory counters.' }

function Assert-NoReparse([string]$Path) {
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Refusing reparse path: $cursor"
            }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Assert-Child([string]$Path, [string]$Root) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($Root.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path is outside the dedicated directory: $full"
    }
    Assert-NoReparse $full
    return $full
}

$artifact = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($TestExecutable)
Assert-NoReparse $artifact
if (-not (Test-Path -LiteralPath $artifact -PathType Leaf) -or [IO.Path]::GetExtension($artifact) -ne '.exe') {
    throw 'Expected an existing Rust library test EXE, not the desktop executable.'
}
if ([IO.Path]::GetFileName($artifact) -notmatch '^antigravity_cockpit_tools_lib-[0-9a-f]+\.exe$') {
    throw 'Use the original Cargo antigravity_cockpit_tools_lib-<hash>.exe test artifact; desktop or renamed executables are refused.'
}
$outputRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ArtifactsDir)
Assert-NoReparse $outputRoot
if ($outputRoot -eq [IO.Path]::GetPathRoot($outputRoot)) { throw 'Use a dedicated artifacts directory, not a drive root.' }
if ((Test-Path -LiteralPath $outputRoot) -and -not (Test-Path -LiteralPath $outputRoot -PathType Container)) {
    throw 'ArtifactsDir exists but is not a directory.'
}
$artifactHash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash
$runRoot = Assert-Child (Join-Path $outputRoot ('compression-memory-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N'))) $outputRoot
New-Item -ItemType Directory -Path $runRoot | Out-Null
Write-Host "Evidence: $runRoot"
$runner = Join-Path $runRoot 'compression-tests.exe'
Copy-Item -LiteralPath $artifact -Destination $runner
if ((Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash -ne $artifactHash) { throw 'Copied test EXE hash differs.' }
$manifest = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency><dependentAssembly>
    <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
  </dependentAssembly></dependency>
</assembly>
'@
[IO.File]::WriteAllText($runner + '.manifest', $manifest, [Text.UTF8Encoding]::new($false))

if (-not ('CockpitCompressionMemoryCountersV1' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CockpitCompressionMemoryCountersV1 {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters {
        public uint cb;
        public uint PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize;
        public UIntPtr QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage;
        public UIntPtr QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage;
        public UIntPtr PagefileUsage, PeakPagefileUsage, PrivateUsage;
    }
    [DllImport("psapi.dll", SetLastError = true)]
    public static extern bool GetProcessMemoryInfo(IntPtr process, out Counters counters, uint size);
}
'@
}
$counterSize = [uint32][Runtime.InteropServices.Marshal]::SizeOf([type][CockpitCompressionMemoryCountersV1+Counters])
$results = [Collections.Generic.List[object]]::new()
$failed = $false
foreach ($size in @(5, 30, 110)) {
    foreach ($mode in @('baseline', 'owned_body')) {
        $name = "compression_memory_${mode}_${size}_mib"
        $filter = 'modules::codex_native_api::http_compression::large_compression_tests::' + $name
        $caseRoot = Assert-Child (Join-Path $runRoot $name) $runRoot
        foreach ($directory in @($caseRoot, (Join-Path $caseRoot 'home'), (Join-Path $caseRoot 'appdata'), (Join-Path $caseRoot 'localappdata'), (Join-Path $caseRoot 'data'), (Join-Path $caseRoot 'temp'))) {
            New-Item -ItemType Directory -Path $directory -Force | Out-Null
        }
        $start = [Diagnostics.ProcessStartInfo]::new()
        $start.FileName = $runner
        $start.WorkingDirectory = $caseRoot
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        foreach ($arg in @($filter, '--exact', '--ignored', '--nocapture', '--test-threads=1')) { $start.ArgumentList.Add($arg) }
        # Forward OS/runtime setup only; do not let real provider credentials,
        # proxy settings or live-test opt-ins reach the synthetic test process.
        foreach ($key in @($start.Environment.Keys)) {
            if ($key -match '(?i)(key|token|secret|password|credential|authorization|proxy|live)|^(OPENAI|ANTHROPIC|CHATGPT|CODEX|COCKPIT|AGTOOLS|AWS|AZURE|GOOGLE|GITHUB|GITLAB|HF)_|^GIT_CONFIG') {
                [void]$start.Environment.Remove($key)
            }
        }
        $start.Environment['HOME'] = Join-Path $caseRoot 'home'
        $start.Environment['USERPROFILE'] = Join-Path $caseRoot 'home'
        $start.Environment['CODEX_HOME'] = Join-Path $caseRoot 'home'
        $start.Environment['APPDATA'] = Join-Path $caseRoot 'appdata'
        $start.Environment['LOCALAPPDATA'] = Join-Path $caseRoot 'localappdata'
        $start.Environment['TEMP'] = Join-Path $caseRoot 'temp'
        $start.Environment['TMP'] = Join-Path $caseRoot 'temp'
        $start.Environment['HOMEDRIVE'] = [IO.Path]::GetPathRoot($caseRoot).TrimEnd('\')
        $start.Environment['HOMEPATH'] = (Join-Path $caseRoot 'home').Substring([IO.Path]::GetPathRoot($caseRoot).Length - 1)
        foreach ($key in @('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME', 'XDG_STATE_HOME')) {
            $start.Environment[$key] = Join-Path $caseRoot 'data'
        }
        foreach ($key in @('COCKPIT_TOOLS_TEST_DATA_DIR', 'COCKPIT_TEST_DATA_DIR', 'COCKPIT_TOOLS_DATA_DIR')) { $start.Environment[$key] = Join-Path $caseRoot 'data' }
        $start.Environment['RUST_BACKTRACE'] = '0'
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $start
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $samples = 0
        $sampleFailures = 0
        $peakWorkingSet = [uint64]0
        $peakCommit = [uint64]0
        $maxObservedWorkingSet = [uint64]0
        $timedOut = $false
        $started = $false
        $finalRead = $false
        $processId = $null
        $exitCode = $null
        $stdout = ''
        $stderr = ''
        $caseError = $null
        $exited = $false
        $cleanupSucceeded = $false
        try {
            if (-not $process.Start()) { throw 'Test process failed to start.' }
            $started = $true
            $processId = $process.Id
            # Hold the real process handle until after the post-exit query. Do
            # not replace it with a PID lookup that could refer to a later process.
            $heldHandle = $process.SafeHandle
            $nativeHandle = $heldHandle.DangerousGetHandle()
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()
            do {
                $counters = [CockpitCompressionMemoryCountersV1+Counters]::new()
                $ok = [CockpitCompressionMemoryCountersV1]::GetProcessMemoryInfo($nativeHandle, [ref]$counters, $counterSize)
                if ($ok) {
                    $samples++
                    $peakWorkingSet = [Math]::Max($peakWorkingSet, $counters.PeakWorkingSetSize.ToUInt64())
                    $peakCommit = [Math]::Max($peakCommit, $counters.PeakPagefileUsage.ToUInt64())
                    $maxObservedWorkingSet = [Math]::Max($maxObservedWorkingSet, $counters.WorkingSetSize.ToUInt64())
                } else { $sampleFailures++ }
                if ($watch.Elapsed.TotalSeconds -gt $CaseTimeoutSeconds) {
                    $timedOut = $true
                    if (-not $process.HasExited) { $process.Kill() }
                    if (-not $process.WaitForExit(5000)) { throw 'Owned test process did not exit after timeout cleanup.' }
                    break
                }
            } while (-not $process.WaitForExit(20))
            $process.WaitForExit()
            $exited = $process.HasExited
            $exitCode = $process.ExitCode
            $counters = [CockpitCompressionMemoryCountersV1+Counters]::new()
            $finalRead = [CockpitCompressionMemoryCountersV1]::GetProcessMemoryInfo($nativeHandle, [ref]$counters, $counterSize)
            if ($finalRead) {
                $peakWorkingSet = [Math]::Max($peakWorkingSet, $counters.PeakWorkingSetSize.ToUInt64())
                $peakCommit = [Math]::Max($peakCommit, $counters.PeakPagefileUsage.ToUInt64())
            }
            [GC]::KeepAlive($heldHandle)
            if (-not $stdoutTask.Wait(5000) -or -not $stderrTask.Wait(5000)) { throw 'Test output capture did not complete.' }
            $stdout = $stdoutTask.GetAwaiter().GetResult()
            $stderr = $stderrTask.GetAwaiter().GetResult()
        } catch {
            $caseError = $_.Exception.Message
        } finally {
            # Only this fresh, known synthetic child is eligible for cleanup.
            try {
                if ($started -and -not $process.HasExited) {
                    $process.Kill()
                    if (-not $process.WaitForExit(5000)) { throw 'Owned test child remains alive after cleanup.' }
                }
                $cleanupSucceeded = -not $started -or $process.HasExited
            } catch {
                $caseError = (@($caseError, $_.Exception.Message) -ne $null) -join '; '
            } finally {
                $process.Dispose()
                $watch.Stop()
            }
        }
        [IO.File]::WriteAllText((Join-Path $caseRoot 'stdout.txt'), $stdout, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $caseRoot 'stderr.txt'), $stderr, [Text.UTF8Encoding]::new($false))
        $fixtureMode = if ($mode -eq 'owned_body') { 'owned-body' } else { 'baseline' }
        $verifiedPattern = 'phase=verified mode=' + $fixtureMode + ' input_mib=' + $size + ' plain_sha256=[0-9a-f]{64}'
        $passed = -not $caseError -and -not $timedOut -and $exited -and $exitCode -eq 0 -and
            $stdout -match 'test result: ok\. 1 passed; 0 failed;' -and $stderr -match $verifiedPattern -and
            $samples -gt 0 -and $sampleFailures -eq 0 -and $peakWorkingSet -gt 0 -and $finalRead -and $cleanupSucceeded
        $record = [pscustomobject]@{
            name = $name; filter = $filter; sizeMiB = $size; mode = $mode; pid = $processId
            exitCode = $exitCode; exited = $exited; timedOut = $timedOut; passed = $passed; error = $caseError
            elapsedMs = $watch.ElapsedMilliseconds; peakWorkingSetBytes = $peakWorkingSet
            peakPagefileUsageBytes = $peakCommit; maximumSampledWorkingSetBytes = $maxObservedWorkingSet
            successfulSamples = $samples; failedSamples = $sampleFailures; postExitQuerySucceeded = $finalRead
            cleanupSucceeded = $cleanupSucceeded; testOutputDirectory = $caseRoot
        }
        $results.Add($record)
        $record | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $caseRoot 'measurement.json') -Encoding utf8
        if (-not $passed) { $failed = $true }
        [pscustomobject]@{name=$name;passed=$record.passed;peakWorkingSetBytes=$record.peakWorkingSetBytes;elapsedMs=$record.elapsedMs} | ConvertTo-Json -Compress
        if (-not $cleanupSucceeded) { break }
    }
    if ($results.Count -gt 0 -and -not $results[$results.Count - 1].cleanupSucceeded) { break }
}
$artifactUnchanged = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash -eq $artifactHash
$runnerUnchanged = (Get-FileHash -LiteralPath $runner -Algorithm SHA256).Hash -eq $artifactHash
$failed = $failed -or $results.Count -ne 6 -or -not $artifactUnchanged -or -not $runnerUnchanged
$summary = [pscustomobject]@{
    schema = 1
    capturedAt = (Get-Date).ToUniversalTime().ToString('o'); artifactSha256 = $artifactHash
    sourceArtifact = $artifact; runner = $runner; scope = 'Fresh synthetic processes; no live endpoint, account, credentials or production configuration.'
    sourceArtifactUnchanged = $artifactUnchanged; copiedRunnerUnchanged = $runnerUnchanged
    measurement = 'Windows process memory counters sampled every 20 ms; retained handle queried after exit; process-wide OS peaks, not allocator-only or production measurements.'
    passed = -not $failed; cases = @($results.ToArray())
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runRoot 'summary.json') -Encoding utf8
Write-Output (Join-Path $runRoot 'summary.json')
if ($failed) { exit 1 }
