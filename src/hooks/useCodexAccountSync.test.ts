import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

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
  assert.deepEqual(calls, ['usage:a:50']);
  for (const payload of [{ platformId: 'grok', accountId: 'a' }, { platformId: 'codex', accountId: 'a', reason: 'delete' }]) {
    listeners.get('accounts')!({ payload }); listeners.get('current')!({ payload });
  }
  hook.unmount(); listeners.get('current')!({ payload: { platformId: 'codex', accountId: 'a' } }); await settlePromises();
  assert.equal(calls.length, 1); assert.deepEqual(stopped, ['accounts', 'current']);
});


test('retired client-auth events do not refresh accounts or usage', async () => {
  const listeners = new Map<string, (event: any) => void>();
  let refreshes = 0;
  const hook = loadHookModule(new URL('./useCodexAccountSync.ts', import.meta.url), {
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts', CURRENT_ACCOUNT_CHANGED_EVENT: 'current' },
    '../utils/tauriEventListener': { subscribeTauriEvent(name: string, handler: (event: any) => void) { listeners.set(name, handler); return () => {}; } },
  });
  hook.render(() => hook.exports.useCodexAccountSync({
    refreshApiKeyUsageByAccountId: async () => { refreshes++; },
  }));
  for (const reason of ['client-auth-launch', 'client-auth-observation']) {
    for (const listener of listeners.values()) listener({ payload: { platformId: 'codex', accountId: 'fixture', reason } });
  }
  await settlePromises();
  assert.equal(refreshes, 0);
  listeners.get('accounts')!({ payload: { platformId: 'codex', accountId: 'fixture', reason: 'update' } });
  await settlePromises();
  assert.equal(refreshes, 1);
  hook.unmount();
});
