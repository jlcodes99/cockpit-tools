import assert from 'node:assert/strict';
import test from 'node:test';
import { createResetCreditLoader } from './codexResetCreditLoader';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';

function clock() {
  let now = 1_800_000_000_000, sequence = 0;
  const timers = new Map<number, { at: number; run: () => void }>();
  return {
    now: () => now,
    setTimer(run: () => void, delay: number) {
      const id = ++sequence;
      timers.set(id, { at: now + delay, run });
      return id as unknown as ReturnType<typeof setTimeout>;
    },
    clearTimer(id: ReturnType<typeof setTimeout>) { timers.delete(id as unknown as number); },
    async next() {
      const [id, timer] = [...timers].sort((a, b) => a[1].at - b[1].at)[0];
      timers.delete(id); now = Math.max(now, timer.at); timer.run(); await settlePromises();
    },
    advance(ms: number) { now += ms; },
  };
}
function storage() {
  const data = new Map<string, string>();
  return { data, getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => { data.set(key, value); },
    removeItem: (key: string) => { data.delete(key); } };
}
const snapshot = (expiry?: number) => ({ available_count: 1, credits: [], next_expires_at: expiry });

test('successful details survive a new loader and refresh after count changes or expiry of cache', async () => {
  const c = clock(), disk = storage(); let calls = 0;
  const fetcher = async () => { calls++; return snapshot(); };
  const options = { ...c, storage: disk, random: () => 0.5 };
  const first = createResetCreditLoader(fetcher, options);
  const result = first.load('a', 1); await c.next(); await result;
  const restarted = createResetCreditLoader(fetcher, options);
  assert.equal(restarted.peek('a', 1)?.available_count, 1);
  await restarted.load('a', 1); assert.equal(calls, 1);
  const changed = restarted.load('a', 2); await c.next(); await changed;
  assert.equal(calls, 2);
  c.advance(7 * 3600_000);
  const stale = restarted.load('a', 2); await c.next(); await stale;
  assert.equal(calls, 3);
});

test('automatic requests are staggered and limited to two in flight', async () => {
  const c = clock(), requests: { id: string; at: number }[] = [];
  const pending = new Map<string, ReturnType<typeof deferred<ReturnType<typeof snapshot>>>>();
  const loader = createResetCreditLoader(async id => {
    requests.push({ id, at: c.now() });
    const wait = deferred<ReturnType<typeof snapshot>>(); pending.set(id, wait); return wait.promise;
  }, { ...c, random: () => 0 });
  const a = loader.load('a', 1), b = loader.load('b', 1), d = loader.load('c', 1);
  assert.equal(loader.load('a', 1), a);
  await c.next(); await c.next();
  assert.equal(requests.length, 2);
  assert.ok(requests[1].at - requests[0].at >= 1000);
  pending.get('a')!.resolve(snapshot()); await a; await settlePromises(); await c.next();
  assert.equal(requests.length, 3);
  assert.ok(requests[2].at - requests[1].at >= 1000);
  pending.get('b')!.resolve(snapshot()); pending.get('c')!.resolve(snapshot()); await Promise.all([b, d]);
});

test('manual opening bypasses fresh cache, promotes queued reads and publishes its result', async () => {
  const c = clock(); let calls = 0, notifications = 0;
  const loader = createResetCreditLoader(async () => { calls++; return snapshot(c.now() / 1000 + 86400 + calls); }, { ...c, random: () => 0.5 });
  const unsubscribe = loader.subscribe('a', () => notifications++);
  const initial = loader.load('a', 1);
  assert.equal(loader.load('a', 1, true), initial);
  const requestedAt = c.now(); await c.next(); await initial;
  assert.equal(c.now(), requestedAt);
  const manual = loader.load('a', 1, true); await c.next(); await manual;
  assert.equal(calls, 2); assert.equal(notifications, 2);
  assert.equal(loader.peek('a', 1)?.next_expires_at, c.now() / 1000 + 86402);
  unsubscribe();
});

test('accounts receive distinct persisted deadlines, and known credit expiry schedules an earlier refresh', async () => {
  const c = clock(), disk = storage(); let random = 0.1;
  const loader = createResetCreditLoader(async id => snapshot(id === 'soon' ? c.now() / 1000 + 3600 : undefined),
    { ...c, storage: disk, random: () => { random += 0.05; return random; } });
  for (const id of ['a', 'b', 'soon']) { const request = loader.load(id, 1, true); await c.next(); await request; }
  const entries = [...disk.data.values()].map(value => JSON.parse(value));
  assert.notEqual(entries[0].refreshAt, entries[1].refreshAt);
  assert.ok(entries[2].refreshAt < c.now() + 2 * 3600_000);
});

test('background failures retain stale data and defer retries; explicit reads can retry immediately', async () => {
  const c = clock(); let fail = false, calls = 0;
  const loader = createResetCreditLoader(async () => { calls++; if (fail) throw Error('offline'); return snapshot(); }, { ...c, random: () => 0 });
  const initial = loader.load('a', 1); await c.next(); await initial;
  c.advance(8 * 3600_000); fail = true;
  const failed = loader.load('a', 1); const rejection = assert.rejects(failed, /offline/); await c.next(); await rejection;
  await loader.load('a', 1); assert.equal(calls, 2); assert.ok(loader.peek('a', 1));
  fail = false;
  const retried = loader.load('a', 1, true); await c.next(); await retried; assert.equal(calls, 3);
});
