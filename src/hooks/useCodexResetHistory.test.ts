import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function setup() {
  const calls: { account: string; source: string; cursor: string | number | null; task: ReturnType<typeof deferred<any>> }[] = [];
  let event: (() => void) | undefined;
  const harness = loadHookModule(new URL('./useCodexResetHistory.ts', import.meta.url), {
    '../services/codexResetHistoryService': {
      loadCodexResetHistory(account: string, source: string, cursor: string | number | null) {
        const task = deferred<any>(); calls.push({ account, source, cursor, task }); return task.promise;
      },
    },
    '../utils/tauriEventListener': { subscribeTauriEvent(_name: string, fn: () => void) {
      event = fn; return () => { event = undefined; };
    } },
  });
  return { ...harness, calls, emit: () => event?.() };
}
const page = (ids: string[], nextCursor: string | number | null = null) => ({
  entries: ids.map(id => ({ id, kind: 'granted', at: 1 })), nextCursor,
});

test('history pagination keeps loaded data and retries the failed cursor', async () => {
  const h = setup();
  h.render(() => h.exports.useCodexResetHistory('a', 'official'));
  h.calls[0].task.resolve(page(['one'], 'opaque+/=')); await settlePromises();
  let state = h.flush(); void state.more(); state = h.flush();
  assert.equal(h.calls[1].cursor, 'opaque+/=');
  h.calls[1].task.reject(new Error('offline')); await settlePromises(); state = h.flush();
  assert.equal(state.entries[0].id, 'one');
  assert.match(state.error, /offline/);
  void state.retry(); state = h.flush();
  assert.equal(h.calls[2].cursor, 'opaque+/=');
  assert.equal(state.error, null);
  h.calls[2].task.resolve(page(['one', 'two'])); await settlePromises(); state = h.flush();
  assert.equal(state.entries.map((entry: any) => entry.id).join(','), 'one,two');
  assert.equal(state.cursor, null);
  h.unmount();
});

test('switching accounts discards late results and local events reload the selected account', async () => {
  const h = setup(); let account = 'a';
  h.render(() => h.exports.useCodexResetHistory(account, 'local'));
  account = 'b'; h.flush();
  h.calls[0].task.resolve(page(['wrong-account']));
  h.calls[1].task.resolve(page(['b-pending'])); await settlePromises();
  let state = h.flush(); assert.equal(state.entries[0].id, 'b-pending');
  h.emit(); h.flush(); assert.equal(h.calls[2].account, 'b');
  h.calls[2].task.resolve(page(['b-completed'])); await settlePromises(); state = h.flush();
  assert.equal(state.entries[0].id, 'b-completed');
  h.unmount();
});

test('a failed refresh retains entries and retries the first page, not the older-page cursor', async () => {
  const h = setup(); h.render(() => h.exports.useCodexResetHistory('a', 'official'));
  h.calls[0].task.resolve(page(['one'], 'older')); await settlePromises();
  let state = h.flush(); void state.refresh(); h.flush();
  h.calls[1].task.reject(new Error('refresh failed')); await settlePromises(); state = h.flush();
  assert.equal(state.entries[0].id, 'one'); void state.retry(); h.flush();
  assert.equal(h.calls[2].cursor, null);
  h.calls[2].task.resolve(page(['new'])); await settlePromises();
  assert.equal(h.flush().entries[0].id, 'new'); h.unmount();
});

test('new local outcomes update in place without discarding loaded older pages', async () => {
  const h = setup(); h.render(() => h.exports.useCodexResetHistory('a', 'local'));
  h.calls[0].task.resolve(page(['pending'], 30)); await settlePromises();
  let state = h.flush(); void state.more(); h.flush();
  h.calls[1].task.resolve(page(['older'], 10)); await settlePromises(); h.flush();
  h.emit(); h.flush();
  h.calls[2].task.resolve({ entries: [{ id: 'pending', kind: 'reset', at: 1 }], nextCursor: 30 });
  await settlePromises(); state = h.flush();
  assert.equal(state.entries.map((entry: any) => entry.id).join(','), 'pending,older');
  assert.equal(state.entries[0].kind, 'reset');
  assert.equal(state.cursor, 10);
  h.unmount();
});
