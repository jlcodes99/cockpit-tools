import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as previewUtils from '../../utils/codexProxyPreview';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';

const status: CodexProxyRuntimeStatus = { account: 'idle', desktop: 'idle', accountPort: null, desktopPort: null,
  proxySource: 'unified', effectiveProxy: { protocol: 'resource', name: 'Unified exit' },
  desktopEntry: { state: 'listening', port: 45101, requestCount: 0, lastRequestState: 'none', lastError: null } };

function harness() {
  const reads: { id: string; request: ReturnType<typeof deferred<CodexProxyRuntimeStatus>> }[] = [];
  const requestReads: { id: string; request: ReturnType<typeof deferred<unknown[]>> }[] = [];
  const timers = new Set<() => void>();
  const h = loadHookModule(new URL('./useCodexProxyPreview.ts', import.meta.url), {
    '../../services/codexAccountProxyService': {
      getCodexProxyRuntimeStatus: (id: string) => { const request = deferred<CodexProxyRuntimeStatus>(); reads.push({ id, request }); return request.promise; },
      getCodexAccountProxyRecentRequests: (id: string) => { const request = deferred<unknown[]>(); requestReads.push({ id, request }); return request.promise; },
    },
    '../../utils/codexProxyPreview': previewUtils,
  }, { setTimeout: (callback: () => void) => { timers.add(callback); return callback; }, clearTimeout: (callback: () => void) => timers.delete(callback) });
  const frames: any[] = [];
  const select = (id: string) => {
    frames.length = 0;
    return h.render(() => { const result = h.exports.useCodexProxyPreview(id); frames.push(result); return result; });
  };
  return { ...h, select, frames, reads, requestReads, timers };
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
