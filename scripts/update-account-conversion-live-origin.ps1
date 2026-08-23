param(
    [switch]$Preflight,
    [switch]$Apply,
    [string]$Confirmation = '',
    [string]$DashboardUrl = 'http://127.0.0.1:47832',
    [string]$RunId = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'account-conversion-live-origin-files.ps1')
. (Join-Path $PSScriptRoot 'account-conversion-maintenance-readiness.ps1')

$expectedConfirmation = 'UPDATE-COCKPIT-LIVE-ORIGIN'
$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseRoot = Join-Path $projectRoot 'outputs\account-conversion-live-origin'
$manifestPath = Join-Path $releaseRoot 'BUILD-MANIFEST.json'
$candidateExe = Join-Path $releaseRoot 'cockpit-tools.exe'
$installedRoot = Join-Path $env:LOCALAPPDATA 'Cockpit Tools'
$installedExe = Join-Path $installedRoot 'cockpit-tools.exe'
$frontendRoot = Join-Path $projectRoot 'outputs\account-conversion-release\frontend-dist'
$oldFrontendRoot = Join-Path $projectRoot 'dist'
$vitePath = Join-Path $projectRoot 'node_modules\vite\bin\vite.js'
$previewMetadataPath = Join-Path $releaseRoot 'PREVIEW-METADATA.json'
$descriptorPath = Join-Path $installedRoot 'account-conversion-bridge.json'

function Get-Listener {
    param([Parameter(Mandatory = $true)][int]$Port)
    return Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue |
        Select-Object -First 1
}

function Wait-ForListener {
    param(
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][bool]$Online,
        [int]$TimeoutSeconds = 20
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $listener = Get-Listener -Port $Port
        if ([bool]$listener -eq $Online) {
            return $listener
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    return Get-Listener -Port $Port
}

function Get-AvailableLoopbackPort {
    $probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    try {
        $probe.Start()
        return ([Net.IPEndPoint]$probe.LocalEndpoint).Port
    }
    finally {
        $probe.Stop()
    }
}

function Get-ProcessRecord {
    param([Parameter(Mandatory = $true)][int]$ProcessId)
    return Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId"
}

function Assert-ExactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $actualPath = [IO.Path]::GetFullPath($Actual).TrimEnd([char]'\')
    $expectedPath = [IO.Path]::GetFullPath($Expected).TrimEnd([char]'\')
    if (-not $actualPath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label path does not match the declared boundary."
    }
}

function Assert-CockpitPreviewProcess {
    param([Parameter(Mandatory = $true)][int]$ProcessId)

    $process = Get-ProcessRecord -ProcessId $ProcessId
    if (-not $process -or $process.Name -ne 'node.exe' -or -not $process.CommandLine) {
        throw 'Port 1420 is not owned by the expected Cockpit Node preview process.'
    }
    Assert-ExactPath -Actual $process.ExecutablePath -Expected (Get-Command 'node.exe' -ErrorAction Stop).Source -Label 'Cockpit preview executable'
    $normalizedCommand = $process.CommandLine.Replace('/', '\')
    $normalizedVite = [IO.Path]::GetFullPath($vitePath).Replace('/', '\')
    if (
        $normalizedCommand.IndexOf($normalizedVite, [StringComparison]::OrdinalIgnoreCase) -lt 0 -or
        $normalizedCommand -notmatch '(?i)\bpreview\b' -or
        $normalizedCommand -notmatch '(?i)--port\s+1420\b'
    ) {
        throw 'Port 1420 is not the declared Cockpit Vite preview command.'
    }
    return $process
}

function Assert-CockpitServiceProcess {
    param([Parameter(Mandatory = $true)][int]$ProcessId)

    $process = Get-ProcessRecord -ProcessId $ProcessId
    if (-not $process -or $process.Name -ne 'cockpit-tools.exe') {
        throw 'Port 19528 is not owned by Cockpit Tools.'
    }
    Assert-ExactPath -Actual $process.ExecutablePath -Expected $installedExe -Label 'Installed Cockpit executable'
    return $process
}

function Start-PreviewProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$LogPrefix
    )

    $nodePath = (Get-Command 'node.exe' -ErrorAction Stop).Source
    $stdoutPath = Join-Path $releaseRoot ($LogPrefix + '.out.log')
    $stderrPath = Join-Path $releaseRoot ($LogPrefix + '.err.log')
    $argumentLine = @(
        ('"' + $vitePath + '"'),
        'preview',
        '--host', '127.0.0.1',
        '--port', [string]$Port,
        '--strictPort',
        '--outDir', ('"' + $Root + '"')
    ) -join ' '
    return Start-Process -FilePath $nodePath -ArgumentList $argumentLine -WorkingDirectory $projectRoot -WindowStyle Hidden -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru
}

function Stop-OwnedProcess {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [int]$TimeoutSeconds = 15
    )

    if ($Process.HasExited) {
        return
    }
    Stop-Process -Id $Process.Id
    $null = $Process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $Process.HasExited) {
        throw "Owned process $($Process.Id) did not exit within the maintenance timeout."
    }
}

function Get-TextSha256 {
    param([Parameter(Mandatory = $true)][string]$Text)
    $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes)) -replace '-', '')
    }
    finally {
        $algorithm.Dispose()
    }
}

function Assert-PreviewContent {
    param(
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $expectedIndex = Get-Content -Raw -Encoding UTF8 -LiteralPath (Join-Path $Root 'index.html')
    $response = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/" -TimeoutSec 5
    if ($response.StatusCode -ne 200) {
        throw "Preview on port $Port did not return HTTP 200."
    }
    if ((Get-TextSha256 -Text $response.Content) -ne (Get-TextSha256 -Text $expectedIndex)) {
        throw "Preview on port $Port is not serving the declared frontend root."
    }
}

function Test-LatestFrontendOnTemporaryPort {
    $port = Get-AvailableLoopbackPort
    $probe = Start-PreviewProcess -Root $frontendRoot -Port $port -LogPrefix ("probe-$port")
    try {
        $listener = Wait-ForListener -Port $port -Online $true -TimeoutSeconds 20
        if (-not $listener -or $listener.OwningProcess -ne $probe.Id) {
            throw 'Temporary preview did not acquire its assigned loopback port.'
        }
        Assert-PreviewContent -Port $port -Root $frontendRoot
        return [ordered]@{
            verified = $true
            port = $port
            frontendRoot = $frontendRoot
        }
    }
    finally {
        Stop-OwnedProcess -Process $probe
        $listener = Wait-ForListener -Port $port -Online $false -TimeoutSeconds 10
        if ($listener) {
            throw 'Temporary preview did not release its loopback port.'
        }
    }
}

function Get-ConversionMaintenanceState {
    if ([string]::IsNullOrWhiteSpace($RunId)) {
        return [ordered]@{
            checked = $false
            maintenanceReady = $false
            reason = 'RunId is required before Apply.'
        }
    }

    $root = Invoke-WebRequest -UseBasicParsing -Uri ($DashboardUrl.TrimEnd('/') + '/') -TimeoutSec 5
    $tokenMatch = [regex]::Match($root.Content, 'const TOKEN = "([A-Za-z0-9_-]+)";')
    if (-not $tokenMatch.Success) {
        throw 'Dashboard token is unavailable.'
    }
    $headers = @{ 'x-cdp-console-token' = $tokenMatch.Groups[1].Value }
    $run = Invoke-RestMethod -Uri ($DashboardUrl.TrimEnd('/') + '/api/conversion/runs/' + [Uri]::EscapeDataString($RunId)) -Headers $headers -TimeoutSec 10
    $preservedAccounts = @($run.accounts | Where-Object {
        @('running', 'waiting_user', 'waiting_queue') -contains $_.status
    } | ForEach-Object {
        [ordered]@{
            slot = $_.slot
            stage = $_.stage
            status = $_.status
            currentChallengeId = $_.currentChallengeId
        }
    })
    $leases = @($run.supervisor.leases)
    $activeRunIds = @($run.supervisor.activeRunIds)
    $maintenancePauseProperty = $run.conversion.PSObject.Properties['maintenancePaused']
    $maintenancePaused = [bool](
        $maintenancePauseProperty -and [bool]$maintenancePauseProperty.Value
    )
    $readiness = Get-AccountConversionMaintenanceReadiness `
        -MaintenancePaused $maintenancePaused `
        -ActiveRunIds $activeRunIds `
        -ActiveManualSlot $run.activeManualSlot `
        -Leases $leases
    return [ordered]@{
        checked = $true
        maintenanceReady = $readiness.maintenanceReady
        maintenancePaused = $readiness.maintenancePaused
        runId = $RunId
        runStatus = $run.run.status
        preservedAccounts = $preservedAccounts
        activeRunIds = $readiness.activeRunIds
        activeManualSlot = $readiness.activeManualSlot
        leaseCount = $readiness.leaseCount
        reason = $readiness.reason
    }
}

function Get-BridgeHealthForProcess {
    param([Parameter(Mandatory = $true)][int]$ExpectedProcessId)

    $deadline = (Get-Date).AddSeconds(30)
    do {
        if (Test-Path -LiteralPath $descriptorPath) {
            try {
                $descriptor = Get-Content -Raw -Encoding UTF8 -LiteralPath $descriptorPath | ConvertFrom-Json
                if ([int]$descriptor.pid -eq $ExpectedProcessId) {
                    $headers = @{ Authorization = "Bearer $($descriptor.token)" }
                    $health = Invoke-RestMethod -Uri ($descriptor.baseUrl.TrimEnd('/') + '/v1/health') -Headers $headers -TimeoutSec 5
                    return $health
                }
            }
            catch {
            }
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw 'Updated Cockpit did not publish a healthy bridge descriptor for its process.'
}

function Start-InstalledCockpitAndVerify {
    param([Parameter(Mandatory = $true)][string]$RequiredCapability)

    $process = Start-Process -FilePath $installedExe -WorkingDirectory $installedRoot -WindowStyle Hidden -PassThru
    $listener = Wait-ForListener -Port 19528 -Online $true -TimeoutSeconds 30
    if (-not $listener -or $listener.OwningProcess -ne $process.Id) {
        throw 'Cockpit did not recover its local service port after restart.'
    }
    $health = Get-BridgeHealthForProcess -ExpectedProcessId $process.Id
    if ($RequiredCapability -and -not (@($health.capabilities) -contains $RequiredCapability)) {
        throw "Updated Cockpit bridge does not advertise $RequiredCapability."
    }
    return [ordered]@{
        process = $process
        health = $health
    }
}

function Assert-Preflight {
    if (-not (Test-Path -LiteralPath $manifestPath) -or -not (Test-Path -LiteralPath $candidateExe)) {
        throw 'The staged live-origin release or manifest is missing.'
    }
    $manifest = Get-Content -Raw -Encoding UTF8 -LiteralPath $manifestPath | ConvertFrom-Json
    $candidateHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $candidateExe).Hash
    if ($candidateHash -ne $manifest.artifact.sha256) {
        throw 'The staged live-origin executable hash does not match its manifest.'
    }
    if ($manifest.webviewOrigin -ne 'http://127.0.0.1:1420') {
        throw 'The transition build does not preserve the current WebView origin.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $frontendRoot 'index.html'))) {
        throw 'The verified latest frontend is missing.'
    }
    if (-not $manifest.frontendSha256) {
        throw 'The staged live-origin manifest does not bind the replacement frontend tree.'
    }
    $frontendHash = Get-VerifiedDirectorySha256 -Path $frontendRoot
    if ($frontendHash -ne $manifest.frontendSha256) {
        throw 'The replacement frontend tree no longer matches the staged live-origin manifest.'
    }

    $cockpitListener = Get-Listener -Port 19528
    if (-not $cockpitListener) {
        throw 'Cockpit is not listening on port 19528.'
    }
    $cockpit = Assert-CockpitServiceProcess -ProcessId $cockpitListener.OwningProcess
    $previewListener = Get-Listener -Port 1420
    if (-not $previewListener) {
        throw 'Cockpit preview is not listening on port 1420.'
    }
    $preview = Assert-CockpitPreviewProcess -ProcessId $previewListener.OwningProcess
    # Rollback can only be truthful when the currently served frontend is the
    # exact tree that this maintenance transaction will restore on failure.
    # Check this before stopping either live process.
    Assert-PreviewContent -Port 1420 -Root $oldFrontendRoot
    $frontendProbe = Test-LatestFrontendOnTemporaryPort
    $conversion = Get-ConversionMaintenanceState

    return [ordered]@{
        ready = $conversion.maintenanceReady
        cockpitPid = $cockpit.ProcessId
        previewPid = $preview.ProcessId
        candidateSha256 = $candidateHash
        frontendSha256 = $frontendHash
        webviewOrigin = $manifest.webviewOrigin
        frontendProbe = $frontendProbe
        conversion = $conversion
        requiredConfirmation = $expectedConfirmation
    }
}

$preflightResult = Assert-Preflight
if (-not $Apply) {
    $preflightResult | ConvertTo-Json -Depth 6
    exit 0
}

if ($Confirmation -ne $expectedConfirmation) {
    throw "Apply requires -Confirmation $expectedConfirmation"
}
if (-not $preflightResult.ready) {
    throw 'Apply is forbidden until the conversion run is durably paused and its scheduler, manual worker, and leases are quiescent.'
}

$cockpitPid = [int]$preflightResult.cockpitPid
$previewPid = [int]$preflightResult.previewPid
$backupPath = Join-Path $installedRoot ("cockpit-tools.exe.pre-live-origin-" + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '.bak')
$newPreview = $null
$newCockpit = $null
$installedChanged = $false
$oldPreviewStopped = $false
$cockpitStopAttempted = $false
$cockpitStopped = $false
$installedOriginalHash = Get-VerifiedFileSha256 -Path $installedExe
$backupHash = New-VerifiedFileBackup -SourcePath $installedExe -BackupPath $backupPath
try {
    Stop-Process -Id $previewPid
    if (Wait-ForListener -Port 1420 -Online $false -TimeoutSeconds 15) {
        throw 'Cockpit preview did not release port 1420.'
    }
    $oldPreviewStopped = $true

    $newPreview = Start-PreviewProcess -Root $frontendRoot -Port 1420 -LogPrefix 'preview-live-origin'
    $newPreviewListener = Wait-ForListener -Port 1420 -Online $true -TimeoutSeconds 20
    if (-not $newPreviewListener -or $newPreviewListener.OwningProcess -ne $newPreview.Id) {
        throw 'Verified latest frontend did not acquire port 1420.'
    }
    Assert-PreviewContent -Port 1420 -Root $frontendRoot

    $previewMetadata = [ordered]@{
        schemaVersion = 1
        pid = $newPreview.Id
        origin = 'http://127.0.0.1:1420'
        frontendRoot = $frontendRoot
        startedAt = [DateTime]::UtcNow.ToString('o')
    }
    $previewMetadata | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $previewMetadataPath -Encoding UTF8

    $cockpitStopAttempted = $true
    Stop-Process -Id $cockpitPid -ErrorAction SilentlyContinue
    # From this point onward the original service can disappear at any time,
    # including immediately after the timeout boundary. Always run the verified
    # executable/service rollback path if a later maintenance step fails.
    $cockpitStopped = $true
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Process -Id $cockpitPid -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 250
    }
    if (Get-Process -Id $cockpitPid -ErrorAction SilentlyContinue) {
        throw 'Cockpit did not exit within the maintenance window.'
    }
    Set-VerifiedFileFromCandidate `
        -CandidatePath $candidateExe `
        -DestinationPath $installedExe `
        -ExpectedSha256 $preflightResult.candidateSha256
    $installedChanged = $true

    $started = Start-InstalledCockpitAndVerify -RequiredCapability 'mfa_full_email_confirmation_v1'
    $newCockpit = $started.process

    [ordered]@{
        updated = $true
        cockpitPid = $newCockpit.Id
        previewPid = $newPreview.Id
        backupPath = $backupPath
        installedSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe).Hash
        bridgeCapabilities = @($started.health.capabilities)
        nextChecks = @(
            'Ask the user to confirm that existing MFA records remain visible.',
            'Resume the same conversion run only after that user confirmation.',
            'Keep the WebView origin on 127.0.0.1:1420 until an explicit opaque migration exists.'
        )
    } | ConvertTo-Json -Depth 5
}
catch {
    $failure = $_
    $rollbackErrors = [Collections.Generic.List[string]]::new()

    try {
        if ($newCockpit -and -not $newCockpit.HasExited) {
            Stop-OwnedProcess -Process $newCockpit
            $null = Wait-ForListener -Port 19528 -Online $false -TimeoutSeconds 15
        }
        elseif ($cockpitStopAttempted) {
            # Start-InstalledCockpitAndVerify can throw before returning its Process.
            # In that case, stop only a listener that still resolves to the exact
            # installed Cockpit executable before restoring the backup.
            $listener = Get-Listener -Port 19528
            if ($listener) {
                $null = Assert-CockpitServiceProcess -ProcessId $listener.OwningProcess
                Stop-Process -Id $listener.OwningProcess
                if (Wait-ForListener -Port 19528 -Online $false -TimeoutSeconds 15) {
                    throw 'The failed replacement Cockpit process did not release port 19528.'
                }
            }
        }
    }
    catch {
        $rollbackErrors.Add("Could not stop the failed replacement Cockpit process: $($_.Exception.Message)")
    }

    if ($cockpitStopAttempted) {
        try {
            $currentInstalledHash = if (Test-Path -LiteralPath $installedExe) {
                (Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe).Hash
            }
            else {
                ''
            }
            if ($currentInstalledHash -ne $backupHash) {
                Restore-VerifiedFileBackup `
                    -BackupPath $backupPath `
                    -DestinationPath $installedExe `
                    -ExpectedBackupSha256 $backupHash
            }
            if (
                -not (Test-Path -LiteralPath $installedExe) -or
                (Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe).Hash -ne $backupHash
            ) {
                throw 'The installed Cockpit executable does not match the verified maintenance backup.'
            }
        }
        catch {
            $rollbackErrors.Add("Could not restore the previous Cockpit executable: $($_.Exception.Message)")
        }
    }

    try {
        if ($newPreview -and -not $newPreview.HasExited) {
            Stop-OwnedProcess -Process $newPreview
            $null = Wait-ForListener -Port 1420 -Online $false -TimeoutSeconds 10
        }
        if ($oldPreviewStopped -and -not (Get-Listener -Port 1420)) {
            $rollbackPreview = Start-PreviewProcess -Root $oldFrontendRoot -Port 1420 -LogPrefix 'preview-rollback'
            $rollbackListener = Wait-ForListener -Port 1420 -Online $true -TimeoutSeconds 20
            if (-not $rollbackListener -or $rollbackListener.OwningProcess -ne $rollbackPreview.Id) {
                throw 'The previous Cockpit preview could not be restored.'
            }
            Assert-PreviewContent -Port 1420 -Root $oldFrontendRoot
        }
    }
    catch {
        $rollbackErrors.Add("Could not restore the previous Cockpit preview: $($_.Exception.Message)")
    }

    if ($cockpitStopAttempted) {
        try {
            $rollbackListener = Get-Listener -Port 19528
            if ($rollbackListener) {
                $null = Assert-CockpitServiceProcess -ProcessId $rollbackListener.OwningProcess
            }
            else {
                $null = Start-InstalledCockpitAndVerify -RequiredCapability ''
            }
        }
        catch {
            $rollbackErrors.Add("Could not restart the previous Cockpit service: $($_.Exception.Message)")
        }
    }

    if ($rollbackErrors.Count -gt 0) {
        throw "Maintenance failed: $($failure.Exception.Message) Rollback also reported: $($rollbackErrors -join ' | ')"
    }
    throw $failure
}
