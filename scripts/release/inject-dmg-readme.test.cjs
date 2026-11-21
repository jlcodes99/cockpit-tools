const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  buildFinderScript,
  describeFinderLayout,
  resolveDmgPaths,
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

test('uses the signed app directly and preserves installer filenames for every architecture', (context) => {
  const bundleDir = makeTempDir();
  context.after(() => fs.rmSync(bundleDir, { recursive: true, force: true }));
  const appBundlePath = path.join(bundleDir, 'macos', 'Cockpit Tools.app');
  fs.mkdirSync(appBundlePath, { recursive: true });

  for (const macArch of ['aarch64', 'x64', 'universal']) {
    assert.deepEqual(resolveDmgPaths({ bundleDir, productName: 'Cockpit Tools', version: '1.2.3', macArch }), {
      appName: 'Cockpit Tools.app',
      appBundlePath,
      outputPath: path.join(bundleDir, 'dmg', `Cockpit Tools_1.2.3_${macArch}.dmg`),
    });
  }
});

test('fails when no app is available rather than using an existing DMG', (context) => {
  const bundleDir = makeTempDir();
  context.after(() => fs.rmSync(bundleDir, { recursive: true, force: true }));
  const dmgDir = path.join(bundleDir, 'dmg');
  fs.mkdirSync(dmgDir, { recursive: true });
  fs.writeFileSync(path.join(dmgDir, 'Cockpit Tools_1.2.3_aarch64.dmg'), 'old build');
  assert.throws(() => resolveDmgPaths({
    bundleDir, productName: 'Cockpit Tools', version: '1.2.3', macArch: 'aarch64',
  }), /Missing built \.app bundle/);
});

test('describes the Finder layout written during packaging', () => {
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
