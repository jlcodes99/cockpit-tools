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
