# Install cockpit + cockpit-cli into %USERPROFILE%\.local\bin.
# MCPX WinSW PATH already starts with that directory, so ArcKnights/cmd.exe
# can resolve `cockpit quota --json` without a service restart after this copy.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $env:USERPROFILE '.local\bin'
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    throw 'cargo is required to install cockpit-cli'
}

$argList = @(
    'install'
    '--path'
    (Join-Path $repoRoot 'crates\cockpit-cli')
    '--root'
    (Join-Path $env:USERPROFILE '.local')
    '--locked'
    '--force'
    '--bins'
)
& $cargo.Source @argList
if ($LASTEXITCODE -ne 0) {
    throw "cargo install failed with exit $LASTEXITCODE"
}

$cockpit = Join-Path $binDir 'cockpit.exe'
$alias = Join-Path $binDir 'cockpit-cli.exe'
if (-not (Test-Path -LiteralPath $cockpit)) {
    throw "expected $cockpit after cargo install"
}
if (-not (Test-Path -LiteralPath $alias)) {
    Copy-Item -LiteralPath $cockpit -Destination $alias -Force
}

Write-Output "installed $cockpit"
Write-Output "installed $alias"
Write-Output 'verify: cockpit quota --json'
