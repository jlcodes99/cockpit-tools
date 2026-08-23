[CmdletBinding()]
param(
    [ValidateSet('production', 'development')]
    [string]$Profile = 'production',

    [ValidateSet('team', 'pro', 'plus', 'free', 'all')]
    [string]$PlanFamily = 'team',

    [ValidateSet('both', 'json', 'csv')]
    [string]$Format = 'both',

    [string]$Datasets = 'accounts,quota,gateway',

    [string]$DataDirectory = '',

    [string]$OutputRoot = '',

    [ValidateRange(1, 10080)]
    [int]$StaleAfterMinutes = 15,

    [switch]$SkipInvalid,

    [switch]$ValidateOnly,

    [switch]$SkipAclHardening
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$CliPath =
    Join-Path `
        $PSScriptRoot `
        'cockpit-account-exporter.cjs'

if (-not (Test-Path -LiteralPath $CliPath -PathType Leaf)) {
    throw "Exporter CLI not found: $CliPath"
}

$NodeCommand =
    Get-Command `
        node.exe `
        -ErrorAction SilentlyContinue

if ($null -ne $NodeCommand) {
    $NodePath = $NodeCommand.Source
} else {
    $BundledNode =
        Join-Path `
            $env:USERPROFILE `
            '.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe'

    if (-not (Test-Path -LiteralPath $BundledNode -PathType Leaf)) {
        throw 'Node.js 18 or newer is required.'
    }
    $NodePath = $BundledNode
}

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot =
        Join-Path `
            (Get-Location).Path `
            'cockpit-account-exports'
}

$OutputDirectory =
    Join-Path `
        $OutputRoot `
        (
            'account-export-' +
            $Profile +
            '-' +
            (Get-Date -Format 'yyyyMMdd-HHmmss')
        )

if (-not $ValidateOnly) {
    if (Test-Path -LiteralPath $OutputDirectory) {
        throw "Output directory already exists: $OutputDirectory"
    }

    if (-not $SkipAclHardening) {
        New-Item `
            -ItemType Directory `
            -Path $OutputDirectory `
            -Force |
        Out-Null

        $CurrentUserSid =
            [Security.Principal.WindowsIdentity]::GetCurrent().User.Value

        & icacls.exe `
            $OutputDirectory `
            /inheritance:r |
        Out-Null

        if ($LASTEXITCODE -ne 0) {
            throw "Unable to disable inherited ACLs on output directory: $OutputDirectory"
        }

        & icacls.exe `
            $OutputDirectory `
            /grant:r `
            "*${CurrentUserSid}:(OI)(CI)(F)" `
            '*S-1-5-18:(OI)(CI)(F)' `
            '*S-1-5-32-544:(OI)(CI)(F)' |
        Out-Null

        if ($LASTEXITCODE -ne 0) {
            throw "Unable to harden output directory ACL: $OutputDirectory"
        }
    }
}

$CliArguments = @(
    $CliPath,
    '--profile',
    $Profile,
    '--plan-family',
    $PlanFamily,
    '--format',
    $Format,
    '--datasets',
    $Datasets,
    '--stale-after-minutes',
    $StaleAfterMinutes.ToString()
)

$DatasetValues = @(
    $Datasets.Split(',') |
    ForEach-Object {
        $_.Trim().ToLowerInvariant()
    } |
    Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    } |
    Select-Object -Unique
)

if ($DatasetValues.Count -eq 0) {
    throw 'At least one dataset must be selected.'
}

$UnsupportedDatasets = @(
    $DatasetValues |
    Where-Object {
        $_ -notin @(
            'accounts',
            'quota',
            'gateway'
        )
    }
)

if ($UnsupportedDatasets.Count -gt 0) {
    throw (
        'Unsupported dataset: ' +
        ($UnsupportedDatasets -join ', ')
    )
}

$DatasetsArgumentIndex =
    [Array]::IndexOf(
        $CliArguments,
        '--datasets'
    ) + 1

$CliArguments[$DatasetsArgumentIndex] =
    $DatasetValues -join ','

if (-not [string]::IsNullOrWhiteSpace($DataDirectory)) {
    $CliArguments += @(
        '--data-dir',
        $DataDirectory
    )
}

if ($SkipInvalid) {
    $CliArguments += '--skip-invalid'
}

if ($ValidateOnly) {
    $CliArguments += '--validate-only'
} else {
    $CliArguments += @(
        '--output-dir',
        $OutputDirectory
    )
}

& $NodePath @CliArguments
$ExporterExitCode = $LASTEXITCODE

if ($ExporterExitCode -ne 0) {
    throw "Cockpit account exporter failed with exit code $ExporterExitCode."
}

if ($ValidateOnly) {
    return
}

Write-Host ''
Write-Host 'Export directory:' -ForegroundColor Green
Write-Host $OutputDirectory

Get-ChildItem `
    -LiteralPath $OutputDirectory `
    -File |
Select-Object `
    Name,
    Length,
    LastWriteTime |
Format-Table -AutoSize
