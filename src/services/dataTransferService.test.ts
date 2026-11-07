import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { ALL_PLATFORM_IDS } from '../types/platform';
import { CODEX_TOP_LAYOUT_STORAGE_KEY, createDefaultCodexTopLayout, normalizeCodexTopLayout } from '../utils/codexTopLayoutPreferences';

type Account = { id: string; email?: string; user_id?: string; account_id?: string; api_key?: string };
const SORT_KEY = 'agtools.codex.accounts.custom_sort_order.v1';
const ACTIVE_KEY = 'agtools.codex.accounts.custom_sort_active.v1';
const SORT_BY_KEY = 'agtools.codex.accounts_overview_filters.sort_by';
const SORT_LIST = 'codex.accounts.custom_sort';

function harness(accounts: Account[] = [], diskOrder: string[] = [], importedAccounts: Account[] = []) {
  const items = new Map<string, string>();
  const calls: string[] = [];
  const events: string[] = [];
  const listeners = new Map<string, Set<() => void>>();
  let failLayoutWrite = false;
  let failSortSave = false;
  let failActiveWrite = false;
  let savedOrder = [...diskOrder];
  const localStorage = {
    getItem: (key: string) => items.get(key) ?? null,
    removeItem: (key: string) => { items.delete(key); },
    setItem: (key: string, value: string) => {
      if (key === CODEX_TOP_LAYOUT_STORAGE_KEY && failLayoutWrite) throw Error('quota');
      if (key === ACTIVE_KEY && failActiveWrite) throw Error('quota');
      items.set(key, value);
    },
  };
  const window = {
    addEventListener(type: string, listener: () => void) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(listener);
    },
    removeEventListener(type: string, listener: () => void) { listeners.get(type)?.delete(listener); },
    dispatchEvent(event: { type: string }) {
      events.push(event.type);
      listeners.get(event.type)?.forEach(listener => listener());
    },
  };
  const globals = { localStorage, window,
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
  const stubImports = (file: ts.SourceFile) => {
    const imports: Record<string, unknown> = {};
    for (const statement of file.statements) {
      if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) imports[statement.moduleSpecifier.text] = fallback;
    }
    return imports;
  };
  const imports = stubImports(source);
  imports['../types/platform'] = { ALL_PLATFORM_IDS };
  imports['../utils/codexTopLayoutPreferences'] = preferences;
  imports['./codexService'] = { listCodexAccounts: async () => accounts };
  imports['./accountTransferService'] = new Proxy(fallback, { get: (target, key) => {
    if (key === 'importAllAccountsFromTransferJson') return async () => {
      accounts.push(...importedAccounts);
      return { imported_count: importedAccounts.length };
    };
    return Reflect.get(target, key);
  } });
  imports['@tauri-apps/api/core'] = { invoke: async (command: string, args?: { items: string[] }) => {
    calls.push(command);
    if (command === 'load_user_memory') return { dismissed: {}, lists: { [SORT_LIST]: [...savedOrder] } };
    if (command === 'save_user_memory_list') {
      if (failSortSave) throw Error('disk unavailable');
      savedOrder = [...args!.items];
      return;
    }
    if (command === 'data_transfer_get_user_config') return {};
    if (command === 'data_transfer_get_instance_store') return { instances: [], defaultSettings: { extraArgs: '' } };
    if (command === 'data_transfer_apply_user_config') return false;
    return [];
  } };
  const memory = loadHookModule(new URL('../utils/userMemory.ts', import.meta.url), {
    '@tauri-apps/api/core': imports['@tauri-apps/api/core'],
  }, globals).exports;
  const filters = loadHookModule(new URL('../utils/accountsOverviewFilterPersistence.ts', import.meta.url), {}, globals).exports;
  const overviewUrl = new URL('../utils/codexAccountOverview.ts', import.meta.url);
  const overviewImports = stubImports(ts.createSourceFile('overview.ts', readFileSync(overviewUrl, 'utf8'), ts.ScriptTarget.ES2022));
  overviewImports['./userMemory'] = memory;
  overviewImports['./accountsOverviewFilterPersistence'] = filters;
  const overview = loadHookModule(overviewUrl, overviewImports, globals).exports;
  imports['../utils/userMemory'] = memory;
  imports['../utils/codexAccountOverview'] = overview;
  const service = loadHookModule(moduleUrl, imports, globals).exports;
  const selection = { includeAccounts: false, includeConfig: true };
  return { items, calls, events, service, selection, memory, overview, window, listeners,
    get savedOrder() { return savedOrder; },
    failSortWrites() { failSortSave = true; },
    failActiveWrites() { failActiveWrite = true; },
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

const sourceAccounts: Account[] = [
  { id: 'source-personal', email: 'same@example.test', user_id: 'user-1', account_id: 'personal' },
  { id: 'source-team', email: 'same@example.test', user_id: 'user-1', account_id: 'team' },
];

async function sortedBundle() {
  const source = harness(sourceAccounts, ['source-team', 'source-personal']);
  source.items.set(ACTIVE_KEY, '1');
  return source.bundle();
}

test('sort export hydrates disk order and includes stable account identities without secrets or local ids', async () => {
  const source = harness(sourceAccounts.map(a => ({ ...a, api_key: 'secret-not-for-sort' })), ['source-team', 'source-personal']);
  source.items.set(ACTIVE_KEY, '1');
  const bundle = await source.bundle();
  assert.deepEqual(bundle.config.codex_account_sort, {
    active: true,
    account_refs: [
      { platform: 'codex', email: 'same@example.test', userId: 'user-1', accountId: 'team' },
      { platform: 'codex', email: 'same@example.test', userId: 'user-1', accountId: 'personal' },
    ],
  });
  assert.ok(!JSON.stringify(bundle.config.codex_account_sort).includes('secret-not-for-sort'));
  assert.ok(!JSON.stringify(bundle.config.codex_account_sort).includes('source-'));
});

test('another device restores order using its own ids, appends local accounts and persists across restart', async () => {
  const targetAccounts = [
    ...sourceAccounts.map(a => ({ ...a, id: a.id.replace('source-', 'target-') })),
    { id: 'local-only', email: 'local@example.test' },
  ];
  const target = harness(targetAccounts, ['local-only', 'target-personal', 'target-team']);
  target.items.set(SORT_BY_KEY, JSON.stringify('weekly'));
  const result = await target.import(await sortedBundle());
  const expected = ['target-team', 'target-personal', 'local-only'];
  assert.equal(result.config_result.unresolved_account_ref_count, 0);
  assert.deepEqual(target.savedOrder, expected);
  assert.deepEqual([...target.overview.readCodexCustomSortOrder()], expected);
  assert.deepEqual(JSON.parse(target.items.get(SORT_KEY)!), expected);
  assert.equal(target.items.get(ACTIVE_KEY), '1');
  assert.equal(JSON.parse(target.items.get(SORT_BY_KEY)!), 'custom');
  assert.ok(target.events.includes('codex:custom-sort-restored'));
  const restarted = harness(targetAccounts, target.savedOrder);
  target.items.forEach((v, k) => restarted.items.set(k, v));
  await restarted.memory.hydrateUserMemory();
  assert.deepEqual([...restarted.overview.readCodexCustomSortOrder()], expected);
  assert.equal(restarted.overview.readCodexCustomSortActive(), true);
});

test('missing or ambiguous identities are counted rather than assigned to the wrong account', async () => {
  const target = harness([
    { ...sourceAccounts[0], id: 'personal-only' },
    { id: 'duplicate-a', email: 'duplicate@example.test' },
    { id: 'duplicate-b', email: 'duplicate@example.test' },
  ], ['duplicate-b', 'personal-only', 'duplicate-a']);
  const bundle = await sortedBundle();
  bundle.config.codex_account_sort.account_refs.push({ platform: 'codex', email: 'duplicate@example.test' });
  const result = await target.import(bundle);
  assert.equal(result.config_result.unresolved_account_ref_count, 2);
  assert.deepEqual(target.savedOrder, ['personal-only', 'duplicate-b', 'duplicate-a']);
});

test('repeated references restore once, and a backup without sorting leaves local sorting unchanged', async () => {
  const target = harness(sourceAccounts, ['source-personal', 'source-team']);
  const bundle = await sortedBundle();
  bundle.config.codex_account_sort.account_refs.push(bundle.config.codex_account_sort.account_refs[0]);
  await target.import(bundle);
  assert.deepEqual(target.savedOrder, ['source-team', 'source-personal']);
  delete bundle.config.codex_account_sort;
  target.items.set(ACTIVE_KEY, '0');
  target.events.length = 0;
  await target.import(bundle);
  assert.deepEqual(target.savedOrder, ['source-team', 'source-personal']);
  assert.equal(target.items.get(ACTIVE_KEY), '0');
  assert.ok(!target.events.includes('codex:custom-sort-restored'));
});

test('inactive sorting removes stale custom mode without replacing another ordinary sort', async () => {
  for (const [existing, expected] of [['custom', 'created_at'], ['weekly', 'weekly']]) {
    const target = harness(sourceAccounts);
    target.items.set(ACTIVE_KEY, '1');
    target.items.set(SORT_BY_KEY, JSON.stringify(existing));
    const bundle = await sortedBundle();
    bundle.config.codex_account_sort.active = false;
    await target.import(bundle);
    assert.equal(target.items.get(ACTIVE_KEY), '0');
    assert.equal(JSON.parse(target.items.get(SORT_BY_KEY)!), expected);
    assert.deepEqual(target.savedOrder, ['source-team', 'source-personal']);
  }
});

test('invalid sort data is rejected before configuration writes', async () => {
  const invalid = [null, { active: 'true', account_refs: [] }, { active: true, account_refs: [null] },
    { active: true, account_refs: [{ platform: 'cursor', email: 'a' }] },
    { active: true, account_refs: [{ platform: 'codex', accountId: 42 }] }];
  for (const value of invalid) {
    const target = harness();
    const bundle = await sortedBundle();
    bundle.config.codex_account_sort = value;
    await assert.rejects(target.import(bundle), /codex_account_sort_invalid/);
    assert.ok(!target.calls.includes('data_transfer_apply_user_config'));
  }
});

test('sort save failure reports incomplete import and does not publish a successful restore', async () => {
  const target = harness(sourceAccounts, ['source-personal', 'source-team']);
  target.failSortWrites();
  await assert.rejects(target.import(await sortedBundle()), /codex_account_sort_import_failed/);
  assert.deepEqual(target.savedOrder, ['source-personal', 'source-team']);
  assert.ok(!target.events.includes('codex:custom-sort-restored'));
});

test('failure to persist the sort mode is not reported as a successful import', async () => {
  const target = harness(sourceAccounts);
  target.failActiveWrites();
  await assert.rejects(target.import(await sortedBundle()), /codex_account_sort_import_failed/);
  assert.ok(!target.events.includes('codex:custom-sort-restored'));
});

test('account-only import does not change sorting preferences', async () => {
  const target = harness(sourceAccounts, ['source-personal', 'source-team']);
  const bundle = await sortedBundle();
  bundle.accounts = { schema: 'test-account-bundle' };
  await target.service.importDataTransferJson(JSON.stringify(bundle), { includeAccounts: true, includeConfig: false });
  assert.deepEqual(target.savedOrder, ['source-personal', 'source-team']);
  assert.equal(target.items.get(ACTIVE_KEY), undefined);
});

test('full restore resolves newly imported accounts after their device ids have been assigned', async () => {
  const imported = sourceAccounts.map(a => ({ ...a, id: a.id.replace('source-', 'imported-') }));
  const target = harness([], [], imported);
  const bundle = await sortedBundle();
  bundle.accounts = { schema: 'test-account-bundle' };
  const result = await target.service.importDataTransferJson(JSON.stringify(bundle), { includeAccounts: true, includeConfig: true });
  assert.equal(result.imported_account_count, 2);
  assert.equal(result.config_result.unresolved_account_ref_count, 0);
  assert.deepEqual(target.savedOrder, ['imported-team', 'imported-personal']);
});

test('config-only restore on an empty device reports missing accounts and saves no foreign ids', async () => {
  const target = harness();
  const result = await target.import(await sortedBundle());
  assert.equal(result.config_result.unresolved_account_ref_count, 2);
  assert.deepEqual(target.savedOrder, []);
  assert.ok(!target.calls.includes('save_user_memory_list'));
});

test('mounted account controller receives restored order and mode and removes its event listener on unmount', async () => {
  // Execute the controller's real subscription effects, excluding unrelated OAuth UI.
  const file = ts.createSourceFile('controller.ts', readFileSync(new URL('../pages/useCodexAccountsOAuthController.ts', import.meta.url), 'utf8'), ts.ScriptTarget.ES2022);
  const effects: string[] = [];
  const collect = (node: ts.Node) => {
    if (ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
      && node.expression.expression.getText(file) === 'useEffect') {
      const body = node.getText(file);
      if (body.includes('subscribeUserMemory(') || body.includes('CODEX_CUSTOM_SORT_RESTORED_EVENT')) effects.push(body);
    }
    ts.forEachChild(node, collect);
  };
  collect(file);
  assert.equal(effects.length, 2);
  for (const active of [true, false]) {
    const target = harness(sourceAccounts, ['source-personal', 'source-team']);
    await target.memory.hydrateUserMemory();
    const state = { order: ['source-personal', 'source-team'], sortBy: active ? 'weekly' : 'custom' };
    const cleanups: (() => void)[] = [];
    vm.runInNewContext(ts.transpileModule(effects.join('\n'), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, {
      ...target.overview, ...target.memory, window: target.window, sortBy: state.sortBy,
      setSortBy(value: string) { assert.equal(typeof value, 'string'); state.sortBy = value; },
      setCustomSortOrder(update: (previous: string[]) => string[]) { state.order = [...update(state.order)]; },
      useEffect(effect: () => () => void) { cleanups.push(effect()); },
    });
    const bundle = await sortedBundle(); bundle.config.codex_account_sort.active = active;
    await target.import(bundle);
    assert.deepEqual(state.order, ['source-team', 'source-personal']);
    assert.equal(state.sortBy, active ? 'custom' : 'created_at');
    cleanups.forEach(cleanup => cleanup());
    assert.equal(target.listeners.get('codex:custom-sort-restored')?.size, 0);
  }
});
