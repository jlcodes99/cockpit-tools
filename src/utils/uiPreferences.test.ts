import assert from 'node:assert/strict';
import test from 'node:test';

type Store = Record<string, string>;

function setup(initial: Store = {}, durable: Store = {}, options: { failLoad?: boolean; failSave?: boolean } = {}) {
  const local: Store = { ...initial };
  const writes: Array<Record<string, string>> = [];
  let revision = 0;
  const storage = {
    getItem: (key: string) => local[key] ?? null,
    setItem: (key: string, value: string) => { local[key] = value; },
    removeItem: (key: string) => { delete local[key]; },
  };
  const invoke = async (command: string, args?: { key: string; value: string; expectedRevision: number }) => {
    if (command === 'load_ui_preferences') {
      if (options.failLoad) throw new Error('load failed');
      return { values: { ...durable }, revisions: { 'agtools.platform_layout.v1': revision } };
    }
    if (command === 'update_ui_preference' && args) {
      writes.push({ [args.key]: args.value });
      if (options.failSave) throw new Error('save failed');
      if (args.expectedRevision !== revision) throw new Error('ui_preference_revision_conflict');
      revision++;
      durable[args.key] = args.value;
      return { values: { ...durable }, revisions: { 'agtools.platform_layout.v1': revision } };
    }
    throw new Error(`unexpected command ${command}`);
  };
  (globalThis as any).localStorage = storage;
  (globalThis as any).window = { dispatchEvent() {}, setTimeout };
  (globalThis as any).window.__TAURI_INTERNALS__ = { invoke };
  return { local, durable, writes, changeDisk(value: string) {
    revision++;
    durable['agtools.platform_layout.v1'] = value;
  } };
}

async function loadFresh() {
  return import(`./uiPreferences.ts?test=${Date.now()}-${Math.random()}`);
}

test('missing local cache restores durable layout', async () => {
  const durable = JSON.stringify({ orderedPlatformIds: ['codex'], _layoutUpdatedAt: Date.now() });
  const { local } = setup({}, { 'agtools.platform_layout.v1': durable });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  assert.equal(mod.readPlatformLayout(), durable);
  assert.equal(local['agtools.platform_layout.v1'], undefined);
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
  assert.equal(state.durable['agtools.platform_layout.v1'], local);
});

test('edits during hydrate are never overwritten', async () => {
  const old = JSON.stringify({ orderedPlatformIds: ['codex'], _layoutUpdatedAt: 100 });
  const edited = JSON.stringify({ orderedPlatformIds: ['grok'], _layoutUpdatedAt: 200 });
  const { local, durable } = setup({ 'agtools.platform_layout.v1': old }, { 'agtools.platform_layout.v1': old });
  const mod = await loadFresh();
  const hydration = mod.hydrateUiPreferences();
  mod.persistPlatformLayout(edited);
  await hydration;
  assert.equal(local['agtools.platform_layout.v1'], old);
  assert.deepEqual(JSON.parse(durable['agtools.platform_layout.v1']).orderedPlatformIds, ['grok']);
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
  assert.equal(JSON.parse(mod.readPlatformLayout()!).orderedPlatformIds[0], 'grok');
  assert.equal(local['agtools.platform_layout.v1'], undefined);
  assert.ok(mod.getPlatformLayoutPersistenceError());
  options.failSave = false;
  await mod.retryPlatformLayoutPersistence();
  assert.equal(JSON.parse(durable['agtools.platform_layout.v1']).orderedPlatformIds[0], 'grok');
  assert.equal(mod.getPlatformLayoutPersistenceError(), null);
});

test('revisions remain increasing for edits in the same millisecond', async () => {
  setup();
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  const originalNow = Date.now;
  Date.now = () => 123456;
  try {
    mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['codex'] }));
    const first = JSON.parse(mod.readPlatformLayout()!)._layoutUpdatedAt;
    mod.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['grok'] }));
    const second = JSON.parse(mod.readPlatformLayout()!)._layoutUpdatedAt;
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

test('restore persists the layout and an explicit clear cannot revive the old WebView value', async () => {
  const key = 'agtools.platform_layout.v1';
  const legacy = JSON.stringify({ orderedPlatformIds: ['grok'] });
  const { local, durable } = setup({ [key]: legacy });
  const mod = await loadFresh();
  const imported = { orderedEntryIds: ['platform:codex'], sidebarEntryIds: [] };
  await mod.restorePlatformLayout(mod.validateImportedPlatformLayout(imported));
  assert.deepEqual(JSON.parse(durable[key]).sidebarEntryIds, []);
  assert.equal(local[key], legacy);
  assert.deepEqual((await mod.exportPlatformLayout() as typeof imported).orderedEntryIds, imported.orderedEntryIds);
  await mod.restorePlatformLayout(mod.validateImportedPlatformLayout(null));
  assert.equal(durable[key], 'null');
  assert.equal(await mod.exportPlatformLayout(), null);
  const reopened = await loadFresh();
  await reopened.hydrateUiPreferences();
  assert.equal(reopened.readPlatformLayout(), null);
  assert.equal(reopened.hasSavedPlatformLayout(), true);
  assert.equal(local[key], legacy);
});

test('a failed restore does not replace the visible layout or overwrite a newer disk revision', async () => {
  const key = 'agtools.platform_layout.v1';
  const initial = JSON.stringify({ orderedPlatformIds: ['grok'], _layoutUpdatedAt: 100 });
  const newer = JSON.stringify({ orderedPlatformIds: ['claude_manager'], _layoutUpdatedAt: 200 });
  const { durable, changeDisk } = setup({}, { [key]: initial });
  const mod = await loadFresh();
  await mod.hydrateUiPreferences();
  changeDisk(newer);
  await assert.rejects(mod.restorePlatformLayout('{"orderedPlatformIds":["codex"]}'), /revision_conflict/);
  assert.equal(mod.readPlatformLayout(), newer);
  assert.equal(durable[key], newer);
  assert.deepEqual((await mod.exportPlatformLayout() as { orderedPlatformIds: string[] }).orderedPlatformIds, ['claude_manager']);
  mod.applyPlatformLayoutSnapshot({ values: { [key]: initial }, revisions: { [key]: 0 } });
  assert.equal(mod.readPlatformLayout(), newer);
  await mod.restorePlatformLayout('{"orderedPlatformIds":["codex"]}');
  assert.deepEqual(JSON.parse(durable[key]).orderedPlatformIds, ['codex']);
});

test('a slow save never allows a later edit to write concurrently', async () => {
  setup();
  let releaseFirst: (() => void) | undefined;
  const firstWrite = new Promise<void>((resolve) => { releaseFirst = resolve; });
  const saved: Store[] = [];
  (globalThis as any).window.__TAURI_INTERNALS__.invoke = async (command: string, args?: { key: string; value: string }) => {
    if (command === 'load_ui_preferences') return { values: {} };
    saved.push({ [args!.key]: args!.value });
    if (saved.length === 1) await firstWrite;
    return { values: { [args!.key]: args!.value } };
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
  assert.equal(state.local[key], cached);
  assert.equal(mod.readPlatformLayout(), durable);
  assert.equal(mod.getPlatformLayoutPersistenceError(), null);
});
