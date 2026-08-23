[CmdletBinding()]
param(
    [ValidateRange(0, 65535)]
    [int]$Port = 0,

    [string]$OutputRoot = '',

    [switch]$NoOpen
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$WebCliPath =
    Join-Path `
        $PSScriptRoot `
        'cockpit-account-exporter-web.cjs'

if (-not (Test-Path -LiteralPath $WebCliPath -PathType Leaf)) {
    throw "Exporter web CLI not found: $WebCliPath"
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

$Arguments = @(
    $WebCliPath,
    '--port',
    $Port.ToString()
)

if (-not [string]::IsNullOrWhiteSpace($OutputRoot)) {
    $Arguments += @(
        '--output-root',
        $OutputRoot
    )
}

if ($NoOpen) {
    $Arguments += '--no-open'
}

& $NodePath @Arguments
exit $LASTEXITCODE
