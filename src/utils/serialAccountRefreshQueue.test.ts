import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createSerialAccountRefreshQueue } from './serialAccountRefreshQueue.ts';

test('100 accounts and 20 overlapping visible accounts share one serial worker', async () => {
  const calls: string[] = [];
  let active = 0;
  let maxActive = 0;
  let release!: () => void;
  const first = new Promise<void>((resolve) => { release = resolve; });
  const queue = createSerialAccountRefreshQueue({
    shouldRun: () => true, retryIntervalMs: 300_000,
    onError: (_id, error) => { throw error; },
    run: async (id) => {
      active++;
      maxActive = Math.max(maxActive, active);
      calls.push(id);
      if (id === '0') await first;
      await Promise.resolve();
      active--;
    },
  });
  const all = queue.enqueue(Array.from({ length: 100 }, (_, i) => `${i}`));
  const visible = queue.enqueue(Array.from({ length: 20 }, (_, i) => `${i}`));
  release();
  await Promise.all([all, visible]);
  assert.equal(calls.length, 100);
  assert.equal(new Set(calls).size, 100);
  assert.equal(maxActive, 1);
});

test('queued accounts are rechecked, errors do not stall the queue, and retries use actual start time', async () => {
  let clock = 0;
  let release!: () => void;
  const first = new Promise<void>((resolve) => { release = resolve; });
  const eligible = new Set(['a', 'b', 'c']);
  const calls: string[] = [];
  const errors: string[] = [];
  const queue = createSerialAccountRefreshQueue({
    now: () => clock, shouldRun: (id) => eligible.has(id), retryIntervalMs: 100,
    run: async (id) => {
      calls.push(id);
      if (id === 'a') { await first; throw new Error('network timeout'); }
    },
    onError: (id) => { errors.push(id); },
  });
  const pending = queue.enqueue(['a', 'b', 'c']);
  eligible.delete('b');
  clock = 150;
  release();
  await pending;
  assert.deepEqual(calls, ['a', 'c']);
  assert.deepEqual(errors, ['a']);
  await queue.enqueue(['c']);
  assert.deepEqual(calls, ['a', 'c']);
  clock = 250;
  await queue.enqueue(['c']);
  assert.deepEqual(calls, ['a', 'c', 'c']);
});
