const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { sha256File, verifyArchive } = require('./verify-proxy-engine-archive.cjs');
const { buildBundleConfig } = require('./proxy-engine-bundle-config.cjs');
const manifest = require('../sidecars/mihomo/upstream-assets.json');

test('manifest pins v1.19.31 for six targets and withholds distribution approval', () => {
  assert.equal(manifest.version, '1.19.31');
  assert.equal(manifest.distributionApproved, false);
  assert.deepEqual(Object.keys(manifest.assets).sort(), [
    'aarch64-apple-darwin',
    'aarch64-pc-windows-msvc',
    'aarch64-unknown-linux-gnu',
    'x86_64-apple-darwin',
    'x86_64-pc-windows-msvc',
    'x86_64-unknown-linux-gnu',
  ]);
  for (const asset of Object.values(manifest.assets)) {
    assert.match(asset.file, /^mihomo-.*-v1\.19\.31\.(gz|zip)$/);
    assert.match(asset.sha256, /^[a-f0-9]{64}$/);
  }
});

test('rejects mismatched archives, directories, and unknown targets', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-proxy-engine-'));
  try {
    const file = path.join(dir, 'wrong.tar.gz');
    fs.writeFileSync(file, 'not-the-archive');
    await assert.rejects(
      () => verifyArchive('aarch64-apple-darwin', file),
      /SHA-256 mismatch/,
    );
    await assert.rejects(
      () => verifyArchive('not-a-target', file),
      /Unsupported proxy engine target/,
    );
    await assert.rejects(
      () => verifyArchive('aarch64-apple-darwin', dir),
      /regular archive file/,
    );
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('prepare flattening keeps the engine binary at the output root', () => {
  const { flattenPreparedEngine } = require('./prepare-proxy-engine.cjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-proxy-prepare-'));
  try {
    const nested = path.join(dir, 'package');
    fs.mkdirSync(nested, { recursive: true });
    fs.writeFileSync(path.join(nested, 'mihomo-windows-amd64-v1.exe'), 'exe');
    fs.writeFileSync(path.join(nested, 'libcronet.dll'), 'dll');
    flattenPreparedEngine(dir, 'x86_64-pc-windows-msvc');
    assert.equal(fs.readFileSync(path.join(dir, 'mihomo.exe'), 'utf8'), 'exe');
    assert.equal(fs.existsSync(path.join(dir, 'libcronet.dll')), false);
    assert.equal(fs.existsSync(nested), false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('packaging is blocked until distribution review is recorded', async () => {
  await assert.rejects(
    () => buildBundleConfig('aarch64-apple-darwin'),
    /distribution has not been approved/,
  );
});

test('approved offline packaging maps only verified engine resources', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-proxy-bundle-'));
  const approved = { ...manifest, distributionApproved: true };
  const target = 'x86_64-pc-windows-msvc';
  try {
    const files = {};
    for (const name of ['mihomo.exe']) {
      const file = path.join(dir, name);
      fs.writeFileSync(file, name);
      files[name] = await sha256File(file);
    }
    fs.writeFileSync(path.join(dir, 'prepared.json'), JSON.stringify({
      target,
      version: manifest.version,
      archiveSha256: manifest.assets[target].sha256,
      files,
    }));
    const config = await buildBundleConfig(target, { manifest: approved, preparedDir: dir });
    assert.deepEqual(Object.values(config.bundle.resources).sort(), [
      'proxy-engine/mihomo.exe',
    ]);
    assert.equal(Object.keys(config.bundle.resources).length, 1);
    await assert.rejects(
      () => buildBundleConfig('aarch64-pc-windows-msvc', { manifest: approved, preparedDir: dir }),
      /target or archive does not match/,
    );
    fs.writeFileSync(path.join(dir, 'mihomo.exe'), 'changed');
    await assert.rejects(
      () => buildBundleConfig(target, { manifest: approved, preparedDir: dir }),
      /missing or changed: mihomo.exe/,
    );
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});


test('prepare rejects multiple binaries that normalize to the same executable', () => {
  const { flattenPreparedEngine } = require('./prepare-proxy-engine.cjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-proxy-duplicates-'));
  try {
    fs.writeFileSync(path.join(dir, 'mihomo.exe'), 'one');
    fs.writeFileSync(path.join(dir, 'mihomo-windows-amd64-v1.exe'), 'two');
    assert.throws(() => flattenPreparedEngine(dir, 'x86_64-pc-windows-msvc'), /Duplicate engine executable/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
