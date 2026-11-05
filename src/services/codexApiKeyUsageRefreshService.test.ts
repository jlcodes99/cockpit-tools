import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('codex-proxy-rs usage follows existing refresh eligibility and unavailable cache rules', async () => {
  const oldSummary = { mode: 'codex_proxy_rs', details: [{ key: 'weekly.remaining', value: '246.90320374' }] };
  let cache: any = { account: { summary: oldSummary, updatedAt: 123, unavailable: true } };
  let fail = true;
  let queryCount = 0;
  const provider = { id: 'provider', baseUrl: 'https://example.com/v1', integrationType: 'codex_proxy_rs' };
  const service = loadHookModule(new URL('./codexApiKeyUsageRefreshService.ts', import.meta.url), {
    './codexModelProviderService': {
      listCodexModelProviders: async () => [provider],
      findCodexModelProviderById: () => provider,
      findCodexModelProviderByBaseUrl: () => provider,
      queryCodexModelProviderUsage: async (input: any) => {
        queryCount++;
        assert.equal(input.integrationType, 'codex_proxy_rs');
        if (fail) throw new Error('PROVIDER_USAGE_HTTP_401');
        return { ...oldSummary, latencyMs: 1, modelStatsCount: 0 };
      },
    },
  }, {
    localStorage: { getItem: () => JSON.stringify(cache), setItem: (_key: string, value: string) => { cache = JSON.parse(value); } },
    window: { dispatchEvent() {} }, CustomEvent: class {},
  }).exports as typeof import('./codexApiKeyUsageRefreshService');
  const accounts: any[] = [{ id: 'account', auth_mode: 'apikey', api_provider_id: 'provider',
    api_base_url: provider.baseUrl, openai_api_key: 'fixture-key', api_wire_api: 'chat_completions' }];
  await service.refreshCodexApiKeyUsageForAccounts(accounts);
  assert.equal(queryCount, 0);
  accounts[0].api_wire_api = 'responses';
  await service.refreshCodexApiKeyUsageForAccounts(accounts);
  assert.equal(queryCount, 0);
  await service.refreshCodexApiKeyUsageForAccounts(accounts, { force: true });
  assert.equal(queryCount, 1);
  assert.ok(cache.account.updatedAt > 123);
  assert.equal(cache.account.summary.details[0].value, '246.90320374');
  assert.equal(cache.account.error, 'PROVIDER_USAGE_HTTP_401');
  assert.equal(cache.account.unavailable, false);
  fail = false;
  await service.refreshCodexApiKeyUsageForAccounts(accounts);
  assert.ok(cache.account.updatedAt > 123);
  assert.equal(cache.account.summary.mode, 'codex_proxy_rs');
  assert.equal(cache.account.error, undefined);
});
