param(
    [switch]$VerifyOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseRoot = Join-Path $projectRoot 'outputs\account-conversion-release'
$frontendRoot = Join-Path $releaseRoot 'frontend-dist'
$targetRoot = Join-Path $releaseRoot 'target'
$candidateExe = Join-Path $targetRoot 'release\cockpit-tools.exe'
$candidateInstaller = Join-Path $targetRoot 'release\bundle\nsis\Cockpit Tools_1.3.16_x64-setup.exe'
$tauriConfig = Join-Path $projectRoot 'src-tauri\tauri.account-conversion.conf.json'
$manifestPath = Join-Path $releaseRoot 'BUILD-MANIFEST.json'
$distRoot = Join-Path $projectRoot 'dist'

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath"
    }
}

function Get-DirectoryFingerprint {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return 'missing'
    }

    $resolvedRoot = (Resolve-Path -LiteralPath $Path).Path
    $entries = foreach ($file in Get-ChildItem -LiteralPath $resolvedRoot -Recurse -File | Sort-Object FullName) {
        $relativePath = $file.FullName.Substring($resolvedRoot.Length).TrimStart([char]'\')
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash
        "$relativePath`t$($file.Length)`t$hash"
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($entries -join "`n"))
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes)) -replace '-', '')
    }
    finally {
        $algorithm.Dispose()
    }
}

function Test-FrontendMarker {
    param(
        [Parameter(Mandatory = $true)][System.IO.FileInfo[]]$Files,
        [Parameter(Mandatory = $true)][string]$Marker
    )

    foreach ($file in $Files) {
        if (Select-String -LiteralPath $file.FullName -SimpleMatch -Quiet -Pattern $Marker) {
            return $true
        }
    }
    return $false
}

function Get-ArtifactRecord {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        path = $item.FullName
        length = $item.Length
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $item.FullName).Hash
        fileVersion = $item.VersionInfo.FileVersion
        productVersion = $item.VersionInfo.ProductVersion
        lastWriteUtc = $item.LastWriteTimeUtc.ToString('o')
    }
}

if (-not (Test-Path -LiteralPath $tauriConfig)) {
    throw "Missing isolated Tauri configuration: $tauriConfig"
}

$distFingerprintBefore = Get-DirectoryFingerprint -Path $distRoot

Push-Location $projectRoot
try {
    if (-not $VerifyOnly) {
        Invoke-CheckedCommand -FilePath 'npm.cmd' -Arguments @(
            'exec', 'vite', '--', 'build',
            '--outDir', 'outputs/account-conversion-release/frontend-dist',
            '--emptyOutDir'
        )

        $previousCargoTargetDir = $env:CARGO_TARGET_DIR
        try {
            $env:CARGO_TARGET_DIR = $targetRoot
            Invoke-CheckedCommand -FilePath (Join-Path $projectRoot 'node_modules\.bin\tauri.cmd') -Arguments @(
                'build',
                '--config', 'src-tauri\tauri.account-conversion.conf.json',
                '--bundles', 'nsis',
                '--ci'
            )
        }
        finally {
            if ([string]::IsNullOrEmpty($previousCargoTargetDir)) {
                Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
            }
            else {
                $env:CARGO_TARGET_DIR = $previousCargoTargetDir
            }
        }
    }

    if (-not (Test-Path -LiteralPath $candidateExe)) {
        throw "Candidate executable was not produced: $candidateExe"
    }
    if (-not (Test-Path -LiteralPath $candidateInstaller)) {
        throw "Candidate NSIS installer was not produced: $candidateInstaller"
    }

    $frontendFiles = @(Get-ChildItem -LiteralPath $frontendRoot -Recurse -Filter '*.js' -File)
    if ($frontendFiles.Count -eq 0) {
        throw "Candidate frontend contains no JavaScript assets: $frontendRoot"
    }

    $frontendMarkers = [ordered]@{
        mfaMatchEmail = Test-FrontendMarker -Files $frontendFiles -Marker 'mfaMatchEmail'
        confirmCommand = Test-FrontendMarker -Files $frontendFiles -Marker 'account_conversion_confirm_challenge'
        expectedEmail = Test-FrontendMarker -Files $frontendFiles -Marker 'expectedEmail'
        authenticatorChallenge = Test-FrontendMarker -Files $frontendFiles -Marker 'authenticator_setup'
        userClipboardAction = Test-FrontendMarker -Files $frontendFiles -Marker 'navigator.clipboard.writeText'
    }
    foreach ($entry in $frontendMarkers.GetEnumerator()) {
        if (-not $entry.Value) {
            throw "Candidate frontend is missing required contract marker: $($entry.Key)"
        }
    }

    $ripgrep = (Get-Command 'rg' -ErrorAction Stop).Source
    $binaryMarkers = [ordered]@{}
    foreach ($marker in @(
        'mfaMatchEmail',
        'mfa_full_email_confirmation_v1',
        'account_conversion_confirm_challenge'
    )) {
        & $ripgrep -a -q --fixed-strings $marker $candidateExe
        $binaryMarkers[$marker] = $LASTEXITCODE -eq 0
        if (-not $binaryMarkers[$marker]) {
            throw "Candidate executable is missing required embedded marker: $marker"
        }
    }

    $distFingerprintAfter = Get-DirectoryFingerprint -Path $distRoot
    if ($distFingerprintAfter -ne $distFingerprintBefore) {
        throw 'The isolated account-conversion build changed the live dist directory.'
    }

    $manifest = [ordered]@{
        schemaVersion = 1
        generatedAt = [DateTime]::UtcNow.ToString('o')
        mode = if ($VerifyOnly) { 'verify-only' } else { 'build-and-verify' }
        isolatedFrontend = $frontendRoot
        isolatedCargoTarget = $targetRoot
        liveDistFingerprint = $distFingerprintAfter
        frontendMarkers = $frontendMarkers
        executableMarkers = $binaryMarkers
        artifacts = @(
            Get-ArtifactRecord -Path $candidateExe
            Get-ArtifactRecord -Path $candidateInstaller
        )
        nonSecretBoundary = @(
            'The loopback bridge returns lifecycle state and timestamps only.',
            'TOTP remains in the Cockpit WebView and user clipboard.',
            'Authenticator confirmation requires an exact full-email MFA selection.'
        )
    }

    New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    Write-Output ("Verified Cockpit account-conversion release: " + $candidateInstaller)
    Write-Output ("Manifest: " + $manifestPath)
}
finally {
    Pop-Location
}
