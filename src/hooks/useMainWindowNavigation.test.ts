import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { isMainWindowNavigablePage, type Page } from '../types/navigation';

function fixture() {
  const config = deferred<{ startup_page?: string }>();
  const pending = deferred<string | null>();
  const targets: Page[] = [];
  const defaults: (string | undefined)[] = [];
  const logs: string[] = [];
  const listeners = new Set<(event: { payload: string }) => void>();
  let pendingReads = 0;
  const hook = loadHookModule(new URL('./useMainWindowNavigation.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke(command: string) {
      if (command === 'get_general_config') return config.promise;
      assert.equal(command, 'main_window_take_pending_navigation');
      pendingReads++;
      return pending.promise;
    } },
    '../types/navigation': { isMainWindowNavigablePage },
    '../utils/codexRequestLogsNavigation': { requestCodexRequestLogsNavigation(target: string) {
      if (target !== 'codex-request-logs') return false;
      logs.push(target);
      return true;
    } },
    '../utils/tauriEventListener': { subscribeTauriEvent(
      event: string, listener: (event: { payload: string }) => void,
    ) {
      assert.equal(event, 'tray:navigate');
      listeners.add(listener);
      return () => listeners.delete(listener);
    } },
  });
  const setPage = (page: Page) => targets.push(page);
  const mount = () => {
    const applyStartup = (preference?: string) => defaults.push(preference);
    return hook.render(() => hook.exports.useMainWindowNavigation(setPage, applyStartup)) as (page: Page) => void;
  };
  return {
    config, pending, targets, defaults, logs, listeners, mount, hook,
    pendingReads: () => pendingReads,
    tray: (payload: string) => listeners.forEach((listener) => listener({ payload })),
  };
}

test('a delayed fixed startup page cannot override a restored request-log target', async () => {
  const h = fixture();
  h.mount();
  h.pending.resolve('codex-request-logs');
  await settlePromises();
  h.config.resolve({ startup_page: 'overview' });
  await settlePromises();
  assert.deepEqual(h.targets, ['codex-api-service']);
  assert.deepEqual(h.logs, ['codex-request-logs']);
  assert.deepEqual(h.defaults, []);
  h.hook.unmount();
});

test('an explicit click or newer tray command wins over both pending startup reads', async () => {
  for (const source of ['click', 'tray']) {
    const h = fixture();
    const navigate = h.mount();
    if (source === 'click') navigate('dashboard');
    else h.tray('settings');
    h.config.resolve({ startup_page: 'overview' });
    h.pending.resolve('codex-request-logs');
    await settlePromises();
    assert.deepEqual(h.targets, [source === 'click' ? 'dashboard' : 'settings']);
    assert.deepEqual(h.logs, []);
    assert.deepEqual(h.defaults, []);
    h.hook.unmount();
  }
});

test('startup preference still applies when there is no explicit navigation', async () => {
  const h = fixture();
  h.mount();
  h.tray('invalid-page');
  h.pending.resolve(null);
  h.config.resolve({ startup_page: 'overview' });
  await settlePromises();
  assert.deepEqual(h.defaults, ['overview']);
  assert.deepEqual(h.targets, []);
  h.hook.unmount();
});

test('effect reinitialization reuses the consumed target and cleans up listeners', async () => {
  const h = fixture();
  h.mount();
  h.mount();
  assert.equal(h.pendingReads(), 1);
  assert.equal(h.listeners.size, 1);
  h.pending.resolve('settings');
  await settlePromises();
  assert.deepEqual(h.targets, ['settings']);
  h.hook.unmount();
  assert.equal(h.listeners.size, 0);
  h.config.resolve({ startup_page: 'overview' });
  await settlePromises();
  assert.deepEqual(h.defaults, []);
});
