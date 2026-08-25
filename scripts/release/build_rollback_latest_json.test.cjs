const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const { REQUIRED_TARGETS, buildRollbackLatestJson } = require('./build_rollback_latest_json.cjs');

const GOOD_VERSION = '1.3.28';
const REPO = 'jlcodes99/cockpit-tools';

function createGoodReleaseAssets(root, { withSignatures = true } = {}) {
  const assetsDir = path.join(root, 'rollback-assets');
  fs.mkdirSync(assetsDir, { recursive: true });

  const assetNames = [
    'Cockpit Tools_1.3.28_aarch64.app.tar.gz',
    'Cockpit Tools_1.3.28_x64.app.tar.gz',
    'Cockpit.Tools_1.3.28_x64_en-US.msi',
    'Cockpit.Tools_1.3.28_x64-setup.exe',
    'Cockpit.Tools_1.3.28_amd64.AppImage',
    'Cockpit.Tools_1.3.28_amd64.deb',
    'cockpit-tools-1.3.28-1.x86_64.rpm',
    'Cockpit.Tools_1.3.28_aarch64.AppImage',
    'Cockpit.Tools_1.3.28_arm64.deb',
    'cockpit-tools-1.3.28-1.aarch64.rpm',
  ];

  for (const name of assetNames) {
    fs.writeFileSync(path.join(assetsDir, name), `payload:${name}`);
    if (withSignatures) {
      fs.writeFileSync(path.join(assetsDir, `${name}.sig`), `sig:${name}`);
    }
  }

  // Noise that must be ignored.
  fs.writeFileSync(
    path.join(assetsDir, 'latest.json'),
    '{"version":"1.3.28","notes":"stale","pub_date":"2026-08-20T10:00:00.000Z","platforms":{}}',
  );
  fs.writeFileSync(
    path.join(assetsDir, 'SHA256SUMS.txt'),
    'noise\n',
  );

  return assetsDir;
}

test('builds a complete rollback manifest pointing at the good release', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-rollback-'));
  const assetsDir = createGoodReleaseAssets(root);
  const notesFile = path.join(root, 'rollback-notes.md');
  fs.writeFileSync(notesFile, 'Rolling back to 1.3.28\n');
  const output = path.join(root, 'out', 'rollback-latest.json');

  const result = buildRollbackLatestJson({
    version: GOOD_VERSION,
    repo: REPO,
    assetsDir,
    notesFile,
    publishedAt: '2026-08-20T10:00:00Z',
    output,
  });

  assert.equal(result.output, output);
  assert.equal(result.version, GOOD_VERSION);
  assert.equal(result.platformCount, 15);

  const manifest = JSON.parse(fs.readFileSync(output, 'utf8'));
  assert.equal(manifest.version, GOOD_VERSION);
  assert.equal(manifest.notes, 'Rolling back to 1.3.28');
  assert.equal(manifest.pub_date, '2026-08-20T10:00:00.000Z');

  // Every required target resolves to a real asset and its signature.
  for (const [target] of REQUIRED_TARGETS) {
    const entry = manifest.platforms[target];
    assert.ok(entry, `missing platform entry ${target}`);
    assert.match(entry.url, new RegExp(`releases/download/v${GOOD_VERSION}/`));
    assert.equal(entry.signature, `sig:${decodeURIComponent(entry.url.split('/').pop())}`);
  }

  // Alias keys share the canonical entry contents.
  assert.deepEqual(manifest.platforms['darwin-aarch64-app'], manifest.platforms['darwin-aarch64']);
  assert.deepEqual(manifest.platforms['windows-x86_64'], manifest.platforms['windows-x86_64-msi']);
  assert.deepEqual(manifest.platforms['linux-x86_64'], manifest.platforms['linux-x86_64-appimage']);

  // URLs point at the good version, never the broken one.
  for (const entry of Object.values(manifest.platforms)) {
    assert.ok(entry.url.includes(`/v${GOOD_VERSION}/`), entry.url);
  }
});

test('rejects an incomplete good-release asset set', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-rollback-'));
  const assetsDir = createGoodReleaseAssets(root);
  fs.unlinkSync(path.join(assetsDir, 'Cockpit.Tools_1.3.28_arm64.deb'));
  const notesFile = path.join(root, 'rollback-notes.md');
  fs.writeFileSync(notesFile, 'notes\n');

  assert.throws(
    () =>
      buildRollbackLatestJson({
        version: GOOD_VERSION,
        repo: REPO,
        assetsDir,
        notesFile,
        publishedAt: '2026-08-20T10:00:00Z',
        output: path.join(root, 'rollback-latest.json'),
      }),
    /Missing required rollback asset for linux-aarch64-deb/,
  );
});

test('rejects assets without signatures instead of emitting unsigned entries', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-rollback-'));
  const assetsDir = createGoodReleaseAssets(root, { withSignatures: false });
  const notesFile = path.join(root, 'rollback-notes.md');
  fs.writeFileSync(notesFile, 'notes\n');

  assert.throws(
    () =>
      buildRollbackLatestJson({
        version: GOOD_VERSION,
        repo: REPO,
        assetsDir,
        notesFile,
        publishedAt: '2026-08-20T10:00:00Z',
        output: path.join(root, 'rollback-latest.json'),
      }),
    /Missing signature file/,
  );
});

test('rejects an invalid pub date and missing inputs', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-rollback-'));
  const assetsDir = createGoodReleaseAssets(root);
  const notesFile = path.join(root, 'rollback-notes.md');
  fs.writeFileSync(notesFile, 'notes\n');
  const base = {
    version: GOOD_VERSION,
    repo: REPO,
    assetsDir,
    notesFile,
  };

  assert.throws(
    () => buildRollbackLatestJson({ ...base, publishedAt: 'not-a-date' }),
    /Invalid --published-at/,
  );
  assert.throws(
    () => buildRollbackLatestJson({ ...base, publishedAt: '2026-08-20T10:00:00Z', assetsDir: path.join(root, 'nope') }),
    /Assets directory not found/,
  );
  assert.throws(
    () => buildRollbackLatestJson({ ...base, publishedAt: '2026-08-20T10:00:00Z', notesFile: path.join(root, 'nope.md') }),
    /Notes file not found/,
  );
});
