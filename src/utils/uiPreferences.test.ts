import assert from 'node:assert/strict';
import test from 'node:test';

type Store = Record<string, string>;

function setup(initial: Store = {}, durable: Store = {}, options: { editDuringLoad?: string; failLoad?: boolean; failSave?: boolean } = {}) {
  const local: Store = { ...initial };
  const writes: Array<Record<string, string>> = [];
  const storage = {
    getItem: (key: string) => local[key] ?? null,
    setItem: (key: string, value: string) => { local[key] = value; },
    removeItem: (key: string) => { delete local[key]; },
  };
  const invoke = async (command: string, args?: { values?: Store }) => {
    if (command === 'load_ui_preferences') {
      if (options.editDuringLoad) local['agtools.platform_layout.v1'] = options.editDuringLoad;
      if (options.failLoad) throw new Error('load failed');
      return { values: { ...durable } };
    }
    if (command === 'save_ui_preferences') {
      writes.push({ ...(args?.values ?? {}) });
      if (options.failSave) throw new Error('save failed');
      Object.assign(durable, args?.values ?? {});
      return { values: { ...durable } };
    }
    throw new Error(`unexpected command ${command}`);
  };
  (globalThis as any).localStorage = storage;
  (globalThis as any).window = { dispatchEvent() {}, setTimeout };
  (globalThis as any).window.__TAURI_INTERNALS__ = { invoke };
  return { local, durable, writes };
}

async function loadFresh() {
  return import(`./uiPreferences.ts?test=${Date.now()}-${Math.random()}`);
}

test('missing local cache restores durable layout', async () => {
  const durable = JSON.stringify({ orderedPlatformIds: ['codex'], _layoutUpdatedAt: Date.now() });
  const { local } = setup({}, { 'agtools.platform_layout.v1': durable });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.equal(local['agtools.platform_layout.v1'], durable);
});

test('legacy local layout migrates without being overwritten', async () => {
  const legacy = JSON.stringify({ orderedPlatformIds: ['grok'] });
  const { durable } = setup({ 'agtools.platform_layout.v1': legacy });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.ok(durable['agtools.platform_layout.v1']);
  assert.deepEqual(JSON.parse(durable['agtools.platform_layout.v1']).orderedPlatformIds, ['grok']);
});

test('newer local cache wins over older durable value', async () => {
  const local = JSON.stringify({ orderedPlatformIds: ['grok'], _layoutUpdatedAt: 200 });
  const old = JSON.stringify({ orderedPlatformIds: ['codex'], _layoutUpdatedAt: 100 });
  const state = setup({ 'agtools.platform_layout.v1': local }, { 'agtools.platform_layout.v1': old });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.equal(state.local['agtools.platform_layout.v1'], local);
});

test('edits during hydrate are never overwritten', async () => {
  const old = JSON.stringify({ orderedPlatformIds: ['codex'], _layoutUpdatedAt: 100 });
  const edited = JSON.stringify({ orderedPlatformIds: ['grok'], _layoutUpdatedAt: 200 });
  const { local } = setup({ 'agtools.platform_layout.v1': old }, { 'agtools.platform_layout.v1': old }, { editDuringLoad: edited });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.equal(local['agtools.platform_layout.v1'], edited);
});

test('load failure leaves local cache and unknown durable file intact', async () => {
  const localValue = JSON.stringify({ orderedPlatformIds: ['grok'] });
  const { local, writes } = setup({ 'agtools.platform_layout.v1': localValue }, {}, { failLoad: true });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.equal(local['agtools.platform_layout.v1'], localValue);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(writes.length, 0);
});

test('invalid durable JSON never gets overwritten by a local fallback', async () => {
  const localValue = JSON.stringify({ orderedPlatformIds: ['grok'] });
  const { durable, writes } = setup({ 'agtools.platform_layout.v1': localValue }, { 'agtools.platform_layout.v1': '{invalid' });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(writes.length, 0);
  assert.equal(durable['agtools.platform_layout.v1'], '{invalid');
});

test('failed save retains newest local layout and can be retried', async () => {
  const options = { failSave: true };
  const { local, durable } = setup({}, {}, options);
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['grok'] }));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(JSON.parse(local['agtools.platform_layout.v1']).orderedPlatformIds[0], 'grok');
  assert.ok(mod.getPlatformLayoutPersistenceError());
  options.failSave = false;
  await mod.retryPlatformLayoutPersistence();
  assert.equal(JSON.parse(durable['agtools.platform_layout.v1']).orderedPlatformIds[0], 'grok');
  assert.equal(mod.getPlatformLayoutPersistenceError(), null);
});

test('revisions remain increasing for edits in the same millisecond', async () => {
  const { local } = setup();
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  const originalNow = Date.now;
  Date.now = () => 123456;
  try {
    mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['codex'] }));
    const first = JSON.parse(local['agtools.platform_layout.v1'])._layoutUpdatedAt;
    mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['grok'] }));
    const second = JSON.parse(local['agtools.platform_layout.v1'])._layoutUpdatedAt;
    assert.ok(second > first);
  } finally {
    Date.now = originalNow;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
});

test('saves are serialized in call order', async () => {
  const state = setup();
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['codex'] }));
  mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['grok'] }));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(JSON.parse(state.durable['agtools.platform_layout.v1']).orderedPlatformIds[0], 'grok');
  assert.equal(state.writes.length, 2);
});

test('a slow save never allows a later edit to write concurrently', async () => {
  setup();
  let releaseFirst: (() => void) | undefined;
  const firstWrite = new Promise<void>((resolve) => { releaseFirst = resolve; });
  const saved: Store[] = [];
  (globalThis as any).window.__TAURI_INTERNALS__.invoke = async (command: string, args?: { values?: Store }) => {
    if (command === 'load_ui_preferences') return { values: {} };
    saved.push({ ...args?.values });
    if (saved.length === 1) await firstWrite;
    return { values: args?.values };
  };
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['codex'] }));
  mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['grok'] }));
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(saved.length, 1);
  await mod.retryPlatformLayoutPersistence();
  assert.ok(mod.getPlatformLayoutPersistenceError(), 'pending write remains visible on retry');
  assert.equal(saved.length, 1, 'retry must not overlap the existing write');
  releaseFirst!();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(saved.length, 2);
  assert.equal(JSON.parse(saved[1]['agtools.platform_layout.v1']).orderedPlatformIds[0], 'grok');
});

test('load timeout is retryable and does not overwrite a newer durable layout', async () => {
  const key = 'agtools.platform_layout.v1';
  const cached = JSON.stringify({ orderedPlatformIds: ['grok'], _layoutUpdatedAt: 10 });
  const durable = JSON.stringify({ orderedPlatformIds: ['codex'], _layoutUpdatedAt: 20 });
  const state = setup({ [key]: cached }, { [key]: durable });
  const originalInvoke = (globalThis as any).window.__TAURI_INTERNALS__.invoke;
  let loads = 0;
  (globalThis as any).window.__TAURI_INTERNALS__.invoke = (command: string, args?: unknown) => {
    if (command === 'load_ui_preferences' && loads++ === 0) return new Promise(() => {});
    return originalInvoke(command, args);
  };
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.ok(mod.getPlatformLayoutPersistenceError());
  assert.equal(state.writes.length, 0);
  assert.equal(state.local[key], cached);
  await mod.retryPlatformLayoutPersistence();
  assert.equal(state.local[key], durable);
  assert.equal(mod.getPlatformLayoutPersistenceError(), null);
});
