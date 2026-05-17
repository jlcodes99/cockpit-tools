const { spawnSync } = require('node:child_process');
const path = require('node:path');

if (process.platform !== 'win32') {
  process.exit(0);
}

const repoRoot = path.resolve(__dirname, '..');
const targetExe = path.join(repoRoot, 'target', 'debug', 'cockpit_tools.exe');
const escapedTarget = targetExe.replace(/'/g, "''").toLowerCase();

const script = `
$ErrorActionPreference = 'Stop'
$target = '${escapedTarget}'
$processes = Get-CimInstance Win32_Process -Filter "Name = 'cockpit_tools.exe'" |
  Where-Object { $_.ExecutablePath -and ($_.ExecutablePath.ToLowerInvariant() -eq $target) }
foreach ($process in $processes) {
  Stop-Process -Id $process.ProcessId -Force
  Write-Output ("Stopped stale Cockpit Tools debug process PID " + $process.ProcessId)
}
`;

const result = spawnSync(
  'powershell.exe',
  ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script],
  {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }
);

if (result.stdout) {
  process.stdout.write(result.stdout);
}

if (result.status !== 0) {
  if (result.stderr) {
    process.stderr.write(result.stderr);
  }
  process.exit(result.status ?? 1);
}
