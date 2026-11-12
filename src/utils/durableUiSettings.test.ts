import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

const PRIVACY = 'agtools.privacy_mode_enabled';
const BADGE = 'agtools.codex_plan_badge_style';
type NativeSnapshot = { values: Record<string, string>; revisions: Record<string, number> };

function harness(old: Record<string, string>, native: NativeSnapshot = { values: {}, revisions: {} }) {
  const writes: Array<{ key: string; value: string; expectedRevision: number }> = [];
  const events: string[] = [];
  const target = new EventTarget();
  const window = {
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    dispatchEvent: (event: Event) => { events.push(event.type); return target.dispatchEvent(event); },
  };
  let rejectLoad = false;
  const copy = () => ({ values: { ...native.values }, revisions: { ...native.revisions } });
  const invoke = async (command: string, args: any) => {
    if (command === 'migrate_ui_preference_values') {
      if (rejectLoad) throw new Error('unreadable preference file');
      for (const [key, value] of Object.entries(args.legacy) as Array<[string, string]>) {
        if (native.values[key] === undefined) {
          native.values[key] = value;
          native.revisions[key] = 1;
        }
      }
      return copy();
    }
    if (command === 'load_ui_preferences') return copy();
    if (command === 'update_ui_preference') {
      writes.push(args);
      const revision = native.revisions[args.key] ?? 0;
      if (revision !== args.expectedRevision) throw new Error('ui_preference_revision_conflict');
      native.values[args.key] = args.value;
      native.revisions[args.key] = revision + 1;
      return copy();
    }
    throw new Error(command);
  };
  const { exports } = loadHookModule(new URL('./durableUiSettings.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke },
  }, {
    localStorage: { getItem: (key: string) => old[key] ?? null, length: Object.keys(old).length, key: (i: number) => Object.keys(old)[i] ?? null },
    Event,
    CustomEvent: class extends Event {},
    window,
  });
  return { exports, native, writes, events, window, rejectLoad: () => { rejectLoad = true; } };
}

test('WebView imports only missing settings, native state wins, and privacy is masked until loaded', async () => {
  const legacy = { [PRIVACY]: '0', [BADGE]: 'outline' };
  const h = harness(legacy, { values: { [PRIVACY]: '1' }, revisions: { [PRIVACY]: 2 } });
  assert.equal(h.exports.readDurableUiSetting(PRIVACY), '1');
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.exports.readDurableUiSetting(PRIVACY), '1');
  assert.equal(h.native.values[BADGE], 'outline');
  assert.deepEqual(legacy, { [PRIVACY]: '0', [BADGE]: 'outline' });
});

test('service display and instance sorting preferences restore independently of WebView storage', async () => {
  const columns = 'codex.apiService.logColumns.v1';
  const pageSize = 'agtools.codex.api_service.request_log_page_size.v1';
  const sort = 'agtools.antigravity_ide.instances.sort_field';
  const localRange = 'agtools.codex.local_access.stats_range.v1';
  const serviceRange = 'agtools.codex.api_service.stats_range.v1';
  const sessionRange = 'agtools.codex.session_usage.range.v1';
  const address = 'agtools.codex.local_access.address_kind.v1';
  const providerOrder = 'agtools.codex.modelProviders.custom_sort_order.v1';
  const providerActive = 'agtools.codex.modelProviders.custom_sort_active.v1';
  const custom = '{"key":"custom","startInput":"2026-08-01","endInput":"2026-08-17"}';
  const legacy = {
    [columns]: '["model","proxy"]', [pageSize]: '50', [sort]: 'lastLaunchedAt',
    [localRange]: custom, [serviceRange]: 'daily', [sessionRange]: 'month', [address]: 'lan',
    [providerOrder]: '["provider-2","provider-1"]', [providerActive]: '1',
  };
  const h = harness(legacy, { values: { [serviceRange]: 'weekly' }, revisions: { [serviceRange]: 3 } });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.native.values[localRange], custom);
  assert.equal(h.native.values[serviceRange], 'weekly');
  assert.equal(h.native.values[sessionRange], 'month');
  await h.exports.saveDurableUiSetting(pageSize, '100');
  assert.equal(legacy[pageSize], '50');

  const reopened = harness({}, h.native);
  await reopened.exports.hydrateDurableUiSettings();
  assert.equal(reopened.exports.readDurableUiSetting(pageSize), '100');
  assert.equal(reopened.exports.readDurableUiSetting(columns), legacy[columns]);
  assert.equal(reopened.exports.readDurableUiSetting(sort), 'lastLaunchedAt');
  assert.equal(reopened.exports.readDurableUiSetting(address), 'lan');
  assert.equal(reopened.exports.readDurableUiSetting(providerOrder), legacy[providerOrder]);
  assert.equal(reopened.exports.readDurableUiSetting(providerActive), '1');
  const backup = await reopened.exports.exportDurableUiSettings();
  assert.equal(backup[localRange], custom);
  assert.equal(backup[address], undefined);
  assert.equal(backup[providerOrder], legacy[providerOrder]);
});

test('queued same-window saves advance their own revisions while stale writes are rejected', async () => {
  const h = harness({}, { values: { [BADGE]: 'default' }, revisions: { [BADGE]: 1 } });
  await h.exports.hydrateDurableUiSettings();
  await Promise.all([
    h.exports.saveDurableUiSetting(BADGE, 'outline'),
    h.exports.saveDurableUiSetting(BADGE, 'soft'),
  ]);
  assert.deepEqual(h.writes.map(write => write.expectedRevision), [1, 2]);
  h.native.values[BADGE] = 'mono';
  h.native.revisions[BADGE] = 4;
  await assert.rejects(h.exports.saveDurableUiSetting(BADGE, 'default'), /revision_conflict/);
  assert.equal(h.exports.readDurableUiSetting(BADGE), 'mono');
  h.exports.applyDurableUiSnapshot({ values: { [BADGE]: 'outline' }, revisions: { [BADGE]: 2 } });
  assert.equal(h.exports.readDurableUiSetting(BADGE), 'mono');
});

test('Antigravity legacy sorting seeds missing fields while the native custom mode stays disabled', async () => {
  const active = 'agtools.antigravity.accounts.custom_sort_active.v1';
  const field = 'agtools.antigravity.accounts_overview_filters.sort_by';
  const direction = 'agtools.antigravity.accounts_overview_filters.sort_direction';
  const h = harness({ [active]: '1', accountsSortBy: 'custom', accountsSortDirection: 'asc' }, {
    values: { [active]: '0', [direction]: 'null' }, revisions: { [active]: 3, [direction]: 2 },
  });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.native.values[active], '0');
  assert.equal(h.native.values[field], '"custom"');
  assert.equal(h.native.values[direction], 'null');
  const reopened = harness({}, h.native);
  await reopened.exports.hydrateDurableUiSettings();
  assert.equal(reopened.exports.readDurableUiSetting(active), '0');
  assert.equal(reopened.exports.readDurableUiSetting(field), '"custom"');
});

test('floating card and Breakout migrate without reviving cleared history or exporting device preferences', async () => {
  const floating = 'agtools.floating_card.platform';
  const history = 'agtools.breakout.history';
  const icons = 'agtools.breakout.drop_icon_assign.v1';
  const h = harness({ [floating]: 'codex', [history]: '[{"id":"old"}]', [icons]: '{}' }, {
    values: { [history]: '[]' }, revisions: { [history]: 2 },
  });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.native.values[floating], 'codex');
  assert.equal(h.native.values[history], '[]');
  assert.equal(h.native.values[icons], '{}');
  const backup = await h.exports.exportDurableUiSettings();
  for (const key of [floating, history, icons]) assert.equal(backup[key], undefined);
});

test('a failed load does not send writes or release privacy masking', async () => {
  const h = harness({ [PRIVACY]: '0' });
  h.rejectLoad();
  await assert.rejects(h.exports.saveDurableUiSetting(BADGE, 'soft'), /unreadable/);
  assert.equal(h.writes.length, 0);
  assert.equal(h.exports.readDurableUiSetting(PRIVACY), '1');
});

test('a window event before hydration cannot suppress migration of missing settings', async () => {
  const h = harness({ [BADGE]: 'outline' }, { values: { [PRIVACY]: '0' }, revisions: { [PRIVACY]: 1 } });
  h.exports.applyDurableUiSnapshot(h.native);
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.exports.readDurableUiSetting(BADGE), 'outline');
  assert.equal(h.native.values[BADGE], 'outline');
});

test('runtime values migrate without duplicating legacy backup fields or exporting machine paths', async () => {
  const runtime = 'agtools.antigravity.runtime_target.v1';
  const directory = 'agtools.claude.cli.last_working_dir';
  const refresh = 'agtools.current_account_refresh_minutes.v1';
  const overrides = 'agtools.account_refresh_overrides.v1';
  const legacy = {
    [runtime]: 'antigravity',
    [directory]: '/example/workspace',
    [refresh]: '{"codex":7}',
    [overrides]: '{"codex":{"example@example.test":-1}}',
  };
  const h = harness(legacy, { values: { [runtime]: 'antigravity_ide' }, revisions: { [runtime]: 2 } });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.exports.readDurableUiSetting(runtime), 'antigravity_ide');
  assert.equal(h.exports.readDurableUiSetting(directory), legacy[directory]);
  assert.equal(h.exports.readDurableUiSetting(refresh), legacy[refresh]);
  const exported = await h.exports.exportDurableUiSettings();
  assert.deepEqual(Object.entries(exported).sort(), [
    [overrides, legacy[overrides]],
    [runtime, 'antigravity_ide'],
  ]);
  await h.exports.saveDurableUiSetting(refresh, '{"codex":8}');
  assert.ok(h.events.includes('agtools:current-account-refresh-preferences-changed'));
  assert.equal(legacy[refresh], '{"codex":7}');
});

test('sidebar migration prefers the last WebView format and preserves newer Rust choices', async () => {
  const key = 'agtools.side_nav.layout.v1';
  const collapsed = 'agtools.side_nav.classic_collapsed.v1';
  const marker = 'agtools.side_nav.classic_first_sync_done.v1';
  const old = {
    [key]: 'classic',
    [collapsed]: '1',
    'agtools.side_nav.store.v2': JSON.stringify({ state: {
      mode: 'original', classicCollapsed: false, classicFirstSyncDone: true,
    } }),
  };
  const h = harness(old, { values: { [collapsed]: '1' }, revisions: { [collapsed]: 3 } });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.native.values[key], 'original');
  assert.equal(h.native.values[collapsed], '1');
  assert.equal(h.native.values[marker], '1');
  assert.equal((await h.exports.exportDurableUiSettings())[marker], undefined);
  assert.ok(old['agtools.side_nav.store.v2']);
});

test('page sizes are discovered by scope and unrelated WebView keys remain local', async () => {
  const size = 'agtools.codex.accounts_page_size';
  const old = { [size]: '50', 'agtools.cache.fixture': 'unrelated' };
  const h = harness(old);
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.native.values[size], '50');
  assert.equal(h.native.values['agtools.cache.fixture'], undefined);
  await h.exports.saveDurableUiSetting(size, '20');
  assert.equal(h.exports.readDurableUiSetting(size), '20');
  assert.equal(old[size], '50');
});

test('account filter selections migrate per platform, respect native clears, and stay out of portable exports', async () => {
  const flag = 'agtools.kiro.accounts_overview_filters.persist_enabled';
  const selection = 'agtools.kiro.accounts_overview_filters.filter_types';
  const cleared = 'agtools.trae.accounts_overview_filters.tag_filter';
  const retired = 'agtools.codex.accounts_overview_filters.expiry_filter';
  const tags = 'agtools.codex.accounts_overview_filters.tags';
  const groupByTag = 'agtools.codex.accounts_overview_filters.group_by_tag';
  const old = {
    [flag]: '0', [selection]: '["test"]', [cleared]: '["old"]', [retired]: '"expired"',
    [tags]: '["work","team","team🔥"]', [groupByTag]: 'true',
  };
  const h = harness(old, { values: { [cleared]: 'null' }, revisions: { [cleared]: 2 } });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.native.values[flag], '0');
  assert.equal(h.native.values[selection], '["test"]');
  assert.equal(h.native.values[cleared], 'null');
  assert.equal(h.native.values[retired], undefined);
  const reopened = harness({}, h.native);
  await reopened.exports.hydrateDurableUiSettings();
  assert.equal(reopened.exports.readDurableUiSetting(tags), '["work","team","team🔥"]');
  assert.equal(reopened.exports.readDurableUiSetting(groupByTag), 'true');
  assert.deepEqual(Object.entries(await h.exports.exportDurableUiSettings()), [[flag, '0']]);
  await h.exports.saveDurableUiSetting(flag, '1');
  assert.ok(h.events.includes('agtools:accounts-overview-filter-persistence-changed'));
  assert.equal(old[cleared], '["old"]');
});

const filterKey = (scope: string, field = 'sort_by') => `agtools.${scope}.accounts_overview_filters.${field}`;

function filterHarness(old: Record<string, string>, native: NativeSnapshot) {
  const storage = harness(old, native);
  const persistence = loadHookModule(new URL('./accountsOverviewFilterPersistence.ts', import.meta.url), {
    './durableUiSettings': storage.exports,
  }).exports;
  const hook = loadHookModule(new URL('../hooks/useAccountsOverviewFilterState.ts', import.meta.url), {
    '../utils/accountsOverviewFilterPersistence': persistence,
    '../utils/durableUiSettings': storage.exports,
  }, { window: storage.window });
  let scope = 'trae';
  const render = () => hook.render(() => hook.exports.useAccountsOverviewFilterState(
    scope, 'sort_by', () => persistence.readAccountsOverviewFilterPersistenceEnabled(scope)
      ? persistence.readAccountsOverviewFilterField(scope, 'sort_by', 'created_at')
      : 'created_at',
  ));
  return { ...storage, persistence, render, selectScope: (next: string) => { scope = next; return render(); } };
}

test('mounted filters display WebView data first, adopt native values without writing, and isolate scope changes', async () => {
  const cn = filterKey('trae_cn');
  const h = filterHarness({ [filterKey('trae')]: '"email"' }, {
    values: { [filterKey('trae')]: '"updated_at"', [cn]: '"last_used"' },
    revisions: { [filterKey('trae')]: 2, [cn]: 1 },
  });
  assert.equal(h.render()[0], 'email');
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.render()[0], 'updated_at');
  assert.equal(h.selectScope('trae_cn')[0], 'last_used');
  assert.deepEqual(h.writes, []);
});

test('an edit made during background migration wins over the initially loaded native value', async () => {
  const key = filterKey('trae');
  const h = filterHarness({}, { values: { [key]: '"last_used"' }, revisions: { [key]: 2 } });
  const [, setSort] = h.render();
  setSort('email');
  assert.equal(h.render()[0], 'email');
  await h.exports.hydrateDurableUiSettings();
  await settlePromises();
  assert.equal(h.render()[0], 'email');
  assert.equal(h.native.values[key], '"email"');
  assert.equal(h.writes[0].expectedRevision, 2);
});

test('disabled filters clear native preferences once and retain the old WebView data during migration', async () => {
  const key = filterKey('trae');
  const flag = filterKey('trae', 'persist_enabled');
  const old = { [key]: '"email"', [flag]: '0' };
  const h = filterHarness(old, { values: { [flag]: '0' }, revisions: { [flag]: 2 } });
  assert.equal(h.render()[0], 'created_at');
  assert.deepEqual(h.writes, []);
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.render()[0], 'created_at');
  await settlePromises();
  assert.equal(h.native.values[key], 'null');
  assert.equal(old[key], '"email"');
  assert.equal(h.writes.length, 1);
  const restarted = filterHarness(old, h.native);
  await restarted.exports.hydrateDurableUiSettings();
  assert.equal(restarted.persistence.readAccountsOverviewFilterField('trae', 'sort_by', 'created_at'), 'created_at');
  assert.equal(restarted.native.values[key], 'null');
});

test('a failed disable toggle does not erase the saved filter', async () => {
  const key = filterKey('trae');
  const flag = filterKey('trae', 'persist_enabled');
  const h = filterHarness({}, { values: { [key]: '"email"', [flag]: '1' }, revisions: { [key]: 1, [flag]: 1 } });
  await h.exports.hydrateDurableUiSettings();
  assert.equal(h.render()[0], 'email');
  h.native.revisions[flag] = 2;
  const disabling = h.exports.saveDurableUiSetting(flag, '0', true);
  h.render();
  await assert.rejects(disabling, /revision_conflict/);
  assert.equal(h.render()[0], 'email');
  assert.equal(h.native.values[key], '"email"');
  assert.deepEqual(h.writes.map(write => write.key), [flag]);
});

function loadUiHook(h: ReturnType<typeof harness>) {
  return loadHookModule(new URL('../hooks/useDurableUiSetting.ts', import.meta.url), {
    '../utils/durableUiSettings': h.exports,
  });
}

test('Codex and Claude display preferences import legacy formats and restore while mounted', async () => {
  const layout = 'agtools.codex.accounts.overview_layout_mode';
  const expanded = 'agtools.codex.local_access_entry_expanded.v1';
  const address = 'agtools.codex.local_access_address_kind.v1';
  const cli = 'cockpit.codex.cli.lastWorkingDir';
  const flow = 'agtools.claude.flow_notice_collapsed';
  const old = {
    'agtools.codex.accounts_view_mode': 'list',
    [expanded]: '0',
    [address]: 'lan',
    [cli]: '/fixture/workspace',
    [flow]: 'true',
  };
  const h = harness(old, { values: { [expanded]: '1' }, revisions: { [expanded]: 3 } });
  const hook = loadUiHook(h);
  const render = () => hook.render(() => hook.exports.useDurableUiBooleanSetting(expanded));
  assert.equal(render()[0], false);
  await h.exports.hydrateDurableUiSettings();
  assert.equal(render()[0], true);
  assert.equal(h.native.values[layout], 'list');
  assert.equal(h.native.values[flow], '1');
  assert.deepEqual(h.writes, []);
  const exported = await h.exports.exportDurableUiSettings();
  assert.equal(exported[address], undefined);
  assert.equal(exported[cli], undefined);
  assert.equal(h.native.values[cli], old[cli]);
  const [, toggle] = render();
  toggle((current: boolean) => !current);
  assert.equal(render()[0], false);
  await settlePromises();
  assert.equal(h.native.values[expanded], '0');
});

test('group tab migration protects a newer selection and persists explicit All across upgrades', async () => {
  const key = 'agtools.group_tab_active.codex';
  const older = 'agtools.codex.accounts_overview_filters.active_group_id';
  const old = { [older]: '"obsolete-group"' };
  const h = harness(old, { values: { [key]: 'new-group' }, revisions: { [key]: 2 } });
  const diskGroups = [{ id: 'new-group' }];
  const platform = loadHookModule(new URL('../services/platformGroupService.ts', import.meta.url), {
    '@tauri-apps/api/core': {
      invoke: async (command: string) => {
        assert.equal(command, 'load_account_group_snapshot');
        return { data: JSON.stringify(diskGroups), revision: 1 };
      },
    },
    './accountGroupService': { parseAccountGroups: JSON.parse },
    './codexAccountGroupService': {},
    '../utils/durableUiSettings': h.exports,
  }).exports;
  const hook = loadHookModule(new URL('../hooks/useActiveGroupTab.ts', import.meta.url), {
    '../services/platformGroupService': platform,
    '../utils/durableUiSettings': h.exports,
    './useDurableUiSetting': { useDurableUiSetting: h.exports.readDurableUiSetting },
  });
  let groups = [{ id: 'new-group' }];
  const render = () => hook.render(() => hook.exports.useActiveGroupTab('codex', groups));
  assert.equal(render(), null);
  await h.exports.hydrateDurableUiSettings();
  assert.deepEqual(h.writes, []);
  assert.equal(render(), 'new-group');
  // A window can still have a stale group list after another window creates and selects a group.
  groups = [];
  assert.equal(render(), null);
  await settlePromises();
  assert.equal(h.native.values[key], 'new-group');
  platform.setActiveGroupTab('codex_api_service', null);
  assert.equal(render(), null);
  await settlePromises();
  assert.equal(h.native.values[key], '__all__');
  const restarted = harness(old, h.native);
  await restarted.exports.hydrateDurableUiSettings();
  assert.equal(restarted.exports.readDurableUiSetting(key), '__all__');
  assert.equal((await restarted.exports.exportDurableUiSettings())[key], undefined);
  const legacyOnly = harness(old);
  await legacyOnly.exports.hydrateDurableUiSettings();
  assert.equal(legacyOnly.native.values[key], 'obsolete-group');
});
