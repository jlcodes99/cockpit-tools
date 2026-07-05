[CmdletBinding()]
param(
  [switch]$SkipNpmInstall
)

$ErrorActionPreference = "Continue"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$LogDir = Join-Path $RepoRoot "docs\refactor\verification-logs"
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$LogPath = Join-Path $LogDir "$Stamp-baseline.log"
$Failures = 0

New-Item -ItemType Directory -Force $LogDir | Out-Null

function Write-Log {
  param([string]$Message)
  $line = "[{0}] {1}" -f (Get-Date -Format "o"), $Message
  Write-Output $line
  Add-Content -Path $LogPath -Value $line
}

function Invoke-BaselineStep {
  param(
    [string]$Name,
    [string[]]$Command,
    [string]$WorkingDirectory = $RepoRoot
  )

  Write-Log ""
  Write-Log "==> $Name"
  Write-Log "cwd: $WorkingDirectory"
  Write-Log "cmd: $($Command -join ' ')"

  if (-not (Test-Path -LiteralPath $WorkingDirectory)) {
    Write-Log "FAIL: working directory does not exist"
    $script:Failures += 1
    return
  }

  $exe = $Command[0]
  if (-not (Get-Command $exe -ErrorAction SilentlyContinue)) {
    Write-Log "SKIP/FAIL: command not found: $exe"
    $script:Failures += 1
    return
  }

  $commandArgs = @()
  if ($Command.Length -gt 1) {
    $commandArgs = $Command[1..($Command.Length - 1)]
  }

  Push-Location $WorkingDirectory
  try {
    & $exe @commandArgs 2>&1 | Tee-Object -FilePath $LogPath -Append
    $exitCode = $LASTEXITCODE
    if ($null -eq $exitCode) {
      $exitCode = 0
    }
    if ($exitCode -ne 0) {
      Write-Log "FAIL: exit code $exitCode"
      $script:Failures += 1
    } else {
      Write-Log "PASS"
    }
  } finally {
    Pop-Location
  }
}

Write-Log "Cockpit Tools refactor baseline verification"
Write-Log "repo: $RepoRoot"
Write-Log "log: $LogPath"

Invoke-BaselineStep -Name "git status" -Command @("git", "status", "--short", "--branch")
Invoke-BaselineStep -Name "node version" -Command @("node", "--version")
Invoke-BaselineStep -Name "npm version" -Command @("npm", "--version")
Invoke-BaselineStep -Name "cargo version" -Command @("cargo", "--version")
Invoke-BaselineStep -Name "rustc version" -Command @("rustc", "--version")
Invoke-BaselineStep -Name "go version" -Command @("go", "version")

if (-not $SkipNpmInstall) {
  Invoke-BaselineStep -Name "npm install" -Command @("npm", "install")
}

Invoke-BaselineStep -Name "npm run typecheck" -Command @("npm", "run", "typecheck")
Invoke-BaselineStep -Name "cargo test --workspace" -Command @("cargo", "test", "--workspace")
Invoke-BaselineStep `
  -Name "go test sidecar" `
  -Command @("go", "test", "./...") `
  -WorkingDirectory (Join-Path $RepoRoot "sidecars\cockpit-cliproxy")

Write-Log ""
if ($Failures -gt 0) {
  Write-Log "Baseline completed with $Failures failure(s) or skipped required tool(s)."
  exit 1
}

Write-Log "Baseline completed successfully."
exit 0
