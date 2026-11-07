import { createCodexProxyStatusEvents } from '../../utils/codexProxyStatusEvents';
import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as readsModule from '../../utils/codexProxyCardReads';
import type { CodexProxyRuntimeStatus, CodexProxyRecentRequest } from '../../services/codexAccountProxyService';

const status = (node: string): CodexProxyRuntimeStatus => ({
  account: 'running', desktop: 'idle', accountPort: 1234, desktopPort: null, accountNode: node,
});
const request = (timestamp: number): CodexProxyRecentRequest => ({ timestamp, modelId: 'model', success: true, httpStatus: 200, latencyMs: 10 });

function harness() {
  const events = createCodexProxyStatusEvents();
  const statusCalls: { id: string; pending: ReturnType<typeof deferred<CodexProxyRuntimeStatus>> }[] = [];
  const requestCalls: { id: string; pending: ReturnType<typeof deferred<CodexProxyRecentRequest[]>> }[] = [];
  const intervals = new Map<number, () => void>();
  let nextInterval = 0;
  const listeners = new Map<string, () => void>();
  const document = {
    visibilityState: 'visible',
    addEventListener(name: string, callback: () => void) { listeners.set(name, callback); },
    removeEventListener(name: string) { listeners.delete(name); },
  };
  const observers: Array<{ callback: (entries: Array<{ isIntersecting: boolean }>) => void; disconnected: boolean }> = [];
  class IntersectionObserver {
    state: typeof observers[number];
    constructor(callback: typeof observers[number]['callback']) {
      this.state = { callback, disconnected: false }; observers.push(this.state);
    }
    observe() {}
    disconnect() { this.state.disconnected = true; }
  }
  const h = loadHookModule(new URL('./useCodexProxyCardData.ts', import.meta.url), {
    '../../utils/codexProxyCardReads': readsModule,
    '../../utils/codexProxyStatusEvents': { codexProxyStatusEvents: events },
    '../../services/codexAccountProxyService': {
      getCodexProxyRuntimeStatus(id: string) { const pending = deferred<CodexProxyRuntimeStatus>(); statusCalls.push({ id, pending }); return pending.promise; },
      getCodexAccountProxyRecentRequests(id: string) { const pending = deferred<CodexProxyRecentRequest[]>(); requestCalls.push({ id, pending }); return pending.promise; },
    },
  }, {
    document, IntersectionObserver,
    setInterval(callback: () => void) { const id = ++nextInterval; intervals.set(id, callback); return id; },
    clearInterval(id: number) { intervals.delete(id); },
  });
  const select = (id: string, detailed = true, binding = 'proxy-a') => h.render(() => {
    const value = h.exports.useCodexProxyCardData({ id, egress_proxy: { name: binding } }, detailed);
    value.ref.current = {};
    return value;
  });
  return { ...h, events, select, statusCalls, requestCalls, intervals, listeners, observers,
    visible(value: boolean) { observers[observers.length - 1]!.callback([{ isIntersecting: value }]); },
    hidden(value: boolean) { document.visibilityState = value ? 'hidden' : 'visible'; listeners.get('visibilitychange')?.(); },
  };
}

test('unread detail waits for visibility and both reads before displaying a genuinely empty history', async () => {
  const h = harness();
  assert.equal(h.select('A').loading, true);
  await settlePromises();
  assert.equal(h.statusCalls.length, 0);
  h.visible(true); await settlePromises();
  assert.equal(h.statusCalls.length, 1);
  assert.equal(h.requestCalls.length, 1);
  h.statusCalls[0].pending.resolve(status('current')); await settlePromises();
  assert.equal(h.flush().status.accountNode, 'current');
  assert.equal(h.flush().loading, true);
  h.requestCalls[0].pending.resolve([]); await settlePromises();
  assert.equal(h.flush().request, null);
  assert.equal(h.flush().loading, false);
  h.unmount();
});

test('summary skips history; enabling details reads and selects the latest timestamp', async () => {
  const h = harness();
  h.select('A', false); h.visible(true); await settlePromises();
  assert.equal(h.requestCalls.length, 0);
  h.statusCalls[0].pending.resolve(status('current')); await settlePromises();
  assert.equal(h.flush().loading, false);
  assert.equal(h.select('A', true).loading, true);
  h.visible(true); await settlePromises();
  h.requestCalls[0].pending.resolve([request(2), request(9), request(1)]); await settlePromises();
  assert.equal(h.flush().request.timestamp, 9);
  assert.equal(h.statusCalls.length, 1);
  assert.equal(h.flush().loading, false);
  h.unmount();
});

test('leaving viewport, hiding window and unmounting stop polls and reject stale work', async () => {
  const h = harness();
  h.select('A'); h.visible(true); await settlePromises();
  assert.equal(h.intervals.size, 1);
  h.visible(false);
  assert.equal(h.intervals.size, 0);
  h.statusCalls[0].pending.resolve(status('stale'));
  h.requestCalls[0].pending.resolve([request(1)]); await settlePromises();
  assert.equal(h.flush().status, null);
  h.flush().refresh(); await settlePromises();
  assert.equal(h.statusCalls.length, 1);
  h.visible(true); await settlePromises();
  assert.equal(h.statusCalls.length, 2);
  h.hidden(true);
  assert.equal(h.intervals.size, 0);
  h.flush().refresh(); await settlePromises();
  assert.equal(h.statusCalls.length, 2);
  h.unmount();
  assert.equal(h.listeners.size, 0);
  assert.equal(h.observers[h.observers.length - 1]!.disconnected, true);
  h.statusCalls[1].pending.resolve(status('after-unmount'));
  h.requestCalls[1].pending.resolve([]); await settlePromises();
  assert.equal(h.intervals.size, 0);
  assert.equal(h.statusCalls.length, 2);
});

for (const next of [{ id: 'B', binding: 'proxy-a' }, { id: 'A', binding: 'proxy-b' }]) {
  test(`identity/binding change ${next.id}/${next.binding} ignores late previous-account results`, async () => {
    const h = harness();
    h.select('A'); h.visible(true); await settlePromises();
    const fresh = h.select(next.id, true, next.binding);
    assert.equal(fresh.status, null);
    assert.equal(fresh.request, null);
    assert.equal(fresh.loading, true);
    h.visible(true); await settlePromises();
    h.statusCalls[1].pending.resolve(status('new-node'));
    h.requestCalls[1].pending.resolve([request(10)]); await settlePromises();
    h.statusCalls[0].pending.resolve(status('old-node'));
    h.requestCalls[0].pending.resolve([request(999)]); await settlePromises();
    assert.equal(h.flush().status.accountNode, 'new-node');
    assert.equal(h.flush().request.timestamp, 10);
    assert.equal(h.flush().loading, false);
    h.unmount();
  });
}

test('manual measurement invalidates a cached card immediately and rejects old polling responses', async () => {
  const h = harness();
  h.select('A', false); h.visible(true); await settlePromises();
  h.statusCalls[0].pending.resolve(status('cached')); await settlePromises();
  h.flush().refresh(); await settlePromises();
  const measured = { ...status('new'), accountSelection: { name: 'new', delayMs: 200, checkedAt: 100 } };
  h.events.publish({ accountId: 'A', status: measured });
  assert.equal(h.flush().status.accountSelection.delayMs, 200);
  await settlePromises();
  assert.equal(h.statusCalls.length, 3, 'bypasses the old flight and the 20-second cache');
  h.statusCalls[2].pending.resolve(measured); await settlePromises();
  h.statusCalls[1].pending.resolve(status('old')); await settlePromises();
  assert.equal(h.flush().status.accountSelection.delayMs, 200);
  assert.equal(h.flush().status.accountNode, 'new');
  h.unmount();
});

test('offscreen invalidation never reads until visible and unmounted cached cards are invalidated', async () => {
  const h = harness();
  h.select('A', false); h.visible(true); await settlePromises();
  h.statusCalls[0].pending.resolve(status('cached')); await settlePromises();
  h.visible(false);
  h.events.publish({ accountId: 'A', status: null }); await settlePromises();
  assert.equal(h.statusCalls.length, 1);
  assert.equal(h.flush().status, null);
  h.visible(true); await settlePromises();
  assert.equal(h.statusCalls.length, 2);
  h.statusCalls[1].pending.resolve(status('fresh')); await settlePromises();
  h.select('B', false);
  h.events.publish({ accountId: 'A', status: null }); await settlePromises();
  assert.equal(h.statusCalls.length, 2);
  assert.equal(h.select('A', false).status, null);
  h.visible(true); await settlePromises();
  assert.equal(h.statusCalls.length, 3);
  h.statusCalls[2].pending.resolve(status('latest')); await settlePromises();
  h.unmount();
});
