import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import * as stats from '../utils/codexAccountWindowStats';
import type { CodexAccount } from '../types/codex';

test('overview stats refresh new requests periodically, ignore display-only rerenders, and isolate window changes', async (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const intervals = new Map<number, () => void>(); let intervalId = 0;
  const listeners = new Map<string, (event: any) => void>();
  const window = Object.assign(new EventTarget(), {
    setInterval(callback: () => void) { intervals.set(++intervalId, callback); return intervalId; },
    clearInterval(id: number) { intervals.delete(id); },
  });
  const document = Object.assign(new EventTarget(), { visibilityState: 'visible' });
  const reads: ReturnType<typeof deferred<any[]>>[] = [];
  const hook = loadHookModule(new URL('./useCodexAccountWindowStats.ts', import.meta.url), {
    '../utils/codexAccountWindowStats': { ...stats,
      createCodexAccountWindowStatsReader: (options: any) => stats.createCodexAccountWindowStatsReader({ ...options, minRefreshMs: 0 }),
    },
    '../services/codexLocalAccessService': { queryCodexLocalAccessAccountWindowStats() { const job = deferred<any[]>(); reads.push(job); return job.promise; } },
    '../utils/accountSyncEvents': { CURRENT_ACCOUNT_CHANGED_EVENT: 'current' },
    '../utils/tauriEventListener': { subscribeTauriEvent(name: string, handler: (event: any) => void) { listeners.set(name, handler); return () => listeners.delete(name); } },
  }, { window, document });
  const first = { id: 'a', quota: { hourly_percentage: 80, weekly_percentage: 90, hourly_reset_time: 2_000_000_000 } } as CodexAccount;
  let accounts = [first]; let active = true;
  const render = () => hook.exports.useCodexAccountWindowStats(active, accounts, ['a']);
  assert.equal(hook.render(render).ready, false);
  context.mock.timers.tick(250); assert.equal(reads.length, 1);
  const row = (count: number) => ({ accountId: 'a', windowKey: 'primary', requestCount: count, inputTokens: 100,
    cachedTokens: 50, outputTokens: 20, totalTokens: 120, estimatedCostUsd: 0.1 });
  reads[0].resolve([row(1)]); await settlePromises();
  assert.equal(hook.flush().byAccountId.a.primary.requestCount, 1);
  for (let i = 0; i < 10; i++) { accounts = [{ ...first, email: `display-${i}` }]; hook.flush(); }
  context.mock.timers.tick(1000); assert.equal(reads.length, 1);
  [...intervals.values()][0](); context.mock.timers.tick(250);
  reads[1].resolve([row(2)]); await settlePromises(); assert.equal(hook.flush().byAccountId.a.primary.requestCount, 2);
  listeners.get('codex:quota-updated')!({ payload: { account_id: 'a' } });
  document.visibilityState = 'hidden'; [...intervals.values()][0](); context.mock.timers.tick(250); assert.equal(reads.length, 2);
  document.visibilityState = 'visible'; document.dispatchEvent(new Event('visibilitychange')); context.mock.timers.tick(250);
  reads[2].resolve([row(3)]); await settlePromises(); assert.equal(hook.flush().byAccountId.a.primary.requestCount, 3);
  accounts = [{ ...first, quota: { ...first.quota!, hourly_reset_time: 2_000_000_100 } }];
  assert.equal(hook.flush().byAccountId.a.primary.requestCount, 3, 'reset-time corrections retain visible counters before effects and while querying');
  context.mock.timers.tick(250);
  reads[3].resolve([row(4)]); await settlePromises(); assert.equal(hook.flush().byAccountId.a.primary.requestCount, 4);
  listeners.get('codex:quota-updated')!({ payload: { account_id: 'other' } }); context.mock.timers.tick(250); assert.equal(reads.length, 4);
  listeners.get('codex:quota-updated')!({ payload: { account_id: 'a' } }); context.mock.timers.tick(250); assert.equal(reads.length, 5);
  active = false; assert.equal(hook.flush().ready, false); assert.equal(intervals.size, 0); assert.equal(listeners.size, 0);
  active = true; assert.equal(hook.flush().ready, false); context.mock.timers.tick(1000);
  assert.equal(reads.length, 5, 'reopening waits for the old physical native query');
  reads[4].resolve([row(999)]); await settlePromises(); assert.equal(hook.flush().ready, false);
  context.mock.timers.tick(250); assert.equal(reads.length, 6);
  reads[5].resolve([row(6)]); await settlePromises(); assert.equal(hook.flush().byAccountId.a.primary.requestCount, 6);
  hook.unmount();
});
