import assert from 'node:assert/strict';
import test from 'node:test';
import { setImmediate, setTimeout as delay } from 'node:timers/promises';
import { createProxyCardReadCache, createProxyCardReadPool } from './codexProxyCardReads';

const key = (id: string, binding = 'a') => JSON.stringify([id, false, { name: binding }]);
const active = () => true;
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((ok, fail) => { resolve = ok; reject = fail; });
  return { promise, resolve, reject };
}

test('visible consumers share native reads, cached values and distinct binding snapshots', async () => {
  const operation = deferred<string>();
  let count = 0;
  const cache = createProxyCardReadCache(async (id) => { count++; return id === 'first' ? operation.promise : id; }, createProxyCardReadPool());
  const first = cache.read(key('first'), active);
  const same = cache.read(key('first'), active);
  assert.equal((await cache.read(key('other'), active)).value, 'other');
  operation.resolve('node-a');
  assert.equal((await first).value, 'node-a');
  assert.equal((await same).value, 'node-a');
  assert.equal((await cache.read(key('first'), active)).value, 'node-a');
  assert.equal(count, 2);
  assert.equal(cache.peek(key('first', 'b')).value, null);
});

test('failed refresh preserves the last successful result and permits explicit retry', async () => {
  let fails = false;
  const cache = createProxyCardReadCache(async () => { if (fails) throw new Error('offline'); return 'last-good'; }, createProxyCardReadPool());
  await cache.read(key('account'), active);
  fails = true;
  const failure = await cache.read(key('account'), active, true);
  assert.equal(failure.error, true);
  assert.equal(failure.value, 'last-good');
  fails = false;
  assert.equal((await cache.read(key('account'), active, true)).error, false);
});

test('timed out native work retains its permit and single flight until completion', async () => {
  const stuck = deferred<string>();
  const started: string[] = [];
  const pool = createProxyCardReadPool(1, 30);
  const cache = createProxyCardReadCache(async (id) => { started.push(id); return id === 'stuck' ? stuck.promise : id; }, pool, 5);
  assert.equal((await cache.read(key('stuck'), active)).error, true);
  assert.equal((await cache.read(key('stuck'), active, true)).error, true);
  assert.equal((await cache.read(key('queued'), active)).error, true);
  assert.deepEqual(started, ['stuck']);
  stuck.resolve('late');
  await setImmediate();
  assert.deepEqual(started, ['stuck']); // expired queued consumer never starts native work
  assert.equal(cache.peek(key('stuck')).value, null); // late completion cannot overwrite UI deadline
  assert.equal((await cache.read(key('recovered'), active)).value, 'recovered');
});

test('pool enforces concurrency and skips requests that leave the viewport while queued', async () => {
  const a = deferred<number>();
  const b = deferred<number>();
  const pool = createProxyCardReadPool(2);
  const started: string[] = [];
  let visible = true;
  const first = pool(() => { started.push('a'); return a.promise; }, active);
  const second = pool(() => { started.push('b'); return b.promise; }, active);
  const hidden = pool(async () => { started.push('hidden'); return 3; }, () => visible);
  const failure = assert.rejects(hidden, /PROXY_CARD_INACTIVE/);
  const last = pool(async () => { started.push('last'); return 4; }, active);
  await setImmediate();
  assert.deepEqual(started, ['a', 'b']);
  visible = false;
  a.resolve(1);
  assert.equal(await first, 1);
  await failure;
  assert.equal(await last, 4);
  assert.deepEqual(started, ['a', 'b', 'last']);
  b.resolve(2);
  await second;
});

test('responses after disposal do not enter the shared binding cache', async () => {
  let visible = true;
  const operation = deferred<string>();
  const cache = createProxyCardReadCache(() => operation.promise, createProxyCardReadPool());
  const request = cache.read(key('account'), () => visible);
  await setImmediate();
  visible = false;
  operation.resolve('old-node');
  assert.equal((await request).value, null);
  assert.equal(cache.peek(key('account')).value, null);
});

test('account invalidation bypasses stale binding caches and rejects pre-invalidation responses', async () => {
  const old = deferred<string>();
  let reads = 0;
  const cache = createProxyCardReadCache(async () => ++reads === 2 ? old.promise : `value-${reads}`, createProxyCardReadPool());
  await cache.read(key('account'), active);
  const stale = cache.read(key('account'), active, true);
  await setImmediate();
  cache.invalidateAccount('account');
  assert.equal(cache.peek(key('account')).value, null);
  assert.equal((await cache.read(key('account'), active)).value, 'value-3');
  old.resolve('stale');
  assert.equal((await stale).value, 'value-3');
  assert.equal(cache.peek(key('account')).value, 'value-3');
});

test('short-lived native pressure retries once per shared flight without exposing an error or losing the snapshot', async () => {
  const retry = deferred<string>();
  let calls = 0;
  const cache = createProxyCardReadCache(async () => {
    calls++;
    if (calls === 1) return 'last-good';
    if (calls === 2) throw new Error('PROXY_RUNTIME_BUSY');
    return retry.promise;
  }, createProxyCardReadPool(), 1000, 0);
  await cache.read(key('A'), active);
  const first = cache.read(key('A'), active, true);
  const second = cache.read(key('A'), active, true);
  await delay(10);
  assert.equal(calls, 3);
  assert.equal(cache.peek(key('A')).value, 'last-good');
  assert.equal(cache.peek(key('A')).error, false);
  retry.resolve('fresh');
  for (const result of await Promise.all([first, second])) {
    assert.equal(result.value, 'fresh');
    assert.equal(result.error, false);
    assert.equal(result.errorKind, undefined);
  }
});

test('persistent native pressure retains data and reports a local busy error after bounded retry', async () => {
  let busy = false;
  let calls = 0;
  const cache = createProxyCardReadCache(async () => {
    calls++;
    if (busy) throw 'PROXY_RUNTIME_LIMIT';
    return 'cached';
  }, createProxyCardReadPool(), 1000, 0);
  await cache.read(key('A'), active);
  busy = true;
  const result = await cache.read(key('A'), active, true);
  assert.equal(calls, 3);
  assert.equal(result.value, 'cached');
  assert.equal(result.error, true);
  assert.equal(result.errorKind, 'busy');
});

test('capacity exhaustion is actionable immediately and is not retried in a loop', async () => {
  let calls = 0;
  const cache = createProxyCardReadCache(async () => { calls++; throw 'PROXY_RUNTIME_CAPACITY'; }, createProxyCardReadPool(), 1000, 0);
  const result = await cache.read(key('A'), active);
  assert.equal(calls, 1);
  assert.equal(result.errorKind, 'capacity');
});

test('a hidden consumer stops automatic pressure retries and does not publish an error', async () => {
  let visible = true;
  let calls = 0;
  const cache = createProxyCardReadCache(async () => { calls++; throw 'PROXY_RUNTIME_BUSY'; }, createProxyCardReadPool(), 1000, 20);
  const pending = cache.read(key('A'), () => visible);
  await setImmediate();
  visible = false;
  const result = await pending;
  assert.equal(calls, 1);
  assert.equal(result.error, false);
  assert.equal(cache.has(key('A')), false);
});
