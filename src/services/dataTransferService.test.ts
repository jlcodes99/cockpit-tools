import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { ALL_PLATFORM_IDS } from '../types/platform';
import { CODEX_TOP_LAYOUT_STORAGE_KEY, createDefaultCodexTopLayout, normalizeCodexTopLayout } from '../utils/codexTopLayoutPreferences';

function harness() {
  const items = new Map<string, string>();
  const calls: string[] = [];
  const events: string[] = [];
  let failLayoutWrite = false;
  const localStorage = {
    getItem: (key: string) => items.get(key) ?? null,
    removeItem: (key: string) => { items.delete(key); },
    setItem: (key: string, value: string) => {
      if (key === CODEX_TOP_LAYOUT_STORAGE_KEY && failLayoutWrite) throw Error('quota');
      items.set(key, value);
    },
  };
  const globals = { localStorage, window: { dispatchEvent: (event: { type: string }) => { events.push(event.type); } },
    Event: class { constructor(readonly type: string) {} },
    CustomEvent: class { constructor(readonly type: string, readonly options: unknown) {} },
  };
  const preferences = loadHookModule(new URL('../utils/codexTopLayoutPreferences.ts', import.meta.url), {}, globals).exports;
  const moduleUrl = new URL('./dataTransferService.ts', import.meta.url);
  const source = ts.createSourceFile('service.ts', readFileSync(moduleUrl, 'utf8'), ts.ScriptTarget.ES2022);
  const fallback = new Proxy({}, { get: (_target, key) => {
    if (key === 'getGroupSettings') return async () => ({ groupMappings: {}, groupNames: {}, groupOrder: [] });
    if (key === 'getCodexWakeupState') return async () => ({ enabled: false, tasks: [], model_presets: [] });
    if (key === 'getCodexWakeupCliStatus') return async () => ({});
    if (key === 'loadCurrentAccountRefreshMinutesMap') return () => ({});
    if (key === 'loadWakeupOfficialLsVersionMode') return () => 'auto';
    if (key === 'DEFAULT_WAKEUP_OFFICIAL_LS_VERSION_MODE') return 'auto';
    return async () => [];
  } });
  const imports: Record<string, unknown> = {};
  for (const statement of source.statements) {
    if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) imports[statement.moduleSpecifier.text] = fallback;
  }
  imports['../types/platform'] = { ALL_PLATFORM_IDS };
  imports['../utils/codexTopLayoutPreferences'] = preferences;
  imports['@tauri-apps/api/core'] = { invoke: async (command: string) => {
    calls.push(command);
    if (command === 'data_transfer_get_user_config') return {};
    if (command === 'data_transfer_get_instance_store') return { instances: [], defaultSettings: { extraArgs: '' } };
    if (command === 'data_transfer_apply_user_config') return false;
    return [];
  } };
  const service = loadHookModule(moduleUrl, imports, globals).exports;
  const selection = { includeAccounts: false, includeConfig: true };
  return { items, calls, events, service, selection,
    failWrites() { failLayoutWrite = true; },
    async bundle() { return JSON.parse(await service.exportDataTransferJson(selection)); },
    async import(bundle: unknown) { return service.importDataTransferJson(JSON.stringify(bundle), selection); },
  };
}

test('config export includes normalized top order and import restores it with a same-window notification', async () => {
  const h = harness(); h.items.set(CODEX_TOP_LAYOUT_STORAGE_KEY, JSON.stringify({ order: ['sessions', 'sessions', 'old', 'proxy'] }));
  const bundle = await h.bundle();
  assert.deepEqual(bundle.config.codex_top_layout, normalizeCodexTopLayout({ order: ['sessions', 'proxy'] }));
  h.items.delete(CODEX_TOP_LAYOUT_STORAGE_KEY);
  const result = await h.import(bundle);
  assert.equal(result.config_result.applied, true);
  assert.deepEqual(JSON.parse(h.items.get(CODEX_TOP_LAYOUT_STORAGE_KEY)!), { order: bundle.config.codex_top_layout.order });
  assert.ok(h.events.includes('codex:top-layout-updated'));
});

test('old backups without a top layout preserve current local ordering', async () => {
  const h = harness(); const bundle = await h.bundle(); delete bundle.config.codex_top_layout;
  const existing = JSON.stringify({ order: ['proxy', 'sessions'] }); h.items.set(CODEX_TOP_LAYOUT_STORAGE_KEY, existing);
  await h.import(bundle);
  assert.equal(h.items.get(CODEX_TOP_LAYOUT_STORAGE_KEY), existing);
  assert.ok(!h.events.includes('codex:top-layout-updated'));
});

test('import normalizes duplicates and unknown tabs but rejects structurally invalid layout before config writes', async () => {
  const h = harness(); const bundle = await h.bundle();
  bundle.config.codex_top_layout = { order: ['proxy', 'old', 'proxy', 'top-layout'], placement: { proxy: 'more' } };
  await h.import(bundle);
  assert.deepEqual(JSON.parse(h.items.get(CODEX_TOP_LAYOUT_STORAGE_KEY)!).order, normalizeCodexTopLayout(bundle.config.codex_top_layout).order);
  h.calls.length = 0; bundle.config.codex_top_layout = { order: [42] };
  await assert.rejects(h.import(bundle), /codex_top_layout_invalid/);
  assert.ok(!h.calls.includes('data_transfer_apply_user_config'));
});

test('layout write failure reports partial import explicitly and preserves the previous saved order', async () => {
  const h = harness(); const bundle = await h.bundle();
  const old = JSON.stringify({ order: ['sessions'] }); h.items.set(CODEX_TOP_LAYOUT_STORAGE_KEY, old); h.failWrites();
  await assert.rejects(h.import(bundle), /codex_top_layout_import_failed/);
  assert.equal(h.items.get(CODEX_TOP_LAYOUT_STORAGE_KEY), old);
  assert.ok(h.calls.includes('data_transfer_apply_user_config'));
  assert.ok(!h.events.includes('codex:top-layout-updated'));
});

test('config export refuses a corrupt saved layout instead of silently backing up defaults', async () => {
  const h = harness(); h.items.set(CODEX_TOP_LAYOUT_STORAGE_KEY, '{broken');
  await assert.rejects(h.bundle(), /codex_top_layout_read_failed/);
  assert.equal(h.items.get(CODEX_TOP_LAYOUT_STORAGE_KEY), '{broken');
  assert.deepEqual(createDefaultCodexTopLayout().order.slice(0, 5), ['overview', 'providers', 'wakeup', 'instances', 'sessions']);
});
