#requires -Version 7.0
<##
Build and run the synthetic native retry-scope regressions on Windows.
Requires the normal Rust/MSVC/Windows SDK build environment; never starts the app.
Logs stay in this checkout's target/native-retry-scope directory.
##>
[CmdletBinding()]
param(
    [string]$Cargo = 'cargo',
    [string]$TargetDirectory = ''
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-PlainPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $cursor = $full
    while ($cursor) {
        if ((Test-Path -LiteralPath $cursor) -and
            ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing a link or reparse point: $cursor"
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    return $full
}

function Write-Json([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
}

function Invoke-Logged([string]$Program, [string[]]$Arguments, [string]$Name,
    [hashtable]$EnvironmentOverrides = @{}, [switch]$IsolatedTest) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Program
    $info.WorkingDirectory = $repo
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $null = $info.ArgumentList.Add($argument) }
    # A prior desktop/release build must not inject configuration or embedding
    # mode into the library test build. Retain the caller's Rust/MSVC/SDK setup.
    foreach ($key in @($info.Environment.Keys)) {
        if ($key -eq 'TAURI_CONFIG' -or $key -like 'TAURI_ENV_*') {
            $null = $info.Environment.Remove($key)
        }
    }
    if ($IsolatedTest) {
        foreach ($key in @($info.Environment.Keys)) {
            if ($key -match '^COCKPIT_.*(?:LIVE|PROBE)' -or
                $key -match '^(?:HTTP|HTTPS|ALL|FTP|NO)_PROXY$') {
                $null = $info.Environment.Remove($key)
            }
        }
        $info.Environment['NO_PROXY'] = 'localhost,127.0.0.1,::1'
    }
    foreach ($key in $EnvironmentOverrides.Keys) { $info.Environment[$key] = $EnvironmentOverrides[$key] }
    $stdoutPath = Join-Path $run "$Name.stdout.log"
    $stderrPath = Join-Path $run "$Name.stderr.log"
    $stdout = [IO.File]::Create($stdoutPath)
    $stderr = [IO.File]::Create($stderrPath)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $started = [DateTimeOffset]::UtcNow
    try {
        if (-not $process.Start()) { throw "Could not start $Name" }
        # Drain both pipes concurrently: a verbose compiler cannot deadlock on stderr.
        $outCopy = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $errCopy = $process.StandardError.BaseStream.CopyToAsync($stderr)
        $null = $process.WaitForExit()
        # PowerShell can surface VoidTaskResult as a pipeline object. Suppress
        # completion results so this function returns only its receipt object.
        $null = $outCopy.GetAwaiter().GetResult()
        $null = $errCopy.GetAwaiter().GetResult()
        return [pscustomobject]@{
            name = $Name; exitCode = $process.ExitCode; pid = $process.Id
            startedAt = $started.ToString('o'); completedAt = [DateTimeOffset]::UtcNow.ToString('o')
            stdout = $stdoutPath; stderr = $stderrPath
        }
    } finally {
        # Dispose handles only. Never kill an existing app or a process by name.
        $null = $stdout.Dispose(); $null = $stderr.Dispose(); $null = $process.Dispose()
    }
}

if (-not $IsWindows) { throw 'This helper is only needed for Windows test executables.' }
$repo = Assert-PlainPath (Join-Path $PSScriptRoot '..\..')
if (-not (Test-Path -LiteralPath (Join-Path $repo 'src-tauri\src\modules\codex_native_api\retry_scope_http_tests.rs'))) {
    throw 'Run this script from a checkout containing the retry-scope regression module.'
}
$cargoCommand = Get-Command $Cargo -CommandType Application -ErrorAction Stop
$cargoPath = $cargoCommand.Source
$output = Assert-PlainPath (Join-Path $repo 'target\native-retry-scope')
if ([string]::IsNullOrWhiteSpace($TargetDirectory)) {
    $buildTarget = Assert-PlainPath (Join-Path $output 'build')
} else {
    if (-not [IO.Path]::IsPathFullyQualified($TargetDirectory)) {
        throw 'TargetDirectory must be an absolute path dedicated to this checkout.'
    }
    $buildTarget = Assert-PlainPath $TargetDirectory
}
if ($buildTarget.TrimEnd([IO.Path]::DirectorySeparatorChar) -eq
    [IO.Path]::GetPathRoot($buildTarget).TrimEnd([IO.Path]::DirectorySeparatorChar) -or
    $buildTarget -eq $repo) { throw 'The build target must not be a filesystem root or the checkout root.' }
$run = Assert-PlainPath (Join-Path $output ('runs\' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N')))
if (Test-Path -LiteralPath $run) { throw 'Evidence directory already exists.' }
$null = New-Item -ItemType Directory -Path $run
$filter = 'modules::codex_native_api::tests::retry_scope_http_tests::'
$receipt = [ordered]@{
    schema = 1; success = $false; filter = $filter; buildTarget = $buildTarget
    run = $run; build = $null; artifact = $null; list = $null; tests = $null; error = $null
}
Write-Host "Evidence: $run"
try {
    $buildArgs = @('test', '--locked', '-p', 'cockpit-tools', '--lib', '--no-run',
        '--message-format=json', '--target-dir', $buildTarget)
    Write-Host 'Building the library test executable (logs are written to the evidence directory)...'
    $receipt.build = Invoke-Logged $cargoPath $buildArgs 'build'
    if ($receipt.build.exitCode -ne 0) { throw "Cargo build failed: $($receipt.build.exitCode). See build.stderr.log." }
    $artifacts = @(
        foreach ($line in [IO.File]::ReadLines($receipt.build.stdout)) {
            if ([string]::IsNullOrWhiteSpace($line)) { continue }
            $message = $line | ConvertFrom-Json
            if ($message.reason -eq 'compiler-artifact' -and $message.profile.test -and
                $message.target.name -eq 'antigravity_cockpit_tools_lib' -and $message.executable) {
                $message.executable
            }
        }
    ) | Select-Object -Unique
    if (@($artifacts).Count -ne 1) { throw 'Cargo must identify exactly one library test executable.' }
    $source = Assert-PlainPath ([string]$artifacts)
    if (-not $source.StartsWith($buildTarget + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.File]::Exists($source)) { throw 'Cargo artifact is outside this build target or missing.' }
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $testExe = Join-Path $run ([IO.Path]::GetFileName($source))
    Copy-Item -LiteralPath $source -Destination $testExe
    if ((Get-FileHash -LiteralPath $testExe -Algorithm SHA256).Hash -ne $sourceHash) { throw 'Copied executable hash mismatch.' }
    # A Windows library test host can need the Common Controls v6 activation
    # context normally supplied by the GUI application. Do not patch its bytes.
    $manifest = @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="Cockpit.NativeRetryScope.Tests" type="win32"/>
  <dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
</assembly>
'@
    [IO.File]::WriteAllText($testExe + '.manifest', $manifest, [Text.UTF8Encoding]::new($false))
    $receipt.artifact = [ordered]@{ source = $source; copiedExecutable = $testExe; sha256 = $sourceHash
        bytes = (Get-Item -LiteralPath $testExe).Length; manifestSha256 = (Get-FileHash -LiteralPath ($testExe + '.manifest') -Algorithm SHA256).Hash }
    $data = Join-Path $run 'data'
    $temporary = Join-Path $run 'tmp'
    $null = New-Item -ItemType Directory -Path $data, $temporary
    $testEnvironment = @{
        COCKPIT_TOOLS_TEST_DATA_DIR = $data; COCKPIT_TEST_DATA_DIR = $data
        COCKPIT_TOOLS_DATA_DIR = $data; TEMP = $temporary; TMP = $temporary
        PATH = ([IO.Path]::GetDirectoryName($source) + [IO.Path]::PathSeparator + $env:PATH)
    }
    $receipt.list = Invoke-Logged $testExe @($filter, '--list', '--format', 'terse') 'list' $testEnvironment -IsolatedTest
    if ($receipt.list.exitCode -ne 0) { throw "Test loader/list failed: $($receipt.list.exitCode). No tests have passed." }
    $listed = @([IO.File]::ReadAllLines($receipt.list.stdout) | Where-Object { $_ -match ': test$' }).Count
    if ($listed -eq 0) { throw 'The fixed regression filter selected zero tests.' }
    Write-Host "Running $listed synthetic loopback tests serially..."
    $receipt.tests = Invoke-Logged $testExe @($filter, '--test-threads=1', '--nocapture', '--color=never') 'tests' $testEnvironment -IsolatedTest
    $result = [regex]::Match([IO.File]::ReadAllText($receipt.tests.stdout),
        'test result: ok\. (\d+) passed; 0 failed; 0 ignored; 0 measured;')
    if ($receipt.tests.exitCode -ne 0 -or -not $result.Success -or
        [int]$result.Groups[1].Value -ne $listed) {
        throw "Regression run failed or did not execute every listed test (exit $($receipt.tests.exitCode))."
    }
    $receipt.tests | Add-Member -NotePropertyName passed -NotePropertyValue $listed
    $receipt.success = $true
} catch {
    $receipt.error = $_.Exception.Message
    throw
} finally {
    Write-Json (Join-Path $run 'receipt.json') $receipt
}
Write-Host "PASS: $($receipt.tests.passed) tests; exact executable SHA256 $sourceHash"
Write-Host "Logs and receipt retained at: $run"
