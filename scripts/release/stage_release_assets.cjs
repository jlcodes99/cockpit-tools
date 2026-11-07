#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

const PLATFORM_PATTERNS = {
  windows: [
    /\.msi(?:\.sig)?$/i,
    /\.msi\.zip(?:\.sig)?$/i,
    /-setup\.exe(?:\.sig)?$/i,
    /-setup\.nsis\.zip(?:\.sig)?$/i,
    /[_-]x64-portable\.zip(?:\.sig)?$/i,
  ],
  macos: [/\.dmg$/i, /\.app\.tar\.gz(?:\.sig)?$/i],
  linux: [/\.AppImage(?:\.sig)?$/, /\.deb(?:\.sig)?$/i, /\.rpm(?:\.sig)?$/i],
};

const MAC_ARCHES = new Set(['aarch64', 'x64', 'universal']);

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

function listFilesRecursive(rootDir) {
  const files = [];
  for (const entry of fs.readdirSync(rootDir, { withFileTypes: true })) {
    const entryPath = path.join(rootDir, entry.name);
    if (entry.isDirectory()) {
      files.push(...listFilesRecursive(entryPath));
    } else if (entry.isFile()) {
      files.push(entryPath);
    }
  }
  return files;
}

function normalizeReleaseAssetName(fileName) {
  return fileName
    .trim()
    .replace(/\s+/g, '.')
    .replace(/[()[\]{}]/g, '.')
    .replace(/\.{2,}/g, '.')
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '');
}

function isAllowedReleaseAsset(platform, fileName) {
  const patterns = PLATFORM_PATTERNS[platform];
  if (!patterns) {
    throw new Error(`Unsupported release platform: ${platform}`);
  }
  return patterns.some((pattern) => pattern.test(fileName));
}

function buildStagedAssetName(platform, fileName, macArch) {
  if (platform !== 'macos' || !/\.app\.tar\.gz(?:\.sig)?$/i.test(fileName)) {
    return normalizeReleaseAssetName(fileName);
  }

  if (!MAC_ARCHES.has(macArch)) {
    throw new Error(`Unsupported or missing --mac-arch value: ${macArch || '<empty>'}`);
  }

  const signatureSuffix = fileName.toLowerCase().endsWith('.sig') ? '.sig' : '';
  const archiveSuffix = '.app.tar.gz';
  const baseLength = fileName.length - archiveSuffix.length - signatureSuffix.length;
  const baseName = fileName.slice(0, baseLength).replace(/_(?:aarch64|x64|universal)$/i, '');
  return normalizeReleaseAssetName(
    `${baseName}_${macArch}${archiveSuffix}${signatureSuffix}`,
  );
}

function stageReleaseAssets(options) {
  const { platform, assetsDir, outputDir, macArch } = options;
  const patterns = PLATFORM_PATTERNS[platform];
  if (!patterns) {
    throw new Error(`Unsupported release platform: ${platform}`);
  }
  if (platform === 'macos' && !MAC_ARCHES.has(macArch)) {
    throw new Error(`Unsupported or missing --mac-arch value: ${macArch || '<empty>'}`);
  }
  if (!fs.existsSync(assetsDir) || !fs.statSync(assetsDir).isDirectory()) {
    throw new Error(`Assets directory not found: ${assetsDir}`);
  }
  if (path.resolve(assetsDir) === path.resolve(outputDir)) {
    throw new Error('Output directory must differ from the source assets directory');
  }

  const stagedByName = new Map();
  for (const sourcePath of listFilesRecursive(assetsDir)) {
    const sourceName = path.basename(sourcePath);
    if (!isAllowedReleaseAsset(platform, sourceName)) continue;

    const stagedName = buildStagedAssetName(platform, sourceName, macArch);
    const existing = stagedByName.get(stagedName);
    if (existing && path.resolve(existing) !== path.resolve(sourcePath)) {
      throw new Error(
        `Release asset name collision for ${stagedName}: ${existing} and ${sourcePath}`,
      );
    }
    stagedByName.set(stagedName, sourcePath);
  }

  if (stagedByName.size === 0) {
    throw new Error(`No whitelisted ${platform} release assets found in ${assetsDir}`);
  }

  fs.rmSync(outputDir, { recursive: true, force: true });
  fs.mkdirSync(outputDir, { recursive: true });

  const outputs = [];
  for (const [stagedName, sourcePath] of [...stagedByName.entries()].sort(([a], [b]) =>
    a.localeCompare(b),
  )) {
    const outputPath = path.join(outputDir, stagedName);
    fs.copyFileSync(sourcePath, outputPath);
    outputs.push(outputPath);
  }
  return outputs;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const platform = requiredArg(args, 'platform');
  const outputs = stageReleaseAssets({
    platform,
    assetsDir: requiredArg(args, 'assets-dir'),
    outputDir: requiredArg(args, 'output-dir'),
    macArch: args['mac-arch'],
  });

  for (const output of outputs) {
    console.log(`Release asset staged at ${output}`);
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`[stage_release_assets] ${error.message}`);
    process.exit(1);
  }
}

module.exports = {
  buildStagedAssetName,
  isAllowedReleaseAsset,
  normalizeReleaseAssetName,
  stageReleaseAssets,
};
