/**
 * 驱动真实 accountsViewModePersistence 读写：列表/平铺偏好不随筛选开关删除，Codex 合并历史键。
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

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}

async function main() {
  const {
    readAccountsViewMode,
    writeAccountsViewMode,
    readCodexOverviewLayoutMode,
    writeCodexOverviewLayoutMode,
    writeAccountsViewModeSafeForCodex,
    getAccountsViewModeStorageKey,
    CODEX_OVERVIEW_LAYOUT_MODE_KEY,
    normalizeAccountsViewMode,
  } = await import('../src/utils/accountsViewModePersistence.ts');

  // --- pure normalize ---
  assert(normalizeAccountsViewMode('list') === 'list', 'normalize list');
  assert(normalizeAccountsViewMode('grid') === 'grid', 'normalize grid');
  assert(
    normalizeAccountsViewMode('compact', { allowCompact: true }) === 'compact',
    'normalize compact',
  );
  assert(normalizeAccountsViewMode('nope') === null, 'normalize invalid');

  // --- always persist dedicated key even when not syncing filter field ---
  store.clear();
  writeAccountsViewMode('codex', 'list', { syncFilterField: false });
  assert(
    store.get(getAccountsViewModeStorageKey('codex')) === 'list',
    'dedicated key not written',
  );
  // 关闭筛选同步时不应删除专用键；再次读取仍是 list
  assert(readAccountsViewMode('codex') === 'list', 'read dedicated list');

  // --- filter field as legacy seed ---
  store.clear();
  store.set(
    'agtools.codex.accounts_overview_filters.view_mode',
    JSON.stringify('list'),
  );
  assert(
    readAccountsViewMode('codex', { fallback: 'grid' }) === 'list',
    'legacy filter field list not restored',
  );

  // --- codex multi-key merge: overview key missing, old accounts_view_mode has list ---
  store.clear();
  store.set('agtools.codex.accounts_view_mode', 'list');
  assert(
    readCodexOverviewLayoutMode() === 'list',
    'codex legacy accounts_view_mode not used',
  );

  // --- writing codex mode updates all keys ---
  writeCodexOverviewLayoutMode('list');
  assert(store.get(CODEX_OVERVIEW_LAYOUT_MODE_KEY) === 'list', 'codex overview key');
  assert(
    store.get(getAccountsViewModeStorageKey('codex')) === 'list',
    'codex dedicated key',
  );
  assert(store.get('agtools.codex.accounts_view_mode') === 'list', 'codex legacy key');

  // --- second load after "update" (clear only non-view keys) still list ---
  writeCodexOverviewLayoutMode('list');
  const after = readCodexOverviewLayoutMode();
  assert(after === 'list', `expected list after reload, got ${after}`);

  // --- default only when empty ---
  store.clear();
  assert(readAccountsViewMode('gemini') === 'grid', 'default grid');

  // --- never wipe: write list, then write with syncFilter false still keeps ---
  writeAccountsViewMode('gemini', 'list', { syncFilterField: true });
  writeAccountsViewMode('gemini', 'list', { syncFilterField: false });
  assert(readAccountsViewMode('gemini') === 'list', 'must not wipe list');

  // --- Codex compact not clobbered by list/grid write from shared hook ---
  store.clear();
  writeCodexOverviewLayoutMode('compact');
  assert(readCodexOverviewLayoutMode() === 'compact', 'compact written');
  writeAccountsViewModeSafeForCodex('codex', 'list', { syncFilterField: true });
  assert(
    readCodexOverviewLayoutMode() === 'compact',
    'compact must survive list write from provider hook',
  );
  assert(
    store.get(CODEX_OVERVIEW_LAYOUT_MODE_KEY) === 'compact',
    'overview key still compact',
  );

  // --- only overview key has list (legacy after update) ---
  store.clear();
  store.set(CODEX_OVERVIEW_LAYOUT_MODE_KEY, 'list');
  assert(
    readCodexOverviewLayoutMode() === 'list',
    'list from overview key alone must restore',
  );

  console.log('PASS accounts view mode persistence tests');
  console.log(
    JSON.stringify(
      {
        listPreserved: true,
        codexLegacyMerge: true,
        defaultGridWhenEmpty: true,
        compactNotClobbered: true,
        overviewKeyOnlyList: true,
      },
      null,
      2,
    ),
  );
}

main().catch((err) => {
  console.error('FAIL', err);
  process.exit(1);
});
