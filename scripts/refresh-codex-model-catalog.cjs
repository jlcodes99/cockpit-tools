#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { randomUUID } = require("node:crypto");

const MAX_CATALOG_BYTES = 16 * 1024 * 1024;
const REASONING_EFFORTS = new Set(["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]);
const HELP = `Add selected missing models to an existing Codex model_catalog_json file.

Usage:
  node scripts/refresh-codex-model-catalog.cjs \\
    --catalog /absolute/path/to/models.json \\
    --base-url https://gateway.example.com/v1 \\
    --client-version 0.153.4 \\
    --model gpt-6.1-sol \\
    --api-key-env CODEX_GATEWAY_API_KEY [--dry-run]

Options:
  --model ID              Repeat to add more than one missing model.
  --dry-run               Show planned additions without writing any files.
  --allow-insecure-http   Allow non-loopback HTTP for a trusted private network.
  --help                  Show this help.

Requires Node.js 20 or newer. Existing entries are never updated or removed.
The tool does not edit config.toml, make inference requests, or restart Codex.
`;

function parseArgs(args) {
  const options = { modelIds: [], dryRun: false, allowInsecureHttp: false };
  const values = {
    "--catalog": "catalogPath",
    "--base-url": "baseUrl",
    "--client-version": "clientVersion",
    "--api-key-env": "apiKeyEnv",
  };
  const switches = { "--dry-run": "dryRun", "--allow-insecure-http": "allowInsecureHttp", "--help": "help" };
  const seen = new Set();
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (Object.hasOwn(switches, arg)) {
      if (seen.has(arg)) throw new Error(`Duplicate option: ${arg}`);
      seen.add(arg);
      options[switches[arg]] = true;
    } else if (Object.hasOwn(values, arg) || arg === "--model") {
      if (arg !== "--model" && seen.has(arg)) throw new Error(`Duplicate option: ${arg}`);
      seen.add(arg);
      const value = args[++i];
      if (!value || value.startsWith("--")) throw new Error(`Missing value for ${arg}`);
      if (arg === "--model") options.modelIds.push(value);
      else options[values[arg]] = value;
    } else {
      // Do not echo unknown arguments: they may contain a mistakenly supplied secret.
      throw new Error("Unknown option; use --help. API keys must be supplied through an environment variable.");
    }
  }
  if (options.help) return options;
  for (const [flag, key] of Object.entries(values)) {
    if (!options[key]) throw new Error(`Required option: ${flag}`);
  }
  if (!options.modelIds.length) throw new Error("At least one --model is required.");
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(options.apiKeyEnv)) throw new Error("Invalid API key environment variable name.");
  return options;
}

function versionParts(value) {
  if (typeof value !== "string" || !/^\d+\.\d+\.\d+$/.test(value)) {
    throw new Error("Client and minimum client versions must have the form X.Y.Z.");
  }
  return value.split(".").map(Number);
}

function isNewer(minimum, current) {
  const a = versionParts(minimum);
  const b = versionParts(current);
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return false;
}

function validateCatalog(catalog, source) {
  if (!catalog || typeof catalog !== "object" || Array.isArray(catalog) || !Array.isArray(catalog.models)) {
    throw new Error(`${source} must be a Codex catalog with a top-level models array, not an OpenAI data list.`);
  }
  const ids = new Set();
  for (const model of catalog.models) {
    if (!model || typeof model.slug !== "string" || !model.slug.trim() || model.slug !== model.slug.trim()) {
      throw new Error(`${source} contains an invalid model slug.`);
    }
    if (ids.has(model.slug)) throw new Error(`${source} contains duplicate model slugs.`);
    ids.add(model.slug);
  }
}

function validateNewModel(model, clientVersion) {
  const textFields = ["display_name", "description", "default_reasoning_level"];
  if (textFields.some(key => typeof model[key] !== "string" || !model[key].trim()) ||
      model.visibility !== "list" || !Number.isSafeInteger(model.context_window) || model.context_window <= 0 ||
      !Array.isArray(model.supported_reasoning_levels) || !model.supported_reasoning_levels.length ||
      model.supported_reasoning_levels.some(level => !level || !REASONING_EFFORTS.has(level.effort) || typeof level.description !== "string") ||
      new Set(model.supported_reasoning_levels.map(level => level.effort)).size !== model.supported_reasoning_levels.length ||
      !model.supported_reasoning_levels.some(level => level.effort === model.default_reasoning_level)) {
    throw new Error("A requested model is hidden or is missing required Codex metadata; no changes were made.");
  }
  if (model.minimal_client_version && isNewer(model.minimal_client_version, clientVersion)) {
    throw new Error("A requested model requires a newer Codex CLI; upgrade the client before refreshing its catalog.");
  }
}

function catalogUrl(baseUrl, clientVersion, allowInsecureHttp) {
  let url;
  try { url = new URL(baseUrl); } catch { throw new Error("Invalid API base URL."); }
  if (url.username || url.password || url.search || url.hash) throw new Error("API base URL must not contain credentials, a query, or a fragment.");
  if (!["http:", "https:"].includes(url.protocol)) throw new Error("API base URL must use HTTP or HTTPS.");
  const loopback = url.hostname === "localhost" || url.hostname === "[::1]" || /^127\.\d+\.\d+\.\d+$/.test(url.hostname);
  if (url.protocol === "http:" && !loopback && !allowInsecureHttp) {
    throw new Error("Non-loopback HTTP exposes the bearer credential. Use HTTPS or explicitly allow trusted private-network HTTP with --allow-insecure-http.");
  }
  url.pathname = url.pathname.replace(/\/$/, "") + "/models";
  url.searchParams.set("client_version", clientVersion);
  return url;
}

async function fetchCatalog(url, apiKey, timeoutMs) {
  let response;
  try {
    response = await fetch(url, {
      headers: { Authorization: `Bearer ${apiKey}`, Accept: "application/json" },
      redirect: "error", signal: AbortSignal.timeout(timeoutMs),
    });
  } catch {
    throw new Error("Catalog request failed or timed out; redirects are not followed. No changes were made.");
  }
  if (!response.ok) throw new Error(`Catalog request returned HTTP ${response.status}; no changes were made.`);
  const chunks = [];
  let size = 0;
  try {
    for await (const chunk of response.body) {
      size += chunk.byteLength;
      if (size > MAX_CATALOG_BYTES) throw new Error("Catalog size limit");
      chunks.push(Buffer.from(chunk));
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    // Never include the response body in errors: a gateway can reflect credentials.
    throw new Error("Catalog response is invalid JSON, incomplete, or too large; no changes were made.");
  }
}

function assertUnchanged(catalogPath, original, stat) {
  const currentStat = fs.lstatSync(catalogPath);
  if (!currentStat.isFile() || currentStat.dev !== stat.dev || currentStat.ino !== stat.ino ||
      currentStat.mode !== stat.mode || currentStat.uid !== stat.uid || currentStat.gid !== stat.gid ||
      !fs.readFileSync(catalogPath).equals(original)) {
    throw new Error("Catalog changed while refreshing; refusing to overwrite another writer's changes.");
  }
}

function writeExclusive(filePath, content, stat, onCreate = () => {}) {
  const mode = stat.mode & 0o777;
  const fd = fs.openSync(filePath, "wx", mode);
  try {
    onCreate(fs.fstatSync(fd));
    // Atomic replacement must not change who can read another user's catalog.
    if (typeof process.getuid === "function") fs.fchownSync(fd, stat.uid, stat.gid);
    // The umask must not silently tighten an existing catalog's permissions.
    fs.fchmodSync(fd, mode);
    fs.writeFileSync(fd, content);
    fs.fsyncSync(fd);
  } finally {
    fs.closeSync(fd);
  }
}

function removeOwnedFile(filePath, identity) {
  if (!identity) return;
  let current;
  try { current = fs.lstatSync(filePath); } catch (error) {
    if (error.code === "ENOENT") return;
    throw error;
  }
  if (!current.isFile() || current.dev !== identity.dev || current.ino !== identity.ino) {
    throw new Error("A refresh temporary or lock path changed; it was not removed.");
  }
  fs.unlinkSync(filePath);
}

async function refreshCatalog(options) {
  const { baseUrl, clientVersion, apiKey, dryRun = false, allowInsecureHttp = false, timeoutMs = 15000 } = options;
  versionParts(clientVersion);
  if (typeof apiKey !== "string" || !apiKey.trim() || /[\r\n]/.test(apiKey)) throw new Error("The API key environment variable is empty or invalid.");
  if (!Array.isArray(options.modelIds) || !options.modelIds.length || options.modelIds.some(id => typeof id !== "string" || !id.trim() || id !== id.trim())) {
    throw new Error("Specify at least one valid model ID.");
  }
  const url = catalogUrl(baseUrl, clientVersion, allowInsecureHttp);
  const catalogPath = path.resolve(options.catalogPath);
  const stat = fs.lstatSync(catalogPath);
  if (!stat.isFile()) throw new Error("Catalog must be an existing regular file, not a symlink or directory.");
  if (stat.size > MAX_CATALOG_BYTES) throw new Error("Local catalog is too large.");
  const original = fs.readFileSync(catalogPath);
  let catalog;
  try { catalog = JSON.parse(original.toString("utf8")); } catch { throw new Error("Local catalog is invalid JSON."); }
  validateCatalog(catalog, "Local catalog");
  const requested = [...new Set(options.modelIds)];
  const existing = new Set(catalog.models.map(model => model.slug));
  const result = { added: [], unchanged: requested.filter(id => existing.has(id)), backupPath: null, dryRun };
  const missing = requested.filter(id => !existing.has(id));
  if (!missing.length) return result;
  const remote = await fetchCatalog(url, apiKey, timeoutMs);
  validateCatalog(remote, "Gateway response");
  const additions = missing.map(id => {
    const model = remote.models.find(entry => entry.slug === id);
    if (!model) throw new Error("A requested model is not available in the gateway's Codex catalog; no changes were made.");
    validateNewModel(model, clientVersion);
    return model;
  });
  result.added = additions.map(model => model.slug);
  assertUnchanged(catalogPath, original, stat);
  if (dryRun) return result;

  const lockPath = catalogPath + ".refresh.lock";
  let lock;
  try { lock = fs.openSync(lockPath, "wx", 0o600); } catch (error) {
    if (error.code === "EEXIST") throw new Error("Another refresh holds the catalog lock; no changes were made.");
    throw error;
  }
  const suffix = new Date().toISOString().replace(/[:.]/g, "-") + "-" + randomUUID();
  const temporary = catalogPath + ".refresh-" + suffix + ".tmp";
  const backup = catalogPath + ".bak-" + suffix;
  const lockIdentity = fs.fstatSync(lock);
  let temporaryIdentity;
  let writeError;
  const cleanupErrors = [];
  try {
    assertUnchanged(catalogPath, original, stat);
    const merged = { ...catalog, models: [...catalog.models, ...additions] };
    writeExclusive(temporary, JSON.stringify(merged, null, 2) + "\n", stat, identity => { temporaryIdentity = identity; });
    writeExclusive(backup, original, stat);
    assertUnchanged(catalogPath, original, stat);
    fs.renameSync(temporary, catalogPath);
    result.backupPath = backup;
  } catch (error) {
    writeError = error;
  } finally {
    try { removeOwnedFile(temporary, temporaryIdentity); } catch (error) { cleanupErrors.push(error); }
    try { fs.closeSync(lock); } catch (error) { cleanupErrors.push(error); }
    try { removeOwnedFile(lockPath, lockIdentity); } catch (error) { cleanupErrors.push(error); }
  }
  if (writeError) throw writeError;
  if (cleanupErrors.length) result.warnings = ["Catalog was updated, but temporary/lock cleanup was incomplete. Inspect these paths before refreshing again."];
  return result;
}

async function main() {
  let secret;
  try {
    const options = parseArgs(process.argv.slice(2));
    if (options.help) { console.log(HELP); return; }
    secret = process.env[options.apiKeyEnv];
    const result = await refreshCatalog({ ...options, apiKey: secret });
    console.log(JSON.stringify(result, null, 2));
    if (result.added.length && !result.dryRun) console.log("Catalog updated. Wait for active tasks to finish, then restart/resume Codex to load it.");
  } catch (error) {
    const message = secret ? String(error.message).split(secret).join("[REDACTED]") : error.message;
    console.error(`Error: ${message}`);
    process.exitCode = 1;
  }
}

module.exports = { refreshCatalog, parseArgs };
if (require.main === module) main();
