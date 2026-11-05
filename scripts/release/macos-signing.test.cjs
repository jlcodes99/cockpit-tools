const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { validateCredentials, prepare, cleanup, verifyApp, notarizeDmg, finalize } = require('./macos-signing.cjs');

const env = {
  APPLE_CERTIFICATE: Buffer.from('test-certificate').toString('base64'),
  APPLE_CERTIFICATE_PASSWORD: 'test-password',
  APPLE_SIGNING_IDENTITY: 'Developer ID Application: Example (ABCDEFGHIJ)',
  APPLE_ID: 'example@example.com', APPLE_PASSWORD: 'test-notary-password', APPLE_TEAM_ID: 'ABCDEFGHIJ',
  COCKPIT_SIGNING_DIR: '/tmp/test-signing',
};
const validInfo = 'Authority=Developer ID Application: Example (ABCDEFGHIJ)\nTeamIdentifier=ABCDEFGHIJ\nCodeDirectory flags=0x10000(runtime)\n';

function temp(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'macos-signing-test-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

test('missing secrets, development certificates and a different team fail before keychain mutations', () => {
  const cases = [
    [{ ...env, APPLE_PASSWORD: '' }, /APPLE_PASSWORD/],
    [{ ...env, APPLE_SIGNING_IDENTITY: 'Apple Development: Example (ABCDEFGHIJ)' }, /Developer ID/],
    [{ ...env, APPLE_TEAM_ID: '1234567890' }, /matching/],
  ];
  for (const [input, error] of cases) {
    assert.throws(() => prepare(input, () => assert.fail('must not call security')), error);
  }
  assert.doesNotThrow(() => validateCredentials(env));
});

test('failed certificate import registers cleanup and removes the exported certificate', (t) => {
  const directory = temp(t);
  const envFile = path.join(directory, 'env');
  const calls = [];
  const run = (exe, args) => {
    calls.push([exe, ...args]);
    if (args[0] === 'import') throw new Error('import failed');
    if (args[0] === 'create-keychain') fs.writeFileSync(args.at(-1), 'keychain');
    return '  "/Users/runner/Library/Keychains/login.keychain-db"\n';
  };
  assert.throws(() => prepare({ ...env, RUNNER_TEMP: directory, GITHUB_ENV: envFile }, run), /import failed/);
  const signingDir = fs.readFileSync(envFile, 'utf8').trim().split('=')[1];
  assert.equal(fs.existsSync(path.join(signingDir, 'certificate.p12')), false);
  cleanup({ COCKPIT_SIGNING_DIR: signingDir }, run);
  assert.equal(fs.existsSync(signingDir), false);
  assert.ok(calls.some((call) => call.join(' ') === 'security list-keychains -d user -s /Users/runner/Library/Keychains/login.keychain-db'));
});

test('valid signature alone is insufficient: require correct authority, team, runtime and ticket', () => {
  for (const invalid of [
    validInfo.replace('Developer ID Application', 'Apple Development'),
    validInfo.replace('TeamIdentifier=ABCDEFGHIJ', 'TeamIdentifier=0123456789'),
    validInfo.replace('0x10000(runtime)', '0x0(none)'),
  ]) {
    assert.throws(() => verifyApp('/test.app', env.APPLE_TEAM_ID, (_exe, args) =>
      args[0] === '--display' ? invalid : ''), /Developer ID/);
  }
  assert.throws(() => verifyApp('/test.app', env.APPLE_TEAM_ID, (exe, args) => {
    if (exe === 'xcrun') throw new Error('no ticket');
    return args[0] === '--display' ? validInfo : '';
  }), /no ticket/);
});

test('rejected or unfinished notarization never staples or accepts a DMG', () => {
  for (const status of ['Invalid', 'In Progress', 'Rejected']) {
    const calls = [];
    assert.throws(() => notarizeDmg('/test.dmg', env, (exe, args) => {
      calls.push([exe, ...args]);
      return JSON.stringify({ status, id: 'test-submission' });
    }), /test-submission/);
    assert.equal(calls.some((call) => call.includes('stapler') || call[0] === 'spctl'), false);
  }
});

test('notarization has a bounded wait and final ticket and Gatekeeper checks', () => {
  const calls = [];
  notarizeDmg('/test.dmg', env, (exe, args, options) => {
    calls.push([exe, args, options]);
    return JSON.stringify({ status: 'Accepted', id: 'test-submission' });
  });
  const submission = calls.find(([, args]) => args.includes('submit'));
  assert.ok(submission[1].includes('30m'));
  assert.equal(submission[2].timeout, 31 * 60_000);
  assert.ok(calls.some(([exe, args]) => exe === 'xcrun' && args[1] === 'validate'));
  assert.equal(calls.at(-1)[0], 'spctl');
});

test('finalization rejects a bad updater app before notarizing or uploading the DMG', (t) => {
  const directory = temp(t);
  fs.mkdirSync(path.join(directory, 'macos/test.app'), { recursive: true });
  fs.mkdirSync(path.join(directory, 'dmg'));
  fs.writeFileSync(path.join(directory, 'macos/test.app.tar.gz'), 'tar');
  fs.writeFileSync(path.join(directory, 'dmg/test.dmg'), 'dmg');
  let cleaned = false;
  let submitted = false;
  assert.throws(() => finalize(directory, env, (exe, args) => {
    if (exe === 'tar') fs.mkdirSync(path.join(args.at(-1), 'test.app'));
    if (exe === 'codesign' && args[0] === '--display') {
      return /[/\\]updater[/\\]/.test(args.at(-1)) ? validInfo.replace('TeamIdentifier=ABCDEFGHIJ', 'TeamIdentifier=WRONGTEAM1') : validInfo;
    }
    if (args.includes('submit')) submitted = true;
    return '';
  }, { installExitHandlers() {}, cleanup() { cleaned = true; } }), /Developer ID/);
  assert.equal(submitted, false);
  assert.equal(cleaned, true);
});
