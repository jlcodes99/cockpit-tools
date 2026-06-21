const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const repoRoot = path.resolve(__dirname, '..');

function commandFor(baseName) {
  return process.platform === 'win32' ? `${baseName}.cmd` : baseName;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    shell: process.platform === 'win32',
    ...options,
  });

  if (result.error) {
    console.error(`Failed to start ${command}: ${result.error.message}`);
    process.exit(1);
  }

  if (result.status !== 0) {
    process.exit(typeof result.status === 'number' ? result.status : 1);
  }
}

function runFinal(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    shell: process.platform === 'win32',
    ...options,
  });

  if (result.error) {
    console.error(`Failed to start ${command}: ${result.error.message}`);
    process.exit(1);
  }

  process.exit(typeof result.status === 'number' ? result.status : 1);
}

const env = {
  ...process.env,
  COCKPIT_TOOLS_PROFILE: process.env.COCKPIT_TOOLS_PROFILE || 'dev',
  COCKPIT_TOOLS_API_PORT: process.env.COCKPIT_TOOLS_API_PORT || '1456',
  VITE_COCKPIT_TOOLS_PROFILE: process.env.VITE_COCKPIT_TOOLS_PROFILE || 'dev',
};
const extraArgs = process.argv.slice(2);
const localTauriCli = path.join(
  repoRoot,
  'node_modules',
  '.bin',
  process.platform === 'win32' ? 'tauri.cmd' : 'tauri',
);

run(commandFor('npm'), ['run', 'sync-version'], { env });

if (fs.existsSync(localTauriCli)) {
  runFinal(localTauriCli, ['dev', '--config', 'src-tauri/tauri.dev.conf.json', ...extraArgs], {
    env,
  });
}

runFinal(commandFor('npx'), ['tauri', 'dev', '--config', 'src-tauri/tauri.dev.conf.json', ...extraArgs], {
  env,
});
