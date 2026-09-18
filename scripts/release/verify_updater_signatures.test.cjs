'use strict';

/**
 * verify_updater_signatures.test.cjs
 *
 * Self-contained test runner (no external test framework, only Node built-ins).
 * It generates real Ed25519 keypairs with `crypto.generateKeyPairSync`, builds
 * valid minisign public-key / signature text blocks, and verifies that the
 * verifier in `verify_updater_signatures.cjs` accepts good inputs and rejects
 * every tampering / misconfiguration scenario.
 *
 * Run with the managed Node, e.g.:
 *   /c/Users/DLX/.workbuddy/binaries/node/versions/22.22.2-3/node \
 *     scripts/release/verify_updater_signatures.test.cjs
 */

const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');

const {
  parsePublicKey,
  parseSignature,
  verifyOne,
  verifyAll,
  findSigFiles,
  blake2bOfFile,
  decodePubkeyValue,
} = require('./verify_updater_signatures.cjs');

const MINISIGN_TRUSTED_PREFIX = 'trusted comment: ';

// ---------------------------------------------------------------------------
// Test scaffolding: build real minisign material from a generated keypair.
// ---------------------------------------------------------------------------

function makeKeyMaterial() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  const rawPub = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32);
  const keyId = crypto.randomBytes(8);
  return { publicKey, privateKey, rawPub, keyId };
}

function buildPubkeyText(rawPub, keyId, untrustedComment) {
  const blob = Buffer.concat([Buffer.from('Ed', 'latin1'), keyId, rawPub]);
  return (
    'untrusted comment: ' + (untrustedComment || 'test public key') + '\n' +
    blob.toString('base64') + '\n'
  );
}

// Synchronous blake2b for small test fixtures.
function blake2bSync(filePath) {
  const hash = crypto.createHash('blake2b512');
  hash.update(fs.readFileSync(filePath));
  return hash.digest();
}

function signAsset(privateKey, assetPath, algorithm) {
  const message =
    algorithm === 'ED'
      ? blake2bSync(assetPath)
      : fs.readFileSync(assetPath);
  return crypto.sign(null, message, privateKey);
}

function buildSigText({ privateKey, fileSignature, keyId, algorithm, trustedComment, untrustedComment }) {
  const sigBlob = Buffer.concat([
    Buffer.from(algorithm, 'latin1'),
    keyId,
    fileSignature,
  ]);
  const globalMessage = Buffer.concat([
    fileSignature,
    Buffer.from(trustedComment, 'utf8'),
  ]);
  const globalSig = crypto.sign(null, globalMessage, privateKey);
  return (
    'untrusted comment: ' + (untrustedComment || 'test signature') + '\n' +
    sigBlob.toString('base64') + '\n' +
    MINISIGN_TRUSTED_PREFIX + trustedComment + '\n' +
    globalSig.toString('base64') + '\n'
  );
}

function writeValidSig(dir, relAsset, material, algorithm, trustedComment) {
  const assetPath = path.join(dir, relAsset);
  fs.mkdirSync(path.dirname(assetPath), { recursive: true });
  fs.writeFileSync(assetPath, `payload for ${relAsset} (${algorithm})`);
  const fileSignature = signAsset(material.privateKey, assetPath, algorithm);
  const sigPath = assetPath + '.sig';
  fs.writeFileSync(
    sigPath,
    buildSigText({
      privateKey: material.privateKey,
      fileSignature,
      keyId: material.keyId,
      algorithm,
      trustedComment: trustedComment || 'trusted:build-123',
    })
  );
  return { assetPath, sigPath };
}

// ---------------------------------------------------------------------------
// Tiny assertion framework.
// ---------------------------------------------------------------------------

let passed = 0;
let failed = 0;
const failures = [];

function assert(cond, msg) {
  if (!cond) throw new Error(msg || 'assertion failed');
}

async function test(name, fn) {
  try {
    await fn();
    passed++;
    console.log('  PASS  ' + name);
  } catch (e) {
    failed++;
    failures.push({ name, error: e });
    console.log('  FAIL  ' + name + '  ->  ' + e.message);
  }
}

function expectReject(name, fn, code, hint) {
  return test(name, async () => {
    let thrown = null;
    try {
      await fn();
    } catch (e) {
      thrown = e;
    }
    assert(thrown, hint || 'expected rejection but call succeeded');
    if (code) {
      assert(
        thrown.code === code,
        `expected error code ${code} but got ${thrown.code || '(none)'}`
      );
    }
  });
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

async function run() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'vus-test-'));

  // --- Ed (raw) success -----------------------------------------------------
  await test('Ed: valid asset + signature verifies', async () => {
    const dir = path.join(tmp, 'ed-ok');
    const m = makeKeyMaterial();
    const pubText = buildPubkeyText(m.rawPub, m.keyId);
    const { assetPath, sigPath } = writeValidSig(dir, 'app.exe', m, 'Ed');
    const pub = parsePublicKey(pubText);
    const r = await verifyOne({
      publicKey: pub.publicKey,
      expectedKeyId: pub.keyId,
      sigPath,
      assetPath,
    });
    assert(r.algorithm === 'Ed', 'algorithm should be Ed');
  });

  // --- ED (prehashed blake2b512) success -----------------------------------
  await test('ED: valid prehashed asset + signature verifies', async () => {
    const dir = path.join(tmp, 'ed-prehash-ok');
    const m = makeKeyMaterial();
    const pubText = buildPubkeyText(m.rawPub, m.keyId);
    const { assetPath, sigPath } = writeValidSig(dir, 'app.AppImage', m, 'ED');
    const pub = parsePublicKey(pubText);
    const r = await verifyOne({
      publicKey: pub.publicKey,
      expectedKeyId: pub.keyId,
      sigPath,
      assetPath,
    });
    assert(r.algorithm === 'ED', 'algorithm should be ED');
    // Cross-check: streaming blake2b matches the synchronous one used to sign.
    const streamed = await blake2bOfFile(assetPath);
    const synced = blake2bSync(assetPath);
    assert(streamed.equals(synced), 'streamed and synced blake2b must match');
  });

  // --- asset tampering ------------------------------------------------------
  await expectReject(
    'Ed: tampered asset content is rejected',
    async () => {
      const dir = path.join(tmp, 'ed-tamper-asset');
      const m = makeKeyMaterial();
      const pubText = buildPubkeyText(m.rawPub, m.keyId);
      const { assetPath, sigPath } = writeValidSig(dir, 'app.exe', m, 'Ed');
      fs.writeFileSync(assetPath, 'I am a different payload now');
      const pub = parsePublicKey(pubText);
      await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
    },
    'BAD_SIGNATURE'
  );

  await expectReject(
    'ED: tampered (streamed) asset content is rejected',
    async () => {
      const dir = path.join(tmp, 'ed-tamper-asset-stream');
      const m = makeKeyMaterial();
      const pubText = buildPubkeyText(m.rawPub, m.keyId);
      const { assetPath, sigPath } = writeValidSig(dir, 'app.AppImage', m, 'ED');
      fs.writeFileSync(assetPath, 'changed bytes for prehashed asset');
      const pub = parsePublicKey(pubText);
      await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
    },
    'BAD_SIGNATURE'
  );

  // --- trusted comment tampering -------------------------------------------
  await expectReject(
    'Ed: tampered trusted comment is rejected (global sig)',
    async () => {
      const dir = path.join(tmp, 'ed-tamper-comment');
      const m = makeKeyMaterial();
      const pubText = buildPubkeyText(m.rawPub, m.keyId);
      const { assetPath, sigPath } = writeValidSig(
        dir,
        'app.exe',
        m,
        'Ed',
        'trusted:build-123'
      );
      // Re-write the sig file with a different trusted comment but keep the
      // original (now invalid) global signature.
      const lines = fs.readFileSync(sigPath, 'utf8').split('\n');
      lines[2] = MINISIGN_TRUSTED_PREFIX + 'trusted:build-999 (tampered)';
      fs.writeFileSync(sigPath, lines.join('\n'));
      const pub = parsePublicKey(pubText);
      await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
    },
    'BAD_GLOBAL_SIGNATURE'
  );

  // --- wrong public key (matching key id, wrong raw key) --------------------
  await expectReject(
    'Ed: wrong public key (matching key id) is rejected',
    async () => {
      const dir = path.join(tmp, 'ed-wrong-key');
      const good = makeKeyMaterial();
      const other = makeKeyMaterial();
      // Publish a public key that carries the SIGNER's key id but a DIFFERENT
      // raw Ed25519 key, so the key-id check passes yet the signature fails.
      const pubText = buildPubkeyText(other.rawPub, good.keyId);
      const { assetPath, sigPath } = writeValidSig(dir, 'app.exe', good, 'Ed');
      const pub = parsePublicKey(pubText);
      await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
    },
    'BAD_SIGNATURE'
  );

  // --- key id mismatch ------------------------------------------------------
  await expectReject(
    'Ed: key id mismatch between pubkey and signature is rejected',
    async () => {
      const dir = path.join(tmp, 'ed-keyid-mismatch');
      const signer = makeKeyMaterial();
      const declared = makeKeyMaterial();
      // Publish a pubkey whose keyId differs from the one used to sign.
      const pubText = buildPubkeyText(declared.rawPub, declared.keyId);
      const { assetPath, sigPath } = writeValidSig(dir, 'app.exe', signer, 'Ed');
      const pub = parsePublicKey(pubText);
      await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
    },
    'KEY_ID_MISMATCH'
  );

  // --- missing asset --------------------------------------------------------
  await expectReject(
    'Ed: signature without matching asset is rejected',
    async () => {
      const dir = path.join(tmp, 'ed-missing-asset');
      const m = makeKeyMaterial();
      const pubText = buildPubkeyText(m.rawPub, m.keyId);
      const { sigPath } = writeValidSig(dir, 'app.exe', m, 'Ed');
      const assetPath = sigPath.replace(/\.sig$/, '');
      fs.unlinkSync(assetPath); // remove the asset, keep the sig
      const pub = parsePublicKey(pubText);
      await verifyOne({
        publicKey: pub.publicKey,
        expectedKeyId: pub.keyId,
        sigPath,
        assetPath,
      });
    },
    'MISSING_ASSET'
  );

  // --- no signatures at all -------------------------------------------------
  await expectReject(
    'verifyAll: directory with no *.sig files is rejected',
    async () => {
      const dir = path.join(tmp, 'no-sigs');
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(path.join(dir, 'orphan.exe'), 'no sig here');
      const m = makeKeyMaterial();
      const pubText = buildPubkeyText(m.rawPub, m.keyId);
      await verifyAll({ assetsDir: dir, pubkeyText: pubText });
    },
    'NO_SIGNATURES'
  );

  // --- subdirectory recursion + multiple sigs (all must pass) ---------------
  await test('verifyAll: nested dirs, mixed Ed/ED, all pass', async () => {
    const dir = path.join(tmp, 'nested');
    const m = makeKeyMaterial();
    const pubText = buildPubkeyText(m.rawPub, m.keyId);
    writeValidSig(dir, path.join('flat', 'a.msi'), m, 'Ed');
    writeValidSig(dir, path.join('flat', 'b.AppImage'), m, 'ED');
    writeValidSig(dir, path.join('deep', 'sub', 'c.dmg'), m, 'Ed');
    const res = await verifyAll({ assetsDir: dir, pubkeyText: pubText });
    assert(res.count === 3, `expected 3 verified, got ${res.count}`);
    const sigs = findSigFiles(dir);
    assert(sigs.length === 3, 'findSigFiles should recurse and find 3');
  });

  // --- one failure fails the whole batch -----------------------------------
  await expectReject(
    'verifyAll: one bad sig among many fails the batch',
    async () => {
      const dir = path.join(tmp, 'mixed');
      const good = makeKeyMaterial();
      const other = makeKeyMaterial();
      const pubText = buildPubkeyText(good.rawPub, good.keyId);
      writeValidSig(dir, 'ok.msi', good, 'Ed');
      const bad = writeValidSig(dir, 'bad.msi', other, 'Ed'); // signed by wrong key
      const res = await verifyAll({ assetsDir: dir, pubkeyText: pubText });
      // should not reach here
      void bad;
      void res;
    },
    'VERIFY_FAILED'
  );

  // --- malformed format rejections -----------------------------------------
  await expectReject('parsePublicKey: garbage is rejected', async () => {
    parsePublicKey('untrusted comment: x\nnot-valid-base64-!!!');
  });
  await expectReject('parseSignature: too few lines is rejected', async () => {
    parseSignature('untrusted comment: x\n' + Buffer.from('Ed').toString('base64') + '\n');
  });

  // --- Tauri config pubkey wrapping (base64 of the whole text) --------------
  await test('decodePubkeyValue: base64-wrapped config value decodes to text', async () => {
    const m = makeKeyMaterial();
    const text = buildPubkeyText(m.rawPub, m.keyId);
    const wrapped = Buffer.from(text, 'utf8').toString('base64');
    const decoded = decodePubkeyValue(wrapped);
    assert(decoded === text, 'wrapped value should decode back to minisign text');
    const pub = parsePublicKey(decoded);
    assert(pub.keyId.equals(m.keyId), 'decoded key id must match');
  });

  await test('decodePubkeyValue: raw minisign text passes through unchanged', async () => {
    const m = makeKeyMaterial();
    const text = buildPubkeyText(m.rawPub, m.keyId);
    assert(decodePubkeyValue(text) === text, 'raw text should pass through');
  });

  // End-to-end through verifyAll using a base64-wrapped config pubkey.
  await test('verifyAll: base64-wrapped config pubkey verifies nested sigs', async () => {
    const dir = path.join(tmp, 'wrapped');
    const m = makeKeyMaterial();
    const text = buildPubkeyText(m.rawPub, m.keyId);
    const wrapped = Buffer.from(text, 'utf8').toString('base64');
    writeValidSig(dir, 'x.msi', m, 'Ed');
    const tauri = writeValidSig(dir, 'y.AppImage', m, 'ED');
    const wrappedSignature = Buffer.from(fs.readFileSync(tauri.sigPath, 'utf8')).toString('base64');
    fs.writeFileSync(tauri.sigPath, wrappedSignature);
    const res = await verifyAll({ assetsDir: dir, pubkeyText: wrapped });
    assert(res.count === 2, `expected 2 verified, got ${res.count}`);
  });

  fs.rmSync(tmp, { recursive: true, force: true });

  console.log('');
  console.log(`Tests: ${passed} passed, ${failed} failed, ${passed + failed} total`);
  if (failed > 0) {
    console.log('');
    for (const f of failures) {
      console.log(`  - ${f.name}: ${f.error.stack || f.error.message}`);
    }
    process.exit(1);
  }
  process.exit(0);
}

run().catch((e) => {
  console.error(e);
  process.exit(1);
});
