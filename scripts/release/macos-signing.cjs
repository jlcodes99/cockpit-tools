#!/usr/bin/env node
// Developer ID credentials stay in a temporary keychain, never in release assets.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');
const { createDmgMounts } = require('./dmg-mounts.cjs');

const PROFILE = 'cockpit-notarization';
const REQUIRED = ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD',
  'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'];

function command(executable, args, options = {}) {
  const { stdoutOnly = false, ...spawnOptions } = options;
  const result = spawnSync(executable, args, {
    encoding: 'utf8', stdio: 'pipe', timeout: 120_000, ...spawnOptions,
  });
  // Never include command arguments: security/notarytool may carry passwords.
  if (result.error || result.status !== 0) {
    let detail = `${result.stdout || ''}${result.stderr || ''}`;
    for (let index = 0; index < args.length - 1; index += 1) {
      if (['--password', '-p', '-P', '-k'].includes(args[index]) && args[index + 1]) {
        detail = detail.split(args[index + 1]).join('[REDACTED]');
      }
    }
    throw new Error(`${executable} ${args[0]} failed (${result.error?.code || result.status}): ${detail.trim()}`);
  }
  return `${result.stdout || ''}${stdoutOnly ? '' : result.stderr || ''}`;
}

function validateCredentials(env) {
  const missing = REQUIRED.filter((name) => !env[name]?.trim());
  if (missing.length) throw new Error(`Missing macOS release secrets: ${missing.join(', ')}`);
  if (!/^[A-Z0-9]{10}$/.test(env.APPLE_TEAM_ID)
      || !env.APPLE_SIGNING_IDENTITY.startsWith('Developer ID Application: ')
      || !env.APPLE_SIGNING_IDENTITY.endsWith(`(${env.APPLE_TEAM_ID})`)) {
    throw new Error('Require a Developer ID Application identity matching APPLE_TEAM_ID');
  }
}

function prepare(env = process.env, run = command) {
  validateCredentials(env);
  if (!env.GITHUB_ENV || !env.RUNNER_TEMP) throw new Error('prepare requires a GitHub Actions runner');
  const directory = fs.mkdtempSync(path.join(env.RUNNER_TEMP, 'cockpit-signing-'));
  fs.chmodSync(directory, 0o700);
  // Register cleanup before any import or network request can fail.
  fs.appendFileSync(env.GITHUB_ENV, `COCKPIT_SIGNING_DIR=${directory}\n`);
  const keychain = path.join(directory, 'signing.keychain-db');
  const original = [...run('security', ['list-keychains', '-d', 'user']).matchAll(/"([^"]+)"/g)]
    .map((match) => match[1]);
  fs.writeFileSync(path.join(directory, 'search-list.json'), JSON.stringify(original), { mode: 0o600 });
  const password = crypto.randomBytes(32).toString('hex');
  const certificate = path.join(directory, 'certificate.p12');
  try {
    run('security', ['create-keychain', '-p', password, keychain]);
    run('security', ['set-keychain-settings', '-lut', '21600', keychain]);
    run('security', ['unlock-keychain', '-p', password, keychain]);
    fs.writeFileSync(certificate, Buffer.from(env.APPLE_CERTIFICATE, 'base64'), { mode: 0o600 });
    run('security', ['import', certificate, '-k', keychain, '-P', env.APPLE_CERTIFICATE_PASSWORD,
      '-T', '/usr/bin/codesign', '-T', '/usr/bin/security']);
    run('security', ['set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password, keychain]);
    run('security', ['list-keychains', '-d', 'user', '-s', keychain, ...original]);
    const identities = run('security', ['find-identity', '-v', '-p', 'codesigning', keychain]);
    if (!identities.includes(`"${env.APPLE_SIGNING_IDENTITY}"`)) {
      throw new Error('Imported certificate does not match APPLE_SIGNING_IDENTITY');
    }
    run('xcrun', ['notarytool', 'store-credentials', PROFILE, '--keychain', keychain,
      '--apple-id', env.APPLE_ID, '--team-id', env.APPLE_TEAM_ID, '--password', env.APPLE_PASSWORD]);
    console.log('Developer ID identity and notarization credentials verified.');
  } finally {
    fs.rmSync(certificate, { force: true });
  }
}

function cleanup(env = process.env, run = command) {
  const directory = env.COCKPIT_SIGNING_DIR;
  if (!directory || !fs.existsSync(directory)) return;
  const searchList = path.join(directory, 'search-list.json');
  if (fs.existsSync(searchList)) {
    run('security', ['list-keychains', '-d', 'user', '-s', ...JSON.parse(fs.readFileSync(searchList))]);
  }
  const keychain = path.join(directory, 'signing.keychain-db');
  if (fs.existsSync(keychain)) run('security', ['delete-keychain', keychain]);
  fs.rmSync(directory, { recursive: true, force: true });
}

function exactlyOne(directory, suffix) {
  const matches = fs.readdirSync(directory).filter((name) => name.endsWith(suffix));
  if (matches.length !== 1) throw new Error(`Expected exactly one ${suffix} in ${directory}`);
  return path.join(directory, matches[0]);
}

function verifyApp(app, team, run = command) {
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', app]);
  const info = run('codesign', ['--display', '--verbose=4', app]);
  if (!info.includes('Authority=Developer ID Application:')
      || !info.split(/\r?\n/).includes(`TeamIdentifier=${team}`)
      || !/flags=.*\bruntime\b/.test(info)) {
    throw new Error('App requires the expected Developer ID team and hardened runtime');
  }
  run('xcrun', ['stapler', 'validate', app]);
  run('spctl', ['--assess', '--type', 'execute', '--verbose=2', app]);
}

function notarizeDmg(dmg, env = process.env, run = command) {
  const keychain = path.join(env.COCKPIT_SIGNING_DIR, 'signing.keychain-db');
  const credentials = ['--keychain-profile', PROFILE, '--keychain', keychain];
  run('codesign', ['--force', '--sign', env.APPLE_SIGNING_IDENTITY, '--timestamp', dmg]);
  run('codesign', ['--verify', '--strict', dmg]);
  const submission = JSON.parse(run('xcrun', ['notarytool', 'submit', dmg, ...credentials,
    '--wait', '--timeout', '30m', '--output-format', 'json'], { timeout: 31 * 60_000, stdoutOnly: true }));
  if (submission.status !== 'Accepted') {
    throw new Error(`DMG notarization ${submission.status}; submission ID: ${submission.id || 'unavailable'}`);
  }
  console.log(`DMG notarization accepted: ${submission.id}`);
  run('xcrun', ['stapler', 'staple', dmg]);
  run('xcrun', ['stapler', 'validate', dmg]);
  run('codesign', ['--verify', '--strict', dmg]);
  run('spctl', ['--assess', '--type', 'open', '--context', 'context:primary-signature', '--verbose=2', dmg]);
}

function finalize(bundleDir, env = process.env, run = command, mounts = createDmgMounts()) {
  if (!env.COCKPIT_SIGNING_DIR || !env.APPLE_TEAM_ID || !env.APPLE_SIGNING_IDENTITY) {
    throw new Error('Signing setup is required before finalizing macOS assets');
  }
  const app = exactlyOne(path.join(bundleDir, 'macos'), '.app');
  const archive = exactlyOne(path.join(bundleDir, 'macos'), '.app.tar.gz');
  const dmg = exactlyOne(path.join(bundleDir, 'dmg'), '.dmg');
  verifyApp(app, env.APPLE_TEAM_ID, run);
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-signed-assets-'));
  mounts.installExitHandlers();
  try {
    const extracted = path.join(work, 'updater');
    fs.mkdirSync(extracted);
    run('tar', ['-xzf', archive, '-C', extracted]);
    verifyApp(exactlyOne(extracted, '.app'), env.APPLE_TEAM_ID, run);
    // inject-dmg-readme rewrites the image: sign/notarize only its final bytes.
    notarizeDmg(dmg, env, run);
    const mounted = path.join(work, 'mounted');
    fs.mkdirSync(mounted);
    const record = mounts.attach(mounted, dmg, { readOnly: true });
    try {
      verifyApp(exactlyOne(mounted, '.app'), env.APPLE_TEAM_ID, run);
    } finally { mounts.detach(record); }
    console.log('App, updater archive and final DMG passed Developer ID / Gatekeeper checks.');
  } finally {
    mounts.cleanup();
    fs.rmSync(work, { recursive: true, force: true });
  }
}

if (require.main === module) {
  try {
    if (process.platform !== 'darwin') throw new Error('macOS is required');
    const [mode, bundleDir] = process.argv.slice(2);
    if (mode === 'prepare') prepare();
    else if (mode === 'cleanup') cleanup();
    else if (mode === 'finalize' && bundleDir) finalize(path.resolve(bundleDir));
    else throw new Error('Usage: macos-signing.cjs prepare|cleanup|finalize <bundle-directory>');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { command, validateCredentials, prepare, cleanup, verifyApp, notarizeDmg, finalize };
