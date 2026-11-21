#!/usr/bin/env node

const fs = require("fs");
const path = require("path");
const { normalizeReleaseAssetName } = require("./stage_release_assets.cjs");

const TARGET_SPECS = {
  "darwin-aarch64-app": /_aarch64\.app\.tar\.gz$/,
  "darwin-x86_64-app": /_x64\.app\.tar\.gz$/,
  "windows-x86_64-msi": /_x64_en-US\.msi$/,
  "windows-x86_64-nsis": /_x64-setup\.exe$/,
  "windows-aarch64-nsis": /_arm64-setup\.exe$/,
  "linux-x86_64-appimage": /_amd64\.AppImage$/,
  "linux-x86_64-deb": /_amd64\.deb$/,
  "linux-x86_64-rpm": /-1\.x86_64\.rpm$/,
  "linux-aarch64-appimage": /_aarch64\.AppImage$/,
  "linux-aarch64-deb": /_arm64\.deb$/,
  "linux-aarch64-rpm": /-1\.aarch64\.rpm$/,
};

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (!token.startsWith("--")) continue;
    const key = token.slice(2);
    const value = argv[i + 1];
    if (!value || value.startsWith("--")) {
      args[key] = "true";
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
  const timestamp = Date.parse((raw || "").trim());
  if (Number.isNaN(timestamp)) {
    throw new Error(`Invalid --published-at value: "${raw}"`);
  }
  return new Date(timestamp).toISOString();
}

function listFilesRecursive(rootDir) {
  const files = [];
  for (const entry of fs.readdirSync(rootDir, { withFileTypes: true })) {
    const entryPath = path.join(rootDir, entry.name);
    if (entry.isDirectory()) {
      files.push(...listFilesRecursive(entryPath));
    } else if (entry.isFile()) {
      files.push({
        name: entry.name,
        path: entryPath,
      });
    }
  }
  return files;
}

function buildUrl(repo, version, fileName) {
  return `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(fileName)}`;
}

function buildTargetManifests(options) {
  const {
    version,
    repo,
    assetsDir,
    notesFile,
    publishedAt,
    outputDir,
    targets,
  } = options;

  if (!fs.existsSync(assetsDir) || !fs.statSync(assetsDir).isDirectory()) {
    throw new Error(`Assets directory not found: ${assetsDir}`);
  }
  if (!fs.existsSync(notesFile)) {
    throw new Error(`Notes file not found: ${notesFile}`);
  }
  if (!Array.isArray(targets) || targets.length === 0) {
    throw new Error("At least one target is required");
  }

  const files = listFilesRecursive(assetsDir);
  const signatures = new Map();
  for (const file of files) {
    if (!file.name.endsWith(".sig")) continue;
    signatures.set(
      file.name.slice(0, -4),
      fs.readFileSync(file.path, "utf8").trim(),
    );
  }

  fs.mkdirSync(outputDir, { recursive: true });
  const notes = fs.readFileSync(notesFile, "utf8").trim();
  const normalizedPubDate = normalizePubDate(publishedAt);
  const outputs = [];

  for (const target of targets) {
    const pattern = TARGET_SPECS[target];
    if (!pattern) {
      throw new Error(`Unsupported updater target: ${target}`);
    }

    const asset = files.find(
      (file) => !file.name.endsWith(".sig") && pattern.test(file.name),
    );
    if (!asset) {
      throw new Error(
        `Missing updater asset for ${target}. Pattern: ${pattern}`,
      );
    }
    if (asset.name !== normalizeReleaseAssetName(asset.name)) {
      throw new Error(
        `Updater asset was not staged with a stable name: ${asset.name}`,
      );
    }

    const signature = signatures.get(asset.name);
    if (!signature) {
      throw new Error(`Missing signature file for asset ${asset.name}`);
    }

    const manifest = {
      version,
      notes,
      pub_date: normalizedPubDate,
      url: buildUrl(repo, version, asset.name),
      signature,
    };
    const output = path.join(outputDir, `latest-${target}.json`);
    fs.writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
    outputs.push(output);
  }

  return outputs;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const targets = requiredArg(args, "targets")
    .split(",")
    .map((target) => target.trim())
    .filter(Boolean);

  const outputs = buildTargetManifests({
    version: requiredArg(args, "version"),
    repo: requiredArg(args, "repo"),
    assetsDir: requiredArg(args, "assets-dir"),
    notesFile: requiredArg(args, "notes-file"),
    publishedAt: requiredArg(args, "published-at"),
    outputDir: args["output-dir"] || ".",
    targets,
  });

  for (const output of outputs) {
    console.log(`Target latest.json generated at ${output}`);
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`[build_target_latest_json] ${error.message}`);
    process.exit(1);
  }
}

module.exports = {
  TARGET_SPECS,
  buildTargetManifests,
};
