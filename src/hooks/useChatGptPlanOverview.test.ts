import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { withChatGptPlanMembership } from '../services/chatgptPlanService';
import { mergeChatGptPlanOverviewAccounts } from '../utils/chatgptPlanOverview';
import type { CodexAccount } from '../types/codex';
const grant = { id: 'saved', label: 'saved', email: 'user@example.com', connected: true, needs_reauth: false, in_service: false };

test('pending metadata settles without a native event and retry work stops on unmount', async () => {
  const timers = new Map<number, () => void>(); let timerId = 0, reads = 0;
  const pending = { ...grant, source_account_id: 'source', source_binding_valid: false };
  const hook = loadHookModule(new URL('./useChatGptPlanOverview.ts', import.meta.url), {
    '../services/chatgptPlanService': {
      listChatGptPlanAccounts: async () => [{ ...pending, source_binding_valid: ++reads > 1 }],
      withChatGptPlanMembership, chatGptPlanErrorKey: () => 'error',
    },
    '../utils/tauriEventListener': { subscribeTauriEvent() { return () => {}; } },
  }, { window: new EventTarget(), setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; }, clearTimeout: (id: number) => timers.delete(id) });
  hook.render(() => hook.exports.useChatGptPlanOverview()); await settlePromises();
  assert.equal(hook.flush().accounts[0].source_binding_valid, false);
  assert.equal(timers.size, 1);
  const retry = [...timers.values()][0]; timers.clear(); retry(); await settlePromises();
  assert.equal(hook.flush().accounts[0].source_binding_valid, true);
  assert.equal(reads, 2); assert.equal(timers.size, 0);
  hook.unmount();
});

test('a stalled metadata worker has bounded reconciliation and keeps the original rows', async () => {
  const timers = new Map<number, () => void>(); let timerId = 0, reads = 0;
  const pending = { ...grant, source_account_id: 'source', source_binding_valid: false };
  const hook = loadHookModule(new URL('./useChatGptPlanOverview.ts', import.meta.url), {
    '../services/chatgptPlanService': { listChatGptPlanAccounts: async () => { reads++; return [pending]; }, withChatGptPlanMembership, chatGptPlanErrorKey: () => 'error' },
    '../utils/tauriEventListener': { subscribeTauriEvent() { return () => {}; } },
  }, { window: new EventTarget(), setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; }, clearTimeout: (id: number) => timers.delete(id) });
  hook.render(() => hook.exports.useChatGptPlanOverview()); await settlePromises(); hook.flush();
  for (let i = 0; i < 4; i++) { const retry = [...timers.values()][0]; assert.ok(retry); timers.clear(); retry(); await settlePromises(); hook.flush(); }
  const state = hook.flush();
  assert.equal(reads, 5); assert.equal(timers.size, 0);
  assert.equal(state.accounts[0].source_binding_valid, false);
  assert.equal(state.error, 'chatgptPlan.error.timeout');
  hook.unmount();
});

test('overview stays usable during grant loading and preserves saved rows across failure and retry', async () => {
  let pending = deferred<any>();
  let reads = 0;
  let members: string[] = [];
  const window = new EventTarget();
  const hook = loadHookModule(new URL('./useChatGptPlanOverview.ts', import.meta.url), {
    '../services/chatgptPlanService': {
      listChatGptPlanAccounts() { reads++; return pending.promise; },
      withChatGptPlanMembership, chatGptPlanErrorKey: () => 'chatgptPlan.error.timeout',
    },
    '../utils/tauriEventListener': { subscribeTauriEvent() { return () => {}; } },
  }, { window });
  let state = hook.render(() => hook.exports.useChatGptPlanOverview(members));
  const ordinary = { id: 'normal' } as CodexAccount;
  assert.deepEqual(mergeChatGptPlanOverviewAccounts([ordinary], state.accounts), [ordinary]);
  assert.equal(state.loading, true);
  pending.resolve([grant]); await settlePromises(); state = hook.flush();
  assert.equal(state.byId.has('chatgpt_plan_saved'), true);
  members = ['chatgpt_plan_saved']; state = hook.flush();
  assert.equal(state.accounts[0].in_service, true);
  state.open('chatgpt_plan_saved'); state = hook.flush();
  assert.equal(state.dialogAccount.id, 'saved');
  pending = deferred<any>();
  const failed = state.reload();
  pending.reject(new Error('timeout')); await failed; state = hook.flush();
  assert.equal(state.error, 'chatgptPlan.error.timeout');
  assert.equal(state.accounts.length, 1);
  assert.equal(state.loading, false);
  pending = deferred<any>();
  const retry = state.reload(); pending.resolve([{ ...grant, needs_reauth: true }]); await retry; state = hook.flush();
  assert.equal(state.error, ''); assert.equal(state.dialogAccount.needs_reauth, true);
  assert.equal(reads, 3);
  state.close(); assert.equal(hook.flush().dialogAccount, undefined);
  hook.unmount();
});

test('repeated updates coalesce into one read plus one refresh, and late unmounted reads are ignored', async () => {
  const window = new EventTarget();
  const first = deferred<any>();
  let reads = 0;
  const hook = loadHookModule(new URL('./useChatGptPlanOverview.ts', import.meta.url), {
    '../services/chatgptPlanService': { listChatGptPlanAccounts() { return ++reads === 1 ? first.promise : Promise.resolve([grant]); }, withChatGptPlanMembership, chatGptPlanErrorKey: () => 'error' },
    '../utils/tauriEventListener': { subscribeTauriEvent() { return () => {}; } },
  }, { window });
  hook.render(() => hook.exports.useChatGptPlanOverview());
  window.dispatchEvent(new Event('chatgpt-plan-accounts-changed'));
  window.dispatchEvent(new Event('chatgpt-plan-accounts-changed'));
  assert.equal(reads, 1);
  first.resolve([]); await settlePromises();
  assert.equal(reads, 2);
  assert.equal(hook.flush().accounts.length, 1);
  hook.unmount();
  window.dispatchEvent(new Event('chatgpt-plan-accounts-changed'));
  assert.equal(reads, 2);
});

test('background rechecks preserve display metadata without promoting pending authorization or retaining invalid links', async () => {
  let next: any[] = [{ ...grant, source_account_id: 'source', source_binding_valid: true, subject: 'subject', client_id: 'client' }];
  let failed = false;
  const hook = loadHookModule(new URL('./useChatGptPlanOverview.ts', import.meta.url), {
    '../services/chatgptPlanService': { listChatGptPlanAccounts: async () => { if (failed) throw new Error('temporary'); return next; }, withChatGptPlanMembership, chatGptPlanErrorKey: () => 'safe-error' },
    '../utils/tauriEventListener': { subscribeTauriEvent() { return () => {}; } },
  }, { window: new EventTarget() });
  hook.render(() => hook.exports.useChatGptPlanOverview(['chatgpt_plan_saved']));
  await settlePromises();
  let state = hook.flush();
  assert.equal(state.displaySourceIds.get('saved'), 'source');
  next = [{ ...next[0], source_binding_valid: false, source_binding_error: null }];
  await state.reload(); state = hook.flush();
  assert.equal(state.accounts[0].source_binding_valid, false);
  assert.equal(state.displaySourceIds.get('saved'), 'source');
  failed = true; await state.reload(); state = hook.flush();
  assert.equal(state.displaySourceIds.get('saved'), 'source');
  assert.equal(state.error, 'safe-error');
  failed = false; next = [{ ...next[0], source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }];
  await state.reload(); state = hook.flush();
  assert.equal(state.displaySourceIds.size, 0);
  next = [{ ...next[0], source_binding_error: null }];
  await state.reload();
  assert.equal(hook.flush().displaySourceIds.size, 0, 'an invalidated association cannot be revived from its old display cache');
  hook.unmount();
});
