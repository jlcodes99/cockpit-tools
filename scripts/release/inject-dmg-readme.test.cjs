const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  buildFinderScript,
  describeFinderLayout,
  findTauriDmg,
  parseArgs,
} = require('./inject-dmg-readme.cjs');

function makeTempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'inject-dmg-readme-test-'));
}

function writeFakeDsStore(dsStorePath, markerCount) {
  const body = Buffer.concat([
    Buffer.from([0x00, 0x00, 0x00, 0x01, 0x42, 0x75, 0x64, 0x31]),
    Buffer.from('Iloc'.repeat(markerCount), 'latin1'),
  ]);
  fs.writeFileSync(dsStorePath, body);
  return body;
}

test('parses the bundle directory argument', () => {
  assert.deepEqual(parseArgs(['--bundle-dir', 'target/release/bundle']), {
    'bundle-dir': 'target/release/bundle',
  });
  assert.deepEqual(parseArgs(['--keep-work-dir']), { 'keep-work-dir': 'true' });
});

test('finds the single Tauri DMG and cleans up stale temporary images', () => {
  const bundleDir = makeTempDir();
  const dmgDir = path.join(bundleDir, 'dmg');
  fs.mkdirSync(dmgDir, { recursive: true });
  fs.writeFileSync(path.join(dmgDir, 'Cockpit Tools_1.2.3_aarch64.dmg'), 'dmg');
  fs.writeFileSync(path.join(dmgDir, 'Cockpit Tools 1.2.3.inject.dmg'), 'stale');

  const found = findTauriDmg(bundleDir);

  assert.equal(path.basename(found), 'Cockpit Tools_1.2.3_aarch64.dmg');
  assert.equal(fs.existsSync(path.join(dmgDir, 'Cockpit Tools 1.2.3.inject.dmg')), false);
});

test('returns nothing when the bundle has no DMG yet', () => {
  const bundleDir = makeTempDir();
  fs.mkdirSync(path.join(bundleDir, 'dmg'), { recursive: true });
  assert.equal(findTauriDmg(bundleDir), null);
});

test('rejects ambiguous bundle directories', () => {
  const bundleDir = makeTempDir();
  const dmgDir = path.join(bundleDir, 'dmg');
  fs.mkdirSync(dmgDir, { recursive: true });
  fs.writeFileSync(path.join(dmgDir, 'a.dmg'), 'dmg');
  fs.writeFileSync(path.join(dmgDir, 'b.dmg'), 'dmg');

  assert.throws(() => findTauriDmg(bundleDir), /expected one DMG/);
});

test('describes the Finder layout written by the rebuild step', () => {
  const workDir = makeTempDir();
  const dsStorePath = path.join(workDir, '.DS_Store');
  const body = writeFakeDsStore(dsStorePath, 3);

  const expected = crypto.createHash('sha1').update(body).digest('hex');
  assert.equal(describeFinderLayout(dsStorePath), `${body.length}:3:${expected}`);
});

test('rejects a layout file without icon positions', () => {
  const workDir = makeTempDir();
  const dsStorePath = path.join(workDir, '.DS_Store');
  writeFakeDsStore(dsStorePath, 0);

  assert.throws(() => describeFinderLayout(dsStorePath), /missing icon positions/);
});

test('positions all three DMG icons in the generated AppleScript', () => {
  const script = buildFinderScript({
    volumeName: 'Cockpit Tools 1.2.3',
    appName: 'Cockpit Tools.app',
    readmeName: '安装与常见问题.txt',
  });

  assert.match(script, /set theDisk to disk "Cockpit Tools 1\.2\.3"/);
  assert.match(script, /set position of item "Cockpit Tools\.app" to \{180, 150\}/);
  assert.match(script, /set position of item "安装与常见问题\.txt" to \{330, 330\}/);
  assert.match(script, /set position of item "Applications" to \{480, 150\}/);
  assert.match(script, /set icon size to 112/);
});
