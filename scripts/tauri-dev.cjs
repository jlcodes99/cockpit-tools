const { spawnSync } = require('node:child_process');

const env = {
  ...process.env,
  COCKPIT_TOOLS_PROFILE: process.env.COCKPIT_TOOLS_PROFILE || 'dev',
  COCKPIT_TOOLS_API_PORT: process.env.COCKPIT_TOOLS_API_PORT || '1456',
  VITE_COCKPIT_TOOLS_PROFILE: process.env.VITE_COCKPIT_TOOLS_PROFILE || 'dev',
};
const extraArgs = process.argv.slice(2);

const syncResult = spawnSync('npm', ['run', 'sync-version'], {
  stdio: 'inherit',
  env,
});

if (syncResult.status !== 0) {
  process.exit(syncResult.status ?? 1);
}

const tauriResult = spawnSync(
  'tauri',
  ['dev', '--config', 'src-tauri/tauri.dev.conf.json', ...extraArgs],
  {
    stdio: 'inherit',
    env,
  },
);

process.exit(tauriResult.status ?? 1);
