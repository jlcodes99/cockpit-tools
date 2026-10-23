#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { setTimeout: sleep } = require("node:timers/promises");

const MAX_ATTEMPTS = 5;
const RETRY_DELAY_MS = 2000;

function commandFailure(result) {
  return [result.stderr, result.stdout, result.error?.message]
    .filter(Boolean)
    .join("\n")
    .trim() || `gh exited with status ${result.status} (${result.signal || "no signal"})`;
}

function isRetryableFailure(result) {
  const message = commandFailure(result);
  // Explicit HTTP errors take precedence over incidental words in a response.
  const httpStatus = /\bHTTP\s+(\d{3})\b/i.exec(message);
  if (httpStatus) {
    const status = Number(httpStatus[1]);
    return status === 429 || (status >= 500 && status <= 599);
  }
  return ["ETIMEDOUT", "ECONNRESET", "ECONNREFUSED", "EAI_AGAIN"].includes(result.error?.code)
    || /\b(?:unexpected EOF|EOF|i\/o timeout|TLS handshake timeout|connection reset by peer|socket hang up|context deadline exceeded|temporary failure in name resolution)\b/i.test(message);
}

function runGh(args) {
  return spawnSync("gh", args, {
    encoding: "utf8",
    shell: false,
    timeout: args[1] === "upload" ? 10 * 60 * 1000 : 60 * 1000,
    maxBuffer: 1024 * 1024,
    windowsHide: true,
  });
}

async function uploadReleaseAssets(tag, files, options = {}) {
  const run = options.run || runGh;
  const wait = options.wait || sleep;
  const log = options.log || console.log;
  const maxAttempts = options.maxAttempts ?? MAX_ATTEMPTS;
  const delayMs = options.delayMs ?? RETRY_DELAY_MS;
  if (!/^v\d+\.\d+\.\d+$/.test(tag) || !files.length) {
    throw new Error("Usage: upload_release_assets.cjs v<major>.<minor>.<patch> <asset> [...assets]");
  }
  if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > MAX_ATTEMPTS || !Number.isFinite(delayMs) || delayMs < 0) {
    throw new Error("Invalid upload retry limits");
  }
  const assets = files.map((file) => path.resolve(file));
  const names = new Set();
  for (const asset of assets) {
    if (!fs.statSync(asset).isFile()) throw new Error(`Release asset is not a file: ${asset}`);
    const name = path.basename(asset);
    if (names.has(name)) throw new Error(`Duplicate release asset name: ${name}`);
    names.add(name);
  }

  // Upload one asset at a time: retrying a failed batch must not replace assets
  // that already succeeded. --clobber repairs an ambiguous/partial failed upload.
  for (const asset of assets) {
    for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
      let result = run(["release", "view", tag, "--json", "isDraft", "--jq", ".isDraft"]);
      let phase = "release lookup";
      if (result.status === 0 && !result.error) {
        if (result.stdout.trim() !== "true") {
          throw new Error(`Release ${tag} is not a draft; refusing to mutate a published release.`);
        }
        phase = "asset upload";
        result = run(["release", "upload", tag, "--clobber", "--", asset]);
        if (result.status === 0 && !result.error) {
          log(`Uploaded ${path.basename(asset)} to ${tag}`);
          break;
        }
      }

      const detail = commandFailure(result);
      if (!isRetryableFailure(result) || attempt === maxAttempts) {
        throw new Error(`${phase} failed for ${path.basename(asset)} (${attempt}/${maxAttempts}): ${detail}`);
      }
      const backoff = delayMs * (2 ** (attempt - 1));
      log(`${phase} failed for ${path.basename(asset)} (${attempt}/${maxAttempts}): ${detail}. Retrying in ${backoff}ms.`);
      await wait(backoff);
      // Check draft state again before every mutation, including retries.
    }
  }
}

if (require.main === module) {
  uploadReleaseAssets(process.argv[2] || "", process.argv.slice(3)).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

module.exports = { uploadReleaseAssets, isRetryableFailure };
