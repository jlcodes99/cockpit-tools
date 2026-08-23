$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'account-conversion-live-origin-files.ps1')
. (Join-Path $PSScriptRoot 'account-conversion-maintenance-readiness.ps1')

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) { throw $Message }
}

function Assert-Throws {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$Message
    )
    $threw = $false
    try { & $Action } catch { $threw = $true }
    Assert-True -Condition $threw -Message $Message
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    'cockpit-live-origin-files-' + [Guid]::NewGuid().ToString('N')
)
New-Item -ItemType Directory -Path $testRoot | Out-Null
try {
    $installed = Join-Path $testRoot 'cockpit-tools.exe'
    $candidate = Join-Path $testRoot 'candidate.exe'
    $backup = Join-Path $testRoot 'cockpit-tools.backup.exe'
    [IO.File]::WriteAllBytes($installed, [byte[]](1..64))
    [IO.File]::WriteAllBytes($candidate, [byte[]](65..128))
    $installedHash = Get-VerifiedFileSha256 -Path $installed
    $candidateHash = Get-VerifiedFileSha256 -Path $candidate

    $frontend = Join-Path $testRoot 'frontend'
    New-Item -ItemType Directory -Path (Join-Path $frontend 'assets') | Out-Null
    [IO.File]::WriteAllText((Join-Path $frontend 'index.html'), '<!doctype html>')
    [IO.File]::WriteAllText((Join-Path $frontend 'assets\app.js'), 'console.log("v1")')
    $frontendHash = Get-VerifiedDirectorySha256 -Path $frontend
    Assert-True -Condition ($frontendHash -eq (Get-VerifiedDirectorySha256 -Path $frontend)) -Message 'Directory fingerprint is not deterministic.'
    [IO.File]::AppendAllText((Join-Path $frontend 'assets\app.js'), ';console.log("tampered")')
    Assert-True -Condition ($frontendHash -ne (Get-VerifiedDirectorySha256 -Path $frontend)) -Message 'Directory fingerprint did not detect a changed asset.'

    $backupHash = New-VerifiedFileBackup -SourcePath $installed -BackupPath $backup
    Assert-True -Condition ($backupHash -eq $installedHash) -Message 'Backup hash mismatch.'

    Assert-Throws -Action {
        Set-VerifiedFileFromCandidate `
            -CandidatePath (Join-Path $testRoot 'missing.exe') `
            -DestinationPath $installed `
            -ExpectedSha256 $candidateHash
    } -Message 'A missing candidate did not fail.'
    Assert-True -Condition ((Get-VerifiedFileSha256 -Path $installed) -eq $installedHash) -Message 'Missing candidate changed the installed file.'

    Assert-Throws -Action {
        Set-VerifiedFileFromCandidate `
            -CandidatePath $candidate `
            -DestinationPath $installed `
            -ExpectedSha256 ('0' * 64)
    } -Message 'A candidate hash mismatch did not fail.'
    Assert-True -Condition ((Get-VerifiedFileSha256 -Path $installed) -eq $installedHash) -Message 'Hash mismatch changed the installed file.'

    Assert-Throws -Action {
        Set-VerifiedFileFromCandidate `
            -CandidatePath $candidate `
            -DestinationPath $installed `
            -ExpectedSha256 $candidateHash `
            -BeforeCommit { throw 'injected before-commit failure' }
    } -Message 'The injected pre-commit failure did not fail.'
    Assert-True -Condition ((Get-VerifiedFileSha256 -Path $installed) -eq $installedHash) -Message 'Pre-commit failure changed the installed file.'
    Assert-True -Condition (@(Get-ChildItem -LiteralPath $testRoot -Filter '*.tmp' -File).Count -eq 0) -Message 'A staged temporary file leaked after failure.'

    Set-VerifiedFileFromCandidate -CandidatePath $candidate -DestinationPath $installed -ExpectedSha256 $candidateHash
    Assert-True -Condition ((Get-VerifiedFileSha256 -Path $installed) -eq $candidateHash) -Message 'Verified candidate was not installed.'

    [IO.File]::WriteAllBytes($installed, [byte[]](200..220))
    Restore-VerifiedFileBackup -BackupPath $backup -DestinationPath $installed -ExpectedBackupSha256 $backupHash
    Assert-True -Condition ((Get-VerifiedFileSha256 -Path $installed) -eq $installedHash) -Message 'Verified backup did not restore the original file.'

    Set-VerifiedFileFromCandidate -CandidatePath $candidate -DestinationPath $installed -ExpectedSha256 $candidateHash
    [IO.File]::WriteAllBytes($backup, [byte[]](10..20))
    Assert-Throws -Action {
        Restore-VerifiedFileBackup -BackupPath $backup -DestinationPath $installed -ExpectedBackupSha256 $backupHash
    } -Message 'A corrupted backup did not fail closed.'
    Assert-True -Condition ((Get-VerifiedFileSha256 -Path $installed) -eq $candidateHash) -Message 'Corrupted backup changed the installed file.'

    $ready = Get-AccountConversionMaintenanceReadiness `
        -MaintenancePaused $true `
        -ActiveRunIds @() `
        -ActiveManualSlot $null `
        -Leases @()
    Assert-True -Condition $ready.maintenanceReady -Message 'A fully quiescent paused run was rejected.'

    $unpaused = Get-AccountConversionMaintenanceReadiness `
        -MaintenancePaused $false `
        -ActiveRunIds @() `
        -ActiveManualSlot $null `
        -Leases @()
    Assert-True -Condition (-not $unpaused.maintenanceReady) -Message 'An unpaused run was accepted.'

    $withLease = Get-AccountConversionMaintenanceReadiness `
        -MaintenancePaused $true `
        -ActiveRunIds @() `
        -ActiveManualSlot $null `
        -Leases @([pscustomobject]@{ runId = 'run-1' })
    Assert-True -Condition (-not $withLease.maintenanceReady) -Message 'A paused run with a lease was accepted.'
    Assert-True -Condition ($withLease.leaseCount -eq 1) -Message 'Lease count was not preserved.'

    $withManual = Get-AccountConversionMaintenanceReadiness `
        -MaintenancePaused $true `
        -ActiveRunIds @() `
        -ActiveManualSlot 'Imported-01' `
        -Leases @()
    Assert-True -Condition (-not $withManual.maintenanceReady) -Message 'A paused run with a manual worker was accepted.'

    $withScheduler = Get-AccountConversionMaintenanceReadiness `
        -MaintenancePaused $true `
        -ActiveRunIds @('run-1') `
        -ActiveManualSlot $null `
        -Leases @()
    Assert-True -Condition (-not $withScheduler.maintenanceReady) -Message 'A paused run with an active scheduler was accepted.'

    [ordered]@{
        ok = $true
        cases = @(
            'verified_backup',
            'deterministic_frontend_fingerprint',
            'frontend_tamper_detection',
            'missing_candidate',
            'candidate_hash_mismatch',
            'pre_commit_failure',
            'verified_install',
            'verified_restore',
            'corrupted_backup_fail_closed',
            'maintenance_quiescent_ready',
            'maintenance_unpaused_rejected',
            'maintenance_lease_rejected',
            'maintenance_manual_worker_rejected',
            'maintenance_scheduler_rejected'
        )
    } | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $testRoot -PathType Container) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
