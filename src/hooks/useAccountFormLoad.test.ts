import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function setup() {
  let nextTimer = 0;
  const timers = new Map<number, () => void>();
  const h = loadHookModule(new URL('./useAccountFormLoad.ts', import.meta.url), {}, {
    setTimeout: (fn: () => void) => { timers.set(++nextTimer, fn); return nextTimer; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  h.render(() => h.exports.useAccountFormLoad());
  return { h, timeout: () => timers.get(nextTimer)?.() };
}

test('the editor times out, reuses the outstanding read on retry and accepts the eventual result once', async () => {
  const { h, timeout } = setup();
  const native = deferred<string>();
  const values: string[] = [];
  let reads = 0;
  h.flush().start(() => { reads++; return native.promise; }, (value: string) => values.push(value), () => {});
  assert.equal(h.flush().status, 'loading');
  timeout();
  assert.equal(h.flush().status, 'failed');
  h.flush().retry();
  assert.equal(h.flush().status, 'loading');
  native.resolve('saved');
  await settlePromises();
  assert.equal(h.flush().status, 'ready');
  assert.equal(reads, 1);
  assert.deepEqual(values, ['saved']);
});

test('closing an editor discards the pending result and a new editor uses its own data', async () => {
  const { h } = setup();
  const native = deferred<string>();
  const values: string[] = [];
  const accept = (value: string) => values.push(value);
  h.flush().start(() => native.promise, accept, () => {});
  h.flush().reset();
  h.flush().start(async () => 'new', accept, () => {});
  native.resolve('closed');
  await settlePromises();
  assert.equal(h.flush().status, 'ready');
  assert.deepEqual(values, ['new']);
});
