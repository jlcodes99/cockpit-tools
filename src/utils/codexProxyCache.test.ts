import assert from 'node:assert/strict';
import test from 'node:test';
import { withoutCachedProxySecret } from './codexProxyCache';
import type { CodexAccount } from '../types/codex';

test('browser cache excludes proxy credentials without mutating the live account', () => {
  const account = {
    id: 'cache', email: 'test@example.com', created_at: 0, last_used: 0,
    tokens: { access_token: 'access', id_token: 'id' },
    egress_proxy_url: 'trojan://TOP_SECRET@node.example:443',
  } satisfies CodexAccount;
  const cached = withoutCachedProxySecret(account);
  assert.equal(cached.egress_proxy_url, undefined);
  assert.equal(JSON.stringify(cached).includes('TOP_SECRET'), false);
  assert.equal(account.egress_proxy_url, 'trojan://TOP_SECRET@node.example:443');
  assert.equal(cached.id, account.id);
});
