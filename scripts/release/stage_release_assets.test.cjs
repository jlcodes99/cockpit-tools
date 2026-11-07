const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  buildStagedAssetName,
  isAllowedReleaseAsset,
  normalizeReleaseAssetName,
  stageReleaseAssets,
} = require('./stage_release_assets.cjs');

function writeFile(filePath, content = 'asset') {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test('normalizes release asset names to the stable GitHub form', () => {
  assert.equal(
    normalizeReleaseAssetName('Cockpit Tools_1.2.3_x64-setup.exe'),
    'Cockpit.Tools_1.2.3_x64-setup.exe',
  );
  assert.equal(
    normalizeReleaseAssetName('Cockpit (Tools)  1.2.3.dmg'),
    'Cockpit.Tools.1.2.3.dmg',
  );
});

test('stages raw macOS updater archives with explicit architecture names', () => {
  assert.equal(
    buildStagedAssetName('macos', 'Cockpit Tools.app.tar.gz', 'aarch64'),
    'Cockpit.Tools_aarch64.app.tar.gz',
  );
  assert.equal(
    buildStagedAssetName('macos', 'Cockpit Tools.app.tar.gz.sig', 'x64'),
    'Cockpit.Tools_x64.app.tar.gz.sig',
  );
});

test('stages only whitelisted macOS release artifacts', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-stage-assets-'));
  const assetsDir = path.join(root, 'bundle');
  const outputDir = path.join(root, 'staged');

  writeFile(path.join(assetsDir, 'dmg', 'Cockpit Tools_1.2.3_aarch64.dmg'));
  writeFile(path.join(assetsDir, 'macos', 'Cockpit Tools.app.tar.gz'));
  writeFile(path.join(assetsDir, 'macos', 'Cockpit Tools.app.tar.gz.sig'), 'signature');
  writeFile(path.join(assetsDir, 'dmg', 'icon.icns'));
  writeFile(path.join(assetsDir, 'dmg', 'bundle_dmg.sh'));
  writeFile(path.join(assetsDir, 'share', 'create-dmg', 'support', 'template.applescript'));
  writeFile(path.join(assetsDir, 'macos', 'Cockpit Tools.app', 'Contents', 'Info.plist'));

  const outputs = stageReleaseAssets({
    platform: 'macos',
    assetsDir,
    outputDir,
    macArch: 'aarch64',
  });

  assert.deepEqual(
    outputs.map((filePath) => path.basename(filePath)),
    [
      'Cockpit.Tools_1.2.3_aarch64.dmg',
      'Cockpit.Tools_aarch64.app.tar.gz',
      'Cockpit.Tools_aarch64.app.tar.gz.sig',
    ],
  );
});

test('whitelist accepts release packages and rejects bundle helpers', () => {
  assert.equal(isAllowedReleaseAsset('windows', 'Cockpit Tools_1.2.3_x64_en-US.msi'), true);
  assert.equal(isAllowedReleaseAsset('windows', 'Cockpit.Tools_1.2.3_x64-portable.zip'), true);
  assert.equal(isAllowedReleaseAsset('windows', 'bundle.wxs'), false);
  assert.equal(isAllowedReleaseAsset('linux', 'Cockpit Tools_1.2.3_amd64.AppImage.sig'), true);
  assert.equal(isAllowedReleaseAsset('linux', 'AppRun'), false);
  assert.equal(isAllowedReleaseAsset('macos', 'Cockpit Tools.app.tar.gz.sig'), true);
  assert.equal(isAllowedReleaseAsset('macos', 'template.applescript'), false);
});
