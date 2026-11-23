import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { resolveAccountSortMode } from '../utils/accountsOverviewFilterPersistence';
import { mergeCodexCardOrder } from '../utils/codexCardOrder';

test('dropping a grid card saves its order and makes custom sorting the restart default', () => {
  let drop!: (ids: string[]) => void;
  let savedOrder: string[] = [];
  let savedActive = '0';
  let selectedMode = '';
  let displayedOrder: string[] = [];
  const handlerImports: Record<string, unknown> = {
    '../utils/codexCardOrder': { mergeCodexCardOrder },
    '../hooks/useCodexCardReorder': {
      useCodexCardReorder(options: { onReorder: typeof drop }) {
        drop = options.onReorder;
        return { cardProps: () => ({}) };
      },
    },
    '../hooks/useCodexAccountWakeupResults': { useCodexAccountWakeupResults: () => new Map() },
    '../utils/codexAccountOverview': {
      writeCodexCustomSortOrder: (ids: string[]) => { savedOrder = Array.from(ids); },
      writeCodexCustomSortActive: (active: boolean) => { savedActive = active ? '1' : '0'; },
    },
  };
  // This test exercises the drop handler; child components require a WebView.
  const renderer = loadHookModule(new URL('./useCodexAccountsRenderers.tsx', import.meta.url), new Proxy(handlerImports, {
    has: (imports, key: string) => key.startsWith('.') || key in imports,
    get: (imports, key: string) => imports[key] ?? {},
  }));
  const accounts = ['a', 'b', 'c'].map(id => ({ id }));
  for (const mode of ['created_at', 'weekly', 'hourly', 'credits', 'weekly_reset', 'hourly_reset', 'subscription_expiry', 'custom']) {
    savedActive = mode === 'custom' ? '1' : '0';
    savedOrder = ['a', 'b', 'c', 'not-loaded'];
    displayedOrder = [...savedOrder];
    selectedMode = mode;
    renderer.render(() => renderer.exports.useCodexAccountsRenderers({
      overviewAccounts: accounts, filteredAccounts: accounts, paginatedAccounts: accounts,
      activeTab: 'overview', overviewLayoutMode: 'grid', sortBy: mode,
      customSortOrder: displayedOrder,
      setCustomSortOrder: (update: string[] | ((ids: string[]) => string[])) => {
        displayedOrder = Array.from(typeof update === 'function' ? update(displayedOrder) : update);
      },
      setSortBy: (value: string) => { selectedMode = value; },
      importApiServiceGuideCount: null, externalImportProgress: { status: 'idle', progress: 0 },
      t: (key: string) => key,
    }));

    drop(['c', 'a', 'b']);
    assert.equal(selectedMode, 'custom', mode);
    assert.deepEqual(displayedOrder, ['c', 'a', 'b', 'not-loaded'], mode);
    assert.deepEqual(savedOrder, displayedOrder, mode);
    assert.equal(resolveAccountSortMode(mode, savedActive, 'created_at'), 'custom', mode);
  }
});
