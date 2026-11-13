import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { LocalHistoryRead } from './useLocalHistory';

type Read = LocalHistoryRead<string>;
const history = (records: string[], migrationPending = false, migrationError: string | null = null): Read =>
  ({ records, migrationPending, migrationError });

function harness(read: (retry: boolean) => Promise<Read>, prepare?: () => Promise<void>) {
  let nextTimer = 0;
  let onChange: (() => void) | undefined;
  const timers = new Map<number, { callback: () => void; ms: number }>();
  const hook = loadHookModule(new URL('./useLocalHistory.ts', import.meta.url), {
    '../utils/tauriEventListener': {
      subscribeTauriEvent(_name: string, callback: () => void) {
        onChange = callback;
        return () => { onChange = undefined; };
      },
    },
  }, {
    setTimeout(callback: () => void, ms: number) {
      timers.set(++nextTimer, { callback, ms });
      return nextTimer;
    },
    clearTimeout(id: number) { timers.delete(id); },
  });
  hook.render(() => hook.exports.useLocalHistory(read, 'changed', prepare));
  return {
    flush: hook.flush,
    close: hook.unmount,
    change: () => onChange?.(),
    fire(ms: number) {
      for (const [id, timer] of [...timers]) {
        if (timer.ms === ms) {
          timers.delete(id);
          timer.callback();
        }
      }
    },
  };
}

test('shows committed history during WebView handoff and adopts each completed batch', async () => {
  const stage = deferred<void>();
  const afterStage = deferred<Read>();
  const results = [
    Promise.resolve(history(['new'])),
    afterStage.promise,
    Promise.resolve(history(['new', 'old'])),
  ];
  const h = harness(() => results.shift()!, () => stage.promise);
  await settlePromises();
  assert.deepEqual(h.flush().records, ['new']);
  assert.equal(h.flush().migrationPending, true);
  stage.resolve();
  await settlePromises();
  assert.equal(h.flush().migrationPending, true);
  afterStage.resolve(history(['new', 'first batch'], true));
  await settlePromises();
  assert.deepEqual(h.flush().records, ['new', 'first batch']);
  h.fire(750);
  await settlePromises();
  assert.deepEqual(h.flush().records, ['new', 'old']);
  assert.equal(h.flush().migrationPending, false);
  h.close();
});

test('clear invalidates the active response and coalesces concurrent refreshes', async () => {
  const old = deferred<Read>();
  const afterClear = deferred<Read>();
  let reads = 0;
  const h = harness(() => (++reads === 1 ? old.promise : afterClear.promise));
  h.change();
  h.change();
  h.flush().cleared();
  assert.equal(reads, 1);
  old.resolve(history(['deleted'], true));
  await settlePromises();
  assert.equal(reads, 2);
  assert.equal(h.flush().records.length, 0);
  afterClear.resolve(history([]));
  await settlePromises();
  assert.equal(h.flush().records.length, 0);
  assert.equal(h.flush().migrationPending, false);
  h.close();
});

test('deleting a displayed batch keeps other records and ignores an older response', async () => {
  const stale = deferred<Read>();
  const current = deferred<Read>();
  const results = [
    Promise.resolve(history(['deleted', 'kept'], true)),
    stale.promise,
    current.promise,
  ];
  const h = harness(() => results.shift()!);
  await settlePromises();
  h.fire(750);
  h.flush().updateRecords((records: string[]) => records.filter(record => record !== 'deleted'));
  stale.resolve(history(['deleted', 'kept'], true));
  await settlePromises();
  assert.deepEqual(h.flush().records, ['kept']);
  assert.equal(h.flush().migrationPending, true);
  current.resolve(history(['kept', 'restored']));
  await settlePromises();
  assert.deepEqual(h.flush().records, ['kept', 'restored']);
  h.close();
});

test('migration failure preserves committed records and explicit retry restarts it', async () => {
  const retries: boolean[] = [];
  const h = harness(async retry => {
    retries.push(retry);
    return retry ? history(['kept', 'recovered']) : history(['kept'], false, 'invalid old file');
  });
  await settlePromises();
  assert.deepEqual(h.flush().records, ['kept']);
  assert.equal(h.flush().error, 'invalid old file');
  h.fire(750);
  assert.deepEqual(retries, [false]);
  h.flush().refresh(true);
  await settlePromises();
  assert.deepEqual(retries, [false, true]);
  assert.deepEqual(h.flush().records, ['kept', 'recovered']);
  assert.equal(h.flush().error, null);
  h.close();
});

test('a timeout ends the loading state without overlapping the physical read', async () => {
  const slow = deferred<Read>();
  let reads = 0;
  const h = harness(() => (++reads === 1 ? slow.promise : Promise.resolve(history(['available']))));
  h.fire(5000);
  assert.equal(h.flush().loading, false);
  assert.equal(h.flush().error, 'local_history_timeout');
  h.flush().refresh(true);
  assert.equal(reads, 1);
  slow.resolve(history([]));
  await settlePromises();
  assert.equal(reads, 2);
  assert.deepEqual(h.flush().records, ['available']);
  h.close();
});
