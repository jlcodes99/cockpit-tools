import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { AutoRefreshSchedulerTask } from '../utils/autoRefreshScheduler';

test('TRAE refresh schedules use native identities without reading old WebView selections', async () => {
  const file = new URL('./useAutoRefresh.ts', import.meta.url);
  const source = ts.createSourceFile(file.pathname, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest);
  const refreshes: string[] = [];
  const requested: string[][] = [];
  let startup: (() => void) | undefined;
  let tasks: AutoRefreshSchedulerTask[] = [];
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
  const imports: Record<string, unknown> = {
    '@tauri-apps/api/core': { invoke: async () => ({
      trae_auto_refresh_minutes: 60,
      trae_solo_auto_refresh_minutes: 60,
    }) },
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
        tasks = next;
        return { start() {}, stop() {} };
      },
    },
    '../services/codexAccountGroupService': {
      getCodexAccountGroups: async () => [],
      getCodexCustomQuotaRefreshAccountIdsByMinutes: async () => new Map(),
    },
    '../utils/tauriEventListener': {
      listenSafely: async () => () => {},
      subscribeTauriEvent: () => () => {},
    },
  };
  const fallback = new Proxy({}, { get: (_, key) => String(key).endsWith('Store') ? store : () => {} });
  for (const statement of source.statements) {
    if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) {
      const name = statement.moduleSpecifier.text;
      if (name !== 'react') imports[name] ??= fallback;
    }
  }
  // The preference load is asynchronous in production.
  imports['../utils/durableUiSettings'] = { hydrateDurableUiSettings: async () => {} };
  const hook = loadHookModule(file, imports, {
    console: { log() {}, warn() {}, error() {} },
    performance: { now: () => 0 },
    localStorage: { getItem: () => { legacyReads++; return 'obsolete-webview-account'; } },
    window: {
      setTimeout: (callback: () => void) => { startup = callback; return 1; },
      clearTimeout() {},
      addEventListener() {},
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
  hook.unmount();
});
