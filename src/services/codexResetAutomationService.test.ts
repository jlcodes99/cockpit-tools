import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, deferred, settlePromises } from '../../tests/helpers/reactHookHarness';

function setup() {
  const requests: { name: string; args: any; task: ReturnType<typeof deferred<any>> }[] = [];
  const invalidated: string[] = [];
  let onEvent: (() => void) | undefined;
  const harness = loadHookModule(new URL('./codexResetAutomationService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke(name: string, args: unknown) {
      const task = deferred<any>(); requests.push({ name, args, task }); return task.promise;
    } },
    '../utils/tauriEventListener': { subscribeTauriEvent(_name: string, fn: () => void) { onEvent = fn; return () => { onEvent = undefined; }; } },
    './codexResetCreditService': { codexResetCreditDetails: { invalidate: (id: string) => invalidated.push(id) } },
  }, { console });
  const service = harness.exports;
  const snapshot = (revision: number, extra = {}) => ({ revision, public_rule: { ...service.defaultResetRule }, accounts: {}, records: [], ...extra });
  return { service, requests, invalidated, snapshot, emit: () => onEvent?.() };
}
test('all card subscribers share one load and receive public-rule saves', async () => {
  const s = setup(); let renders = 0;
  const stopA = s.service.subscribeResetAutomation(() => renders++);
  const stopB = s.service.subscribeResetAutomation(() => renders++);
  const a = s.service.loadResetAutomation(), b = s.service.loadResetAutomation();
  assert.equal(a, b); assert.equal(s.requests.length, 1);
  s.requests[0].task.resolve(s.snapshot(0)); await a;
  const rule = { ...s.service.defaultResetRule, enabled: true };
  const saved = s.service.saveResetPublicRule(rule, s.service.defaultResetRule);
  s.requests[1].task.resolve(s.snapshot(1, { public_rule: rule })); await saved;
  assert.equal(s.service.getResetAutomationSnapshot().public_rule.enabled, true);
  assert.equal(renders, 4);
  stopA(); stopB();
});
test('late reads and late save responses cannot restore old shared rules', async () => {
  const s = setup();
  const load = s.service.loadResetAutomation();
  const save = s.service.saveResetPublicRule({ ...s.service.defaultResetRule, enabled: true }, s.service.defaultResetRule);
  s.requests[1].task.resolve(s.snapshot(2, { public_rule: { ...s.service.defaultResetRule, enabled: true } })); await save;
  s.requests[0].task.resolve(s.snapshot(0)); await load;
  assert.equal(s.service.getResetAutomationSnapshot().revision, 2);
  const oldSave = s.service.saveResetAccountRule('a', { mode: 'off' }, { mode: 'inherit' });
  const newSave = s.service.saveResetAccountRule('b', { mode: 'off' }, { mode: 'inherit' });
  s.requests[3].task.resolve(s.snapshot(4, { accounts: { a: { mode: 'off' }, b: { mode: 'off' } } })); await newSave;
  s.requests[2].task.resolve(s.snapshot(3, { accounts: { a: { mode: 'off' } } })); await oldSave;
  assert.equal(s.service.getResetAutomationSnapshot().accounts.b.mode, 'off');
});
test('an update during a load is fetched again; successful redemptions invalidate details once', async () => {
  const s = setup(); const stop = s.service.subscribeResetAutomation(() => {});
  const loading = s.service.loadResetAutomation(); s.emit();
  s.requests[0].task.resolve(s.snapshot(0)); await loading;
  assert.equal(s.requests.length, 2);
  const record = { id: 'receipt-1', account_id: 'a', at: 1, reason: 'expiry', status: 'success' };
  s.requests[1].task.resolve(s.snapshot(1, { records: [record] })); await settlePromises();
  assert.deepEqual(s.invalidated, ['a']);
  s.emit(); s.requests[2].task.resolve(s.snapshot(1, { records: [record] })); await settlePromises();
  assert.deepEqual(s.invalidated, ['a']); stop();
});
test('failed loads stay unknown and threshold boundaries remain editable', async () => {
  const s = setup(); const load = s.service.loadResetAutomation();
  s.requests[0].task.reject(new Error('disk failure')); await assert.rejects(load, /disk failure/);
  assert.equal(s.service.getResetAutomationSnapshot(), null);
  for (const remaining_percent of [0, 10, 100]) assert.equal(s.service.validateResetRule({ ...s.service.defaultResetRule, remaining_percent }), null);
  assert.equal(s.service.validateResetRule({ ...s.service.defaultResetRule, remaining_percent: 101 }), 'invalidPercent');
  assert.equal(s.service.validateResetRule({ ...s.service.defaultResetRule, remaining_percent: NaN }), 'invalidPercent');
  assert.equal(s.service.validateResetRule({ ...s.service.defaultResetRule, before_seconds: 0 }), 'invalidTime');
  assert.equal(s.service.validateResetRule({ ...s.service.defaultResetRule, before_seconds: 90 }), null);
});

test('disabled unfinished conditions keep saved values and never block turning automation off', () => {
  const s = setup(); const previous = { ...s.service.defaultResetRule, enabled: true, before_seconds: 7200, remaining_percent: 15 };
  const disabled = s.service.prepareResetRuleDraft({ ...previous, enabled: false, before_seconds: NaN, remaining_percent: NaN }, previous);
  assert.equal(disabled.before_seconds, 7200); assert.equal(disabled.remaining_percent, 15);
  assert.equal(s.service.validateResetRule(disabled), null);
  const inactive = s.service.prepareResetRuleDraft({ ...previous, quota_enabled: false, remaining_percent: NaN }, previous);
  assert.equal(inactive.remaining_percent, 15); assert.equal(s.service.validateResetRule(inactive), null);
  const valid = s.service.prepareResetRuleDraft({ ...previous, quota_enabled: false, remaining_percent: 0 }, previous);
  assert.equal(valid.remaining_percent, 0);
  const active = s.service.prepareResetRuleDraft({ ...previous, remaining_percent: NaN }, previous);
  assert.equal(s.service.validateResetRule(active), 'invalidPercent');
});

test('effective status follows public settings and enabled conditions, never a mode alone', () => {
  const s = setup();
  assert.equal(s.service.resolveResetAutomation(null, 'a').enabled, false);
  const state: any = s.snapshot(0, { public_rule: { ...s.service.defaultResetRule, enabled: true } });
  assert.equal(s.service.resolveResetAutomation(state, 'a').enabled, true);
  state.accounts = { a: { mode: 'off' } };
  assert.equal(s.service.resolveResetAutomation(state, 'a').enabled, false);
  state.accounts = { a: { mode: 'custom', rule: { ...state.public_rule, enabled: false } } };
  assert.equal(s.service.resolveResetAutomation(state, 'a').enabled, false);
  state.accounts.a.rule.enabled = true;
  state.accounts.a.rule.expiry_enabled = false; state.accounts.a.rule.quota_enabled = false;
  assert.equal(s.service.resolveResetAutomation(state, 'a').enabled, false);
  state.accounts.a.rule.quota_enabled = true;
  assert.equal(s.service.resolveResetAutomation(state, 'a').enabled, true);
});

test('bulk public saves forward the explicit scope and expected account settings', async () => {
  const s = setup();
  const rule = { ...s.service.defaultResetRule, enabled: true };
  const accounts = { a: { mode: 'off' }, b: { mode: 'custom', rule: s.service.defaultResetRule } };
  const saved = s.service.saveResetPublicRule(rule, s.service.defaultResetRule, true, accounts);
  assert.equal(s.requests[0].name, 'save_codex_reset_public_rule');
  assert.equal(s.requests[0].args.rule, rule);
  assert.equal(s.requests[0].args.expected, s.service.defaultResetRule);
  assert.equal(s.requests[0].args.applyToAll, true);
  assert.equal(s.requests[0].args.expectedAccounts, accounts);
  const next = s.snapshot(2, { public_rule: rule, accounts: { a: { mode: 'custom', rule }, b: { mode: 'custom', rule } } });
  s.requests[0].task.resolve(next); await saved;
  assert.equal(s.service.getResetAutomationSnapshot(), next);
  const defaultsOnly = s.service.saveResetPublicRule(rule, rule);
  assert.equal(s.requests[1].args.applyToAll, false); assert.equal(s.requests[1].args.expectedAccounts, undefined);
  s.requests[1].task.resolve(s.snapshot(3)); await defaultsOnly;
});
