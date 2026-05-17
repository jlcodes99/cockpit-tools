#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (!token.startsWith('--')) continue;
    const key = token.slice(2);
    const value = argv[i + 1];
    if (!value || value.startsWith('--')) {
      args[key] = 'true';
      continue;
    }
    args[key] = value;
    i += 1;
  }
  return args;
}

function requiredArg(args, key) {
  const value = args[key];
  if (!value) {
    throw new Error(`Missing required argument --${key}`);
  }
  return value;
}

function normalizePubDate(raw) {
  const value = (raw || '').trim();
  if (!value || value === 'true' || value === 'null') {
    throw new Error(`Invalid --published-at value: "${raw}"`);
  }
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) {
    throw new Error(`Invalid --published-at value: "${raw}"`);
  }
  return new Date(timestamp).toISOString();
}

function readText(filePath) {
  return fs.readFileSync(filePath, 'utf8').trim();
}

function buildUrl(repo, version, fileName) {
  const encoded = encodeURIComponent(fileName);
  return `https://github.com/${repo}/releases/download/v${version}/${encoded}`;
}

function findAsset(assets, pattern, label) {
  const hit = assets.find((name) => pattern.test(name));
  if (!hit) {
    throw new Error(`Missing required updater asset for ${label}. Pattern: ${pattern}`);
  }
  return hit;
}

function buildPlatformEntry(assetName, signatures, repo, version) {
  const signature = signatures.get(assetName);
  if (!signature) {
    throw new Error(`Missing signature file for asset ${assetName}`);
  }
  return {
    signature,
    url: buildUrl(repo, version, assetName),
  };
}

function cloneEntry(entry) {
  return { signature: entry.signature, url: entry.url };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const version = requiredArg(args, 'version');
  const repo = requiredArg(args, 'repo');
  const assetsDir = requiredArg(args, 'assets-dir');
  const notesFile = requiredArg(args, 'notes-file');
  const publishedAt = normalizePubDate(requiredArg(args, 'published-at'));
  const output = args.output || 'latest.json';

  if (!fs.existsSync(assetsDir) || !fs.statSync(assetsDir).isDirectory()) {
    throw new Error(`Assets directory not found: ${assetsDir}`);
  }
  if (!fs.existsSync(notesFile)) {
    throw new Error(`Notes file not found: ${notesFile}`);
  }

  const files = fs
    .readdirSync(assetsDir)
    .filter((name) => fs.statSync(path.join(assetsDir, name)).isFile());

  const signatures = new Map();
  for (const name of files) {
    if (!name.endsWith('.sig')) continue;
    const assetName = name.slice(0, -4);
    signatures.set(assetName, readText(path.join(assetsDir, name)));
  }

  const assets = files.filter(
    (name) => !name.endsWith('.sig') && name !== 'latest.json' && name !== 'SHA256SUMS.txt'
  );

  const darwinAarch64Tar = findAsset(assets, /_aarch64\.app\.tar\.gz$/, 'darwin-aarch64');
  const darwinX64Tar = findAsset(assets, /_x64\.app\.tar\.gz$/, 'darwin-x86_64');
  const windowsMsi = findAsset(assets, /_x64_en-US\.msi$/, 'windows-x86_64-msi');
  const windowsNsis = findAsset(assets, /_x64-setup\.exe$/, 'windows-x86_64-nsis');
  const linuxX64AppImage = findAsset(assets, /_amd64\.AppImage$/, 'linux-x86_64-appimage');
  const linuxArmAppImage = findAsset(assets, /_aarch64\.AppImage$/, 'linux-aarch64-appimage');
  const linuxX64Deb = findAsset(assets, /_amd64\.deb$/, 'linux-x86_64-deb');
  const linuxArmDeb = findAsset(assets, /_arm64\.deb$/, 'linux-aarch64-deb');
  const linuxX64Rpm = findAsset(assets, /-1\.x86_64\.rpm$/, 'linux-x86_64-rpm');
  const linuxArmRpm = findAsset(assets, /-1\.aarch64\.rpm$/, 'linux-aarch64-rpm');

  const darwinAarch64Entry = buildPlatformEntry(darwinAarch64Tar, signatures, repo, version);
  const darwinX64Entry = buildPlatformEntry(darwinX64Tar, signatures, repo, version);
  const windowsMsiEntry = buildPlatformEntry(windowsMsi, signatures, repo, version);
  const windowsNsisEntry = buildPlatformEntry(windowsNsis, signatures, repo, version);
  const linuxX64AppImageEntry = buildPlatformEntry(linuxX64AppImage, signatures, repo, version);
  const linuxArmAppImageEntry = buildPlatformEntry(linuxArmAppImage, signatures, repo, version);
  const linuxX64DebEntry = buildPlatformEntry(linuxX64Deb, signatures, repo, version);
  const linuxArmDebEntry = buildPlatformEntry(linuxArmDeb, signatures, repo, version);
  const linuxX64RpmEntry = buildPlatformEntry(linuxX64Rpm, signatures, repo, version);
  const linuxArmRpmEntry = buildPlatformEntry(linuxArmRpm, signatures, repo, version);

  const latest = {
    version,
    notes: readText(notesFile),
    pub_date: publishedAt,
    platforms: {
      'darwin-aarch64': darwinAarch64Entry,
      'darwin-aarch64-app': cloneEntry(darwinAarch64Entry),
      'darwin-x86_64': darwinX64Entry,
      'darwin-x86_64-app': cloneEntry(darwinX64Entry),
      // Keep Windows fallback aligned to NSIS so updater fallback does not switch installer type.
      'windows-x86_64': windowsNsisEntry,
      'windows-x86_64-msi': cloneEntry(windowsMsiEntry),
      'windows-x86_64-nsis': windowsNsisEntry,
      'linux-x86_64': linuxX64AppImageEntry,
      'linux-x86_64-appimage': cloneEntry(linuxX64AppImageEntry),
      'linux-x86_64-deb': linuxX64DebEntry,
      'linux-x86_64-rpm': linuxX64RpmEntry,
      'linux-aarch64': linuxArmAppImageEntry,
      'linux-aarch64-appimage': cloneEntry(linuxArmAppImageEntry),
      'linux-aarch64-deb': linuxArmDebEntry,
      'linux-aarch64-rpm': linuxArmRpmEntry,
    },
  };

  fs.writeFileSync(output, `${JSON.stringify(latest, null, 2)}\n`);
  console.log(`Merged latest.json generated at ${output}`);
  console.log(`platform count=${Object.keys(latest.platforms).length}`);
}

try {
  main();
} catch (error) {
  console.error(`[build_merged_latest_json] ${error.message}`);
  process.exit(1);
}
