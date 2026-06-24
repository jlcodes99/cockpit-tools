#!/usr/bin/env node

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { execFileSync } = require('node:child_process');

const repoRoot = path.resolve(__dirname, '../..');
const scriptPath = path.join(__dirname, 'build_merged_latest_json.cjs');

function writeAsset(assetsDir, name) {
  fs.writeFileSync(path.join(assetsDir, name), 'asset');
  fs.writeFileSync(path.join(assetsDir, `${name}.sig`), `${name}.sig`);
}

test('windows fallback points to MSI while explicit NSIS target stays available', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-latest-json-'));
  const assetsDir = path.join(root, 'assets');
  const notesFile = path.join(root, 'notes.md');
  const outputFile = path.join(root, 'latest.json');
  fs.mkdirSync(assetsDir);
  fs.writeFileSync(notesFile, 'release notes');

  [
    'Cockpit.Tools_aarch64.app.tar.gz',
    'Cockpit.Tools_x64.app.tar.gz',
    'Cockpit.Tools_1.2.3_x64_en-US.msi',
    'Cockpit.Tools_1.2.3_x64-setup.exe',
    'Cockpit.Tools_1.2.3_amd64.AppImage',
    'Cockpit.Tools_1.2.3_aarch64.AppImage',
    'Cockpit.Tools_1.2.3_amd64.deb',
    'Cockpit.Tools_1.2.3_arm64.deb',
    'Cockpit.Tools-1.2.3-1.x86_64.rpm',
    'Cockpit.Tools-1.2.3-1.aarch64.rpm',
  ].forEach((name) => writeAsset(assetsDir, name));

  execFileSync(
    process.execPath,
    [
      scriptPath,
      '--version',
      '1.2.3',
      '--repo',
      'jlcodes99/cockpit-tools',
      '--assets-dir',
      assetsDir,
      '--notes-file',
      notesFile,
      '--published-at',
      '2026-06-24T00:00:00Z',
      '--output',
      outputFile,
    ],
    { cwd: repoRoot, stdio: 'pipe' }
  );

  const latest = JSON.parse(fs.readFileSync(outputFile, 'utf8'));
  assert.match(latest.platforms['windows-x86_64'].url, /_x64_en-US\.msi$/);
  assert.match(latest.platforms['windows-x86_64-msi'].url, /_x64_en-US\.msi$/);
  assert.match(latest.platforms['windows-x86_64-nsis'].url, /_x64-setup\.exe$/);

  fs.rmSync(root, { recursive: true, force: true });
});
