import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex.ts';
import {
  resolveInstanceEffectiveBindAccountId,
  resolveLaunchPreviewRoutingBaseAccount,
  resolveLaunchPreviewRoutingBindAccountId,
} from './codexLaunchPreviewRoutingBinding.ts';

function oauthAccount(id: string): CodexAccount {
  return {
    id,
    email: `${id}@example.com`,
    tokens: {
      id_token: 'id-token',
      access_token: 'access-token',
      refresh_token: 'refresh-token',
    },
    created_at: 1,
    last_used: 1,
  } as CodexAccount;
}

function apiKeyAccount(id: string): CodexAccount {
  return {
    id,
    email: `${id}@example.com`,
    auth_mode: 'apikey',
    openai_api_key: 'sk-test',
    created_at: 1,
    last_used: 1,
  } as CodexAccount;
}

test('routing base account prefers the account being previewed', () => {
  const oauth = oauthAccount('oauth-base');
  const apiKey = apiKeyAccount('api-key');
  assert.equal(resolveLaunchPreviewRoutingBaseAccount([oauth, apiKey], oauth)?.id, 'oauth-base');
});

test('routing base account falls back to another OAuth account for API key previews', () => {
  const oauth = oauthAccount('oauth-base');
  const otherOauth = oauthAccount('other-oauth');
  const apiKey = apiKeyAccount('api-key');
  assert.equal(
    resolveLaunchPreviewRoutingBaseAccount([oauth, otherOauth, apiKey], apiKey)?.id,
    'oauth-base',
  );
  // 实例当前绑定已经是可用的 OAuth 底座时优先保持不动，避免顺手改掉用户的底座账号。
  assert.equal(
    resolveLaunchPreviewRoutingBaseAccount([oauth, otherOauth, apiKey], apiKey, 'other-oauth')?.id,
    'other-oauth',
  );
  // 预览账号本身是 OAuth 时，仍然以预览账号为准。
  assert.equal(
    resolveLaunchPreviewRoutingBaseAccount([oauth, otherOauth], otherOauth, 'oauth-base')?.id,
    'other-oauth',
  );
  assert.equal(resolveLaunchPreviewRoutingBaseAccount([apiKey], apiKey), null);
});

test('instance effective bind follows the local account for default instances', () => {
  assert.equal(
    resolveInstanceEffectiveBindAccountId({
      isDefaultInstance: true,
      followLocalAccount: true,
      bindAccountId: 'stale-bind',
      localCurrentAccountId: 'local-current',
    }),
    'local-current',
  );
  assert.equal(
    resolveInstanceEffectiveBindAccountId({
      isDefaultInstance: false,
      followLocalAccount: true,
      bindAccountId: 'instance-bind',
      localCurrentAccountId: 'local-current',
    }),
    'instance-bind',
  );
  assert.equal(
    resolveInstanceEffectiveBindAccountId({
      isDefaultInstance: true,
      followLocalAccount: false,
      bindAccountId: null,
      localCurrentAccountId: 'local-current',
    }),
    null,
  );
});

test('routing bind is submitted when the instance is not bound to the routing account', () => {
  // 用户场景：实例当前绑定 API Key / API 服务，预览的是 OAuth 订阅账号。
  assert.equal(
    resolveLaunchPreviewRoutingBindAccountId({
      routingEnabled: true,
      routingBaseAccountId: 'oauth-base',
      instanceEffectiveBindAccountId: '__api_service__',
    }),
    'oauth-base',
  );
  assert.equal(
    resolveLaunchPreviewRoutingBindAccountId({
      routingEnabled: true,
      routingBaseAccountId: 'oauth-base',
      instanceEffectiveBindAccountId: '__provider_gateway__:api-key',
    }),
    'oauth-base',
  );
});

test('routing bind stays untouched when it already matches or routing is off', () => {
  assert.equal(
    resolveLaunchPreviewRoutingBindAccountId({
      routingEnabled: true,
      routingBaseAccountId: 'oauth-base',
      instanceEffectiveBindAccountId: 'oauth-base',
    }),
    undefined,
  );
  assert.equal(
    resolveLaunchPreviewRoutingBindAccountId({
      routingEnabled: true,
      routingBaseAccountId: 'oauth-base',
      instanceEffectiveBindAccountId: ' oauth-base ',
    }),
    undefined,
  );
  assert.equal(
    resolveLaunchPreviewRoutingBindAccountId({
      routingEnabled: false,
      routingBaseAccountId: 'oauth-base',
      instanceEffectiveBindAccountId: '__api_service__',
    }),
    undefined,
  );
  assert.equal(
    resolveLaunchPreviewRoutingBindAccountId({
      routingEnabled: true,
      routingBaseAccountId: null,
      instanceEffectiveBindAccountId: '__api_service__',
    }),
    undefined,
  );
});
