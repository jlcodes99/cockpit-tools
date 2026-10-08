import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MFA_STORAGE_KEY_HISTORY,
  loadMfaHistoryRecords,
  parseGoogleAuthenticatorMigrationBatch,
  parseGoogleAuthenticatorMigrationInput,
  parseMfaCredentialInputs,
  rememberMfaQuery,
} from './mfaVault.ts';

function toBase64Url(bytes: number[]): string {
  return Buffer.from(bytes).toString('base64url');
}

test('parses TOTP accounts from a Google Authenticator migration URI', () => {
  const otpParameters = [
    0x0a, 0x05, 0x48, 0x45, 0x4c, 0x4c, 0x4f,
    0x12, 0x11, ...Buffer.from('alice@example.com'),
    0x1a, 0x07, ...Buffer.from('Example'),
    0x30, 0x02,
  ];
  const payload = [0x0a, otpParameters.length, ...otpParameters];
  const uri = `otpauth-migration://offline?data=${toBase64Url(payload)}`;

  assert.deepEqual(parseGoogleAuthenticatorMigrationInput(uri), [{
    accountName: 'Example:alice@example.com',
    secret: 'JBCUYTCP',
  }]);
  assert.deepEqual(parseMfaCredentialInputs(uri), [{
    accountName: 'Example:alice@example.com',
    secret: 'JBCUYTCP',
  }]);
});

test('ignores non-TOTP entries and rejects malformed migration data', () => {
  const hotpParameters = [0x0a, 0x05, 0x48, 0x45, 0x4c, 0x4c, 0x4f, 0x30, 0x01];
  const payload = [0x0a, hotpParameters.length, ...hotpParameters];
  const hotpUri = `otpauth-migration://offline?data=${toBase64Url(payload)}`;

  assert.deepEqual(parseGoogleAuthenticatorMigrationInput(hotpUri), []);
  assert.deepEqual(parseGoogleAuthenticatorMigrationInput('otpauth-migration://offline?data=not-valid'), []);
});

test('preserves migration batch metadata for out-of-order and duplicate QR handling', () => {
  const otpParameters = [0x0a, 0x05, 0x48, 0x45, 0x4c, 0x4c, 0x4f, 0x30, 0x02];
  const payload = [
    0x0a, otpParameters.length, ...otpParameters,
    0x18, 0x03,
    0x20, 0x01,
    0x28, 0x2a,
  ];
  const uri = `otpauth-migration://offline?data=${toBase64Url(payload)}`;

  assert.deepEqual(parseGoogleAuthenticatorMigrationBatch(uri), {
    batchId: 42,
    batchIndex: 1,
    batchSize: 3,
    credentials: [{ accountName: '', secret: 'JBCUYTCP' }],
  });
});

test('persists generated OTP inputs in query history without automatic eviction', () => {
  const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const storage = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
    },
  });

  try {
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
    const history = Array.from({ length: 55 }, (_, index) => ({
      id: `history-${index}`,
      accountName: `account-${index}`,
      secret: `JBSWY3DPEHPK3P${alphabet[Math.floor(index / alphabet.length)]}${alphabet[index % alphabet.length]}`,
      remark: '',
      time: index + 1,
    }));
    storage.set(MFA_STORAGE_KEY_HISTORY, JSON.stringify(history));

    assert.equal(loadMfaHistoryRecords().length, 55);

    const remembered = rememberMfaQuery({
      secret: 'JBSWY3DPEHPK3PXP',
      accountName: 'alice@example.com',
    });
    assert.equal(remembered.length, 56);
    assert.equal(remembered[0]?.secret, 'JBSWY3DPEHPK3PXP');
    assert.equal(remembered[0]?.accountName, 'alice@example.com');
    assert.equal(loadMfaHistoryRecords().length, 56);

    const repeated = rememberMfaQuery({
      secret: 'JBSWY3DPEHPK3PXP',
      accountName: 'alice@example.com',
    });
    assert.equal(repeated.length, 56);
    assert.equal(repeated[0]?.secret, 'JBSWY3DPEHPK3PXP');

    const invalid = rememberMfaQuery({ secret: 'not-a-valid-secret!' });
    assert.equal(invalid.length, 56);
  } finally {
    if (originalStorage) {
      Object.defineProperty(globalThis, 'localStorage', originalStorage);
    } else {
      Reflect.deleteProperty(globalThis, 'localStorage');
    }
  }
});
