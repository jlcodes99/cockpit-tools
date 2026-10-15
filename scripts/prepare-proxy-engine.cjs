// Explicit, offline package preparation. The caller supplies a verified archive;
// this script never downloads or executes the third-party engine.
const fs = require('node:fs');
const path = require('node:path');
const { createGunzip } = require('node:zlib');
const { pipeline } = require('node:stream/promises');
const { execFileSync } = require('node:child_process');
const { sha256File, verifyArchive } = require('./verify-proxy-engine-archive.cjs');
const manifest = require('../sidecars/mihomo/upstream-assets.json');

function required(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required; downloading proxy engines is intentionally disabled`);
  return value;
}

async function main() {
  const target = required('COCKPIT_PROXY_ENGINE_TARGET');
  const archive = path.resolve(required('COCKPIT_PROXY_ENGINE_ARCHIVE'));
  const output = path.resolve(process.env.COCKPIT_PROXY_ENGINE_OUTPUT || 'sidecars/mihomo/bin');
  const verified = await verifyArchive(target, archive);
  fs.rmSync(output, { recursive: true, force: true });
  fs.mkdirSync(output, { recursive: true });
  const extension = manifest.assets[target].file.endsWith('.zip') ? 'zip' : 'gz';
  if (extension === 'zip') {
    execFileSync('unzip', ['-q', archive, '-d', output], { stdio: 'inherit' });
  } else {
    await pipeline(fs.createReadStream(archive), createGunzip(), fs.createWriteStream(path.join(output, 'mihomo'), { mode: 0o700 }));
  }
  flattenPreparedEngine(output, target);
  const binaryName = target.includes('windows') ? 'mihomo.exe' : 'mihomo';
  const binary = path.join(output, binaryName);
  if (!fs.statSync(binary).isFile()) throw new Error('Verified archive did not contain the expected mihomo executable');
  if (!target.includes('windows')) fs.chmodSync(binary, 0o755);
  const files = [binaryName];
  const digests = {};
  for (const name of files) digests[name] = await sha256File(path.join(output, name));
  fs.writeFileSync(path.join(output, 'prepared.json'), `${JSON.stringify({
    target,
    version: manifest.version,
    archiveSha256: manifest.assets[target].sha256,
    files: digests,
  }, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({ ...verified, output, prepared: true, distributionApproved: false }));
}

function flattenPreparedEngine(output, target) {
  const binaryName = target.includes('windows') ? 'mihomo.exe' : 'mihomo';
  const required = [binaryName];
  const upstreamName = manifest.assets[target].file.replace(/-v[0-9.]+\.zip$/, '.exe');
  const located = new Map();
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else {
        const name = entry.name === upstreamName ? binaryName : entry.name;
        if (required.includes(name)) {
          if (located.has(name)) throw new Error(`Duplicate engine executable: ${name}`);
          located.set(name, full);
        }
      }
    }
  };
  walk(output);
  for (const name of required) {
    const source = located.get(name);
    if (!source) throw new Error(`Verified archive did not contain ${name}`);
    const dest = path.join(output, name);
    if (path.resolve(source) !== path.resolve(dest)) fs.renameSync(source, dest);
  }
  const keep = new Set(required);
  for (const entry of fs.readdirSync(output, { withFileTypes: true })) {
    const full = path.join(output, entry.name);
    if (entry.isDirectory() || !keep.has(entry.name)) fs.rmSync(full, { recursive: true, force: true });
  }
}
if (require.main === module) main().catch((error) => { console.error(error.message); process.exitCode = 1; });
module.exports = { main, flattenPreparedEngine };
