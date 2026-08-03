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

function jwt(payload: Record<string, unknown>): string {
  const encode = (value: Record<string, unknown>) =>
    Buffer.from(JSON.stringify(value)).toString('base64url');
  return `${encode({ alg: 'none', typ: 'JWT' })}.${encode(payload)}.`;
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

test('sub2api OAuth export preserves official expiry and login-provider fields', () => {
  const accessToken = jwt({
    exp: 1_800_000_000,
    'https://api.openai.com/auth': {
      chatgpt_account_id: 'account-token',
      chatgpt_user_id: 'user-token',
      poid: 'org-token',
      chatgpt_plan_type: 'plus',
      chatgpt_subscription_active_until: '2090-01-01T00:00:00Z',
    },
  });
  const idToken = jwt({
    auth_provider: 'google',
  });
  const account: CodexAccount = {
    id: 'codex-token-fixture',
    email: 'token@example.com',
    account_id: 'account-token',
    plan_type: 'plus',
    subscription_active_until: '2027-01-02T03:04:05+00:00',
    tokens: {
      id_token: idToken,
      access_token: accessToken,
      refresh_token: 'refresh-token-fixture',
    },
    created_at: 1,
    last_used: 1,
  };

  const exported = JSON.parse(
    transformCodexExportJson(JSON.stringify([account]), 'sub2api'),
  ) as {
    accounts: Array<{
      type: string;
      credentials: Record<string, unknown>;
      extra?: Record<string, unknown>;
      expires_at?: number;
      concurrency: number;
      priority: number;
    }>;
  };
  const item = exported.accounts[0];

  assert.equal(item.type, 'oauth');
  assert.equal(item.credentials.client_id, 'app_EMoamEEZ73f0CkXaXp7hrann');
  assert.equal(item.credentials.expires_at, '2027-01-15T08:00:00.000Z');
  assert.equal(item.credentials.subscription_expires_at, '2027-01-02T03:04:05.000Z');
  assert.equal(item.credentials.chatgpt_user_id, 'user-token');
  assert.equal(item.credentials.organization_id, 'org-token');
  assert.equal(item.extra?.auth_provider, 'google');
  assert.equal(item.expires_at, undefined);
  assert.equal(item.concurrency, 3);
  assert.equal(item.priority, 50);
});

test('sub2api access-token-only export auto-pauses at token expiry', () => {
  const account: CodexAccount = {
    id: 'codex-at-only',
    email: 'at-only@example.com',
    tokens: {
      id_token: '',
      access_token: jwt({ exp: 1_800_000_000 }),
    },
    created_at: 1,
    last_used: 1,
  };

  const exported = JSON.parse(
    transformCodexExportJson(JSON.stringify([account]), 'sub2api'),
  ) as {
    accounts: Array<{
      expires_at?: number;
      auto_pause_on_expired?: boolean;
    }>;
  };

  assert.equal(exported.accounts[0].expires_at, 1_800_000_000);
  assert.equal(exported.accounts[0].auto_pause_on_expired, true);
});

test('sub2api API Key export uses native apikey credentials', () => {
  const account: CodexAccount = {
    id: 'codex-api-key',
    email: 'API Key',
    auth_mode: 'apikey',
    openai_api_key: 'sk-test',
    api_base_url: 'https://relay.example.com/v1',
    tokens: {
      id_token: '',
      access_token: '',
    },
    created_at: 1,
    last_used: 1,
  };

  const exported = JSON.parse(
    transformCodexExportJson(JSON.stringify([account]), 'sub2api'),
  ) as {
    accounts: Array<{ type: string; credentials: Record<string, unknown> }>;
  };

  assert.equal(exported.accounts[0].type, 'apikey');
  assert.deepEqual(exported.accounts[0].credentials, {
    base_url: 'https://relay.example.com/v1',
    api_key: 'sk-test',
  });
});
