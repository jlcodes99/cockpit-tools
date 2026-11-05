import assert from 'node:assert/strict';
import test from 'node:test';
import { isOpenAIOfficialUsageAccount, isOpenAIOfficialUsageUrl, resolveAccountUsageBaseUrl, openAIUsageErrorKey } from './openaiOfficialUsage';
import { shouldShowCodexApiKeyUsagePanel, isCodexApiKeyUsageQueryEligible } from './codexDeepSeekAccess';
import type { CodexAccount } from '../types/codex';

test('OpenAI Official uses the default URL even when the stored built-in account omits it', () => {
  const account = { api_provider_mode: 'openai_builtin' as const };
  assert.equal(resolveAccountUsageBaseUrl(account), 'https://api.openai.com/v1');
  assert.equal(isOpenAIOfficialUsageAccount(account), true);
  assert.equal(resolveAccountUsageBaseUrl({ api_provider_mode: 'custom' }), '');
  assert.equal(isOpenAIOfficialUsageAccount({ ...account, api_provider_id: 'cockpit_chatgpt_plan' }), false);
});

test('an explicit upstream and other providers retain their query destination', () => {
  const account = { api_provider_mode: 'openai_builtin' as const, api_base_url: 'https://relay.example/v1' };
  assert.equal(resolveAccountUsageBaseUrl(account), account.api_base_url);
  assert.equal(isOpenAIOfficialUsageAccount(account), false);
  for (const url of ['https://api.openai.com.example/v1', 'http://api.openai.com/v1', 'https://user@api.openai.com/v1', 'https://api.openai.com/other', 'https://api.openai.com/v1?secret=x']) {
    assert.equal(isOpenAIOfficialUsageUrl(url), false, url);
  }
});

test('permissions, invalid credentials and network failures have different user guidance', () => {
  assert.equal(openAIUsageErrorKey('OPENAI_USAGE_PERMISSION_REQUIRED'), 'codex.openaiUsage.permissionRequired');
  assert.equal(openAIUsageErrorKey('OPENAI_USAGE_INVALID_KEY'), 'codex.openaiUsage.invalidKey');
  assert.equal(openAIUsageErrorKey('OPENAI_USAGE_NETWORK_FAILED'), 'codex.openaiUsage.networkFailed');
  assert.equal(openAIUsageErrorKey('OPENAI_USAGE_HTTP_500'), 'codex.openaiUsage.queryFailed');
});

test('hiding relay quota does not hide an OpenAI Official account or prevent its refresh', () => {
  const account = { id: 'official', auth_mode: 'apikey', api_provider_mode: 'openai_builtin', openai_api_key: 'synthetic' } as CodexAccount;
  assert.equal(shouldShowCodexApiKeyUsagePanel(account, true), true);
  assert.equal(isCodexApiKeyUsageQueryEligible(account), true);
});

test('query configuration matches the exact model key and only the official endpoint', async () => {
  const { resolveOpenAIUsageConfig } = await import('./openaiOfficialUsage');
  const provider = { id: 'p', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', supportsWebsockets: false, createdAt: 1, updatedAt: 1,
    apiKeys: [{ id: 'a', name: '', apiKey: 'model-a', usageApiKey: 'admin-a', usageKeyVersion: 2, createdAt: 1, updatedAt: 1 },
      { id: 'b', name: '', apiKey: 'model-b', createdAt: 1, updatedAt: 1 }] };
  assert.deepEqual(resolveOpenAIUsageConfig(provider, ' model-a '), { apiKey: 'admin-a', binding: 'p:a:2:costs-v2' });
  assert.equal(resolveOpenAIUsageConfig(provider, 'model-b'), null);
  assert.equal(resolveOpenAIUsageConfig(provider, 'model-c'), null);
  assert.equal(resolveOpenAIUsageConfig({ ...provider, baseUrl: 'https://relay.example/v1' }, 'model-a'), null);
});
