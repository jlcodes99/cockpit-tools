import assert from 'node:assert/strict';
import test from 'node:test';
import { startLatencyBatch, type LatencyState } from './codexProxyLatency';
test('batch tests only requested unique leaves with at most three active requests', async () => {
  let seq = 0; let active = 0; let peak = 0;
  const pending: Array<() => void> = [];
  const called: string[] = [];
  const states = new Map<string, LatencyState>();
  const batch = startLatencyBatch(['a', 'b', 'a', 'c', 'd', 'e'], {
    id: () => String(++seq),
    measure: (id) => { called.push(id); active++; peak = Math.max(peak, active); return new Promise((resolve) => pending.push(() => { active--; resolve({ latencyMs: 12, checkedAt: 1 }); })); },
    cancel: async () => {},
    update: (id, value) => states.set(id, value),
  });
  assert.deepEqual(called, ['a', 'b', 'c']);
  while (pending.length) { pending.shift()!(); await new Promise((resolve) => setImmediate(resolve)); }
  await batch.done;
  assert.equal(peak, 3); assert.deepEqual(called, ['a', 'b', 'c', 'd', 'e']);
  assert.equal([...states.values()].filter((v) => v.status === 'success').length, 5);
});
test('cancel stops queued nodes and ignores late success or failure', async () => {
  let seq = 0; const requests: string[] = []; const cancels: string[] = [];
  const pending: Array<() => void> = []; const states = new Map<string, LatencyState>();
  const batch = startLatencyBatch(['a', 'b', 'c', 'd', 'e'], {
    id: () => String(++seq),
    measure: (id) => { requests.push(id); return new Promise((resolve) => pending.push(() => resolve({ latencyMs: 1, checkedAt: 1 }))); },
    cancel: async (id) => { cancels.push(id); },
    update: (id, value) => states.set(id, value),
  });
  batch.cancel(); batch.cancel();
  pending.forEach((resolve) => resolve()); await batch.done;
  assert.deepEqual(requests, ['a', 'b', 'c']); assert.deepEqual(cancels, ['1', '2', '3']);
  assert.ok([...states.values()].every((v) => v.status === 'cancelled'));
});
test('one failed node does not abort other group members', async () => {
  const states = new Map<string, LatencyState>();
  const batch = startLatencyBatch(['failed', 'good'], {
    id: () => crypto.randomUUID(),
    measure: async (id) => { if (id === 'failed') throw new Error('PROXY_PROBE_TIMEOUT'); return { latencyMs: 2, checkedAt: 1 }; },
    cancel: async () => {},
    update: (id, value) => states.set(id, value),
  });
  await batch.done; assert.equal(states.get('failed')?.status, 'error'); assert.equal(states.get('good')?.status, 'success');
});

test('latency freshness rejects expired and future timestamps', async () => {
  const { latencyFresh } = await import('./codexProxyLatency');
  assert.equal(latencyFresh({ status: 'success', value: { latencyMs: 10, checkedAt: 1000 } }, 2000), true);
  assert.equal(latencyFresh({ status: 'success', value: { latencyMs: 10, checkedAt: 1000 } }, 61000), false);
  assert.equal(latencyFresh({ status: 'success', value: { latencyMs: 10, checkedAt: 3000 } }, 2000), false);
  assert.equal(latencyFresh({ status: 'running' }, 2000), false);
});
