import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('instance identity refresh reads once per event and unsubscribes on unmount', () => {
  let reads = 0;
  let cache: Record<string, unknown> = {};
  const listeners = new Map<string, () => void>();
  const h = loadHookModule(new URL('./useCodexRemoteUsageCache.ts', import.meta.url), {
    '../services/codexApiKeyUsageRefreshService': {
      CODEX_API_KEY_USAGE_REFRESHED_EVENT: 'usage-refreshed',
      readCodexApiKeyUsageCache: () => { reads++; return cache; },
    },
  }, { window: {
    addEventListener: (name: string, callback: () => void) => listeners.set(name, callback),
    removeEventListener: (name: string) => listeners.delete(name),
  } });
  h.render(() => h.exports.useCodexRemoteUsageCache());
  const initial = reads;
  for (let i = 0; i < 50; i++) h.flush();
  assert.equal(reads, initial);
  cache = { gateway: { summary: { cockpit: { product: 'cockpit-tools' } } } };
  listeners.get('usage-refreshed')!();
  assert.equal(h.flush(), cache);
  assert.equal(reads, initial + 1);
  h.unmount();
  assert.equal(listeners.size, 0);
});
