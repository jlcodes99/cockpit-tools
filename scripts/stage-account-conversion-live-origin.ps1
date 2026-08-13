param(
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'account-conversion-live-origin-files.ps1')
$sourceExe = Join-Path $projectRoot 'target\release\cockpit-tools.exe'
$releaseRoot = Join-Path $projectRoot 'outputs\account-conversion-live-origin'
$stagedExe = Join-Path $releaseRoot 'cockpit-tools.exe'
$manifestPath = Join-Path $releaseRoot 'BUILD-MANIFEST.json'
$frontendRoot = Join-Path $projectRoot 'outputs\account-conversion-release\frontend-dist'

function Test-BinaryMarker {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Marker
    )

    $ripgrep = (Get-Command 'rg' -ErrorAction Stop).Source
    & $ripgrep -a -q --fixed-strings $Marker $Path
    return $LASTEXITCODE -eq 0
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

Push-Location $projectRoot
try {
    if ($Build) {
        $previousCargoTargetDir = $env:CARGO_TARGET_DIR
        try {
            $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'
            & cargo build --manifest-path '.\src-tauri\Cargo.toml' --release --bin cockpit-tools --no-default-features
            if ($LASTEXITCODE -ne 0) {
                throw "Live-origin Cockpit build failed with exit code ${LASTEXITCODE}"
            }
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

    if (-not (Test-Path -LiteralPath $sourceExe)) {
        throw "Live-origin executable is missing: $sourceExe"
    }
    $frontendFiles = @(Get-ChildItem -LiteralPath $frontendRoot -Recurse -Filter '*.js' -File)
    if ($frontendFiles.Count -eq 0) {
        throw "Latest account-conversion frontend is missing: $frontendRoot"
    }

    $binaryMarkers = [ordered]@{
        liveOrigin = Test-BinaryMarker -Path $sourceExe -Marker 'http://127.0.0.1:1420'
        bridgeCapability = Test-BinaryMarker -Path $sourceExe -Marker 'mfa_full_email_confirmation_v1'
        confirmCommand = Test-BinaryMarker -Path $sourceExe -Marker 'account_conversion_confirm_challenge'
        fullEmailPayload = Test-BinaryMarker -Path $sourceExe -Marker 'mfaMatchEmail'
    }
    $frontendMarkers = [ordered]@{
        fullEmailPayload = Test-FrontendMarker -Files $frontendFiles -Marker 'mfaMatchEmail'
        confirmCommand = Test-FrontendMarker -Files $frontendFiles -Marker 'account_conversion_confirm_challenge'
        expectedEmail = Test-FrontendMarker -Files $frontendFiles -Marker 'expectedEmail'
        userClipboardAction = Test-FrontendMarker -Files $frontendFiles -Marker 'navigator.clipboard.writeText'
    }
    foreach ($entry in @($binaryMarkers.GetEnumerator()) + @($frontendMarkers.GetEnumerator())) {
        if (-not $entry.Value) {
            throw "Live-origin release is missing required marker: $($entry.Key)"
        }
    }

    New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
    Copy-Item -LiteralPath $sourceExe -Destination $stagedExe -Force
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourceExe).Hash
    $stagedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $stagedExe).Hash
    if ($sourceHash -ne $stagedHash) {
        throw 'The staged live-origin executable does not match the verified source.'
    }

    $item = Get-Item -LiteralPath $stagedExe
    $manifest = [ordered]@{
        schemaVersion = 1
        generatedAt = [DateTime]::UtcNow.ToString('o')
        releaseKind = 'live-origin-transition'
        webviewOrigin = 'http://127.0.0.1:1420'
        purpose = 'Preserve the existing Cockpit WebView localStorage origin while adding the account-conversion bridge capability.'
        frontendRoot = $frontendRoot
        frontendSha256 = Get-VerifiedDirectorySha256 -Path $frontendRoot
        binaryMarkers = $binaryMarkers
        frontendMarkers = $frontendMarkers
        artifact = [ordered]@{
            path = $item.FullName
            length = $item.Length
            sha256 = $stagedHash
            fileVersion = $item.VersionInfo.FileVersion
            productVersion = $item.VersionInfo.ProductVersion
            lastWriteUtc = $item.LastWriteTimeUtc.ToString('o')
        }
        invariants = @(
            'Do not install unless port 1420 serves the verified latest frontend.',
            'Do not change the WebView origin before an explicit MFA localStorage migration exists.',
            'Do not read or export MFA localStorage values during maintenance.',
            'After restart, require the bridge health capability and user confirmation that existing MFA records remain visible.'
        )
    }
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    Write-Output ("Staged Cockpit live-origin transition release: " + $stagedExe)
    Write-Output ("Manifest: " + $manifestPath)
}
finally {
    Pop-Location
}
