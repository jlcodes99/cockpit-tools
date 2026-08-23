import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MFA_STORAGE_KEY_SAVED,
  loadSavedMfaRecords,
  parseGoogleAuthenticatorMigrationBatch,
  parseGoogleAuthenticatorMigrationInput,
  parseMfaCredentialInputs,
} from './mfaVault.ts';

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return [...this.values.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

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

test('loads current and legacy MFA localStorage records without changing their schema', () => {
  const previousDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const storage = new MemoryStorage();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: storage,
  });
  try {
    storage.setItem(MFA_STORAGE_KEY_SAVED, JSON.stringify([{
      accountName: 'Current:current@example.com',
      secret: 'JBSWY3DPEHPK3PXP',
      remark: 'current',
      time: 3,
    }]));
    storage.setItem('agtools.mfa.vault.v1', JSON.stringify([{
      accountName: 'LegacyVault:legacy-vault@example.com',
      secret: 'GEZDGNBVGY3TQOJQ',
      remark: 'legacy-vault',
      time: 2,
    }]));
    storage.setItem('agtools.two_factor_auth.saved.v2', JSON.stringify([{
      accountName: 'Legacy2FA:legacy-2fa@example.com',
      secret: 'MFRGGZDFMZTWQ2LK',
      remark: 'legacy-2fa',
      time: 1,
    }]));

    assert.deepEqual(
      loadSavedMfaRecords().map((record) => record.accountName),
      [
        'Current:current@example.com',
        'LegacyVault:legacy-vault@example.com',
        'Legacy2FA:legacy-2fa@example.com',
      ],
    );
    assert.equal(storage.getItem(MFA_STORAGE_KEY_SAVED)?.includes('current@example.com'), true);
    assert.equal(storage.getItem('agtools.mfa.vault.v1')?.includes('legacy-vault@example.com'), true);
    assert.equal(storage.getItem('agtools.two_factor_auth.saved.v2')?.includes('legacy-2fa@example.com'), true);
  } finally {
    if (previousDescriptor) {
      Object.defineProperty(globalThis, 'localStorage', previousDescriptor);
    } else {
      Reflect.deleteProperty(globalThis, 'localStorage');
    }
  }
});
