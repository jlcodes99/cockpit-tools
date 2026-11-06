import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { findCodexModelProviderByBaseUrl, findCodexModelProviderById, type CodexModelProvider } from './codexModelProviderService';
import type { CodexAccount } from '../types/codex';

function harness(usageApiKey?: string) {
  let providers: CodexModelProvider[] = [{ id: 'official-provider', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', supportsWebsockets: false, createdAt: 1, updatedAt: 1,
    apiKeys: [{ id: 'k', name: '', apiKey: 'model-key', usageApiKey, usageKeyVersion: 1, createdAt: 1, updatedAt: 1 }] }];
  const storage = new Map<string, string>();
  const calls: any[] = [];
  let query: () => Promise<unknown> = async () => ({ mode: 'openai_official', totalCost: 10 });
  const h = loadHookModule(new URL('./codexApiKeyUsageRefreshService.ts', import.meta.url), {
    './codexModelProviderService': {
      listCodexModelProviders: async () => structuredClone(providers), findCodexModelProviderById, findCodexModelProviderByBaseUrl,
      queryCodexModelProviderUsage(input: unknown) { calls.push(input); return query(); },
    },
  }, { localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) }, window: new EventTarget(), CustomEvent });
  const api = h.exports as typeof import('./codexApiKeyUsageRefreshService');
  const account = { id: 'account', auth_mode: 'apikey', api_provider_mode: 'openai_builtin', openai_api_key: 'model-key', api_provider_id: 'openai_official' } as CodexAccount;
  return { api, account, calls, query(fn: typeof query) { query = fn; }, changeKey(key?: string) { providers[0].apiKeys[0].usageApiKey = key; providers[0].apiKeys[0].usageKeyVersion!++; } };
}

test('legacy official accounts without a query key never issue an organization request', async () => {
  const h = harness();
  await h.api.refreshCodexApiKeyUsageForAccounts([h.account], { force: true });
  assert.equal(h.calls.length, 0);
  assert.deepEqual(Object.keys(h.api.readCodexApiKeyUsageCache()), []);
});

test('built-in accounts resolve the official provider even with no stored base URL', async () => {
  const h = harness('admin-key');
  await h.api.refreshCodexApiKeyUsageForAccounts([h.account]);
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0].apiKey, 'model-key');
  assert.equal(h.calls[0].usageApiKey, 'admin-key');
  assert.equal(h.api.readCodexApiKeyUsageCache().account.sourceBinding, 'official-provider:k:1:costs-v2');
});

test('a slow query cannot restore statistics after its configuration is removed', async () => {
  const h = harness('admin-key');
  const gate = deferred<unknown>(); h.query(() => gate.promise);
  const pending = h.api.refreshCodexApiKeyUsageForAccounts([h.account]);
  await settlePromises();
  h.changeKey(); gate.resolve({ mode: 'openai_official', totalCost: 99 }); await pending;
  assert.deepEqual(Object.keys(h.api.readCodexApiKeyUsageCache()), []);
});

test('a failed new query never relabels previous organization statistics as current', async () => {
  const h = harness('admin-old');
  await h.api.refreshCodexApiKeyUsageForAccounts([h.account]);
  h.changeKey('admin-new'); h.query(async () => { throw new Error('OPENAI_USAGE_NETWORK_FAILED'); });
  await h.api.refreshCodexApiKeyUsageForAccounts([h.account]);
  const state = h.api.readCodexApiKeyUsageCache().account;
  assert.equal(state.summary, undefined);
  assert.equal(state.sourceBinding, 'official-provider:k:2:costs-v2');
  assert.equal(state.error, 'OPENAI_USAGE_NETWORK_FAILED');
});

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
