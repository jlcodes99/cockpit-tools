import assert from 'node:assert/strict';
import test from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { ALL_PLATFORM_IDS } from '../types/platform';

test('only Codex and its API service are grouped by default and explicit layouts survive hydration', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const key = 'agtools.platform_layout.v1';
  const values = new Map<string, string>();
  const storage = {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => { values.set(name, value); },
    removeItem: (name: string) => { values.delete(name); },
  };
  const windowMock = Object.assign(new EventTarget(), {
    localStorage: storage,
    setTimeout: () => 0,
    clearTimeout: () => {},
  });
  const layout = (client: 'antigravity' | 'antigravity_ide', includeIde: boolean) => JSON.stringify({
    orderedPlatformIds: ['antigravity', 'antigravity_ide'],
    platformGroups: [{
      id: 'antigravity-suite', name: 'My Antigravity',
      platformIds: includeIde ? ['antigravity', 'antigravity_ide'] : ['antigravity'],
      defaultPlatformId: client, iconKind: 'platform', iconPlatformId: client,
    }],
  });
  try {
    Object.defineProperty(globalThis, 'window', { configurable: true, value: windowMock });
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
    mockIPC(() => null);
    const savedAppLayout = layout('antigravity', false);
    storage.setItem(key, savedAppLayout);
    const { usePlatformLayoutStore } = await import('./usePlatformLayoutStore.ts');
    const group = () => usePlatformLayoutStore.getState().platformGroups.find(
      (item) => item.id === 'antigravity-suite',
    )!;
    assert.deepEqual(group().platformIds, ['antigravity']);
    assert.equal(group().defaultPlatformId, 'antigravity');
    assert.equal(group().iconPlatformId, 'antigravity');
    assert.equal(storage.getItem(key), savedAppLayout, 'startup normalization must not rewrite preferences');
    for (const client of ['antigravity_ide', 'antigravity'] as const) {
      storage.setItem(key, layout(client, true));
      windowMock.dispatchEvent(new Event('agtools:platform-layout-hydrated'));
      assert.equal(group().defaultPlatformId, client);
      assert.equal(group().iconPlatformId, client);
    }
    const hydrate = (value: object | null) => {
      if (value) storage.setItem(key, JSON.stringify(value));
      else storage.removeItem(key);
      windowMock.dispatchEvent(new Event('agtools:platform-layout-hydrated'));
      return usePlatformLayoutStore.getState();
    };
    const assertDefaultGrouping = (state: ReturnType<typeof usePlatformLayoutStore.getState>) => {
      assert.equal(state.platformGroups.length, 1);
      assert.deepEqual(state.platformGroups[0].platformIds, ['codex', 'codex_api_service']);
      for (const id of ALL_PLATFORM_IDS.filter((id) => id !== 'codex' && id !== 'codex_api_service')) {
        assert.ok(state.orderedEntryIds.includes(`platform:${id}`));
      }
    };
    const independent = hydrate(null);
    assertDefaultGrouping(independent);
    assert.deepEqual(independent.orderedEntryIds, ALL_PLATFORM_IDS.filter((id) => id !== 'codex_api_service')
      .map((id) => id === 'codex' ? 'group:codex-suite' : `platform:${id}`));
    assert.equal(independent.independentPlatformsMigrated, true);
    assert.equal(independent.codexApiServiceDefaultGrouped, true);
    assertDefaultGrouping(hydrate({ orderedPlatformIds: ['codex'] }));
    assertDefaultGrouping(hydrate({ platformGroups: [], traeSuiteDefaultGroupRestored: false, codexApiServiceSuiteMigrated: false }));

    // Older standalone items must not be reassembled into automatic suites.
    const singletons = ALL_PLATFORM_IDS.map((id) => ({
      id: `platform-${id}`, name: id === 'codex' ? 'Codex' : id,
      platformIds: [id], defaultPlatformId: id, iconKind: 'platform', iconPlatformId: id,
    }));
    const oldStandalone = hydrate({ platformGroups: [singletons.find((item) => item.id === 'platform-codex')] });
    assertDefaultGrouping(oldStandalone);
    for (const [platformId, members] of [
      ['antigravity', ['antigravity', 'antigravity_ide']],
      ['codex', ['codex', 'codex_api_service']],
    ] as const) {
      const legacyExpanded = hydrate({ platformGroups: [{
        id: `platform-${platformId}`, name: platformId === 'codex' ? 'Codex' : 'Antigravity',
        platformIds: members, defaultPlatformId: platformId, iconKind: 'platform', iconPlatformId: platformId,
      }], orderedEntryIds: [`group:platform-${platformId}`] });
      assertDefaultGrouping(legacyExpanded);
      if (platformId === 'codex') {
        assert.equal(legacyExpanded.orderedEntryIds[0], 'group:platform-codex');
      } else {
        assert.deepEqual(legacyExpanded.orderedEntryIds.slice(0, 2), members.map((id) => `platform:${id}`));
      }
    }

    const suites = [
      { id: 'antigravity-suite', name: 'Antigravity', platformIds: ['antigravity', 'antigravity_ide'], defaultPlatformId: 'antigravity_ide', iconKind: 'platform', iconPlatformId: 'antigravity_ide' },
      { id: 'codex-suite', name: 'Codex', platformIds: ['codex', 'codex_api_service'], defaultPlatformId: 'codex', iconKind: 'platform', iconPlatformId: 'codex' },
      { id: 'codebuddy-suite', name: 'CodeBuddy', platformIds: ['codebuddy', 'codebuddy_cn', 'workbuddy'], defaultPlatformId: 'codebuddy', iconKind: 'platform', iconPlatformId: 'codebuddy' },
      { id: 'trae-suite', name: 'Trae', platformIds: ['trae', 'trae_solo', 'trae_cn', 'trae_solo_cn'], defaultPlatformId: 'trae', iconKind: 'platform', iconPlatformId: 'trae' },
    ];
    const old = {
      platformGroups: suites,
      orderedEntryIds: ['group:codex-suite', 'platform:zed', 'group:trae-suite', 'group:antigravity-suite', 'group:codebuddy-suite'],
      hiddenEntryIds: ['group:codex-suite'], sidebarEntryIds: ['group:trae-suite'],
      apiRelayEntryOrder: 1,
    };
    const migrated = hydrate(old);
    assertDefaultGrouping(migrated);
    assert.deepEqual(migrated.orderedEntryIds.slice(0, 2), ['group:codex-suite', 'platform:zed']);
    assert.deepEqual(migrated.hiddenEntryIds, ['group:codex-suite']);
    assert.deepEqual(migrated.sidebarEntryIds, ['platform:trae', 'platform:trae_solo', 'platform:trae_cn', 'platform:trae_solo_cn']);
    assert.equal(migrated.apiRelayEntryOrder, 1);
    assert.deepEqual(JSON.parse(storage.getItem(key)!), old, 'loading must not overwrite durable preferences');

    for (const customized of [
      { ...suites[1], name: 'My models' },
      { ...suites[1], defaultPlatformId: 'codex_api_service' },
      { ...suites[1], platformIds: ['codex'] },
      { ...suites[1], iconKind: 'custom', iconCustomDataUrl: 'data:image/png;base64,AQ==' },
      { ...suites[1], childConfigs: [{ platformId: 'codex', name: 'My Codex' }] },
      { ...suites[0], name: 'Antigravity IDE' },
    ]) {
      const kept = hydrate({ platformGroups: [customized] });
      const savedGroup = kept.platformGroups.find((item) => item.id === customized.id)!;
      assert.equal(savedGroup.name, customized.name);
      assert.deepEqual(savedGroup.platformIds, customized.platformIds);
    }
    hydrate(null);
    usePlatformLayoutStore.getState().upsertPlatformGroup(suites[1] as Parameters<typeof independent.upsertPlatformGroup>[0]);
    const manuallySaved = JSON.parse(storage.getItem(key)!);
    assert.equal(manuallySaved.independentPlatformsMigrated, true);
    assert.equal(manuallySaved.codexApiServiceDefaultGrouped, true);
    assert.equal(hydrate(manuallySaved).platformGroups[0].id, 'codex-suite', 'explicit regrouping survives reload');
    usePlatformLayoutStore.getState().removePlatformGroup('codex-suite');
    assert.deepEqual(usePlatformLayoutStore.getState().platformGroups, []);
    assert.ok(usePlatformLayoutStore.getState().orderedEntryIds.includes('platform:codex'));
    assert.ok(usePlatformLayoutStore.getState().orderedEntryIds.includes('platform:codex_api_service'));
    assert.deepEqual(hydrate(JSON.parse(storage.getItem(key)!)).platformGroups, [], 'a later explicit split must not be re-grouped');
    assertDefaultGrouping(hydrate({ platformGroups: suites, orderedEntryIds: 'invalid' }));
    const previewUpgrade = hydrate({ independentPlatformsMigrated: true, platformGroups: [],
      orderedEntryIds: ['platform:codex', 'platform:codex_api_service', 'platform:zed'],
      hiddenEntryIds: ['platform:codex_api_service'], sidebarEntryIds: ['platform:codex'], apiRelayEntryOrder: 2 });
    assertDefaultGrouping(previewUpgrade);
    assert.equal(previewUpgrade.orderedEntryIds[0], 'group:codex-suite');
    assert.equal(previewUpgrade.apiRelayEntryOrder, 1);
    assert.deepEqual(previewUpgrade.hiddenEntryIds, ['group:codex-suite']);
    assert.deepEqual(previewUpgrade.sidebarEntryIds, ['group:codex-suite']);
    const customCodex = { ...suites[1], id: 'my-codex', name: 'My Codex', platformIds: ['codex'] };
    assert.equal(hydrate({ platformGroups: [customCodex] }).platformGroups.length, 1);
    assert.equal(usePlatformLayoutStore.getState().platformGroups[0].id, 'my-codex');
    assert.ok(usePlatformLayoutStore.getState().orderedEntryIds.includes('platform:codex_api_service'));
    usePlatformLayoutStore.getState().resetPlatformLayout();
    assertDefaultGrouping(usePlatformLayoutStore.getState());
    assertDefaultGrouping(hydrate(JSON.parse(storage.getItem(key)!)));
  } finally {
    // Allow the sidebar store's deferred legacy-key cleanup to finish before
    // restoring global storage, including when an assertion fails.
    await new Promise((resolve) => setTimeout(resolve, 0));
    clearMocks();
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (previousStorage) Object.defineProperty(globalThis, 'localStorage', previousStorage);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  }
});
