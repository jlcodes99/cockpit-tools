// Explicit packaging step. The default app build never bundles the engine.
// Distribution must first be approved in the pinned upstream manifest.
const fs = require('node:fs');
const path = require('node:path');
const { sha256File } = require('./verify-proxy-engine-archive.cjs');
const upstream = require('../sidecars/mihomo/upstream-assets.json');

const repoRoot = path.resolve(__dirname, '..');
const preparedDir = path.join(repoRoot, 'sidecars', 'mihomo', 'bin');

async function buildBundleConfig(target, options = {}) {
  const manifest = options.manifest || upstream;
  const asset = manifest.assets[target];
  if (!asset) throw new Error('Unsupported proxy engine target');
  if (manifest.distributionApproved !== true) {
    throw new Error('Proxy engine distribution has not been approved');
  }
  const dir = options.preparedDir || preparedDir;
  const metadataPath = path.join(dir, 'prepared.json');
  if (!fs.existsSync(metadataPath) || !fs.lstatSync(metadataPath).isFile()) {
    throw new Error('Prepared proxy engine metadata is missing');
  }
  const metadata = JSON.parse(fs.readFileSync(metadataPath, 'utf8'));
  if (metadata.target !== target || metadata.version !== manifest.version ||
      metadata.archiveSha256 !== asset.sha256) {
    throw new Error('Prepared proxy engine target or archive does not match the pinned manifest');
  }
  const names = target.includes('windows') ? ['mihomo.exe'] : ['mihomo'];
  const resources = {};
  for (const name of names) {
    const source = path.join(dir, name);
    if (!fs.existsSync(source) || !fs.lstatSync(source).isFile() ||
        fs.lstatSync(source).isSymbolicLink() ||
        !metadata.files || metadata.files[name] !== await sha256File(source)) {
      throw new Error(`Prepared proxy engine file is missing or changed: ${name}`);
    }
    // Tauri resolves resource source paths from src-tauri/tauri.conf.json.
    resources[path.relative(path.join(repoRoot, 'src-tauri'), source).split(path.sep).join('/')] =
      `proxy-engine/${name}`;
  }
  return { bundle: { resources } };
}

async function main() {
  const [target, extra] = process.argv.slice(2);
  if (!target || extra) {
    throw new Error('Usage: node scripts/proxy-engine-bundle-config.cjs <rust-target>');
  }
  const config = await buildBundleConfig(target);
  const output = path.join(repoRoot, '.tmp', 'proxy-engine', `tauri.${target}.conf.json`);
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(config, null, 2)}\n`);
  console.log(output);
}

if (require.main === module) main().catch((error) => { console.error(error.message); process.exitCode = 1; });
module.exports = { buildBundleConfig };
