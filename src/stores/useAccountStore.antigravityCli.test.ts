import assert from 'node:assert/strict';
import test from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Account } from '../types/account';
import * as accountService from '../services/accountService';
import { normalizeAntigravityAccountTarget, normalizeAntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';

const account = (id: string): Account => ({
  id, email: `${id}@example.com`, created_at: 0, last_used: 0,
  token: { access_token: 'fake-access', refresh_token: `fake-${id}`, expires_in: 3600, expiry_timestamp: 0, token_type: 'Bearer' },
});

test('CLI account target does not become an IDE navigation target', () => {
  assert.equal(normalizeAntigravityAccountTarget('antigravity_cli'), 'antigravity_cli');
  assert.equal(normalizeAntigravityAccountTarget('antigravity'), 'antigravity');
  assert.equal(normalizeAntigravityRuntimeTarget('antigravity_cli'), 'antigravity_ide');
});

test('CLI switch, import, logout and keyring errors preserve desktop selection', async () => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {} });
  const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: () => null, setItem: () => {}, removeItem: () => {},
  } });
  const { useAccountStore: store } = await import('./useAccountStore');
  const desktop = account('desktop');
  const cli = account('cli');
  const calls: Array<{ cmd: string; payload: Record<string, unknown> }> = [];
  let current: Account | null | Promise<Account | null> = cli;
  let keyringError = false;
  try {
    mockIPC((cmd, raw) => {
      const payload = (raw ?? {}) as Record<string, unknown>;
      calls.push({ cmd, payload });
      if (cmd === 'list_accounts') return [desktop, cli];
      if (cmd === 'switch_account' || cmd === 'import_from_local' || cmd === 'fetch_account_quota') return cli;
      if (cmd === 'get_current_account') {
        if (keyringError) throw new Error('Keyring locked');
        return current;
      }
      return null;
    });
    store.setState({ accounts: [desktop, cli], currentAccount: desktop,
      currentAccountsByTarget: { antigravity: desktop, antigravity_ide: desktop, antigravity_cli: null } });
    await store.getState().switchAccount(cli.id, 'antigravity_cli');
    assert.equal(store.getState().currentAccountsByTarget.antigravity_cli?.id, cli.id);
    assert.equal(store.getState().currentAccount?.id, desktop.id);
    assert.equal(store.getState().currentAccountsByTarget.antigravity_ide?.id, desktop.id);
    assert.deepEqual(calls.find((call) => call.cmd === 'switch_account')?.payload,
      { accountId: cli.id, runtimeTarget: 'antigravity_cli' });
    assert.ok(!calls.some((call) => call.cmd === 'plugin:event|emit'), 'CLI does not emit a desktop switch');

    await accountService.importFromLocal('antigravity_cli');
    assert.deepEqual(calls.find((call) => call.cmd === 'import_from_local')?.payload,
      { runtimeTarget: 'antigravity_cli' });
    await store.getState().refreshQuota(cli.id, 'antigravity_cli');
    assert.equal(store.getState().currentAccount?.id, desktop.id, 'CLI quota refresh preserves desktop selection');
    await new Promise((resolve) => setTimeout(resolve, 120));
    // A logout must clear even a populated UI cache; a missing CLI account must
    // never fall back to the global IDE account.
    current = null;
    await store.getState().fetchCurrentAccount('antigravity_cli');
    assert.equal(store.getState().currentAccountsByTarget.antigravity_cli, null);
    assert.equal(store.getState().currentAccount?.id, desktop.id);

    await new Promise((resolve) => setTimeout(resolve, 120));
    store.setState({ currentAccountsByTarget: { antigravity: desktop, antigravity_ide: desktop, antigravity_cli: cli } });
    keyringError = true;
    await store.getState().fetchCurrentAccount('antigravity_cli');
    assert.equal(store.getState().currentAccountsByTarget.antigravity_cli, null);
    assert.equal(store.getState().currentAccountsByTarget.antigravity?.id, desktop.id);

    await new Promise((resolve) => setTimeout(resolve, 120));
    keyringError = false;
    let resolveStale!: (value: Account | null) => void;
    current = new Promise<Account | null>((resolve) => { resolveStale = resolve; });
    const staleRead = store.getState().fetchCurrentAccount('antigravity_cli');
    await store.getState().switchAccount(cli.id, 'antigravity_cli');
    resolveStale(null);
    await staleRead;
    assert.equal(store.getState().currentAccountsByTarget.antigravity_cli?.id, cli.id,
      'late reads cannot overwrite a completed CLI switch');
  } finally {
    clearMocks();
    if (descriptor) Object.defineProperty(globalThis, 'window', descriptor);
    else Reflect.deleteProperty(globalThis, 'window');
    if (storageDescriptor) Object.defineProperty(globalThis, 'localStorage', storageDescriptor);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  }
});
