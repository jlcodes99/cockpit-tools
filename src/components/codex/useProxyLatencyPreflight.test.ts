import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function harness() {
  const checks: ReturnType<typeof deferred<void>>[] = [];
  const prompts: boolean[] = [];
  const batches: string[][] = [];
  const source = { id: 'source', revision: '1', nodes: [{ id: 'a', supported: true }, { id: 'b', supported: true }] };
  const h = loadHookModule(new URL('./useProxyLatency.ts', import.meta.url), {
    '../../services/codexProxyCatalogService': { cancelProxyCatalog: async () => {}, measureProxyLatency: async () => {}, catalogErrorKey: () => 'codex.proxy.engineMissing' },
    '../../services/codexProxyEngineService': { preflightCodexProxyEngine: (showPrompt: boolean) => { const task = deferred<void>(); checks.push(task); prompts.push(showPrompt); return task.promise; } },
    '../../utils/codexProxyLatency': { latencyFresh: () => false, startLatencyBatch: (ids: string[]) => { batches.push(ids); return { done: Promise.resolve(), cancel() {} }; } },
  });
  h.render(() => h.exports.useProxyLatency(source));
  return { ...h, checks, batches, prompts };
}

test('missing engine blocks the batch without marking every node as failed', async () => {
  const h = harness(); h.flush().measure(['a', 'b']); await settlePromises();
  assert.equal(h.checks.length, 1); assert.equal(h.batches.length, 0);
  assert.equal(Object.keys(h.flush().results).length, 0);
  h.checks[0].reject(new Error('PROXY_ENGINE_MISSING')); await settlePromises();
  assert.equal(h.batches.length, 0);
  assert.equal(Object.keys(h.flush().results).length, 0);
  assert.equal(h.flush().errorKey, 'codex.proxy.engineMissing');
  assert.equal(h.flush().running, false); h.unmount();
});

test('cancelling while checking cannot start a late batch; a later retry can proceed', async () => {
  const h = harness(); h.flush().measure(['a']); await settlePromises();
  h.flush().cancel(); h.checks[0].resolve(); await settlePromises();
  assert.equal(h.batches.length, 0); assert.equal(h.flush().running, false);
  h.flush().measure(['b']); await settlePromises(); h.checks[1].resolve(); await settlePromises();
  assert.deepEqual(Array.from(h.batches[0]), ['b']); h.unmount();
});

test('background stale-result refresh uses inline feedback rather than opening a setup dialog', async () => {
  const h = harness(); h.flush().measure(['a'], true); await settlePromises();
  assert.deepEqual(h.prompts, [false]);
  h.checks[0].reject(new Error('PROXY_ENGINE_MISSING')); await settlePromises();
  assert.equal(h.flush().errorKey, 'codex.proxy.engineMissing'); h.unmount();
});
