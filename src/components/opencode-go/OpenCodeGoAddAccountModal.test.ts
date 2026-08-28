import assert from 'node:assert/strict';
import test from 'node:test';
import {
  describeOpenCodeGoAddError,
  initialOpenCodeGoAddAccountForm,
  submitOpenCodeGoAddAccount,
  validateOpenCodeGoAddAccount,
} from './openCodeGoAddAccountForm.ts';

test('add-account validation trims values and requires a whitespace-free API key', () => {
  assert.deepEqual(
    validateOpenCodeGoAddAccount({ name: '  Primary  ', email: '  owner@example.com  ', apiKey: '  ocg-secret  ' }),
    { values: { name: 'Primary', email: 'owner@example.com', apiKey: 'ocg-secret' }, errors: {} },
  );

  assert.deepEqual(
    validateOpenCodeGoAddAccount({ name: '', email: '', apiKey: '  ' }).errors,
    { apiKey: 'required' },
  );
  assert.deepEqual(
    validateOpenCodeGoAddAccount({ name: '', email: '', apiKey: 'not a key' }).errors,
    { apiKey: 'invalid' },
  );
});

test('submit passes the normalized secret to the boundary without returning or retaining it', async () => {
  const calls: Array<{ name: string; email: string; apiKey: string }> = [];
  const result = await submitOpenCodeGoAddAccount(
    { name: '  Backup  ', email: '  owner@example.com  ', apiKey: '  ocg-test-value  ' },
    async (input) => {
      calls.push(input);
      return { id: 'connection-1', name: input.name, keyHint: 'ocg-****alue' };
    },
  );

  assert.deepEqual(calls, [{ name: 'Backup', email: 'owner@example.com', apiKey: 'ocg-test-value' }]);
  assert.deepEqual(result, {
    ok: true,
    connection: { id: 'connection-1', name: 'Backup', keyHint: 'ocg-****alue' },
  });
  assert.deepEqual(initialOpenCodeGoAddAccountForm(), { name: '', email: '', apiKey: '' });
  assert.equal(JSON.stringify(result).includes('ocg-test-value'), false);
});

test('submit blocks invalid values before calling the persistence boundary', async () => {
  let called = false;
  const result = await submitOpenCodeGoAddAccount(
    { name: 'Primary', email: '', apiKey: 'bad key' },
    async () => {
      called = true;
      return { id: 'never', name: '', keyHint: '' };
    },
  );

  assert.equal(called, false);
  assert.deepEqual(result, { ok: false, errors: { apiKey: 'invalid' } });
});

test('email validation rejects malformed identities before persistence', () => {
  assert.deepEqual(
    validateOpenCodeGoAddAccount({ name: 'Primary', email: 'not-an-email', apiKey: 'ocg-key' }).errors,
    { email: 'invalidEmail' },
  );
});

test('command errors map to safe messages and never echo upstream details', () => {
  assert.equal(
    describeOpenCodeGoAddError('OPENCODE_GO_API_KEY_EXISTS: ocg-do-not-echo'),
    'duplicate',
  );
  assert.equal(
    describeOpenCodeGoAddError('OPENCODE_GO_CONNECTION_LIMIT_REACHED'),
    'limit',
  );
  assert.equal(describeOpenCodeGoAddError('ocg-do-not-echo'), 'unavailable');
});
