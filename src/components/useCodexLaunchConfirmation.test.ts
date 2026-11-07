import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, deferred, settlePromises } from '../../tests/helpers/reactHookHarness.ts';
import { confirmedCodexLaunch } from '../utils/codexLaunchConfirmation.ts';

const target = { instanceId: '__default__', steps: [{ id: 'checkInstance', details: { userDataDir: 'C:/Users/a/.codex' } }] };
const instance = { id: '__default__', userDataDir: 'C:/Users/a/.codex', running: true, lastPid: 42 };

function harness(listInstances: () => Promise<unknown[]>) {
  const confirmed: unknown[] = [];
  const timers = new Map<number, () => void>();
  let timerId = 0;
  let now = 0;
  const hook = loadHookModule(new URL('./useCodexLaunchConfirmation.ts', import.meta.url), {
    '../services/codexInstanceService': { listInstancesForLaunchConfirmation: listInstances },
    '../utils/codexLaunchConfirmation': { confirmedCodexLaunch },
  }, {
    Date: { now: () => now },
    setTimeout(callback: () => void) { timers.set(++timerId, callback); return timerId; },
    clearTimeout(id: number) { timers.delete(id); },
  });
  let current: typeof target | null = target;
  const render = () => hook.render(() => hook.exports.useCodexLaunchConfirmation(current, (value: unknown) => confirmed.push(value)));
  return { ...hook, confirmed, timers, render,
    advance(ms: number) { now += ms; },
    runNextTimer() { const [id, callback] = timers.entries().next().value!; timers.delete(id); callback(); },
    setTarget(value: typeof target | null) { current = value; return render(); } };
}

test('automatic and manual checks share one request and late startup resolves without relaunch', async () => {
  const reply = deferred<unknown[]>();
  let calls = 0;
  const h = harness(() => { calls++; return reply.promise; });
  const result = h.render();
  const manual = result.check();
  assert.equal(calls, 1);
  reply.resolve([instance]);
  await manual;
  await settlePromises();
  assert.deepEqual(h.confirmed, [target]);
  assert.equal(h.timers.size, 0);
  h.unmount();
});

test('closing or changing the target ignores an outstanding status response', async () => {
  for (const replacement of [null, { ...target, instanceId: 'another' }]) {
    const reply = deferred<unknown[]>();
    const h = harness(() => reply.promise);
    h.render();
    h.setTarget(replacement);
    reply.resolve([instance]);
    await settlePromises();
    assert.deepEqual(h.confirmed, []);
    h.unmount();
    assert.equal(h.timers.size, 0);
  }
});

test('failed/empty queries stay unconfirmed, and a later manual check can recover', async () => {
  let rows: unknown[] = [];
  let fail = true;
  const h = harness(async () => { if (fail) throw new Error('temporary query error'); return rows; });
  h.render();
  await settlePromises();
  let result = h.flush();
  await result.check();
  result = h.flush();
  assert.equal(result.error, 'checkFailed');
  fail = false;
  await result.check();
  result = h.flush();
  assert.equal(result.error, 'notRunning');
  assert.deepEqual(h.confirmed, []);
  rows = [instance];
  await result.check();
  assert.deepEqual(h.confirmed, [target]);
  h.unmount();
});

test('automatic reconciliation stops after its budget and never polls indefinitely', async () => {
  let calls = 0;
  const h = harness(async () => { calls++; return []; });
  h.render();
  await settlePromises();
  assert.equal(h.timers.size, 1);
  h.advance(31_000);
  h.runNextTimer();
  await settlePromises();
  assert.equal(calls, 1);
  assert.equal(h.timers.size, 0);
  assert.deepEqual(h.confirmed, []);
  h.unmount();
});

test('an unresponsive status query releases the check button after a bounded wait', async () => {
  const h = harness(() => new Promise(() => {}));
  h.render();
  assert.equal(h.flush().checking, true);
  h.runNextTimer();
  await settlePromises();
  assert.equal(h.flush().checking, false);
  assert.deepEqual(h.confirmed, []);
  h.unmount();
});
