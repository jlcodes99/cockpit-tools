import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { CodexAccount } from '../types/codex.ts';
import { mergeCodexQuotaUpdate, type CodexQuotaUpdate } from './codexQuotaSync.ts';

const account = {
  id: 'local', account_id: 'workspace', organization_id: 'org', token_generation: 2,
  tokens: { access_token: 'fixture', id_token: 'fixture-id' }, account_note: 'user note',
  usage_updated_at: 100, quota_error: { message: 'old failure' },
} as CodexAccount;
const update = {
  account_id: 'local', remote_account_id: 'workspace', organization_id: 'org', token_generation: 2,
  usage_updated_at: 101, sequence: 1, quota: { hourly_percentage: 10, weekly_percentage: 50 }, plan_type: 'plus',
} as CodexQuotaUpdate;

test('a quota event updates one account immediately and retains credentials and user edits', () => {
  const merged = mergeCodexQuotaUpdate(account, update);
  assert.equal(merged.quota, update.quota);
  assert.equal(merged.tokens, account.tokens);
  assert.equal(merged.account_note, account.account_note);
  assert.equal(merged.quota_error, undefined);
  assert.equal(merged.usage_updated_at, 101);
});

test('late events for another account, workspace, organization or generation are rejected', () => {
  for (const patch of [
    { account_id: 'other' }, { remote_account_id: 'other-workspace' }, { organization_id: null },
    { token_generation: 1 }, { usage_updated_at: 99 },
  ]) assert.equal(mergeCodexQuotaUpdate(account, { ...update, ...patch }), account);
});

test('a delayed account-list read merges a newer quota event while preserving unrelated fields', () => {
  const pendingListResult = { ...account, account_note: 'new user note' };
  const merged = mergeCodexQuotaUpdate(pendingListResult, update);
  assert.equal(merged.quota, update.quota);
  assert.equal(merged.account_note, 'new user note');
  const fresherDiskRead = { ...account, usage_updated_at: 102 };
  assert.equal(mergeCodexQuotaUpdate(fresherDiskRead, update), fresherDiskRead);
});

test('a committed event from newer credentials can refresh quota without fabricating tokens', () => {
  const merged = mergeCodexQuotaUpdate(account, { ...update, token_generation: 3 });
  assert.equal(merged.quota, update.quota);
  assert.equal(merged.tokens, account.tokens);
  assert.equal(merged.token_generation, 2);
});
