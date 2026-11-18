import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { Account } from '../types/account';
import type { AccountSyncEventPayload } from '../utils/accountSyncEvents';
import type { AutoRefreshSchedulerTask } from '../utils/autoRefreshScheduler';
import { createCodexAutomaticQuotaRefreshPredicate } from '../utils/codexAutoRefreshPlanScope';

const hookFile = new URL('./useAutoRefresh.ts', import.meta.url);

function fillAutoRefreshImports(imports: Record<string, unknown>, fallbackStore: unknown) {
  const source = ts.createSourceFile(hookFile.pathname, readFileSync(hookFile, 'utf8'), ts.ScriptTarget.Latest);
  const fallback = new Proxy({}, { get: (_, key) => String(key).endsWith('Store') ? fallbackStore : () => {} });
  for (const statement of source.statements) {
    if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) {
      const name = statement.moduleSpecifier.text;
      if (name !== 'react') imports[name] ??= fallback;
    }
  }
  return imports;
}

test('TRAE refresh schedules use native identities without reading old WebView selections', async () => {
  const refreshes: string[] = [];
  const requested: string[][] = [];
  const codexRefreshes: Array<{ ids: string[]; background: boolean }> = [];
  const currentCodexRefreshes: Array<boolean | undefined> = [];
  let codexReloads = 0;
  let startup: (() => void) | undefined;
  let tasks: AutoRefreshSchedulerTask[] = [];
  let schedulersCreated = 0;
  let configUpdated: (() => void) | undefined;
  let legacyReads = 0;
  const account = { id: 'global-current', email: 'global@example.test' };
  const paused = { id: 'solo-current', email: 'solo@example.test' };
  let globalCurrent = account;

  const state = new Proxy({ accounts: [], currentAccountId: null, currentAccount: null }, {
    get(target, key) {
      if (key === 'refreshToken') return async (id: string) => { refreshes.push(id); };
      return Reflect.get(target, key) ?? (() => {});
    },
  });
  const store = Object.assign((select: (value: typeof state) => unknown) => select(state), {
    getState: () => ({ accounts: [], currentAccountId: null, currentAccount: null }),
  });
  const codexState = {
    accounts: [
      { id: 'codex-plus', plan_type: 'plus' },
      { id: 'codex-free', plan_type: 'free' },
      { id: 'codex-group-override', plan_type: 'plus' },
    ].map(account => ({ ...account, tokens: { access_token: 'fixture-access', refresh_token: 'fixture-refresh' } })),
    currentAccount: { id: 'codex-plus', email: 'codex@example.test' },
    fetchAccounts: async () => { codexReloads++; },
    fetchCurrentAccount: async () => {},
  };
  const imports: Record<string, unknown> = {
    '@tauri-apps/api/core': { invoke: async (command: string, args?: { background?: boolean }) => {
      if (command === 'refresh_current_codex_quota') {
        currentCodexRefreshes.push(args?.background);
        return;
      }
      return {
        codex_auto_refresh_minutes: 1,
        codex_auto_refresh_plan_types: ['plus'],
        trae_auto_refresh_minutes: 60,
        trae_solo_auto_refresh_minutes: 60,
      };
    } },
    '../services/traeService': {
      getTraeCurrentAccounts: async (platforms: string[]) => {
        requested.push([...platforms]);
        return { trae: globalCurrent, trae_solo: paused };
      },
    },
    '../utils/currentAccountRefresh': {
      loadCurrentAccountRefreshMinutesMap: () => ({ trae: 30, trae_solo: 30 }),
      getAccountRefreshMinutes: (_platform: string, email: string) => email === paused.email ? -1 : 7,
    },
    '../utils/autoRefreshScheduler': {
      createAutoRefreshScheduler: (next: AutoRefreshSchedulerTask[]) => {
        schedulersCreated++;
        tasks = next;
        return { start() {}, stop() {}, replaceTasks(updated: AutoRefreshSchedulerTask[]) { tasks = updated; } };
      },
    },
    '../services/codexAccountGroupService': {
      getCodexAccountGroups: async () => [],
      resolveCodexGroupQuotaAutoRefreshMinutes: () => undefined,
      getCodexCustomQuotaRefreshAccountIdsByMinutes: async () => new Map(),
      getCodexInheritPlatformQuotaRefreshAccountIds: async (ids: string[]) => ids.filter(id => id !== 'codex-group-override'),
    },
    '../stores/useCodexAccountStore': {
      useCodexAccountStore: Object.assign((select: (value: typeof codexState) => unknown) => select(codexState), {
        getState: () => codexState,
      }),
    },
    '../utils/codexAutoRefreshPlanScope': { createCodexAutomaticQuotaRefreshPredicate },
    '../services/codexService': {
      refreshCodexQuotasBatch: async (ids: string[], options: { background: boolean }) => {
        codexRefreshes.push({ ids: Array.from(ids), background: options.background });
      },
    },
    '../services/codexApiKeyUsageRefreshService': {
      refreshCodexApiKeyUsageForAccounts: async () => {},
    },
    '../utils/tauriEventListener': {
      listenSafely: async () => () => {},
      subscribeTauriEvent: () => () => {},
    },
  };
  // The preference load is asynchronous in production.
  imports['../utils/durableUiSettings'] = { hydrateDurableUiSettings: async () => {} };
  const hook = loadHookModule(hookFile, fillAutoRefreshImports(imports, store), {
    console: { log() {}, warn() {}, error() {} },
    performance: { now: () => 0 },
    localStorage: { getItem: () => { legacyReads++; return 'obsolete-webview-account'; } },
    window: {
      setTimeout: (callback: () => void) => { startup = callback; return 1; },
      clearTimeout() {},
      addEventListener(event: string, handler: () => void) {
        if (event === 'config-updated') configUpdated = handler;
      },
      removeEventListener() {},
    },
  });
  hook.render(() => hook.exports.useAutoRefresh());
  assert.ok(startup);
  startup();
  await settlePromises();

  assert.deepEqual(requested, [['trae', 'trae_solo']]);
  assert.equal(legacyReads, 0);
  assert.equal(tasks.find(task => task.key === 'full:trae_solo')?.intervalMs, 60 * 60_000);
  assert.ok(!tasks.some(task => task.key === 'current:trae_solo'));
  const current = tasks.find(task => task.key === 'current:trae');
  assert.ok(current);
  assert.equal(current.intervalMs, 7 * 60_000);
  await current.run();
  globalCurrent = paused;
  await current.run();
  assert.deepEqual(refreshes, ['global-current']);

  const codex = tasks.find(task => task.key === 'full:codex');
  assert.ok(codex);
  assert.equal(codex.intervalMs, 60_000);
  await codex.run();
  assert.deepEqual(codexRefreshes, [{ ids: ['codex-plus'], background: true }]);
  assert.equal(codexReloads, 1);
  const currentCodex = tasks.find(task => task.key === 'current:codex');
  assert.ok(currentCodex);
  await currentCodex.run();
  assert.deepEqual(currentCodexRefreshes, [true]);
  assert.equal(codexReloads, 2);
  assert.ok(configUpdated);
  configUpdated();
  await settlePromises();
  assert.equal(schedulersCreated, 1, 'an unrelated configuration event must not reset the refresh countdown');
  hook.unmount();
});

test('Antigravity automatic quota refresh keeps cached accounts visible while fetching updated quotas', async () => {
  const account: Account = {
    id: 'fixture',
    email: 'fixture@example.test',
    token: { access_token: '', refresh_token: '', expires_in: 0, expiry_timestamp: 0, token_type: 'Bearer' },
    created_at: 1,
    last_used: 1,
    quota: { models: [{ name: 'fixture-model', percentage: 75, reset_time: '' }], last_updated: 1 },
  };
  const listRead = deferred<Account[]>();
  const runtimeTarget = {
    DEFAULT_ANTIGRAVITY_RUNTIME_TARGET: 'antigravity_ide',
    normalizeAntigravityRuntimeTarget: (target: string) => target,
    getAntigravityRuntimeTarget: () => 'antigravity_ide',
  };
  const { exports } = loadHookModule(new URL('../stores/useAccountStore.ts', import.meta.url), {
    '../services/accountService': {
      listAccounts: () => listRead.promise,
      getCurrentAccount: async () => account,
    },
    '../utils/accountSyncEvents': {},
    '../utils/providerAccountDisplayCache': {},
    '../utils/antigravityRuntimeTarget': runtimeTarget,
  }, {
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    setTimeout: () => 1,
  });
  const store: typeof import('../stores/useAccountStore').useAccountStore = exports.useAccountStore;
  store.setState({ accounts: [account] });

  type RefreshListener = (event: { payload: AccountSyncEventPayload }) => Promise<void>;
  let receiveRefresh: RefreshListener | undefined;
  const hook = loadHookModule(hookFile, fillAutoRefreshImports({
    '../stores/useAccountStore': { useAccountStore: store },
    '../utils/antigravityRuntimeTarget': runtimeTarget,
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts:changed' },
    '../utils/tauriEventListener': {
      listenSafely: async () => () => {},
      subscribeTauriEvent: (name: string, listener: RefreshListener) => {
        if (name === 'accounts:changed') receiveRefresh = listener;
        return () => {};
      },
    },
  }, (selector: (state: object) => unknown) => selector({})), {
    window: { setTimeout: () => 1, clearTimeout() {}, addEventListener() {}, removeEventListener() {} },
  });
  hook.render(() => hook.exports.useAutoRefresh());
  assert.ok(receiveRefresh);
  const refresh = receiveRefresh({
    payload: { platformId: 'antigravity', reason: 'automatic-quota-refresh' },
  });

  assert.equal(store.getState().loading, false);
  assert.equal(store.getState().accounts[0], account);
  listRead.resolve([{
    ...account,
    quota: { models: [{ name: 'fixture-model', percentage: 55, reset_time: '' }], last_updated: 2 },
  }]);
  await refresh;
  assert.equal(store.getState().accounts[0].quota?.models[0].percentage, 55);
  hook.unmount();
});
