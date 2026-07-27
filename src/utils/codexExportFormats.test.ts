import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex.ts';
import {
  hasCodexExportAgentIdentity,
  transformCodexExportJson,
} from './codexExportFormats.ts';

function agentIdentityAccount(): CodexAccount {
  return {
    id: 'codex-agent-fixture',
    email: 'fixture@example.com',
    auth_mode: 'oauth',
    account_id: 'account-fixture',
    user_id: 'user-fixture',
    plan_type: 'plus',
    agent_identity: {
      agent_runtime_id: 'runtime-fixture',
      agent_private_key: 'private-key-fixture',
      task_id: 'task-fixture',
      account_id: 'account-fixture',
      chatgpt_user_id: 'user-fixture',
      email: 'fixture@example.com',
      plan_type: 'plus',
      chatgpt_account_is_fedramp: true,
    },
    tokens: {
      id_token: '',
      access_token: '',
      refresh_token: '',
    },
    created_at: 1,
    last_used: 1,
  };
}

test('Cockpit Tools export preserves portable Agent Identity credentials', () => {
  const raw = JSON.stringify([agentIdentityAccount()]);
  const exported = JSON.parse(
    transformCodexExportJson(raw, 'cockpit_tools'),
  ) as Array<Record<string, unknown>>;
  const account = exported[0];
  const identity = account.agent_identity as Record<string, unknown>;

  assert.equal(account.auth_mode, 'agentIdentity');
  assert.equal(account.type, 'codex');
  assert.equal(account.access_token, undefined);
  assert.equal(identity.agent_runtime_id, 'runtime-fixture');
  assert.equal(identity.agent_private_key, 'private-key-fixture');
  assert.equal(identity.task_id, 'task-fixture');
  assert.equal(identity.account_id, 'account-fixture');
  assert.equal(identity.chatgpt_user_id, 'user-fixture');
  assert.equal(identity.chatgpt_account_is_fedramp, true);
  assert.equal(hasCodexExportAgentIdentity(raw), true);
});

test('sub2api export preserves Agent Identity credentials', () => {
  const exported = JSON.parse(
    transformCodexExportJson(JSON.stringify([agentIdentityAccount()]), 'sub2api'),
  ) as { accounts: Array<{ credentials: Record<string, unknown> }> };
  const credentials = exported.accounts[0].credentials;

  assert.equal(credentials.auth_mode, 'agentIdentity');
  assert.equal(credentials.agent_runtime_id, 'runtime-fixture');
  assert.equal(credentials.agent_private_key, 'private-key-fixture');
  assert.equal(credentials.task_id, 'task-fixture');
  assert.equal(credentials.chatgpt_account_id, 'account-fixture');
  assert.equal(credentials.chatgpt_user_id, 'user-fixture');
});

test('CPA export rejects Agent Identity instead of producing empty tokens', () => {
  assert.throws(
    () => transformCodexExportJson(JSON.stringify([agentIdentityAccount()]), 'cpa'),
    /CPA format does not support Codex Agent Identity accounts/,
  );
});

test('regular token accounts keep their existing Cockpit Tools export shape', () => {
  const account: CodexAccount = {
    id: 'codex-token-fixture',
    email: 'token@example.com',
    account_id: 'account-token',
    tokens: {
      id_token: 'id-token-fixture',
      access_token: 'access-token-fixture',
      refresh_token: 'refresh-token-fixture',
    },
    created_at: 1,
    last_used: 1,
  };
  const exported = JSON.parse(
    transformCodexExportJson(JSON.stringify([account]), 'cockpit_tools'),
  ) as Array<Record<string, unknown>>;

  assert.equal(exported[0].access_token, 'access-token-fixture');
  assert.equal(exported[0].refresh_token, 'refresh-token-fixture');
  assert.equal(exported[0].agent_identity, undefined);
});
