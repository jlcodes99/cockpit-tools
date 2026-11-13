import assert from 'node:assert/strict';
import test from 'node:test';
import { toCodexAccountDisplayCache, withoutCachedProxySecret } from './codexProxyCache';
import { isCodexAccountDisplayCache, isCodexPendingOAuthAccount, type CodexAccount } from '../types/codex';

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

test('Codex startup cache drops account credentials while retaining pending and agent account identities', () => {
  const base: CodexAccount = { id: 'test', email: 'test@example.com', created_at: 0, last_used: 0,
    tokens: { id_token: 'fixture-id', access_token: 'fixture-access', refresh_token: 'fixture-refresh' },
    openai_api_key: 'fixture-api', two_factor_secret: 'fixture-mfa', account_password: 'fixture-password',
    account_note: 'fixture-note', phone_number: 'fixture-phone', mail_url: 'fixture-mail',
  };
  const cached = toCodexAccountDisplayCache(base);
  assert.equal(isCodexAccountDisplayCache(cached), true);
  assert.equal(isCodexPendingOAuthAccount(cached), false);
  assert.equal(isCodexPendingOAuthAccount(toCodexAccountDisplayCache(JSON.parse(JSON.stringify(cached)))), false);
  assert.doesNotMatch(JSON.stringify(cached), /fixture-/);
  assert.equal(base.tokens.access_token, 'fixture-access');

  const pending = toCodexAccountDisplayCache({ ...base, tokens: { id_token: '', access_token: '' } });
  assert.equal(isCodexPendingOAuthAccount(pending), true);
  const agent = toCodexAccountDisplayCache({ ...base, agent_identity: {
    agent_runtime_id: 'agent', agent_private_key: 'fixture-agent', account_id: 'account', chatgpt_user_id: 'user',
  } });
  assert.equal(agent.agent_identity?.agent_runtime_id, 'agent');
  assert.doesNotMatch(JSON.stringify(agent), /fixture-/);
});
