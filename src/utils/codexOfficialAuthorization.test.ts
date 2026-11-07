import assert from 'node:assert/strict';
import test from 'node:test';
import { codexOfficialAuthorizationRows, codexOfficialAuthorizationRelatedIds, codexOfficialAuthorizationDeadline, needsCodexOfficialAuthorizationReview } from './codexOfficialAuthorization';
import type { CodexAccount } from '../types/codex';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
const source = (id: string) => ({ id, email: `${id}@example.com` }) as CodexAccount;
const grant = (id: string, sourceId?: string) => ({ id, source_account_id: sourceId, source_binding_valid: true, connected: true, needs_reauth: false }) as ChatGptPlanAccount;

test('official guide uses real eligible accounts, valid bindings and persisted membership order', () => {
  const rows = codexOfficialAuthorizationRows([source('a'), source('b'), source('c'), { ...source('key'), auth_mode: 'apikey' }],
    [grant('gb', 'b'), { ...grant('gc', 'c'), needs_reauth: true }], ['chatgpt_plan_gb', 'c']);
  assert.deepEqual(rows.map((row) => row.account.id), ['b', 'c', 'a']);
  assert.equal(rows[0].official, true);
  assert.equal(rows[0].authorized, true);
  assert.equal(rows[1].joined, true);
  assert.equal(rows[1].authorized, false);
  assert.equal(rows[2].joined, false);
});

test('matching email, invalid binding or multiple grants never silently choose authorization', () => {
  const account = source('a');
  for (const grants of [
    [{ ...grant('g'), email: account.email }],
    [grant('g', 'a'), grant('other', 'a')],
  ]) {
    const [row] = codexOfficialAuthorizationRows([account], grants, ['a']);
    assert.equal(row.grant, undefined);
    assert.equal(row.authorized, false);
  }
  assert.deepEqual(codexOfficialAuthorizationRelatedIds('a', [grant('g', 'a'), grant('other', 'b')]), ['a', 'chatgpt_plan_g']);
});

test('metadata deadline rejects without retrying the underlying operation', async () => {
  let resolve!: (value: string) => void;
  const operation = new Promise<string>((next) => { resolve = next; });
  await assert.rejects(codexOfficialAuthorizationDeadline(operation, 5), /CHATGPT_PLAN_TIMEOUT/);
  resolve('late');
  assert.equal(await operation, 'late');
  assert.equal(await codexOfficialAuthorizationDeadline(Promise.resolve('ready')), 'ready');
});


test('invalid bindings remain manageable but never count as a usable authorization', () => {
  const invalid = { ...grant('g', 'a'), source_binding_valid: false, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' };
  const [row] = codexOfficialAuthorizationRows([source('a')], [invalid], ['chatgpt_plan_g']);
  assert.equal(row.grant, invalid);
  assert.equal(row.authorized, false);
  assert.equal(row.joined, true);
  assert.deepEqual(codexOfficialAuthorizationRelatedIds('a', [invalid]), ['a', 'chatgpt_plan_g']);
});

test('an existing connected pending binding does not trigger an unapproved reauthorization reminder', () => {
  const pending = { ...grant('g', 'a'), source_binding_valid: false };
  const [row] = codexOfficialAuthorizationRows([source('a')], [pending], ['chatgpt_plan_g']);
  assert.equal(row.authorized, false, 'pending is not promoted to verified authorization');
  assert.equal(needsCodexOfficialAuthorizationReview(row), false);
  for (const patch of [{ needs_reauth: true }, { connected: false }, { source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }]) {
    assert.equal(needsCodexOfficialAuthorizationReview({ ...row, grant: { ...pending, ...patch } }), true);
  }
  assert.equal(needsCodexOfficialAuthorizationReview({ ...row, grant: undefined }), true);
});

test('paused service members survive detached grants without becoming an authorization or reminder', () => {
  const [row] = codexOfficialAuthorizationRows([source('a')], [], ['a'], ['a'], [{ sourceAccountId: 'a', accountId: 'a', mode: 'codex', paused: true }]);
  assert.equal(row.joined, true);
  assert.equal(row.paused, true);
  assert.equal(row.authorized, false);
  assert.equal(needsCodexOfficialAuthorizationReview(row), false);
  const [isolated] = codexOfficialAuthorizationRows([source('a')], [], ['chatgpt_plan_old'], ['chatgpt_plan_old'], [{ sourceAccountId: 'a', accountId: 'chatgpt_plan_old', mode: 'official', paused: true }]);
  assert.equal(isolated.joined, true);
  assert.equal(isolated.official, true);
  assert.equal(isolated.grant, undefined);
});
