import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('a quota refresh re-reads the wakeup plan after an older read finishes', async () => {
  const overview = (nextRunAt: number) => ({
    runtime: null,
    state: { tasks: [{ next_run_at: nextRunAt }] },
  });
  let finishOldRead!: (value: ReturnType<typeof overview>) => void;
  const oldRead = new Promise<ReturnType<typeof overview>>((resolve) => {
    finishOldRead = resolve;
  });
  let reads = 0;
  const { exports } = loadHookModule(new URL('./useCodexWakeupStore.ts', import.meta.url), {
    '../services/codexWakeupService': {
      getCodexWakeupOverview: () => ++reads === 1 ? oldRead : Promise.resolve(overview(500)),
    },
  });
  const store = exports.useCodexWakeupStore;
  const initial = store.getState().loadAll();
  const refresh = store.getState().loadAll(true);
  const duplicate = store.getState().loadAll(true);
  assert.equal(reads, 1);

  finishOldRead(overview(700));
  await Promise.all([initial, refresh, duplicate]);

  assert.equal(reads, 2);
  assert.equal(store.getState().state.tasks[0].next_run_at, 500);
});
