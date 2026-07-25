import assert from 'node:assert/strict';
import test from 'node:test';

import {
  parseGoogleAuthenticatorMigrationBatch,
  parseGoogleAuthenticatorMigrationInput,
  parseMfaCredentialInputs,
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
