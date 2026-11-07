const { spawnSync } = require('node:child_process');
const path = require('node:path');
const { runDevCommand } = require('./dev-process-session.cjs');

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

async function main() {
  const cwd = path.resolve(__dirname, '..');
  const synced = await runDevCommand(process.execPath, [path.join(__dirname, 'sync-version.js')], { cwd, env });
  if (synced !== 0) return synced;
  return runDevCommand(process.execPath, [require.resolve('@tauri-apps/cli/tauri.js'),
    'dev', '--config', 'src-tauri/tauri.dev.conf.json', ...extraArgs], { cwd, env });
}

main().then(code => { process.exitCode = code; }).catch(error => {
  console.error(`[tauri-dev] ${error.message}`);
  process.exitCode = 1;
});
