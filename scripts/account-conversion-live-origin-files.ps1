Set-StrictMode -Version Latest

function Get-VerifiedFileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Verified file is missing: $Path"
    }
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash
}

function Get-VerifiedDirectorySha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "Verified directory is missing: $Path"
    }
    $resolvedRoot = (Resolve-Path -LiteralPath $Path).Path.TrimEnd([char]'\')
    $entries = foreach ($file in Get-ChildItem -LiteralPath $resolvedRoot -Recurse -File | Sort-Object FullName) {
        $relativePath = $file.FullName.Substring($resolvedRoot.Length).TrimStart([char]'\').Replace('\', '/')
        $hash = Get-VerifiedFileSha256 -Path $file.FullName
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

function New-VerifiedFileBackup {
    param(
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$BackupPath
    )

    $sourceHash = Get-VerifiedFileSha256 -Path $SourcePath
    if (Test-Path -LiteralPath $BackupPath) {
        throw "Refusing to overwrite an existing maintenance backup: $BackupPath"
    }
    Copy-Item -LiteralPath $SourcePath -Destination $BackupPath
    $backupHash = Get-VerifiedFileSha256 -Path $BackupPath
    if ($backupHash -ne $sourceHash) {
        throw 'The maintenance backup does not match its source file.'
    }
    return $backupHash
}

function Set-VerifiedFileFromCandidate {
    param(
        [Parameter(Mandatory = $true)][string]$CandidatePath,
        [Parameter(Mandatory = $true)][string]$DestinationPath,
        [Parameter(Mandatory = $true)][string]$ExpectedSha256,
        [scriptblock]$BeforeCommit
    )

    $candidateHash = Get-VerifiedFileSha256 -Path $CandidatePath
    if ($candidateHash -ne $ExpectedSha256) {
        throw 'The candidate file hash does not match the expected artifact hash.'
    }
    $destinationDirectory = Split-Path -Parent $DestinationPath
    if (-not (Test-Path -LiteralPath $destinationDirectory -PathType Container)) {
        throw "Destination directory is missing: $destinationDirectory"
    }
    $temporaryPath = Join-Path $destinationDirectory (
        ([IO.Path]::GetFileName($DestinationPath)) +
        '.transition-' + [Diagnostics.Process]::GetCurrentProcess().Id +
        '-' + [Guid]::NewGuid().ToString('N') + '.tmp'
    )
    try {
        Copy-Item -LiteralPath $CandidatePath -Destination $temporaryPath
        if ((Get-VerifiedFileSha256 -Path $temporaryPath) -ne $ExpectedSha256) {
            throw 'The staged candidate copy failed its hash verification.'
        }
        if ($BeforeCommit) {
            & $BeforeCommit $temporaryPath
        }
        Move-Item -LiteralPath $temporaryPath -Destination $DestinationPath -Force
        if ((Get-VerifiedFileSha256 -Path $DestinationPath) -ne $ExpectedSha256) {
            throw 'The committed destination file failed its hash verification.'
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
    }
}

function Restore-VerifiedFileBackup {
    param(
        [Parameter(Mandatory = $true)][string]$BackupPath,
        [Parameter(Mandatory = $true)][string]$DestinationPath,
        [Parameter(Mandatory = $true)][string]$ExpectedBackupSha256
    )

    Set-VerifiedFileFromCandidate `
        -CandidatePath $BackupPath `
        -DestinationPath $DestinationPath `
        -ExpectedSha256 $ExpectedBackupSha256
}
