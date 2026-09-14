const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const repoRoot = path.resolve(__dirname, '..');
const goBinPath = 'C:\\Program Files\\Go\\bin';
const cargoBinPath = path.join(os.homedir(), '.cargo', 'bin');

function withToolchainPaths(options = {}) {
  const currentPath = process.env.PATH || '';
  const extraPaths = [cargoBinPath, goBinPath].filter((dir) => fs.existsSync(dir));
  const pathValue = extraPaths.length > 0
    ? `${extraPaths.join(path.delimiter)}${path.delimiter}${currentPath}`
    : currentPath;

  return {
    ...options,
    env: {
      ...process.env,
      ...options.env,
      PATH: pathValue,
    },
  };
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    shell: false,
    ...withToolchainPaths(options),
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    process.exit(typeof result.status === 'number' ? result.status : 1);
  }
}

function runFinal(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    shell: false,
    ...withToolchainPaths(options),
  });

  if (result.error) {
    throw result.error;
  }

  process.exit(typeof result.status === 'number' ? result.status : 1);
}

function runTauriDirect() {
  run('npm.cmd', ['run', 'sync-version'], { shell: process.platform === 'win32' });
  runFinal('npx.cmd', ['tauri', ...process.argv.slice(2)], { shell: process.platform === 'win32' });
}

if (process.platform !== 'win32') {
  run('npm', ['run', 'sync-version']);
  runFinal('npx', ['tauri', ...process.argv.slice(2)]);
}

const vcvarsCandidates = [
  'C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional\\VC\\Auxiliary\\Build\\vcvars64.bat',
  'C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise\\VC\\Auxiliary\\Build\\vcvars64.bat',
  'C:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat',
  'C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat',
];
const vcvars64Path = vcvarsCandidates.find((candidate) => fs.existsSync(candidate)) || '';

if (!vcvars64Path) {
  console.warn('vcvars64.bat not found, falling back to the existing shell environment.');
  runTauriDirect();
}

const tempScriptPath = path.join(os.tmpdir(), `cockpit-tools-tauri-${process.pid}.cmd`);
const tauriCliPath = path.join(repoRoot, 'node_modules', '.bin', 'tauri.cmd');
const tauriArgs = process.argv.slice(2);

if (!fs.existsSync(tauriCliPath)) {
  console.warn('Local tauri CLI not found, falling back to the existing shell environment.');
  runTauriDirect();
}

const quotedArgs = tauriArgs.map((arg) => {
  if (/[\s"]/u.test(arg)) {
    return `"${arg.replace(/"/g, '""')}"`;
  }
  return arg;
});
const scriptBody = [
  '@echo off',
  `set "PATH=${cargoBinPath};${goBinPath};%PATH%"`,
  `call "${vcvars64Path}"`,
  'if errorlevel 1 exit /b %errorlevel%',
  'call npm.cmd run sync-version',
  'if errorlevel 1 exit /b %errorlevel%',
  `call "${tauriCliPath}" ${quotedArgs.join(' ')}`.trim(),
].join('\r\n');

fs.writeFileSync(tempScriptPath, scriptBody);

try {
  runFinal('cmd.exe', ['/d', '/c', tempScriptPath]);
} finally {
  fs.rmSync(tempScriptPath, { force: true });
}
