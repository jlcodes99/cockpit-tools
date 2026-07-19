const { spawnSync } = require('node:child_process');

const env = {
  ...process.env,
  COCKPIT_TOOLS_PROFILE: 'test',
  COCKPIT_TOOLS_BUILD_PROFILE: 'test',
  VITE_COCKPIT_TOOLS_PROFILE: 'test',
};

const extraArgs = process.argv.slice(2);
const tauriArgs = [
  'tauri',
  'build',
  '--config',
  'src-tauri/tauri.test.conf.json',
  ...extraArgs,
];

const syncResult = spawnSync('npm', ['run', 'sync-version'], {
  stdio: 'inherit',
  env,
});

if (syncResult.status !== 0) {
  process.exit(syncResult.status ?? 1);
}

const buildResult = spawnSync('npx', tauriArgs, {
  stdio: 'inherit',
  env,
});

process.exit(buildResult.status ?? 1);
