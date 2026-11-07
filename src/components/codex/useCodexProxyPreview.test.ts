import { createCodexProxyStatusEvents } from '../../utils/codexProxyStatusEvents';
import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as previewUtils from '../../utils/codexProxyPreview';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';

const status: CodexProxyRuntimeStatus = { account: 'idle', desktop: 'idle', accountPort: null, desktopPort: null,
  proxySource: 'unified', effectiveProxy: { protocol: 'resource', name: 'Unified exit' },
  desktopEntry: { state: 'listening', port: 45101, requestCount: 0, lastRequestState: 'none', lastError: null } };

function harness() {
  const events = createCodexProxyStatusEvents();
  const reads: { id: string; request: ReturnType<typeof deferred<CodexProxyRuntimeStatus>> }[] = [];
  const requestReads: { id: string; request: ReturnType<typeof deferred<unknown[]>> }[] = [];
  const timers = new Set<() => void>();
  const h = loadHookModule(new URL('./useCodexProxyPreview.ts', import.meta.url), {
    '../../utils/codexProxyStatusEvents': { codexProxyStatusEvents: events },
    '../../services/codexAccountProxyService': {
      getCodexProxyRuntimeStatus: (id: string) => { const request = deferred<CodexProxyRuntimeStatus>(); reads.push({ id, request }); return request.promise; },
      getCodexAccountProxyRecentRequests: (id: string) => { const request = deferred<unknown[]>(); requestReads.push({ id, request }); return request.promise; },
    },
    '../../utils/codexProxyPreview': previewUtils,
  }, { setTimeout: (callback: () => void) => { timers.add(callback); return callback; }, clearTimeout: (callback: () => void) => timers.delete(callback) });
  const frames: any[] = [];
  const select = (id: string, binding = '') => {
    frames.length = 0;
    return h.render(() => { const result = h.exports.useCodexProxyPreview(id, binding); frames.push(result); return result; });
  };
  return { ...h, events, select, frames, reads, requestReads, timers };
}

test('switching accounts never renders the previous source, ports, requests or history even before effects', async () => {
  const h = harness();
  h.select('A'); await settlePromises();
  h.reads[0].request.resolve(status);
  h.requestReads[0].request.resolve([{ modelId: 'old-model' }]);
  await settlePromises();
  assert.equal(h.flush().status.desktopEntry.port, 45101);
  assert.equal(h.flush().history.length, 1);
  assert.equal(h.select('B').status, null);
  const firstFrame = h.frames[0];
  assert.equal(firstFrame.status, null);
  assert.equal(firstFrame.history.length, 0);
  assert.equal(firstFrame.requests.length, 0);
  assert.equal(firstFrame.loading, true);
  await settlePromises();
  assert.deepEqual(h.reads.map((entry) => entry.id), ['A', 'B']);
  assert.deepEqual(h.requestReads.map((entry) => entry.id), ['A', 'B'], 'unified and unbound accounts also read historical API requests');
  h.reads[1].request.resolve({ ...status, desktopEntry: { ...status.desktopEntry!, port: 45103 } });
  h.requestReads[1].request.resolve([]);
  await settlePromises();
  assert.equal(h.flush().status.desktopEntry.port, 45103);
  assert.equal(h.flush().history.length, 1);
  h.unmount();
  assert.equal(h.timers.size, 0);
});

for (const fails of [false, true]) test(`late ${fails ? 'failed' : 'successful'} reads cannot populate the next account or finish its loading state`, async () => {
  const h = harness();
  h.select('A'); await settlePromises();
  h.select('B'); await settlePromises();
  if (fails) { h.reads[0].request.reject(new Error('old status')); h.requestReads[0].request.reject(new Error('old requests')); }
  else { h.reads[0].request.resolve(status); h.requestReads[0].request.resolve([{ modelId: 'old' }]); }
  await settlePromises();
  assert.equal(h.flush().status, null);
  assert.equal(h.flush().history.length, 0);
  assert.equal(h.flush().requests.length, 0);
  assert.equal(h.flush().statusError, false);
  assert.equal(h.flush().requestsError, false);
  assert.equal(h.flush().loading, true);
  h.reads[1].request.resolve(status); h.requestReads[1].request.resolve([]);
  await settlePromises();
  assert.equal(h.flush().loading, false);
  h.unmount();
});

test('a failed status refresh clears the live readiness while retaining historical observations; retry recovers', async () => {
  const h = harness();
  h.select('A'); await settlePromises();
  h.reads[0].request.resolve(status); h.requestReads[0].request.resolve([]);
  await settlePromises();
  h.flush().refresh(); h.flush(); await settlePromises();
  h.reads[1].request.reject(new Error('read failure')); h.requestReads[1].request.reject(new Error('request failure'));
  await settlePromises();
  assert.equal(h.flush().status, null);
  assert.equal(h.flush().statusError, true);
  assert.equal(h.flush().requestsError, true);
  assert.equal(h.flush().history.length, 1);
  assert.equal(h.flush().loading, false);
  h.flush().refresh(); h.flush(); await settlePromises();
  h.reads[2].request.resolve({ ...status, desktopEntry: { ...status.desktopEntry!, requestCount: 1, lastRequestState: 'forwarded' } });
  h.requestReads[2].request.resolve([]);
  await settlePromises();
  assert.equal(h.flush().statusError, false);
  assert.equal(h.flush().requestsError, false);
  assert.equal(h.flush().history.length, 2);
  h.unmount();
});

test('manual measurement beats an older status poll and does not update a different account', async () => {
  const h = harness();
  h.select('A'); await settlePromises();
  const measured = { ...status, accountSelection: { name: 'node', delayMs: 200, checkedAt: 99 } };
  h.events.publish({ accountId: 'A', status: measured });
  assert.equal(h.flush().status.accountSelection.delayMs, 200);
  h.reads[0].request.resolve(status); h.requestReads[0].request.resolve([]);
  await settlePromises();
  assert.equal(h.flush().status.accountSelection.delayMs, 200);
  assert.equal(h.timers.size, 1, 'obsolete poll does not schedule an extra timer');
  h.select('B'); await settlePromises();
  h.events.publish({ accountId: 'A', status: measured });
  assert.equal(h.flush().status, null);
  h.reads[1].request.resolve(status); h.requestReads[1].request.resolve([]);
  await settlePromises();
  h.unmount();
});

test('measurement failure clears previous latency and ignores late poll success', async () => {
  const h = harness();
  h.select('A'); await settlePromises();
  h.events.publish({ accountId: 'A', status: { ...status, accountSelection: { name: 'node', delayMs: null, checkedAt: 100 } } });
  h.reads[0].request.resolve({ ...status, accountSelection: { name: 'node', delayMs: 200, checkedAt: 90 } });
  h.requestReads[0].request.resolve([]); await settlePromises();
  assert.equal(h.flush().status.accountSelection.delayMs, null);
  assert.equal(h.flush().status.accountSelection.checkedAt, 100);
  h.events.publish({ accountId: 'A', status: null });
  assert.equal(h.flush().status, null);
  assert.equal(h.flush().statusError, true);
  h.unmount();
});

test('a same-account binding change starts a fresh read and rejects the previous route snapshot', async () => {
  const h = harness();
  h.select('A', 'old-route'); await settlePromises();
  assert.equal(h.select('A', 'new-route').status, null);
  assert.equal(h.frames[0].status, null, 'even the render before effect cleanup hides the old route');
  await settlePromises();
  assert.equal(h.reads.length, 2, 'new binding must not reuse the previous native status flight');
  h.reads[1].request.resolve({ ...status, accountNode: 'new' });
  h.requestReads[0].request.resolve([]); await settlePromises();
  h.reads[0].request.resolve({ ...status, accountNode: 'old' }); await settlePromises();
  assert.equal(h.flush().status.accountNode, 'new');
  h.unmount();
});
