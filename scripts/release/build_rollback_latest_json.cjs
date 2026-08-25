#!/usr/bin/env node
// Build a rollback legacy latest.json for the updater WITHOUT publishing anything.
//
// Purpose: when a released build is broken (bad update, signing regression,
// corrupted upload), point the live updater back to the last known-good
// release by re-uploading that release's complete latest.json as the current
// release's manifest. This script only WRITES A FILE locally; it never talks
// to GitHub, never publishes, and never mutates the repository.
//
// Inputs: the full set of assets of the good release (e.g. downloaded with
// `gh release download <good-tag> --dir rollback-assets`), plus --version,
// --repo, notes and pub-date for the good release. Output layout matches
// build_merged_latest_json.cjs exactly so the updater accepts it unchanged.
//
// Usage:
//   node scripts/release/build_rollback_latest_json.cjs \
//     --version 1.3.28 \
//     --repo jlcodes99/cockpit-tools \
//     --assets-dir ./rollback-assets \
//     --notes-file ./rollback-notes.md \
//     --published-at 2026-08-20T10:00:00Z \
//     [--output ./rollback-latest.json]
//
// Apply (manual, out of scope here):
//   gh release upload <current-tag> rollback-latest.json --clobber (renamed latest.json)

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
  const timestamp = Date.parse((raw || '').trim());
  if (Number.isNaN(timestamp)) {
    throw new Error(`Invalid --published-at value: "${raw}"`);
  }
  return new Date(timestamp).toISOString();
}

function readText(filePath) {
  return fs.readFileSync(filePath, 'utf8').trim();
}

function buildUrl(repo, version, fileName) {
  return `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(fileName)}`;
}

function findAsset(assets, pattern, label) {
  const hit = assets.find((name) => pattern.test(name));
  if (!hit) {
    throw new Error(`Missing required rollback asset for ${label}. Pattern: ${pattern}`);
  }
  return hit;
}

function buildPlatformEntry(assetName, signatures, repo, version) {
  const signature = signatures.get(assetName);
  if (!signature) {
    throw new Error(`Missing signature file for asset ${assetName}`);
  }
  return { signature, url: buildUrl(repo, version, assetName) };
}

function cloneEntry(entry) {
  return { signature: entry.signature, url: entry.url };
}

const REQUIRED_TARGETS = [
  ['darwin-aarch64', /_aarch64\.app\.tar\.gz$/],
  ['darwin-x86_64', /_x64\.app\.tar\.gz$/],
  ['windows-x86_64-msi', /_x64_en-US\.msi$/],
  ['windows-x86_64-nsis', /_x64-setup\.exe$/],
  ['linux-x86_64-appimage', /_amd64\.AppImage$/],
  ['linux-x86_64-deb', /_amd64\.deb$/],
  ['linux-x86_64-rpm', /-1\.x86_64\.rpm$/],
  ['linux-aarch64-appimage', /_aarch64\.AppImage$/],
  ['linux-aarch64-deb', /_arm64\.deb$/],
  ['linux-aarch64-rpm', /-1\.aarch64\.rpm$/],
];

function buildRollbackLatestJson(options) {
  const { version, repo, assetsDir, notesFile, publishedAt } = options;
  let output = options.output;
  if (!output || output === 'true') {
    output = 'latest.json';
  }

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
    signatures.set(name.slice(0, -4), readText(path.join(assetsDir, name)));
  }

  const assets = files.filter(
    (name) => !name.endsWith('.sig') && name !== 'latest.json' && name !== 'SHA256SUMS.txt',
  );

  const resolved = {};
  const platformEntries = {};
  for (const [target, pattern] of REQUIRED_TARGETS) {
    const assetName = findAsset(assets, pattern, target);
    resolved[target] = assetName;
    platformEntries[target] = buildPlatformEntry(assetName, signatures, repo, version);
  }

  const latest = {
    version,
    notes: readText(notesFile),
    pub_date: normalizePubDate(publishedAt),
    platforms: {
      'darwin-aarch64': cloneEntry(platformEntries['darwin-aarch64']),
      'darwin-aarch64-app': cloneEntry(platformEntries['darwin-aarch64']),
      'darwin-x86_64': cloneEntry(platformEntries['darwin-x86_64']),
      'darwin-x86_64-app': cloneEntry(platformEntries['darwin-x86_64']),
      // Unknown/generic Windows target prefers MSI (see #1320).
      'windows-x86_64': cloneEntry(platformEntries['windows-x86_64-msi']),
      'windows-x86_64-msi': cloneEntry(platformEntries['windows-x86_64-msi']),
      'windows-x86_64-nsis': cloneEntry(platformEntries['windows-x86_64-nsis']),
      'linux-x86_64': cloneEntry(platformEntries['linux-x86_64-appimage']),
      'linux-x86_64-appimage': cloneEntry(platformEntries['linux-x86_64-appimage']),
      'linux-x86_64-deb': cloneEntry(platformEntries['linux-x86_64-deb']),
      'linux-x86_64-rpm': cloneEntry(platformEntries['linux-x86_64-rpm']),
      'linux-aarch64': cloneEntry(platformEntries['linux-aarch64-appimage']),
      'linux-aarch64-appimage': cloneEntry(platformEntries['linux-aarch64-appimage']),
      'linux-aarch64-deb': cloneEntry(platformEntries['linux-aarch64-deb']),
      'linux-aarch64-rpm': cloneEntry(platformEntries['linux-aarch64-rpm']),
    },
  };

  fs.mkdirSync(path.dirname(path.resolve(output)), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(latest, null, 2)}\n`);
  return {
    output,
    version,
    platformCount: Object.keys(latest.platforms).length,
    assetNames: resolved,
  };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const result = buildRollbackLatestJson({
    version: requiredArg(args, 'version'),
    repo: requiredArg(args, 'repo'),
    assetsDir: requiredArg(args, 'assets-dir'),
    notesFile: requiredArg(args, 'notes-file'),
    publishedAt: requiredArg(args, 'published-at'),
    output: args.output,
  });
  console.log(
    `Rollback latest.json generated at ${result.output} (points at v${result.version}, ${result.platformCount} platforms)`,
  );
  for (const [target, name] of Object.entries(result.assetNames)) {
    console.log(`  ${target}: ${name}`);
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`[build_rollback_latest_json] ${error.message}`);
    process.exit(1);
  }
}

module.exports = { REQUIRED_TARGETS, buildRollbackLatestJson };
