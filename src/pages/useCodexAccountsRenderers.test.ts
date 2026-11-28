import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { resolveAccountSortMode } from '../utils/accountsOverviewFilterPersistence';
import { mergeCodexCardOrder } from '../utils/codexCardOrder';
import * as codexTypes from '../types/codex';
import * as workbuddyCredits from '../utils/codexWorkbuddyCredits';
import type { WorkbuddyAccount } from '../types/workbuddy';

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
    '../hooks/useCodexWorkbuddyCredits': { useCodexWorkbuddyCredits: () => ({ sources: [], loaded: true, readFailed: false }) },
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

test('compact WorkBuddy rows resolve source credits even when relay balances are hidden', () => {
  const Credits = () => null;
  const accounts = [{ id: 'linked', auth_mode: 'apikey', api_credential_kind: 'workbuddy_acp',
    api_credential_ref: 'source', plan_type: 'Cockpit Api' }] as codexTypes.CodexAccount[];
  const source = { id: 'source' } as WorkbuddyAccount;
  const handlerImports: Record<string, unknown> = {
    '../types/codex': codexTypes,
    '../utils/codexWorkbuddyCredits': workbuddyCredits,
    '../components/codex/CodexWorkbuddyCredits': { CodexWorkbuddyCredits: Credits },
    '../hooks/useCodexWorkbuddyCredits': { useCodexWorkbuddyCredits: () => ({ sources: [source], loaded: true, readFailed: false }) },
    '../hooks/useCodexAccountWakeupResults': { useCodexAccountWakeupResults: () => new Map() },
    '../hooks/useCodexCardReorder': { useCodexCardReorder: () => ({ cardProps: () => ({}) }) },
    '../utils/codexSwitchAuthFailure': {
      isCodexServerRevokedReauth: () => false, isCodexRefreshTokenReusedAccount: () => false,
      isCodexApiUsableReauthNotice: () => false, isCodexAccountReauthRequired: () => false,
    },
  };
  const renderer = loadHookModule(new URL('./useCodexAccountsRenderers.tsx', import.meta.url), new Proxy(handlerImports, {
    has: (imports, key: string) => key.startsWith('.') || key in imports,
    get: (imports, key: string) => imports[key] ?? {},
  }));
  const findCredits = (node: any): any => {
    if (!node || typeof node !== 'object') return undefined;
    if (node.type === Credits) return node;
    if (Array.isArray(node)) return node.map(findCredits).find(Boolean);
    return findCredits(node.props?.children);
  };
  for (const hideRelayQuota of [false, true]) {
    const result = renderer.render(() => renderer.exports.useCodexAccountsRenderers({
      accounts, overviewAccounts: accounts, filteredAccounts: accounts, paginatedAccounts: accounts,
      activeTab: 'overview', overviewLayoutMode: 'compact', selected: new Set(),
      planOverview: { accounts: [] }, localAccessAccountIdSet: new Set(), hideRelayQuota,
      resolvePresentation: () => ({ displayName: 'WorkBuddy', quotaItems: [] }),
      resolveSubscriptionPresentation: () => ({ bucket: 'missing' }),
      getCodexSwitchOrLaunchBlockedReason: () => null,
      applyWindowStatsToQuotaItems: (_account: unknown, items: unknown[]) => items,
      maskAccountText: (value: string) => value,
      renderResetCreditControls: () => null, renderAddLocalAccessAccountButton: () => null,
      renderApiKeyUsagePanel: () => { throw new Error('WorkBuddy must not enter relay usage rendering'); },
      importApiServiceGuideCount: null, externalImportProgress: { status: 'idle', progress: 0 },
      t: (key: string) => key,
    }));
    const node = findCredits(result.renderCompactRows(accounts));
    assert.ok(node);
    assert.equal(node.props.source, source);
    assert.equal(node.props.compact, true);
  }
  renderer.unmount();
});
