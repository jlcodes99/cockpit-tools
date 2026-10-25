import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('routing saves forward both mapping states and return the host-confirmed state', async () => {
  const calls: any[] = [];
  const state = { collection: { sessionIdentityMappingEnabled: false } };
  const h = loadHookModule(new URL('./codexLocalAccessService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (...args: unknown[]) => { calls.push(args); return state; } },
  });
  for (const enabled of [false, true]) {
    const payload = { sessionIdentityMappingEnabled: enabled, sessionAffinity: true, sessionAffinityTtlMs: 3600000,
      responsesWebsocketsEnabled: false, maxRetryCredentials: 0, maxRetryIntervalMs: 3000,
      disableCooling: false, immediateSseResponse: false, maxConcurrentImageRequests: 1,
      maxAccountConcurrency: 2, accountConcurrencyWaitMs: 120000 };
    assert.equal(await h.exports.updateCodexLocalAccessRoutingOptions(payload), state);
    assert.equal(calls[calls.length - 1][0], 'codex_local_access_update_routing_options');
    assert.equal(calls[calls.length - 1][1], payload);
    assert.equal(calls[calls.length - 1][1].sessionIdentityMappingEnabled, enabled);
  }
});

test('member saves preserve explicit false, while callers without this setting leave it unchanged', async () => {
  const calls: any[] = [];
  const h = loadHookModule(new URL('./codexLocalAccessService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (...args: unknown[]) => { calls.push(args); return {}; } },
  });
  for (const enabled of [false, true, undefined]) {
    await h.exports.saveCodexLocalAccessAccounts(['account'], false, [], [], true, 3600000, {}, {}, enabled);
    assert.equal(calls[calls.length - 1][0], 'codex_local_access_save_accounts');
    assert.equal(calls[calls.length - 1][1].sessionIdentityMappingEnabled, enabled ?? null);
    assert.equal(calls[calls.length - 1][1].sessionAffinity, true);
  }
});
