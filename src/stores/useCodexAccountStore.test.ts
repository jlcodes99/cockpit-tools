import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';
import { mergeCodexQuotaUpdate } from '../utils/codexQuotaSync';

function setup(options: { missing?: boolean; fail?: boolean; failAfterCommit?: boolean; deferCurrent?: boolean } = {}) {
  let lists = 0, currents = 0, switches = 0;
  let currentId = 'b';
  const currentRead = options.deferCurrent ? deferred<any>() : null;
  const events: any[] = [], hydrations: any[] = [];
  const authoritative = { id: 'a', email: 'fresh@example.com', token_generation: 2, quota: { hourly_percentage: 70, weekly_percentage: 80 } };
  const service = {
    async listCodexAccounts() { lists++; return options.missing ? [] : [authoritative, { id: 'b' }]; },
    async getCurrentCodexAccount() { currents++; return currentRead ? currentRead.promise : { id: currentId }; },
    async switchCodexAccount() {
      switches++;
      if (options.fail) throw new Error('requires_reauth');
      currentId = 'a';
      if (options.failAfterCommit) throw new Error('CODEX_SESSION_STARTUP_QUICK_TIMEOUT');
      return { ...authoritative, email: 'switched@example.com' };
    },
  };
  const imports: Record<string, any> = {
    '../types/codex': { hasCodexAccountStructure: () => true, hasCodexAccountName: () => true,
      isCodexTeamLikePlan: () => false, isCodexPendingOAuthAccount: () => false, isCodexApiKeyAccount: () => false, isCodexAgentIdentityAccount: () => false },
    '../services/codexService': service,
    '../services/codexAccountGroupService': {},
    '../utils/accountSyncEvents': { emitCurrentAccountChanged: async (event: any) => { events.push(event); } },
    '../utils/codexProxyCache': { withoutCachedProxySecret: (value: any) => value },
    '../utils/serialAccountRefreshQueue': { createSerialAccountRefreshQueue: () => ({ enqueue: async (ids: string[]) => { hydrations.push(ids); } }) },
    '../utils/coalescedWriter': { createCoalescedWriter: () => ({ schedule() {}, flush() {} }) },
    '../utils/codexProfileRefresh': {},
    '../utils/codexQuotaSync': { mergeCodexQuotaUpdate },
  };
  const source = readFileSync(new URL('./useCodexAccountStore.ts', import.meta.url), 'utf8')
    .replace('import.meta.env.VITE_COCKPIT_TOOLS_PROFILE', "''");
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports: any = {};
  const require = createRequire(import.meta.url);
  runInNewContext(compiled, { exports, require: (name: string) => name in imports ? imports[name] : require(name),
    window: { addEventListener() {} }, localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    console: { info() {}, warn() {}, error() {} }, performance });
  const store = exports.useCodexAccountStore;
  return { store, events, hydrations, counts: () => ({ lists, currents, switches }),
    resolveCurrent: () => currentRead?.resolve({ id: currentId }) };
}

test('successful switch checks the authoritative list once and applies the returned snapshot without a second full-list refresh', async () => {
  const s = setup();
  const returned = await s.store.getState().switchAccount('a'); await settlePromises();
  assert.deepEqual(s.counts(), { lists: 1, currents: 0, switches: 1 });
  assert.equal(s.store.getState().currentAccount, returned);
  assert.equal(s.store.getState().accounts.find((account: any) => account.id === 'a').email, 'switched@example.com');
  assert.equal(JSON.stringify(s.events), JSON.stringify([{ platformId: 'codex', accountId: 'a', reason: 'switch' }]));
  assert.equal(JSON.stringify(s.hydrations), JSON.stringify([['a']]));
});

test('explicit reconciliation keeps both authoritative reads, while missing accounts never reach the switch command', async () => {
  const s = setup(); await s.store.getState().switchAccount('a', { reconcileAfterSwitch: true });
  assert.deepEqual(s.counts(), { lists: 2, currents: 1, switches: 1 });
  const missing = setup({ missing: true }); missing.store.setState({ accounts: [{ id: 'a' }] });
  await assert.rejects(missing.store.getState().switchAccount('a'), /CODEX_STALE_ACCOUNT/);
  assert.deepEqual(missing.counts(), { lists: 1, currents: 1, switches: 0 });
  assert.equal(missing.store.getState().accounts.length, 0);
});

test('failed switch still reloads authoritative reauthorization state and emits no successful current-account event', async () => {
  const s = setup({ fail: true });
  s.store.setState({ currentAccount: { id: 'b' } });
  await assert.rejects(s.store.getState().switchAccount('a'), /requires_reauth/); await settlePromises();
  assert.deepEqual(s.counts(), { lists: 2, currents: 1, switches: 1 }); assert.equal(s.events.length, 0);
  assert.equal(s.store.getState().currentAccount.id, 'b', 'credential rejection must retain the actual old account');
});

test('a post-commit startup failure reports immediately and later reconciles the actual switched current account', async () => {
  const s = setup({ failAfterCommit: true, deferCurrent: true });
  s.store.setState({ currentAccount: { id: 'b' } });
  await assert.rejects(s.store.getState().switchAccount('a', { launchAfterSwitch: true }), /CODEX_SESSION_STARTUP_QUICK_TIMEOUT/);
  assert.deepEqual(s.counts(), { lists: 2, currents: 1, switches: 1 });
  assert.equal(s.store.getState().currentAccount.id, 'b', 'do not guess that the requested account became current');
  s.resolveCurrent(); await settlePromises();
  assert.equal(s.store.getState().currentAccount.id, 'a', 'authoritative disk state wins after the asynchronous read');
  assert.equal(s.events.length, 0, 'the failed startup must not emit a successful switch event');
});

test('confirmed newer quota updates survive the initial authoritative list and the returned account snapshot', async () => {
  const s = setup();
  s.store.getState().applyQuotaUpdate({ account_id: 'a', token_generation: 2, usage_updated_at: 1234, sequence: 1,
    quota: { hourly_percentage: 42, weekly_percentage: 43 } });
  const returned = await s.store.getState().switchAccount('a');
  assert.equal(returned.quota.hourly_percentage, 42);
  assert.equal(s.store.getState().accounts.find((account: any) => account.id === 'a').quota.weekly_percentage, 43);
  assert.equal(s.store.getState().currentAccount, returned);
});
