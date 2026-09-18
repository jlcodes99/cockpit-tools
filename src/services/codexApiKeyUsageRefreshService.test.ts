import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import { invalidateCodexModelProviderCache } from './codexModelProviderService';
import {
  CODEX_API_KEY_USAGE_CACHE_KEY, CODEX_API_KEY_USAGE_REFRESHED_EVENT,
  readCodexApiKeyUsageCache, refreshCodexApiKeyUsageForAccounts,
} from './codexApiKeyUsageRefreshService';

test('Ainipy retries an old unavailable cache, refreshes it, and preserves balance when login expires', async () => {
  const descriptors = new Map(['window', 'localStorage'].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const storage = new Map<string, string>();
  const events: string[] = [];
  let expired = false;
  let queries = 0;
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    dispatchEvent: (event: Event) => { events.push(event.type); return true; },
    __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, unknown>) => {
      if (command === 'load_codex_model_providers') return '[]';
      if (command === 'codex_query_model_provider_usage') {
        queries++;
        assert.equal(args.baseUrl, 'https://www.ainipy.com/api/desktop/v1');
        assert.equal(args.apiKey, 'test-key');
        if (expired) throw new Error('AINIPY_LOGIN_REQUIRED');
        return { mode: 'ainipy', balance: 1234, unit: 'tokens', modelStatsCount: 0, latencyMs: 1 };
      }
      throw new Error(`Unexpected command: ${command}`);
    } },
  } });
  const account = { id: 'ainipy', auth_mode: 'apikey', api_base_url: 'https://www.ainipy.com/api/desktop/v1',
    openai_api_key: 'test-key', api_wire_api: 'responses' } as CodexAccount;
  invalidateCodexModelProviderCache();
  try {
    storage.set(CODEX_API_KEY_USAGE_CACHE_KEY, JSON.stringify({ ainipy: { unavailable: true, updatedAt: 1 } }));
    await refreshCodexApiKeyUsageForAccounts([account]);
    assert.equal(queries, 1);
    assert.equal(readCodexApiKeyUsageCache().ainipy.summary?.balance, 1234);
    assert.equal(readCodexApiKeyUsageCache().ainipy.unavailable, false);
    expired = true;
    await refreshCodexApiKeyUsageForAccounts([account]);
    const state = readCodexApiKeyUsageCache().ainipy;
    assert.equal(state.summary?.balance, 1234);
    assert.equal(state.error, 'AINIPY_LOGIN_REQUIRED');
    assert.equal(state.unavailable, false);
    assert.deepEqual(events, [CODEX_API_KEY_USAGE_REFRESHED_EVENT, CODEX_API_KEY_USAGE_REFRESHED_EVENT]);
  } finally {
    invalidateCodexModelProviderCache();
    for (const [key, descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
});
