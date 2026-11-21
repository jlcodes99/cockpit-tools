import assert from 'node:assert/strict';
import test from 'node:test';
import { settlePromises } from '../../tests/helpers/reactHookHarness';
import { createAutoRefreshScheduler, type AutoRefreshSchedulerTask } from './autoRefreshScheduler';

test('configuration updates preserve pending refreshes and replace callbacks', async (context) => {
  let now = 0;
  let tick: (() => void) | undefined;
  let ticksStarted = 0;
  context.mock.method(Date, 'now', () => now);
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      setInterval(callback: () => void) {
        ticksStarted++;
        tick = callback;
        return ticksStarted;
      },
      clearInterval() { tick = undefined; },
    },
  });

  const calls: string[] = [];
  let finish: (() => void) | undefined;
  const task = (intervalMs: number, name: string): AutoRefreshSchedulerTask => ({
    key: 'full:codex',
    label: 'Codex',
    intervalMs,
    initialDelayMs: 10_000,
    async run() {
      calls.push(name);
      if (name === 'updated') await new Promise<void>(resolve => { finish = resolve; });
    },
  });
  const scheduler = createAutoRefreshScheduler([task(60_000, 'original')], { maxConcurrent: 2 });
  const advance = (time: number) => {
    now = time;
    tick?.();
  };
  context.after(() => {
    scheduler.stop();
    if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow);
    else Reflect.deleteProperty(globalThis, 'window');
  });
  scheduler.start();

  for (const time of [2_000, 4_000, 8_000]) {
    now = time;
    scheduler.replaceTasks([task(60_000, 'updated')]);
    scheduler.start();
    tick?.();
  }
  assert.equal(ticksStarted, 1);
  assert.deepEqual(calls, []);

  advance(10_000);
  await settlePromises();
  assert.deepEqual(calls, ['updated']);

  now = 20_000;
  scheduler.replaceTasks([task(120_000, 'changed interval')]);
  advance(30_000);
  await settlePromises();
  assert.deepEqual(calls, ['updated'], 'a refresh already in progress must finish first');
  assert.ok(finish);
  finish();
  await settlePromises();
  assert.deepEqual(calls, ['updated', 'changed interval']);

  scheduler.replaceTasks([]);
  assert.equal(tick, undefined);
  scheduler.replaceTasks([task(60_000, 'reenabled')]);
  scheduler.start();
  advance(40_000);
  await settlePromises();
  assert.deepEqual(calls, ['updated', 'changed interval', 'reenabled']);
});
