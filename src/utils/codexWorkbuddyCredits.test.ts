import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { WorkbuddyAccount } from '../types/workbuddy';
import { toCodexAccountDisplayCache } from './codexProxyCache';
import { codexWorkbuddySourceId, resolveCodexWorkbuddySource, workbuddyCreditDisplaySource } from './codexWorkbuddyCredits';

const linked = (extra: Partial<CodexAccount> = {}) => ({
  id: 'linked', auth_mode: 'apikey', api_credential_kind: 'workbuddy_acp',
  api_credential_ref: ' source ', plan_type: 'Cockpit Api', ...extra,
}) as CodexAccount;

test('WorkBuddy association requires the exact adapter and non-empty source reference', () => {
  assert.equal(codexWorkbuddySourceId(linked()), 'source');
  for (const extra of [
    { auth_mode: 'oauth' }, { api_credential_kind: 'zcode_api_key' },
    { api_credential_kind: null }, { api_credential_ref: '  ' }, { api_credential_ref: null },
  ]) assert.equal(codexWorkbuddySourceId(linked(extra)), null);
  assert.equal(codexWorkbuddySourceId(linked({ plan_type: 'WorkBuddy' })), 'source');
});

test('same-email accounts cannot substitute for a deleted or different WorkBuddy source', () => {
  const source = { id: 'source', email: 'same@example.invalid' } as WorkbuddyAccount;
  const other = { ...source, id: 'other' };
  assert.equal(resolveCodexWorkbuddySource(linked(), [other, source]), source);
  assert.equal(resolveCodexWorkbuddySource(linked(), [other]), undefined);
  assert.equal(resolveCodexWorkbuddySource(linked(), []), undefined);
});

test('Codex credit display projection retains billing inputs but never source credentials', () => {
  const source = {
    id: 'source', email: 'private@example.invalid', access_token: 'secret-access',
    refresh_token: 'secret-refresh', auth_raw: { token: 'secret-auth' },
    profile_raw: { private: 'secret-profile' }, quota_raw: { balance: 12 },
    usage_raw: { cached: true }, usage_updated_at: 100,
    quota_query_last_error: 'fixture failure',
  } as WorkbuddyAccount;
  const display = workbuddyCreditDisplaySource(source);
  assert.deepEqual(display.quota_raw, source.quota_raw);
  assert.deepEqual(display.usage_raw, source.usage_raw);
  assert.equal(display.usage_updated_at, 100);
  assert.equal(display.quota_query_last_error, 'fixture failure');
  assert.doesNotMatch(JSON.stringify(display), /secret-|private@example/);
  assert.equal(source.access_token, 'secret-access');
});

test('display-only Codex cache preserves references without adding source data', () => {
  const cached = toCodexAccountDisplayCache(linked({ tokens: { access_token: 'secret', id_token: 'secret' } }));
  assert.equal(codexWorkbuddySourceId(cached), 'source');
  assert.doesNotMatch(JSON.stringify(cached), /secret|quota_raw|usage_raw/);
});
