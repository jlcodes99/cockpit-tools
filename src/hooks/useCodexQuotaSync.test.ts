import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('reloads saved import quota results and ignores unrelated account events', () => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  const stopped: string[] = [];
  let reads = 0;
  const hook = loadHookModule(new URL('./useCodexQuotaSync.ts', import.meta.url), {
    '../utils/durableUiSettings': { hydrateDurableUiSettings: () => Promise.resolve() },
    '@tauri-apps/api/core': { invoke: () => Promise.resolve() },
    '../stores/useCodexAccountStore': {
      useCodexAccountStore: { getState: () => ({ fetchAccounts: async () => { reads++; } }) },
    },
    '../utils/currentAccountRefresh': { loadAccountRefreshOverrides: () => undefined },
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts:changed' },
    '../utils/tauriEventListener': {
      subscribeTauriEvent(name: string, handler: (event: { payload: unknown }) => void) {
        listeners.set(name, handler);
        return () => { stopped.push(name); };
      },
    },
  }, { window: new EventTarget() });
  hook.render(() => hook.exports.useCodexQuotaSync());
  const onAccountsChanged = listeners.get('accounts:changed')!;
  onAccountsChanged({ payload: { platformId: 'codex', reason: 'import' } });
  onAccountsChanged({ payload: { platformId: 'antigravity', reason: 'import-quota-refresh' } });
  assert.equal(reads, 0);
  onAccountsChanged({ payload: { platformId: 'codex', reason: 'import-quota-refresh' } });
  assert.equal(reads, 1);
  hook.unmount();
  assert.ok(stopped.includes('accounts:changed'));
});
