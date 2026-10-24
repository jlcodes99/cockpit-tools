const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const zlib = require('node:zlib');

const { buildWindowsPortableArchive, collectPortableFiles } = require('./build_windows_portable.cjs');

test('portable file manifests separate native source paths from POSIX archive paths', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-portable-paths-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const executablePath = path.join(root, 'cockpit_tools.exe');
  const helperPath = path.join(root, 'resources', 'scripts', 'nested', 'helper.cjs');
  fs.mkdirSync(path.dirname(helperPath), { recursive: true });
  fs.writeFileSync(executablePath, 'exe');
  fs.writeFileSync(helperPath, 'nested helper');

  assert.deepEqual(collectPortableFiles({ releaseDir: root, executablePath }), [
    { sourcePath: executablePath, archivePath: 'cockpit_tools.exe' },
    { sourcePath: helperPath, archivePath: 'resources/scripts/nested/helper.cjs' },
  ]);

  const zip = fs.readFileSync(buildWindowsPortableArchive({
    releaseDir: root, outputDir: path.join(root, 'output'), version: '1.2.3',
  }));
  const entries = new Map();
  let offset = 0;
  while (zip.readUInt32LE(offset) === 0x04034b50) {
    const compressedSize = zip.readUInt32LE(offset + 18);
    const nameLength = zip.readUInt16LE(offset + 26);
    const extraLength = zip.readUInt16LE(offset + 28);
    const name = zip.subarray(offset + 30, offset + 30 + nameLength).toString('utf8');
    const dataStart = offset + 30 + nameLength + extraLength;
    entries.set(name, zlib.inflateRawSync(zip.subarray(dataStart, dataStart + compressedSize)).toString('utf8'));
    offset = dataStart + compressedSize;
  }
  assert.deepEqual([...entries.keys()], [
    'Cockpit.Tools_1.2.3_x64-portable/cockpit_tools.exe',
    'Cockpit.Tools_1.2.3_x64-portable/resources/scripts/nested/helper.cjs',
    'Cockpit.Tools_1.2.3_x64-portable/README.txt',
  ]);
  assert.equal(entries.get('Cockpit.Tools_1.2.3_x64-portable/resources/scripts/nested/helper.cjs'), 'nested helper');
});

test('builds a portable archive with the executable, resources and loader DLL', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-portable-'));
  const releaseDir = path.join(root, 'release');
  const outputDir = path.join(root, 'output');
  fs.mkdirSync(path.join(releaseDir, 'resources', 'scripts'), { recursive: true });
  fs.writeFileSync(path.join(releaseDir, 'cockpit_tools.exe'), 'exe');
  fs.writeFileSync(path.join(releaseDir, 'WebView2Loader.dll'), 'dll');
  fs.writeFileSync(path.join(releaseDir, 'resources', 'sidecar.exe'), 'sidecar');
  fs.writeFileSync(path.join(releaseDir, 'resources', 'scripts', 'helper.cjs'), 'helper');

  assert.deepEqual(
    collectPortableFiles({
      releaseDir,
      executablePath: path.join(releaseDir, 'cockpit_tools.exe'),
    }).map((file) => file.archivePath),
    ['cockpit_tools.exe', 'resources/scripts/helper.cjs', 'resources/sidecar.exe', 'WebView2Loader.dll'],
  );

  const archivePath = buildWindowsPortableArchive({
    releaseDir,
    outputDir,
    version: '1.2.3',
  });
  assert.equal(path.basename(archivePath), 'Cockpit.Tools_1.2.3_x64-portable.zip');
  const archive = fs.readFileSync(archivePath).toString('latin1');
  assert.match(archive, /Cockpit\.Tools_1\.2\.3_x64-portable\/cockpit_tools\.exe/);
  assert.match(archive, /Cockpit\.Tools_1\.2\.3_x64-portable\/resources\/sidecar\.exe/);
  assert.match(archive, /Cockpit\.Tools_1\.2\.3_x64-portable\/README\.txt/);
});
