import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as latencyUtils from '../../utils/codexProxyLatency';

function harness() {
  const requests: { node: string; revision: string; requestId: string; groupId?: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const cancelled: string[] = [];
  const h = loadHookModule(new URL('./useProxyLatency.ts', import.meta.url), {
    '../../services/codexProxyCatalogService': {
      catalogErrorKey: (error: unknown) => String(error),
      measureProxyLatency: (requestId: string, _source: string, node: string, revision: string, groupId?: string) => {
        const result = deferred(); requests.push({ node, revision, requestId, groupId, result }); return result.promise;
      },
      cancelProxyCatalog: async (id: string) => { cancelled.push(id); },
    },
    '../../services/codexProxyEngineService': { preflightCodexProxyEngine: async () => {} },
    '../../utils/codexProxyLatency': latencyUtils,
  });
  const source = (revision = '1') => ({ id: 'source', revision, nodes: ['a', 'b', 'c', 'd'].map((id) => ({ id, supported: id !== 'd' })) });
  let selected = source();
  return { ...h, requests, cancelled,
    start: () => h.render(() => h.exports.useProxyLatency(selected)),
    change: () => { selected = source('2'); return h.render(() => h.exports.useProxyLatency(selected)); },
  };
}

test('replacement cancels old work, waits for it and ignores stale results', async () => {
  const h = harness(); h.start().measure(['a']); await settlePromises();
  h.flush().measure(['b']); await settlePromises();
  assert.deepEqual(h.cancelled, [h.requests[0].requestId]);
  assert.equal(h.requests.length, 1);
  h.requests[0].result.resolve({ latencyMs: 5, checkedAt: Date.now() }); await settlePromises();
  assert.equal(h.requests.length, 2); assert.equal(h.requests[1].node, 'b');
  assert.notEqual(h.flush().results.a.status, 'success');
  h.requests[1].result.resolve({ latencyMs: 12, checkedAt: Date.now() }); await settlePromises();
  assert.equal(h.flush().results.b.value.latencyMs, 12); assert.equal(h.flush().running, false);
  h.unmount();
});

test('the same node is measured independently for different group URLs and cached within its group', async () => {
  const h = harness();
  h.start().measure(['a'], true, 'group-a'); await settlePromises();
  assert.equal(h.requests[0].groupId, 'group-a');
  h.requests[0].result.resolve({ latencyMs: 10, checkedAt: Date.now() }); await settlePromises();
  h.flush().measure(['a'], true, 'group-b'); await settlePromises();
  assert.equal(h.requests.length, 2, 'fresh results from another group must not be reused');
  assert.equal(h.requests[1].groupId, 'group-b');
  assert.notEqual(h.flush().results.a?.status, 'success');
  h.requests[1].result.resolve({ latencyMs: 200, checkedAt: Date.now() }); await settlePromises();
  assert.equal(h.flush().results.a.value.latencyMs, 200);
  h.flush().measure(['a'], true, 'group-a'); await settlePromises();
  assert.equal(h.requests.length, 2);
  assert.equal(h.flush().results.a.value.latencyMs, 10);
  h.flush().measure(['a'], true); await settlePromises();
  assert.equal(h.requests.length, 3, 'standalone checks use their own default test URL');
  assert.equal(h.requests[2].groupId, undefined);
  h.unmount(); h.requests[2].result.reject(new Error('cancelled')); await settlePromises();
});

test('fresh results are reused, expired results and source changes are checked again', async () => {
  const h = harness(); h.start().measure(['a', 'd']); await settlePromises();
  assert.equal(h.requests.length, 1, 'unsupported candidates are not tested');
  h.requests[0].result.resolve({ latencyMs: 5, checkedAt: Date.now() }); await settlePromises();
  h.flush().measure(['a'], true); await settlePromises(); assert.equal(h.requests.length, 1);
  h.flush().measure(['a']); await settlePromises();
  h.requests[1].result.resolve({ latencyMs: 8, checkedAt: Date.now() - 60_001 }); await settlePromises();
  h.flush().measure(['a'], true); await settlePromises(); assert.equal(h.requests.length, 3);
  h.change(); assert.equal(Object.keys(h.flush().results).length, 0);
  h.requests[2].result.resolve({ latencyMs: 99, checkedAt: Date.now() }); await settlePromises();
  assert.equal(Object.keys(h.flush().results).length, 0);
  h.flush().measure(['a'], true); await settlePromises(); assert.equal(h.requests[3].revision, '2');
  h.unmount(); assert.ok(h.cancelled.includes(h.requests[3].requestId));
  h.requests[3].result.reject(new Error('cancelled')); await settlePromises();
});

test('an empty selection changes the displayed cache without testing any node', async () => {
  const h = harness(); h.start().measure(['a'], true, 'group-a'); await settlePromises();
  h.requests[0].result.resolve({ latencyMs: 30, checkedAt: Date.now() }); await settlePromises();
  assert.equal(h.flush().results.a.value.latencyMs, 30);
  h.flush().measure([], true); await settlePromises();
  assert.equal(h.requests.length, 1);
  assert.equal(Object.keys(h.flush().results).length, 0);
  assert.equal(h.flush().running, false);
  h.flush().measure([], true, 'group-a'); await settlePromises();
  assert.equal(h.requests.length, 1);
  assert.equal(h.flush().results.a.value.latencyMs, 30);
  h.unmount();
});
