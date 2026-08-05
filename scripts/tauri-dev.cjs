const { spawnSync } = require('node:child_process');
const path = require('node:path');

const repoRoot = path.resolve(__dirname, '..');

function exitOnFailure(result, label) {
  if (result.error) {
    console.error(`${label} failed to start: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(typeof result.status === 'number' ? result.status : 1);
  }
}

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
const npmExecPath = process.env.npm_execpath;
const npmCommand = npmExecPath ? process.execPath : process.platform === 'win32' ? 'npm.cmd' : 'npm';
const npmArgs = npmExecPath
  ? [npmExecPath, 'run', 'sync-version']
  : ['run', 'sync-version'];

const syncResult = spawnSync(npmCommand, npmArgs, {
  cwd: repoRoot,
  stdio: 'inherit',
  env,
  shell: !npmExecPath && process.platform === 'win32',
});
exitOnFailure(syncResult, 'Version synchronization');

const tauriCliPath = require.resolve('@tauri-apps/cli/tauri.js', { paths: [repoRoot] });
const tauriResult = spawnSync(
  process.execPath,
  [tauriCliPath, 'dev', '--config', 'src-tauri/tauri.dev.conf.json', ...extraArgs],
  {
    cwd: repoRoot,
    stdio: 'inherit',
    env,
  },
);
exitOnFailure(tauriResult, 'Tauri development server');
