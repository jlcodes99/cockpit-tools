#requires -Version 5.1
<##
Verify the exact release EXE in its explicit isolated frontend-smoke mode.
The application source intentionally remains identical to the tested release.
App::run does not return, so exit code zero alone is NEVER a passing result.
##>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [ValidateRange(50, 180)][int]$TimeoutSeconds = 60
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-PlainAbsolutePath([string]$Path) {
    if (-not [IO.Path]::IsPathRooted($Path)) { throw 'Use an absolute path.' }
    $full = [IO.Path]::GetFullPath($Path)
    $cursor = $full
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Refusing a link or reparse point: $cursor"
            }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    return $full
}

function Get-Sha([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Test-AssetMarker([string]$Path) {
    # Search the actual bytes, including markers split across read boundaries.
    $needle = 'assets/index-'
    $stream = [IO.File]::OpenRead($Path)
    try {
        $buffer = [byte[]]::new(1048576)
        $tail = ''
        while (($count = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $chunk = $tail + [Text.Encoding]::ASCII.GetString($buffer, 0, $count)
            if ($chunk.IndexOf($needle, [StringComparison]::Ordinal) -ge 0) { return $true }
            $tail = $chunk.Substring([Math]::Max(0, $chunk.Length - $needle.Length + 1))
        }
        return $false
    } finally { $stream.Dispose() }
}

function Write-Json([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
}

function Stop-OwnedTestProcess($Process, [datetime]$StartTime, [string]$Path) {
    $Process.Refresh()
    if ($Process.HasExited) { return }
    $current = Get-Process -Id $Process.Id -ErrorAction SilentlyContinue
    if ($null -eq $current) { return }
    if ($current.Path -ine $Path -or $current.StartTime -ne $StartTime) {
        throw 'Test PID identity changed; refusing to stop an unrelated process.'
    }
    # Stop only the exact host launched here. Never kill by name, a process tree,
    # a browser, or any existing Cockpit instance.
    Stop-Process -InputObject $current -Force
    if (-not $Process.WaitForExit(5000)) { throw 'Owned test process did not exit after cleanup.' }
}

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This WebView2 release verifier requires Windows.'
}
$Executable = Assert-PlainAbsolutePath $Executable
$EvidenceRoot = Assert-PlainAbsolutePath $EvidenceRoot
if (-not [IO.File]::Exists($Executable)) { throw 'Release executable does not exist.' }
if (-not [IO.Directory]::Exists($EvidenceRoot)) { throw 'EvidenceRoot must be an existing directory.' }
$ExpectedSha256 = $ExpectedSha256.ToUpperInvariant()
if ((Get-Sha $Executable) -ne $ExpectedSha256) { throw 'Release SHA256 differs from the expected artifact.' }
if (-not (Test-AssetMarker $Executable)) { throw 'Release executable lacks the assets/index- marker.' }

# Only create a fresh, uniquely named child of the caller-selected evidence root.
# Existing directories or old receipts can never satisfy this invocation.
$runName = 'frontend-smoke-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N')
$run = [IO.Path]::GetFullPath((Join-Path $EvidenceRoot $runName))
$rootPrefix = $EvidenceRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $run.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $run)) {
    throw 'Evidence directory is not a fresh child of EvidenceRoot.'
}
$null = New-Item -ItemType Directory -Path $run -ErrorAction Stop
$run = Assert-PlainAbsolutePath $run
[IO.File]::WriteAllText((Join-Path $run 'frontend-smoke.allowed'), 'cockpit-frontend-smoke-v1', [Text.UTF8Encoding]::new($false))
$process = $null
$testStartTime = [datetime]::MinValue
$startedAt = [DateTimeOffset]::UtcNow
try {
    $process = Start-Process -FilePath $Executable -ArgumentList @('--frontend-smoke-test', ('"' + $run + '"')) `
        -WorkingDirectory ([IO.Path]::GetDirectoryName($Executable)) -WindowStyle Hidden -PassThru
    # Hold the handle immediately: on Windows this also makes ExitCode reliable
    # after a short-lived Start-Process -PassThru child has terminated.
    $testHandle = $process.Handle
    $testStartTime = $process.StartTime
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-OwnedTestProcess $process $testStartTime $Executable
        throw 'Isolated WebView test exceeded its deadline.'
    }
    $exitCode = $process.ExitCode
    $evidencePath = Assert-PlainAbsolutePath (Join-Path $run 'webview-evidence.json')
    if (-not [IO.File]::Exists($evidencePath)) { throw 'WebView did not write its evidence file.' }
    $evidence = Get-Content -LiteralPath $evidencePath -Raw | ConvertFrom-Json
    if ($null -eq $exitCode -or $exitCode -ne 0 -or $evidence.status -ne 'Passed' `
        -or $evidence.artifactSha256 -ne $ExpectedSha256 -or $evidence.pid -ne $process.Id `
        -or $evidence.executable -ine $Executable -or $evidence.frontendReady -ne $true `
        -or $evidence.eventLoopExited -ne $true) {
        throw 'WebView exit, artifact identity, or final Passed evidence did not match.'
    }
    if ($evidence.productionHandlersRegistered -ne $false -or $evidence.productionPluginsRegistered -ne $false `
        -or $evidence.webviewDataDirectory -ine (Join-Path $run 'webview')) {
        throw 'WebView evidence does not match the isolated test scope.'
    }
    if ('script_loaded' -notin @($evidence.frontendStages) `
        -or @($evidence.pageLoads | Where-Object { $_.event -eq 'Finished' -and $_.url -match '^(http://tauri\.localhost/|tauri://localhost/)' }).Count -lt 1 `
        -or $evidence.consecutiveValidSnapshots -lt 2 -or $evidence.stableForMs -lt 2000) {
        throw 'Page completion, real frontend startup, or stable snapshots are missing.'
    }
    $snapshots = @($evidence.snapshots)
    if ($snapshots.Count -lt 2) { throw 'Too few WebView snapshots.' }
    foreach ($snapshot in $snapshots) {
        if (@($snapshot.resourceErrors).Count -ne 0 -or @($snapshot.fatalErrors).Count -ne 0) {
            throw 'WebView reported a failed resource or fatal JavaScript error.'
        }
    }
    $last = $snapshots[-1]
    if ($last.readyState -ne 'complete' -or $last.rootChildren -lt 1 -or $last.appVisible -ne $true `
        -or $last.mainVisible -ne $true -or $last.hasIndexScript -ne $true `
        -or $last.indexScriptLoaded -ne $true -or $last.styleSheets -lt 1) {
        throw 'Final snapshot did not contain a loaded, rendered application.'
    }
    if ((Get-Sha $Executable) -ne $ExpectedSha256) { throw 'Executable changed during verification.' }
    $item = Get-Item -LiteralPath $Executable
    $receiptPath = Join-Path $run 'final-acceptance.json'
    $receipt = [ordered]@{
        schemaVersion = 1; status = 'Passed'; artifactSha256 = $ExpectedSha256
        executable = $Executable; bytes = $item.Length; mebibytes = [math]::Round($item.Length / 1MB, 2)
        frontendAssetMarkerFound = $true; sameExecutable = $true; frontendReady = $true
        pid = $process.Id; exitCode = $exitCode; testProcessExited = $true
        scope = 'Exact embedded frontend in isolated WebView2; production business API is not exercised.'
        startedAt = $startedAt.ToString('o'); finishedAt = [DateTimeOffset]::UtcNow.ToString('o')
        evidence = @{ path = $evidencePath; sha256 = (Get-Sha $evidencePath) }
    }
    Write-Json $receiptPath $receipt
    [pscustomobject]@{
        status = 'Passed'; artifactSha256 = $ExpectedSha256; mebibytes = $receipt.mebibytes
        receipt = $receiptPath; receiptSha256 = (Get-Sha $receiptPath); exitCode = $exitCode
    } | ConvertTo-Json
} catch {
    [IO.File]::WriteAllText((Join-Path $run 'runner-failure.txt'), $_.Exception.Message, [Text.UTF8Encoding]::new($false))
    throw
} finally {
    if ($null -ne $process) {
        try { Stop-OwnedTestProcess $process $testStartTime $Executable }
        finally { $process.Dispose() }
    }
}
