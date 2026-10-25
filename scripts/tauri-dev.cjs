const { spawnSync } = require('node:child_process');
const path = require('node:path');

const repoRoot = path.resolve(__dirname, '..');

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
    const value = String(probed.stdout || '').trim();
    if (value) return value;
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

// `tauri.cjs` owns the Windows-specific plumbing this script cannot do on its
// own: resolving npm.cmd/npx.cmd (a bare spawnSync gets ENOENT/EINVAL for
// .cmd files) and initializing the MSVC environment through vcvars64.bat.
// Delegating keeps one implementation of that logic instead of two drifting
// copies, and stops `tauri:dev` from exiting 1 without printing anything.
const args = ['dev', '--config', 'src-tauri/tauri.dev.conf.json', ...process.argv.slice(2)];
const result = spawnSync(process.execPath, [path.join(__dirname, 'tauri.cjs'), ...args], {
  cwd: repoRoot,
  stdio: 'inherit',
  env,
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
