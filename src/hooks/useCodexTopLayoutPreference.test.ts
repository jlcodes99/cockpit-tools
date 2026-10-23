import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import * as preferences from '../utils/codexTopLayoutPreferences';

function harness(raw: string | null = null) {
  let stored = raw;
  let writes = 0;
  let failRead = false;
  let failWrite = false;
  const listeners = new Map<string, Set<(event: any) => void>>();
  const storage = {
    getItem() { if (failRead) throw Error('unavailable'); return stored; },
    setItem(_key: string, value: string) { writes++; if (failWrite) throw Error('full'); stored = value; },
  };
  const window = {
    addEventListener(name: string, callback: (event: any) => void) {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name)!.add(callback);
    },
    removeEventListener(name: string, callback: (event: any) => void) { listeners.get(name)?.delete(callback); },
    dispatchEvent(event: { type: string; key?: string }) { listeners.get(event.type)?.forEach(callback => callback(event)); },
  };
  const utils = loadHookModule(new URL('../utils/codexTopLayoutPreferences.ts', import.meta.url), {}, {
    localStorage: storage, window, CustomEvent: class { constructor(readonly type: string, readonly detail: unknown) {} },
  }).exports;
  const hook = loadHookModule(new URL('./useCodexTopLayoutPreference.ts', import.meta.url), {
    '../utils/codexTopLayoutPreferences': utils,
  }, { window });
  let state: any;
  const flush = () => state = hook.flush();
  hook.render(() => hook.exports.useCodexTopLayoutPreference()); flush();
  return { hook, flush, utils, window, get state() { return state; }, get writes() { return writes; }, get stored() { return stored; },
    failReads(value: boolean) { failRead = value; }, failWrites(value: boolean) { failWrite = value; },
    replace(value: string | null) { stored = value; window.dispatchEvent({ type: 'storage', key: preferences.CODEX_TOP_LAYOUT_STORAGE_KEY }); flush(); },
  };
}

test('mounting with corrupt storage uses a preview without overwriting it; read retry restores recovered data', () => {
  const h = harness('{broken');
  assert.equal(h.state.storageError, 'read'); assert.equal(h.writes, 0); assert.equal(h.stored, '{broken');
  h.replace(JSON.stringify({ order: ['sessions'] }));
  assert.equal(h.state.storageError, null); assert.equal(h.state.layout.order[0], 'sessions'); assert.equal(h.writes, 0);
  h.failReads(true); h.window.dispatchEvent({ type: 'storage', key: preferences.CODEX_TOP_LAYOUT_STORAGE_KEY }); h.flush();
  assert.equal(h.state.storageError, 'read'); assert.equal(h.state.layout.order[0], 'sessions');
  h.failReads(false); h.state.retry(); h.flush(); assert.equal(h.state.storageError, null); assert.equal(h.writes, 0);
  h.hook.unmount();
});

test('failed save retains immediate ordering and retries the same preview after storage recovers', () => {
  const h = harness(); const edited = preferences.moveCodexTopLayoutTab(preferences.createDefaultCodexTopLayout(), 6, 0);
  h.failWrites(true); h.state.changeLayout(edited); h.flush();
  assert.equal(h.state.storageError, 'save'); assert.deepEqual(h.state.layout.order, edited.order); assert.equal(h.stored, null);
  h.replace(JSON.stringify({ order: ['sessions'] }));
  assert.deepEqual(h.state.layout.order, edited.order); assert.equal(h.state.storageError, 'save');
  h.failWrites(false); h.state.retry(); h.flush();
  assert.equal(h.state.storageError, null); assert.deepEqual(JSON.parse(h.stored!).order, edited.order); assert.equal(h.writes, 2);
  h.hook.unmount();
});

test('same-window import events reread persisted order without writing or looping', () => {
  const h = harness(); const imported = preferences.moveCodexTopLayoutTab(preferences.createDefaultCodexTopLayout(), 5, 0);
  h.utils.writeCodexTopLayoutPreference(imported); h.flush();
  assert.deepEqual(Array.from(h.state.layout.order), imported.order); assert.equal(h.writes, 1);
  h.hook.unmount();
});
