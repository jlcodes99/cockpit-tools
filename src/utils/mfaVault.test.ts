import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MFA_STORAGE_KEY_HISTORY,
  loadMfaHistoryRecords,
  parseGoogleAuthenticatorMigrationBatch,
  parseGoogleAuthenticatorMigrationInput,
  parseMfaCredentialInputs,
  rememberMfaQuery,
  sortMfaRecords,
  splitMfaAccountName,
  filterMfaRecords,
  exportGoogleAuthenticatorMigrationUris,
  type MfaRecord,
} from './mfaVault.ts';

function toBase64Url(bytes: number[]): string {
  return Buffer.from(bytes).toString('base64url');
}

function createRecord(id: string, accountName: string, time = 1000): MfaRecord {
  return { id, accountName, time, secret: `test-secret-${id}` };
}

test('splits only the first nonempty issuer separator without guessing a missing type', () => {
  assert.deepEqual(splitMfaAccountName(' OpenAI: alice:work@example.com '), { issuer: 'OpenAI', account: 'alice:work@example.com' });
  for (const name of ['alice@example.com', 'OpenAI:', ':alice', '']) {
    assert.deepEqual(splitMfaAccountName(name), { issuer: '', account: name });
  }
});

test('searches names and types case-insensitively without matching secrets', () => {
  const records = [createRecord('one', 'OpenAI:Alice@example.com'), createRecord('two', '谷歌:张三')];
  records[0].secret = 'ONLY_SECRET';
  assert.deepEqual(filterMfaRecords(records, ' OPENai ').map(row => row.id), ['one']);
  assert.deepEqual(filterMfaRecords(records, 'ALICE').map(row => row.id), ['one']);
  assert.deepEqual(filterMfaRecords(records, '张三').map(row => row.id), ['two']);
  assert.deepEqual(filterMfaRecords(records, 'ONLY_SECRET'), []);
  assert.deepEqual(filterMfaRecords(records, '  '), records);
});

test('migration exports round-trip names, types and bytes without changing stored records', () => {
  const records = ['OpenAI:alice@example.com', 'OpenAI:OpenAI:alice@example.com', '谷歌:张三:工作', 'plain@example.com', ''].map((accountName, i) => ({
    ...createRecord(String(i), accountName), secret: 'JBSWY3DPEHPK3PXP',
  }));
  const original = JSON.stringify(records);
  const pages = exportGoogleAuthenticatorMigrationUris(records);
  assert.equal(pages.length, 1);
  assert.equal(pages[0].count, records.length);
  assert.deepEqual(parseGoogleAuthenticatorMigrationInput(pages[0].uri),
    records.map(({ accountName, secret }) => ({ accountName, secret })));
  assert.equal(JSON.stringify(records), original);
  assert.deepEqual(exportGoogleAuthenticatorMigrationUris([]), []);
});

test('migration export splits batches by count and encoded capacity with consistent metadata', () => {
  const records = Array.from({ length: 27 }, (_, i) => ({
    ...createRecord(String(i), `Service:user${i}@example.com`), secret: 'JBSWY3DPEHPK3PXP',
  }));
  const pages = exportGoogleAuthenticatorMigrationUris(records);
  assert.equal(pages.length, 3);
  const batches = pages.map(page => parseGoogleAuthenticatorMigrationBatch(page.uri)!);
  assert.deepEqual(batches.map(batch => batch.batchIndex), [0, 1, 2]);
  assert.ok(batches.every(batch => batch.batchId === batches[0].batchId && batch.batchSize === 3));
  assert.deepEqual(batches.flatMap(batch => batch.credentials).map(row => row.accountName), records.map(row => row.accountName));
  const longRecords = records.slice(0, 3).map(row => ({ ...row, accountName: `类型:${'中文'.repeat(90)}${row.id}` }));
  const longPages = exportGoogleAuthenticatorMigrationUris(longRecords);
  assert.ok(longPages.length > 1);
  assert.ok(longPages.every(page => page.uri.length <= 1500));
  assert.equal(longPages.reduce((sum, page) => sum + page.count, 0), 3);
});

test('migration export rejects invalid, oversized and nonstandard credentials without leaking values', () => {
  const valid = { ...createRecord('one', 'Example:alice'), secret: 'JBSWY3DPEHPK3PXP' };
  assert.throws(() => exportGoogleAuthenticatorMigrationUris([{ ...valid, secret: 'invalid-secret!' }]),
    { message: 'mfaVault.migrationInvalidRecord' });
  assert.throws(() => exportGoogleAuthenticatorMigrationUris([{ ...valid,
    secret: `otpauth://hotp/Example:alice?secret=${valid.secret}&counter=1`,
  }]), { message: 'mfaVault.migrationInvalidRecord' });
  assert.throws(() => exportGoogleAuthenticatorMigrationUris([{ ...valid, accountName: '私'.repeat(1000) }]),
    { message: 'mfaVault.migrationRecordTooLarge' });
  for (const settings of ['algorithm=SHA256', 'digits=8', 'period=60']) {
    assert.throws(() => exportGoogleAuthenticatorMigrationUris([{ ...valid,
      secret: `otpauth://totp/Example:alice?secret=${valid.secret}&${settings}`,
    }]), { message: 'mfaVault.migrationUnsupportedParameters' });
  }
});

test('sorts complete account names naturally without case sensitivity in both directions', () => {
  const records = [
    createRecord('openai', 'OpenAI:alice@example.com'),
    createRecord('ten', 'Google:user10@example.com'),
    createRecord('two', 'google:User2@example.com'),
    createRecord('one', 'Google:user1@example.com'),
  ];

  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'asc' }).map(record => record.id), [
    'one', 'two', 'ten', 'openai',
  ]);
  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'desc' }).map(record => record.id), [
    'openai', 'ten', 'two', 'one',
  ]);
});

test('sorts Chinese and English account names using a consistent locale', () => {
  const records = [
    createRecord('beta', 'Beta'),
    createRecord('zhang', '张三'),
    createRecord('alpha', 'alpha'),
    createRecord('li', '李明'),
    createRecord('a', '阿明'),
  ];

  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'asc' }).map(record => record.id), [
    'a', 'li', 'zhang', 'alpha', 'beta',
  ]);
  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'desc' }).map(record => record.id), [
    'beta', 'alpha', 'zhang', 'li', 'a',
  ]);
});

test('keeps empty and whitespace-only names last in either account name direction', () => {
  const records = [
    createRecord('blank', '', 2),
    createRecord('beta', 'Beta', 3),
    createRecord('spaces', ' \t ', 1),
    createRecord('alpha', ' alpha ', 4),
  ];

  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'asc' }).map(record => record.id), [
    'alpha', 'beta', 'spaces', 'blank',
  ]);
  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'desc' }).map(record => record.id), [
    'beta', 'alpha', 'blank', 'spaces',
  ]);
});

test('orders equal account names by time and then original position', () => {
  const records = [
    createRecord('newer', 'Alice', 2),
    createRecord('older-first', 'alice', 1),
    createRecord('older-second', 'ALICE', 1),
  ];

  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'asc' }).map(record => record.id), [
    'older-first', 'older-second', 'newer',
  ]);
  assert.deepEqual(sortMfaRecords(records, { field: 'accountName', direction: 'desc' }).map(record => record.id), [
    'newer', 'older-second', 'older-first',
  ]);
});

test('sorts by precise timestamps even within the same displayed minute or with empty names', () => {
  const records = [
    createRecord('latest', 'Alpha', 60002),
    createRecord('earliest', '', 60000),
    createRecord('middle', 'Zulu', 60001),
  ];

  assert.deepEqual(sortMfaRecords(records, { field: 'time', direction: 'asc' }).map(record => record.id), [
    'earliest', 'middle', 'latest',
  ]);
  assert.deepEqual(sortMfaRecords(records, { field: 'time', direction: 'desc' }).map(record => record.id), [
    'latest', 'middle', 'earliest',
  ]);
});

test('reverses accounts imported at the same timestamp using names and original position', () => {
  const records = [
    createRecord('beta', 'Beta'),
    createRecord('alpha-first', 'Alpha'),
    createRecord('alpha-second', 'alpha'),
  ];

  assert.deepEqual(sortMfaRecords(records, { field: 'time', direction: 'asc' }).map(record => record.id), [
    'alpha-first', 'alpha-second', 'beta',
  ]);
  assert.deepEqual(sortMfaRecords(records, { field: 'time', direction: 'desc' }).map(record => record.id), [
    'beta', 'alpha-second', 'alpha-first',
  ]);
});

test('handles empty and single-record lists for every sort choice', () => {
  const record = createRecord('only', 'Only');
  for (const field of ['accountName', 'time'] as const) {
    for (const direction of ['asc', 'desc'] as const) {
      assert.deepEqual(sortMfaRecords([], { field, direction }), []);
      assert.deepEqual(sortMfaRecords([record], { field, direction }), [record]);
    }
  }
});

test('sorting leaves the source array and record contents unchanged', () => {
  const beta = Object.freeze(createRecord('beta', 'Beta', 2));
  const alpha = Object.freeze(createRecord('alpha', ' alpha ', 1));
  const records = Object.freeze([beta, alpha]);

  for (const field of ['accountName', 'time'] as const) {
    const sorted = sortMfaRecords(records, { field, direction: 'asc' });
    assert.notEqual(sorted, records);
    assert.equal(sorted[0], alpha);
    assert.equal(sorted[1], beta);
    assert.deepEqual(records, [beta, alpha]);
    assert.equal(alpha.accountName, ' alpha ');
  }
});

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
