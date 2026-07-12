/**
 * 真实 hydrate 入口回归：自定义布局二次加载不丢序、新平台合并不重置、默认才 Grok 靠 Codex。
 */
const store = new Map<string, string>();
(globalThis as unknown as { localStorage: Storage }).localStorage = {
  getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
  setItem: (k: string, v: string) => {
    store.set(k, String(v));
  },
  removeItem: (k: string) => {
    store.delete(k);
  },
  clear: () => store.clear(),
  key: (i: number) => [...store.keys()][i] ?? null,
  get length() {
    return store.size;
  },
} as Storage;
(globalThis as unknown as { window: unknown }).window = globalThis;

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}

function relativeOrderPreserved(saved: string[], after: string[], ids: string[]): boolean {
  const filteredSaved = saved.filter((id) => ids.includes(id) && after.includes(id));
  const filteredAfter = after.filter((id) => filteredSaved.includes(id));
  return filteredSaved.join('|') === filteredAfter.join('|');
}

async function main() {
  const {
    hydratePlatformLayoutFromPersisted,
    getDefaultPlatformOrder,
  } = await import('../src/stores/usePlatformLayoutStore.ts');

  const customOrder = [
    'cursor',
    'gemini',
    'codex',
    'windsurf',
    'claude_manager',
    'antigravity',
    'zed',
    'github-copilot',
    'kiro',
    'grok',
    'qoder',
  ] as const;

  const customSidebar = ['cursor', 'gemini', 'windsurf', 'codex'] as const;
  const customHidden = ['kiro'] as const;
  const customTray = ['codex', 'cursor'] as const;

  const fixture = {
    orderedPlatformIds: [...customOrder],
    hiddenPlatformIds: [...customHidden],
    sidebarPlatformIds: [...customSidebar],
    trayPlatformIds: [...customTray],
    traySortMode: 'manual' as const,
    orderedEntryIds: customOrder.map((id) => `platform:${id}` as const),
    hiddenEntryIds: customHidden.map((id) => `platform:${id}` as const),
    sidebarEntryIds: customSidebar.map((id) => `platform:${id}` as const),
    antigravityGroupFirstMigrated: true,
    traeSuiteDefaultGroupRestored: true,
    apiRelaySidebarVisible: true,
    apiRelayDashboardVisible: true,
    apiRelayEntryOrder: 2,
  };

  const once = hydratePlatformLayoutFromPersisted(fixture);
  const twice = hydratePlatformLayoutFromPersisted({
    orderedPlatformIds: once.orderedPlatformIds,
    hiddenPlatformIds: once.hiddenPlatformIds,
    sidebarPlatformIds: once.sidebarPlatformIds,
    trayPlatformIds: once.trayPlatformIds,
    traySortMode: once.traySortMode,
    orderedEntryIds: once.orderedEntryIds,
    hiddenEntryIds: once.hiddenEntryIds,
    sidebarEntryIds: once.sidebarEntryIds,
    platformGroups: once.platformGroups,
    antigravityGroupFirstMigrated: once.antigravityGroupFirstMigrated,
    traeSuiteDefaultGroupRestored: once.traeSuiteDefaultGroupRestored,
    apiRelaySidebarVisible: once.apiRelaySidebarVisible,
    apiRelayDashboardVisible: once.apiRelayDashboardVisible,
    apiRelayEntryOrder: once.apiRelayEntryOrder,
  });

  assert(
    relativeOrderPreserved([...customOrder], once.orderedPlatformIds, [...customOrder]),
    `custom platform relative order lost: ${once.orderedPlatformIds.join(',')}`,
  );
  assert(
    once.orderedPlatformIds.join('|') === twice.orderedPlatformIds.join('|'),
    'second hydrate changed orderedPlatformIds',
  );
  assert(
    relativeOrderPreserved([...customSidebar], once.sidebarPlatformIds, [...customSidebar]),
    `sidebar order not preserved: ${once.sidebarPlatformIds.join(',')}`,
  );
  assert(
    once.sidebarPlatformIds.join('|') === twice.sidebarPlatformIds.join('|'),
    'second hydrate changed sidebar',
  );
  assert(once.traySortMode === 'manual', 'traySortMode not preserved');
  assert(twice.traySortMode === 'manual', 'traySortMode lost on second load');
  assert(once.hiddenPlatformIds.includes('kiro'), 'hidden platforms not preserved');

  const codexIdx = once.orderedPlatformIds.indexOf('codex');
  const grokIdx = once.orderedPlatformIds.indexOf('grok');
  assert(codexIdx >= 0 && grokIdx >= 0, 'codex/grok missing');
  assert(
    grokIdx !== codexIdx + 1,
    `Grok force-moved beside Codex (codex=${codexIdx}, grok=${grokIdx})`,
  );

  const withoutQoder = {
    ...fixture,
    orderedPlatformIds: customOrder.filter((id) => id !== 'qoder'),
    orderedEntryIds: customOrder
      .filter((id) => id !== 'qoder')
      .map((id) => `platform:${id}` as const),
  };
  const merged = hydratePlatformLayoutFromPersisted(withoutQoder);
  const userPart = withoutQoder.orderedPlatformIds;
  assert(
    relativeOrderPreserved(userPart, merged.orderedPlatformIds, userPart),
    'user relative order rewritten when merging missing platforms',
  );
  const defaultSidebar = [
    'claude_manager',
    'codex',
    'grok',
    'antigravity',
    'zed',
    'github-copilot',
  ];
  assert(
    merged.sidebarPlatformIds.join('|') !== defaultSidebar.join('|'),
    'sidebar was reset to defaultSidebarPlatformIds',
  );
  assert(
    relativeOrderPreserved([...customSidebar], merged.sidebarPlatformIds, [...customSidebar]),
    'sidebar membership/order reset on catalog merge',
  );

  const defaults = hydratePlatformLayoutFromPersisted(null);
  const dOrder = getDefaultPlatformOrder();
  const dCodex = dOrder.indexOf('codex');
  const dGrok = dOrder.indexOf('grok');
  assert(dCodex >= 0 && dGrok === dCodex + 1, 'default order should place Grok after Codex');
  assert(
    defaults.orderedPlatformIds.indexOf('grok')
      === defaults.orderedPlatformIds.indexOf('codex') + 1,
    'default hydrate should place Grok after Codex',
  );

  console.log('PASS platform layout persistence tests');
  console.log(
    JSON.stringify(
      {
        customOrderPreserved: true,
        sidebarPreserved: once.sidebarPlatformIds,
        grokNotForcedBesideCodex: { codexIdx, grokIdx },
        traySortMode: once.traySortMode,
        defaultGrokAfterCodex: dGrok === dCodex + 1,
        secondLoadIdentical: once.orderedPlatformIds.join('|') === twice.orderedPlatformIds.join('|'),
      },
      null,
      2,
    ),
  );
  // 避免 store 初始化后的 tray sync timer 在无 Tauri 环境下刷屏
  process.exit(0);
}

main().catch((err) => {
  console.error('FAIL', err);
  process.exit(1);
});
