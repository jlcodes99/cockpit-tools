'use strict';

/**
 * verify_updater_signatures.cjs
 *
 * Real (non-empty only is NOT enough) signature verification for a Tauri
 * updater build directory. Every `*.sig` file found (recursively) is matched
 * to its sibling asset (the same path without the `.sig` suffix) and verified
 * against the Ed25519 public key declared in `plugins.updater.pubkey` of the
 * Tauri config.
 *
 * Both the public key and the signature files follow the minisign text format
 * wrapped in base64:
 *
 *   public key file (2 lines):
 *     untrusted comment: ...
 *     <base64( "Ed"(2) + keyId(8) + ed25519RawPubKey(32) )>      // 42 bytes
 *
 *   signature file (4 lines):
 *     untrusted comment: ...
 *     <base64( algo(2 "Ed"|"ED") + keyId(8) + fileSignature(64) )>   // 74 bytes
 *     trusted comment: <comment text>
 *     <base64( globalSignature(64) )>                            // 64 bytes
 *
 *   - "Ed": file signature is over the raw asset bytes.
 *   - "ED": file signature is over blake2b512(asset) (prehashed).
 *   - The global signature is ALWAYS over (fileSignature || trustedComment utf8)
 *     using plain Ed25519, and protects the trusted comment against tampering.
 *
 * Only Node's built-in `crypto` is used. Nothing is loaded into memory that is
 * not needed: prehashed (ED) assets are hashed through a streaming read so that
 * arbitrarily large installers are supported.
 */

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

// 12-byte SPKI DER header for a raw 32-byte Ed25519 public key.
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

const MINISIGN_TRUSTED_PREFIX = 'trusted comment: ';
const PUBKEY_BLOB_LEN = 42; // "Ed"(2) + keyId(8) + pubkey(32)
const SIG_BLOB_LEN = 74; // algo(2) + keyId(8) + fileSignature(64)
const SIG_LEN = 64;
const KEYID_LEN = 8;

/** Wrap a raw 32-byte Ed25519 public key into an SPKI DER Buffer. */
function rawEd25519ToSpki(rawKey) {
  if (!Buffer.isBuffer(rawKey) || rawKey.length !== 32) {
    throw new Error('invalid raw Ed25519 public key: expected 32 bytes');
  }
  return Buffer.concat([ED25519_SPKI_PREFIX, rawKey]);
}

/** Extract the raw 32-byte public key from an SPKI DER Buffer. */
function spkiToRawEd25519(der) {
  if (!Buffer.isBuffer(der) || der.length < 32) {
    throw new Error('invalid SPKI DER: too short');
  }
  return der.subarray(der.length - 32);
}

/** Build a Node Ed25519 CryptoKeyObject from a raw 32-byte key. */
function createEd25519PublicKey(rawKey) {
  return crypto.createPublicKey({
    key: rawEd25519ToSpki(rawKey),
    format: 'der',
    type: 'spki',
  });
}

/**
 * Parse a minisign public key text block.
 * @returns {{ keyId: Buffer, rawKey: Buffer, publicKey: crypto.KeyObject, algorithm: string }}
 */
function parsePublicKey(text) {
  const lines = String(text)
    .split(/\r?\n/)
    .filter((l) => l.trim().length > 0);
  if (lines.length < 2) {
    throw new Error('malformed public key: expected at least 2 lines');
  }
  let blob;
  try {
    blob = Buffer.from(lines[1].trim(), 'base64');
  } catch (e) {
    throw new Error('malformed public key: second line is not valid base64');
  }
  if (blob.length !== PUBKEY_BLOB_LEN) {
    throw new Error(
      `malformed public key blob: expected ${PUBKEY_BLOB_LEN} bytes, got ${blob.length}`
    );
  }
  const algorithm = blob.subarray(0, 2).toString('latin1');
  if (algorithm !== 'Ed') {
    throw new Error(`unsupported public key algorithm: ${JSON.stringify(algorithm)}`);
  }
  const keyId = blob.subarray(2, 2 + KEYID_LEN);
  const rawKey = blob.subarray(2 + KEYID_LEN);
  return {
    keyId,
    rawKey,
    publicKey: createEd25519PublicKey(rawKey),
    algorithm,
  };
}

/**
 * Parse a minisign signature text block.
 * @returns {{ algorithm: string, keyId: Buffer, fileSignature: Buffer, trustedComment: string, globalSignature: Buffer }}
 */
function parseSignature(text) {
  const lines = decodePubkeyValue(text)
    .split(/\r?\n/)
    .filter((l) => l.trim().length > 0);
  if (lines.length < 4) {
    throw new Error(`malformed signature: expected 4 lines, got ${lines.length}`);
  }
  let blob;
  try {
    blob = Buffer.from(lines[1].trim(), 'base64');
  } catch (e) {
    throw new Error('malformed signature: second line is not valid base64');
  }
  if (blob.length !== SIG_BLOB_LEN) {
    throw new Error(
      `malformed signature blob: expected ${SIG_BLOB_LEN} bytes, got ${blob.length}`
    );
  }
  const algorithm = blob.subarray(0, 2).toString('latin1');
  if (algorithm !== 'Ed' && algorithm !== 'ED') {
    throw new Error(`unsupported signature algorithm: ${JSON.stringify(algorithm)}`);
  }
  const keyId = blob.subarray(2, 2 + KEYID_LEN);
  const fileSignature = blob.subarray(2 + KEYID_LEN);

  const trustedLine = lines[2];
  if (!trustedLine.startsWith(MINISIGN_TRUSTED_PREFIX)) {
    throw new Error('malformed signature: missing trusted comment line');
  }
  const trustedComment = trustedLine.slice(MINISIGN_TRUSTED_PREFIX.length);

  let globalSignature;
  try {
    globalSignature = Buffer.from(lines[3].trim(), 'base64');
  } catch (e) {
    throw new Error('malformed signature: fourth line is not valid base64');
  }
  if (globalSignature.length !== SIG_LEN) {
    throw new Error(
      `malformed global signature: expected ${SIG_LEN} bytes, got ${globalSignature.length}`
    );
  }

  return { algorithm, keyId, fileSignature, trustedComment, globalSignature };
}

/**
 * Streaming blake2b512 of a file. The file is never fully buffered, which makes
 * this safe for very large installer assets.
 * @param {string} filePath
 * @returns {Promise<Buffer>} 64-byte digest
 */
function blake2bOfFile(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('blake2b512');
    const stream = fs.createReadStream(filePath);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest()));
    stream.on('error', reject);
    stream.on('aborted', () => reject(new Error('read stream aborted')));
  });
}

/**
 * Build the message that the file signature was computed over.
 *   - "Ed": the raw asset bytes.
 *   - "ED": blake2b512(asset) via streaming read.
 * @returns {Promise<Buffer>}
 */
function readAssetMessage(assetPath, algorithm) {
  if (algorithm === 'ED') {
    return blake2bOfFile(assetPath);
  }
  return Promise.resolve(fs.readFileSync(assetPath));
}

/** Verify the per-asset signature. */
function verifyFileSignature(publicKey, algorithm, fileSignature, message) {
  return crypto.verify(null, message, publicKey, fileSignature);
}

/**
 * Verify the global signature, which protects the trusted comment. The signed
 * message is always (fileSignature || trustedComment utf8) regardless of the
 * per-asset algorithm.
 */
function verifyGlobalSignature(publicKey, fileSignature, trustedComment, globalSignature) {
  const message = Buffer.concat([
    fileSignature,
    Buffer.from(trustedComment, 'utf8'),
  ]);
  return crypto.verify(null, message, publicKey, globalSignature);
}

/** Recursively find all `*.sig` files under a directory (sorted, stable). */
function findSigFiles(dir) {
  const results = [];
  const walk = (current) => {
    const entries = fs.readdirSync(current, { withFileTypes: true });
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.isFile() && entry.name.endsWith('.sig')) {
        results.push(full);
      }
    }
  };
  walk(dir);
  results.sort();
  return results;
}

/**
 * Verify a single signature/asset pair.
 * @param {object} opts
 * @param {crypto.KeyObject} opts.publicKey
 * @param {Buffer} opts.expectedKeyId  key id declared in the public key
 * @param {string} opts.sigPath
 * @param {string} opts.assetPath
 * @returns {Promise<{ sigPath: string, assetPath: string, algorithm: string }>}
 */
async function verifyOne({ publicKey, expectedKeyId, sigPath, assetPath }) {
  if (!fs.existsSync(sigPath)) {
    const err = new Error(`signature file missing: ${sigPath}`);
    err.code = 'MISSING_SIG';
    throw err;
  }
  const sigText = fs.readFileSync(sigPath, 'utf8');
  const sig = parseSignature(sigText);

  if (!sig.keyId.equals(expectedKeyId)) {
    const err = new Error(
      `key id mismatch for ${sigPath}: signature ${sig.keyId.toString('hex')} vs public key ${expectedKeyId.toString('hex')}`
    );
    err.code = 'KEY_ID_MISMATCH';
    throw err;
  }

  if (!fs.existsSync(assetPath)) {
    const err = new Error(`missing asset for signature: ${assetPath}`);
    err.code = 'MISSING_ASSET';
    throw err;
  }

  const message = await readAssetMessage(assetPath, sig.algorithm);
  if (!verifyFileSignature(publicKey, sig.algorithm, sig.fileSignature, message)) {
    const err = new Error(`asset signature verification failed: ${assetPath}`);
    err.code = 'BAD_SIGNATURE';
    throw err;
  }

  if (
    !verifyGlobalSignature(
      publicKey,
      sig.fileSignature,
      sig.trustedComment,
      sig.globalSignature
    )
  ) {
    const err = new Error(`global signature / trusted comment tampered: ${sigPath}`);
    err.code = 'BAD_GLOBAL_SIGNATURE';
    throw err;
  }

  return { sigPath, assetPath, algorithm: sig.algorithm };
}

/**
 * Verify every `*.sig` under `assetsDir` against the given minisign pubkey text.
 * Requires at least one signature and that all of them pass.
 * @param {object} opts
 * @param {string} opts.assetsDir
 * @param {string} opts.pubkeyText  value of plugins.updater.pubkey
 * @returns {Promise<{ verified: Array, count: number }>}
 */
async function verifyAll({ assetsDir, pubkeyText }) {
  if (!fs.existsSync(assetsDir)) {
    const err = new Error(`assets directory does not exist: ${assetsDir}`);
    err.code = 'NO_DIR';
    throw err;
  }
  const pub = parsePublicKey(decodePubkeyValue(pubkeyText));
  const sigFiles = findSigFiles(assetsDir);

  if (sigFiles.length === 0) {
    const err = new Error(`no signature files found in ${assetsDir}`);
    err.code = 'NO_SIGNATURES';
    throw err;
  }

  const results = [];
  const failures = [];
  for (const sigPath of sigFiles) {
    const assetPath = sigPath.replace(/\.sig$/, '');
    try {
      const r = await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
      results.push({ ok: true, ...r });
    } catch (e) {
      results.push({
        ok: false,
        sigPath,
        assetPath,
        error: e.message,
        code: e.code,
      });
      failures.push(e);
    }
  }

  if (failures.length > 0) {
    const err = new Error(
      `${failures.length} of ${sigFiles.length} signature(s) failed verification`
    );
    err.code = 'VERIFY_FAILED';
    err.results = results;
    throw err;
  }

  return { verified: results, count: results.length };
}

/** Minimal CLI argument parser for `--key value` and `--key=value`. */
function parseArgs(argv) {
  const args = { 'assets-dir': null, config: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--assets-dir') {
      args['assets-dir'] = argv[++i];
    } else if (a === '--config') {
      args['config'] = argv[++i];
    } else if (a.startsWith('--assets-dir=')) {
      args['assets-dir'] = a.slice('--assets-dir='.length);
    } else if (a.startsWith('--config=')) {
      args['config'] = a.slice('--config='.length);
    } else if (a === '-h' || a === '--help') {
      args.help = true;
    }
  }
  return args;
}

/**
 * Tauri stores `plugins.updater.pubkey` as a single base64 string wrapping the
 * whole minisign public-key text block. Decode it when that is the case;
 * otherwise pass the value through (it already is minisign text).
 */
function decodePubkeyValue(value) {
  const str = String(value);
  // Raw minisign public-key text is always multi-line. The Tauri config value
  // is a single base64 line wrapping that whole text block.
  if (str.includes('\n')) {
    return str;
  }
  const trimmed = str.trim();
  try {
    const decoded = Buffer.from(trimmed, 'base64').toString('utf8');
    if (decoded.includes('untrusted comment:') && decoded.split('\n').length >= 2) {
      return decoded;
    }
  } catch (e) {
    // fall through
  }
  return str;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(
      'Usage: node verify_updater_signatures.cjs --assets-dir DIR --config src-tauri/tauri.conf.json'
    );
    process.exit(0);
  }
  if (!args['assets-dir'] || !args['config']) {
    console.error(
      'Usage: node verify_updater_signatures.cjs --assets-dir DIR --config src-tauri/tauri.conf.json'
    );
    process.exit(2);
  }

  let config;
  try {
    config = JSON.parse(fs.readFileSync(args['config'], 'utf8'));
  } catch (e) {
    console.error(`Failed to read config ${args['config']}: ${e.message}`);
    process.exit(2);
  }

  const rawPubkey =
    config &&
    config.plugins &&
    config.plugins.updater &&
    config.plugins.updater.pubkey;
  if (!rawPubkey) {
    console.error('plugins.updater.pubkey not found in config');
    process.exit(2);
  }
  const pubkeyText = decodePubkeyValue(rawPubkey);

  try {
    const res = await verifyAll({
      assetsDir: args['assets-dir'],
      pubkeyText,
    });
    console.log(
      `OK: verified ${res.count} signature(s) in ${args['assets-dir']}`
    );
    for (const r of res.verified) {
      console.log(
        `  [${r.algorithm}] ${path.relative(args['assets-dir'], r.assetPath)}  <- ${path.basename(r.sigPath)}`
      );
    }
    process.exit(0);
  } catch (e) {
    console.error(`FAILED: ${e.message}`);
    if (e.results) {
      for (const r of e.results) {
        if (!r.ok) {
          console.error(`  [${r.code || 'ERROR'}] ${(r.assetPath || r.sigPath)}: ${r.error}`);
        }
      }
    }
    process.exit(1);
  }
}

module.exports = {
  ED25519_SPKI_PREFIX,
  MINISIGN_TRUSTED_PREFIX,
  rawEd25519ToSpki,
  spkiToRawEd25519,
  createEd25519PublicKey,
  parsePublicKey,
  parseSignature,
  blake2bOfFile,
  readAssetMessage,
  verifyFileSignature,
  verifyGlobalSignature,
  findSigFiles,
  verifyOne,
  verifyAll,
  parseArgs,
  decodePubkeyValue,
};

if (require.main === module) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
