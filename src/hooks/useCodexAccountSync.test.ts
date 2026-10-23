import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

test('rapid account renders retain exactly two native subscriptions and dispatch to the latest callbacks', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const listeners = new Map<string, (event: any) => void>();
  const calls: string[] = []; const stopped: string[] = []; let registrations = 0;
  const hook = loadHookModule(new URL('./useCodexAccountSync.ts', import.meta.url), {
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts', CURRENT_ACCOUNT_CHANGED_EVENT: 'current' },
    '../utils/tauriEventListener': { subscribeTauriEvent(name: string, handler: (event: any) => void) { registrations++; listeners.set(name, handler); return () => { stopped.push(name); }; } },
  });
  let revision = 0;
  const render = () => {
    const version = revision;
    return hook.exports.useCodexAccountSync({ fetchAccounts: async () => { calls.push(`list:${version}`); },
      fetchCurrentAccount: async () => { calls.push(`current:${version}`); },
      refreshApiKeyUsageByAccountId: async (id: string, options: any) => { assert.equal(options.force, false); calls.push(`usage:${id}:${version}`); } });
  };
  hook.render(render);
  for (revision = 1; revision <= 50; revision++) hook.flush();
  assert.equal(registrations, 2);
  listeners.get('current')!({ payload: { platformId: 'codex', accountId: 'a', reason: 'switch' } });
  listeners.get('accounts')!({ payload: { platformId: 'codex', reason: 'client-auth-observation' } });
  context.mock.timers.tick(250);
  await settlePromises();
  assert.deepEqual(calls, ['usage:a:50', 'list:50', 'current:50']);
  for (const payload of [{ platformId: 'grok', accountId: 'a' }, { platformId: 'codex', accountId: 'a', reason: 'delete' }]) {
    listeners.get('accounts')!({ payload }); listeners.get('current')!({ payload });
  }
  hook.unmount(); listeners.get('current')!({ payload: { platformId: 'codex', accountId: 'a' } }); await settlePromises();
  assert.equal(calls.length, 3); assert.deepEqual(stopped, ['accounts', 'current']);
});

test('bursty auth observations share one native reload and one later refresh, waiting for both reads and stopping on unmount', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const listeners = new Map<string, (event: any) => void>();
  const lists: ReturnType<typeof deferred<void>>[] = [], currents: ReturnType<typeof deferred<void>>[] = [];
  const hook = loadHookModule(new URL('./useCodexAccountSync.ts', import.meta.url), {
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts', CURRENT_ACCOUNT_CHANGED_EVENT: 'current' },
    '../utils/tauriEventListener': { subscribeTauriEvent(name: string, handler: (event: any) => void) { listeners.set(name, handler); return () => {}; } },
  }, { console: { warn() {} } });
  hook.render(() => hook.exports.useCodexAccountSync({
    fetchAccounts: () => { const task = deferred<void>(); lists.push(task); return task.promise; },
    fetchCurrentAccount: () => { const task = deferred<void>(); currents.push(task); return task.promise; },
    refreshApiKeyUsageByAccountId: async () => {},
  }));
  const emit = () => listeners.get('accounts')!({ payload: { platformId: 'codex', reason: 'client-auth-launch' } });
  for (let i = 0; i < 100; i++) emit();
  context.mock.timers.tick(250); assert.equal(lists.length, 1); assert.equal(currents.length, 1);
  for (let i = 0; i < 100; i++) emit();
  lists[0].reject(new Error('list failed')); await settlePromises(); context.mock.timers.tick(1000);
  assert.equal(lists.length, 1, 'failed list must not release the slot while current-account read is still running');
  currents[0].resolve(); await settlePromises(); context.mock.timers.tick(250);
  assert.equal(lists.length, 2); assert.equal(currents.length, 2);
  emit(); hook.unmount(); lists[1].resolve(); currents[1].resolve(); await settlePromises();
  context.mock.timers.tick(1000); assert.equal(lists.length, 2);
});

test('hidden windows postpone authoritative reloads until visible and cancel a queued reload on unmount', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const document = Object.assign(new EventTarget(), { visibilityState: 'hidden' });
  let listener: ((event: any) => void) | undefined; let reads = 0;
  const hook = loadHookModule(new URL('./useCodexAccountSync.ts', import.meta.url), {
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts', CURRENT_ACCOUNT_CHANGED_EVENT: 'current' },
    '../utils/tauriEventListener': { subscribeTauriEvent(name: string, handler: (event: any) => void) { if (name === 'accounts') listener = handler; return () => {}; } },
  }, { document });
  hook.render(() => hook.exports.useCodexAccountSync({ fetchAccounts: async () => { reads++; },
    fetchCurrentAccount: async () => {}, refreshApiKeyUsageByAccountId: async () => {} }));
  listener!({ payload: { platformId: 'codex', reason: 'client-auth-launch' } });
  context.mock.timers.tick(1000); assert.equal(reads, 0);
  document.visibilityState = 'visible'; document.dispatchEvent(new Event('visibilitychange'));
  context.mock.timers.tick(250); await settlePromises(); assert.equal(reads, 1);
  listener!({ payload: { platformId: 'codex', reason: 'client-auth-observation' } }); hook.unmount();
  context.mock.timers.tick(1000); await settlePromises(); assert.equal(reads, 1);
});
