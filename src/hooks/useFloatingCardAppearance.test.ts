import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { normalizeFloatingCardOpacity } from '../utils/floatingCardAppearance';

function harness() {
  const loads: ReturnType<typeof deferred<any>>[] = [];
  const saves: { patch: any; task: ReturnType<typeof deferred<any>> }[] = [];
  const listeners = new Map<string, (event: any) => void>();
  const cleaned: string[] = [];
  const hook = loadHookModule(new URL('./useFloatingCardAppearance.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke() { const task = deferred<any>(); loads.push(task); return task.promise; } },
    '@tauri-apps/api/event': { TauriEvent: { WINDOW_FOCUS: 'focus' }, async listen(event: string, callback: (event: any) => void) { listeners.set(event, callback); return () => { cleaned.push(event); listeners.delete(event); }; } },
    '../services/floatingCardService': { FLOATING_CARD_APPEARANCE_CHANGED_EVENT: 'appearance', updateFloatingCardAppearance(patch: any) { const task = deferred<any>(); saves.push({ patch, task }); return task.promise; } },
    '../utils/floatingCardAppearance': { normalizeFloatingCardOpacity },
  });
  hook.render(() => hook.exports.useFloatingCardAppearance());
  return { hook, loads, saves, listeners, cleaned, async ready() { await settlePromises(); loads[0].resolve({}); await settlePromises(); return hook.flush(); } };
}

test('floating appearance loads legacy defaults without writing config and cleans listeners', async () => {
  const h = harness();
  const state = await h.ready();
  assert.equal(state.minimal, false); assert.equal(state.backgroundOpacity, 1); assert.equal(state.busy, false);
  assert.equal(h.saves.length, 0);
  h.hook.unmount(); assert.deepEqual(h.cleaned.sort(), ['appearance', 'focus']);
});
test('failed appearance save keeps current settings, clears old error on retry, and deduplicates submissions', async () => {
  const h = harness(); let state = await h.ready();
  const first = state.update({ minimal: true });
  await state.update({ backgroundOpacity: .4 }); assert.equal(h.saves.length, 1);
  h.saves[0].task.reject(new Error('disk full')); await first;
  state = h.hook.flush(); assert.equal(state.minimal, false); assert.match(state.error, /disk full/); assert.equal(state.busy, false);
  const retry = state.update({ backgroundOpacity: .4 });
  assert.equal(h.hook.flush().error, '');
  assert.deepEqual(Object.keys(h.saves[1].patch), ['backgroundOpacity']);
  h.saves[1].task.resolve({ minimal: false, backgroundOpacity: .4 }); await retry;
  assert.equal(h.hook.flush().backgroundOpacity, .4);
});
test('cross-window appearance event wins over delayed bootstrap and save responses', async () => {
  const h = harness(); await settlePromises();
  h.listeners.get('appearance')!({ payload: { minimal: true, backgroundOpacity: .4 } });
  h.loads[0].resolve({ floating_card_minimal: false, floating_card_background_opacity: 1 }); await settlePromises();
  let state = h.hook.flush(); assert.equal(state.minimal, true); assert.equal(state.backgroundOpacity, .4);
  const pending = state.update({ minimal: false });
  h.listeners.get('appearance')!({ payload: { minimal: false, backgroundOpacity: .25 } });
  h.saves[0].task.resolve({ minimal: false, backgroundOpacity: .4 }); await pending;
  state = h.hook.flush(); assert.equal(state.backgroundOpacity, .25);
});
