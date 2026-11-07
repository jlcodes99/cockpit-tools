import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { CodexAccount } from '../types/codex.ts';
import { mergeCodexProfileResult } from './codexProfileRefresh.ts';

const original = {
  id: 'test', account_id: 'workspace', account_name: undefined, account_structure: undefined,
  tokens: { access_token: 'local-test-token', id_token: 'local-test-id' },
} as CodexAccount;

test('empty or unchanged profile responses keep the original reference and skip cache work', () => {
  assert.equal(mergeCodexProfileResult(original, original, { ...original }), original);
});

test('profile updates preserve concurrent quota, token and user edits', () => {
  const refreshed = { ...original, account_name: 'Remote', account_structure: 'personal' };
  const current = { ...original, account_name: 'User edit', quota: { marker: 'fresh' } } as unknown as CodexAccount;
  const merged = mergeCodexProfileResult(current, original, refreshed);
  assert.equal(merged.account_name, 'User edit');
  assert.equal(merged.account_structure, 'personal');
  assert.equal(merged.quota, current.quota);
  assert.equal(merged.tokens, current.tokens);
  const reauthorized = { ...current, tokens: { ...current.tokens, access_token: 'new-test-token' } };
  assert.equal(mergeCodexProfileResult(reauthorized, original, refreshed), reauthorized);
});
