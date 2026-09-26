// Offline verification only: never download, execute, unpack or approve distribution.
const fs = require('node:fs');
const crypto = require('node:crypto');
const path = require('node:path');
const manifest = require('../sidecars/mihomo/upstream-assets.json');

async function sha256File(filePath) {
  const digest = crypto.createHash('sha256');
  for await (const chunk of fs.createReadStream(filePath)) digest.update(chunk);
  return digest.digest('hex');
}

async function verifyArchive(target, archivePath) {
  const asset = manifest.assets[target];
  if (!asset) throw new Error('Unsupported proxy engine target');
  const stat = await fs.promises.lstat(archivePath);
  if (!stat.isFile() || stat.isSymbolicLink()) throw new Error('Expected a regular archive file');
  if (await sha256File(archivePath) !== asset.sha256) throw new Error('Proxy engine archive SHA-256 mismatch');
  return { target, version: manifest.version, file: asset.file, verified: true, distributionApproved: false };
}

if (require.main === module) {
  const [target, archivePath, extra] = process.argv.slice(2);
  if (!target || !archivePath || extra) {
    console.error('Usage: node scripts/verify-proxy-engine-archive.cjs <rust-target> <archive-path>');
    process.exitCode = 1;
  } else {
    verifyArchive(target, path.resolve(archivePath)).then((result) => console.log(JSON.stringify(result)))
      .catch((error) => { console.error(error.message); process.exitCode = 1; });
  }
}
module.exports = { sha256File, verifyArchive };
