import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import { proxyAssignmentAccounts } from './codexProxyAssignment';
import { batchBindTargets, executeProxyBatch, type BatchBindResult } from './codexProxyBatch';

const oauth = (id: string): CodexAccount => ({ id, email: `${id}@example.test`, auth_mode: 'oauth',
  tokens: { access_token: 'a', id_token: 'i' }, created_at: 0, last_used: 0 });
const accounts: CodexAccount[] = [oauth('first'), oauth('second'),
  { ...oauth('provider'), api_provider_mode: 'custom' }, { ...oauth('pending'), authorization_status: 'pending' },
  { ...oauth('key'), auth_mode: 'apikey' }];

test('opening assignment never selects an account and stale/ineligible selections cannot become write targets', () => {
  assert.deepEqual(proxyAssignmentAccounts(accounts, []), []);
  assert.deepEqual(proxyAssignmentAccounts(accounts, ['missing', 'provider', 'pending', 'key']), []);
  assert.deepEqual(proxyAssignmentAccounts(accounts, ['second', 'second', 'provider']).map((a) => a.id), ['second']);
});

test('resource assignment writes only the explicit scope and allows retry after partial failure', async () => {
  const selected = proxyAssignmentAccounts(accounts, ['first', 'second', 'provider']);
  const targets = batchBindTargets(selected, (a) => a.egress_proxy, 'source', 'node');
  const writes: string[] = [];
  const applied: string[] = [];
  let results: BatchBindResult[] = [];
  await executeProxyBatch(targets, {
    cancelled: () => false,
    bind: async (account) => { writes.push(account.id); if (account.id === 'second') throw new Error('failed'); return account; },
    applied: (account) => applied.push(account.id), errorKey: () => 'codex.proxy.catalog.failed',
    progress: (next) => { results = next; },
  });
  assert.deepEqual(writes, ['first', 'second']);
  assert.deepEqual(applied, ['first']);
  assert.deepEqual(results.map((result) => result.ok), [true, false]);
  const retry = selected.filter((account) => !applied.includes(account.id));
  assert.deepEqual(retry.map((account) => account.id), ['second']);
});
