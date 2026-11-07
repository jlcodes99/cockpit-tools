import assert from 'node:assert/strict';
import test from 'node:test';
import { canChooseCodexServiceAuthorization, rebaseCodexMemberSelection, requestCodexServiceAuthorization, requestCodexServiceAuthorizations, takeCodexServiceAuthorizationRequests } from './codexAccountServiceNavigation';
import type { CodexAccount } from '../types/codex';

test('dual authorization choices exclude credential kinds that cannot bind a Codex identity', () => {
  const account = { id: 'a', tokens: { access_token: 'access', id_token: 'id' } } as CodexAccount;
  assert.equal(canChooseCodexServiceAuthorization(account), true);
  for (const extra of [{ auth_mode: 'apikey' }, { token_source_mode: 'chatgpt_web_session' }, { agent_identity: { agent_runtime_id: 'runtime' } }, { authorization_status: 'pending' }, { api_provider_mode: 'custom' }, { upstream_grok_account_id: 'grok' }, { api_provider_id: 'provider' }, { openai_api_key: 'key' }]) {
    assert.equal(canChooseCodexServiceAuthorization({ ...account, ...extra } as CodexAccount), false);
  }
});

test('batch entry emits one request and normalizes/deduplicates source IDs', () => {
  const previous = globalThis.window;
  const window = new EventTarget();
  Object.assign(globalThis, { window });
  let events = 0;
  window.addEventListener('codex-account-service-request', () => events++);
  try {
    takeCodexServiceAuthorizationRequests();
    requestCodexServiceAuthorizations(['one', ' two ', 'one', '']);
    assert.equal(events, 1);
    assert.deepEqual(takeCodexServiceAuthorizationRequests(), [{ accountId: 'one' }, { accountId: 'two' }]);
  } finally { Object.assign(globalThis, { window: previous }); }
});

test('inner successful route changes rebase the outer draft without losing unrelated edits', () => {
  const before = ['source', 'keep', 'user-removed'];
  const draft = new Set(['source', 'keep', 'user-added']);
  const after = ['official', 'keep', 'user-removed'];
  assert.deepEqual([...rebaseCodexMemberSelection(draft, before, after)].sort(), ['keep', 'official', 'user-added']);
  assert.deepEqual([...draft], ['source', 'keep', 'user-added']);
  assert.deepEqual([...rebaseCodexMemberSelection(draft, before, before)], [...draft]);
});

test('entry requests preserve explicit mode, queue batch accounts and deduplicate', () => {
  const previous = globalThis.window;
  Object.assign(globalThis, { window: new EventTarget() });
  try {
    takeCodexServiceAuthorizationRequests();
    requestCodexServiceAuthorization('source', 'official');
    requestCodexServiceAuthorization('source', 'codex');
    requestCodexServiceAuthorization('other');
    requestCodexServiceAuthorization(' ');
    assert.deepEqual(takeCodexServiceAuthorizationRequests(), [{ accountId: 'source', initialMode: 'official' }, { accountId: 'other', initialMode: undefined }]);
    assert.deepEqual(takeCodexServiceAuthorizationRequests(), []);
  } finally { Object.assign(globalThis, { window: previous }); }
});


test('draft results are separate from live membership events and delivered once per source', async () => {
  const nav = await import('./codexAccountServiceNavigation');
  const previous = globalThis.window;
  const window = new EventTarget();
  Object.assign(globalThis, { window });
  const results: any[] = []; let commits = 0;
  window.addEventListener(nav.CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, (event) => results.push((event as CustomEvent).detail));
  window.addEventListener(nav.CODEX_ACCOUNT_SERVICE_APPLIED, () => commits++);
  try {
    takeCodexServiceAuthorizationRequests();
    const requestId = nav.requestCodexServiceDraftAuthorizations(['one', ' two ', 'one']);
    const pending = takeCodexServiceAuthorizationRequests();
    assert.deepEqual(pending, [{ accountId: 'one', draftRequestId: requestId }, { accountId: 'two', draftRequestId: requestId }]);
    assert.equal(nav.announceCodexServiceDraftResult({ requestId, sourceAccountId: 'one', status: 'selected', mode: 'codex' }), true);
    assert.equal(nav.announceCodexServiceDraftResult({ requestId, sourceAccountId: 'one', status: 'selected', mode: 'official', registrationId: 'other' }), false);
    assert.equal(nav.announceCodexServiceDraftResult({ requestId, sourceAccountId: 'two', status: 'selected', mode: 'official', registrationId: 'grant' }), true);
    assert.equal(nav.isCodexServiceDraftRequestActive(requestId), false);
    assert.equal(results.length, 2);
    assert.equal(commits, 0);
  } finally { Object.assign(globalThis, { window: previous }); }
});

test('cancelling a draft drops queued and late choices without cancelling an unrelated immediate join', async () => {
  const nav = await import('./codexAccountServiceNavigation');
  const previous = globalThis.window;
  const window = new EventTarget(); Object.assign(globalThis, { window });
  const results: any[] = [];
  window.addEventListener(nav.CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, (event) => results.push((event as CustomEvent).detail));
  try {
    takeCodexServiceAuthorizationRequests();
    const requestId = nav.requestCodexServiceDraftAuthorizations(['one']);
    requestCodexServiceAuthorization('one');
    nav.cancelCodexServiceDraftAuthorizations(requestId);
    assert.deepEqual(takeCodexServiceAuthorizationRequests(), [{ accountId: 'one', initialMode: undefined }]);
    assert.equal(nav.announceCodexServiceDraftResult({ requestId, sourceAccountId: 'one', status: 'selected', mode: 'official', registrationId: 'late' }), false);
    nav.requeueCodexServiceAuthorizationRequests([{ accountId: 'one', draftRequestId: requestId }]);
    assert.deepEqual(takeCodexServiceAuthorizationRequests(), []);
    assert.deepEqual(results, [{ requestId, sourceAccountId: 'one', status: 'cancelled' }]);
  } finally { Object.assign(globalThis, { window: previous }); }
});
