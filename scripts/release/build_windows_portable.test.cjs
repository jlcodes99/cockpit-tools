const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const { buildWindowsPortableArchive, collectPortableFiles } = require('./build_windows_portable.cjs');

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
