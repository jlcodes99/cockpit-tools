import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';
import { mergeCodexQuotaUpdate } from '../utils/codexQuotaSync';
import { mergeCodexSubscriptionUpdate } from '../utils/codexSubscriptionSync';
import { isCodexAccountDisplayCache } from '../types/codex';
import { toCodexAccountDisplayCache } from '../utils/codexProxyCache';

function setup(options: { missing?: boolean; fail?: boolean; failAfterCommit?: boolean; deferCurrent?: boolean; cache?: boolean; deferList?: boolean } = {}) {
  let lists = 0, currents = 0, switches = 0;
  let currentId = 'b';
  const currentRead = options.deferCurrent ? deferred<any>() : null;
  const listRead = options.deferList ? deferred<any>() : null;
  const events: any[] = [], hydrations: any[] = [];
  const authoritative = { id: 'a', email: 'fresh@example.com', token_generation: 2, quota: { hourly_percentage: 70, weekly_percentage: 80 },
    account_password: 'fixture-password' };
  const service = {
    async listCodexAccounts() { lists++; return listRead ? listRead.promise : options.missing ? [] : [authoritative, { id: 'b' }]; },
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
      isCodexTeamLikePlan: () => false, isCodexPendingOAuthAccount: () => false, isCodexApiKeyAccount: () => false,
      isCodexAgentIdentityAccount: () => false, isCodexAccountDisplayCache },
    '../services/codexService': service,
    '../services/codexAccountGroupService': {},
    '../utils/accountSyncEvents': { emitCurrentAccountChanged: async (event: any) => { events.push(event); } },
    '../utils/codexProxyCache': { toCodexAccountDisplayCache },
    '../utils/providerAccountDisplayCache': { replaceLegacyAccountDisplayCache() {} },
    '../utils/serialAccountRefreshQueue': { createSerialAccountRefreshQueue: () => ({ enqueue: async (ids: string[]) => { hydrations.push(ids); } }) },
    '../utils/coalescedWriter': { createCoalescedWriter: () => ({ schedule() {}, flush() {} }) },
    '../utils/codexProfileRefresh': {},
    '../utils/codexQuotaSync': { mergeCodexQuotaUpdate },
    '../utils/codexSubscriptionSync': { mergeCodexSubscriptionUpdate },
  };
  const source = readFileSync(new URL('./useCodexAccountStore.ts', import.meta.url), 'utf8')
    .replace('import.meta.env.VITE_COCKPIT_TOOLS_PROFILE', "''");
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports: any = {};
  const require = createRequire(import.meta.url);
  runInNewContext(compiled, { exports, require: (name: string) => name in imports ? imports[name] : require(name),
    window: { addEventListener() {} }, localStorage: { getItem: (key: string) => options.cache && key === 'agtools.codex.accounts.cache'
      ? JSON.stringify([{ ...authoritative, tokens: { id_token: '', access_token: 'fixture-access' } }]) : null, setItem() {}, removeItem() {} },
    console: { info() {}, warn() {}, error() {} }, performance });
  const store = exports.useCodexAccountStore;
  return { store, events, hydrations, readForEditing: exports.readCodexAccountForEditing, counts: () => ({ lists, currents, switches }),
    resolveList: () => listRead?.resolve([authoritative]),
    resolveCurrent: () => currentRead?.resolve({ id: currentId }) };
}

test('opening a credential editor from the startup list reads the native record first', async () => {
  const s = setup({ cache: true });
  const preview = s.store.getState().accounts[0];
  assert.equal(preview.account_password, undefined);
  assert.equal((await s.readForEditing(preview)).account_password, 'fixture-password');
  assert.equal(s.store.getState().accounts[0].account_password, 'fixture-password');
  assert.equal(s.counts().lists, 1);
});

test('a newly authorized account appears immediately and survives an older pending list read', async () => {
  const s = setup({ cache: true, deferList: true });
  const pending = s.store.getState().fetchAccounts();
  s.store.getState().applyAccountSnapshot({
    id: 'new-oauth',
    email: 'new@example.invalid',
    token_generation: 1,
  });
  assert.ok(s.store.getState().accounts.some((account: { id: string }) => account.id === 'new-oauth'));

  s.resolveList();
  await pending;
  const ids = s.store.getState().accounts.map((account: { id: string }) => account.id);
  assert.ok(ids.includes('new-oauth'));
  assert.ok(ids.includes('a'));
});

test('a pending credential read cannot restore a deleted account or replace a newer native result', async () => {
  for (const deleted of [true, false]) {
    const s = setup({ cache: true, deferList: true });
    const result = s.readForEditing(s.store.getState().accounts[0]);
    const newer = { id: 'a', email: 'newer@example.com', account_password: 'fixture-newer' };
    s.store.setState({ accounts: deleted ? [] : [newer] });
    s.resolveList();
    if (deleted) {
      await assert.rejects(result, /CODEX_STALE_ACCOUNT/);
      assert.equal(s.store.getState().accounts.length, 0);
    } else {
      assert.equal(await result, newer);
      assert.equal(s.store.getState().accounts[0], newer);
    }
  }
});

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


test('a background subscription result updates the matching account and survives an older account read', async () => {
  const s = setup();
  await s.store.getState().fetchAccounts();
  const original = s.store.getState().accounts[0];
  s.store.getState().applySubscriptionUpdate({
    id: 'a',
    previous_account_id: null,
    account_id: null,
    organization_id: null,
    token_generation: 2,
    plan_type: 'plus',
    usage_updated_at: null,
    subscription_active_until: '2030-01-01T14:23:45Z',
    subscription_query_last_attempt_at: 100,
    subscription_query_last_success_at: 101,
    subscription_query_last_error: null,
  });
  assert.equal(s.counts().lists, 1, 'the event should update the existing account without reloading the list');
  assert.equal(s.store.getState().accounts[0].subscription_active_until, '2030-01-01T14:23:45Z');
  assert.equal(s.store.getState().accounts[0].quota, original.quota);

  await s.store.getState().fetchAccounts();
  assert.equal(s.store.getState().accounts[0].subscription_active_until, '2030-01-01T14:23:45Z');
  s.store.getState().applyAccountSnapshot({ ...original, token_generation: 3 });
  assert.equal(s.store.getState().accounts[0].subscription_active_until, undefined);
});
