const { spawnSync } = require('node:child_process');

function resolveMacosSdkRoot() {
  if (process.platform !== 'darwin') {
    return process.env.SDKROOT;
  }
  // Some shells/Xcode setups leave SDKROOT pointing at iPhoneOS, which breaks
  // swift-rs / macos-native-menu (targets arm64-apple-macosx).
  const current = process.env.SDKROOT || '';
  if (current && /MacOSX\.platform|MacOSX[^/]*\.sdk/i.test(current)) {
    return current;
  }
  const probed = spawnSync('xcrun', ['--sdk', 'macosx', '--show-sdk-path'], {
    encoding: 'utf8',
  });
  if (probed.status === 0) {
    const path = String(probed.stdout || '').trim();
    if (path) return path;
  }
  return current || undefined;
}

const env = {
  ...process.env,
  COCKPIT_TOOLS_PROFILE: process.env.COCKPIT_TOOLS_PROFILE || 'dev',
  COCKPIT_TOOLS_API_PORT: process.env.COCKPIT_TOOLS_API_PORT || '1456',
  VITE_COCKPIT_TOOLS_PROFILE: process.env.VITE_COCKPIT_TOOLS_PROFILE || 'dev',
};
const macosSdkRoot = resolveMacosSdkRoot();
if (macosSdkRoot) {
  env.SDKROOT = macosSdkRoot;
}
const extraArgs = process.argv.slice(2);

const isWin = process.platform === 'win32';

const syncResult = spawnSync(
  isWin ? 'npm.cmd' : 'npm',
  ['run', 'sync-version'],
  {
    stdio: 'inherit',
    env,
    shell: isWin,
  },
);

if (syncResult.status !== 0) {
  process.exit(syncResult.status ?? 1);
}

const tauriResult = spawnSync(
  isWin ? 'npx.cmd' : 'npx',
  ['tauri', 'dev', '--config', 'src-tauri/tauri.dev.conf.json', ...extraArgs],
  {
    stdio: 'inherit',
    env,
    shell: isWin,
  },
);

process.exit(tauriResult.status ?? 1);
