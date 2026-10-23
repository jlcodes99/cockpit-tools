import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const compiled = ts.transpileModule(readFileSync(new URL('./chatgptPlanService.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
function harness(handler?: () => Promise<unknown>, timers: Record<string, unknown> = {}) {
  const calls: unknown[] = [];
  const exports: Record<string, any> = {};
  vm.runInNewContext(compiled, { exports, URL, Error, setTimeout, clearTimeout, ...timers, require: () => ({ async invoke(command: string, args?: unknown) {
    calls.push({ command, args }); return handler ? handler() : {};
  } }) });
  return { calls, service: exports };
}

test('plan commands use separate registration identities and join performs no extra test request', async () => {
  const { service, calls } = harness();
  await service.startChatGptPlanLogin();
  await service.startChatGptPlanLogin('registration');
  await service.joinChatGptPlan('registration', 'model');
  await service.removeChatGptPlanFromService('registration');
  await service.disconnectChatGptPlan('registration');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'chatgpt_plan_start_login', args: { registrationId: null } },
    { command: 'chatgpt_plan_start_login', args: { registrationId: 'registration' } },
    { command: 'chatgpt_plan_join', args: { registrationId: 'registration', model: 'model' } },
    { command: 'chatgpt_plan_remove_from_service', args: { registrationId: 'registration' } },
    { command: 'chatgpt_plan_disconnect', args: { registrationId: 'registration' } },
  ]));
});

test('timed-out account reads can retry and late results cannot replace the new single-flight read', async () => {
  const pending: ((value: unknown) => void)[] = [];
  let timeout: (() => void) | undefined;
  const { service, calls } = harness(() => new Promise((resolve) => pending.push(resolve)), {
    setTimeout(callback: () => void, milliseconds: number) { assert.equal(milliseconds, 15_000); timeout = callback; return 1; },
    clearTimeout() {},
  });
  const first = service.listChatGptPlanAccounts();
  timeout!();
  await assert.rejects(first, /CHATGPT_PLAN_TIMEOUT/);
  const second = service.listChatGptPlanAccounts();
  assert.equal(calls.length, 2);
  pending[0]([{ id: 'stale' }]);
  await Promise.resolve();
  assert.equal(service.listChatGptPlanAccounts(), second);
  pending[1]([{ id: 'fresh' }]);
  assert.equal((await second)[0].id, 'fresh');
});

test('plan error messages allowlist codes and never interpolate raw credentials or server errors', () => {
  const { service } = harness();
  for (const failure of ['https://user:secret@host/?token=secret', '/private/auth.json', 'CHATGPT_PLAN_NETWORK: secret', new Error('access_token=secret')]) {
    assert.equal(service.chatGptPlanErrorKey(failure), 'chatgptPlan.error.generic');
  }
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_NATIVE_REQUIRED'), 'chatgptPlan.error.native');
  assert.equal(service.chatGptPlanErrorKey(new Error('CHATGPT_PLAN_IDENTITY_MISMATCH')), 'chatgptPlan.error.mismatch');
});

test('opening authorization only accepts the official HTTPS identity host', () => {
  const { service } = harness();
  assert.equal(service.chatGptPlanAuthorizationUrl('https://auth.openai.com/api/accounts/authorize?state=value'), 'https://auth.openai.com/api/accounts/authorize?state=value');
  for (const url of ['http://auth.openai.com/', 'https://auth.openai.com.evil.test/', 'javascript:alert(1)', 'https://user:pass@auth.openai.com/']) {
    assert.throws(() => service.chatGptPlanAuthorizationUrl(url), /CHATGPT_PLAN_INVALID_CALLBACK/);
  }
});

test('authorization, verification and API Service membership remain distinct states', () => {
  const { service } = harness();
  const account = { connected: true, needs_reauth: false, in_service: false };
  assert.equal(service.chatGptPlanStatusKey(account), 'chatgptPlan.status.ready');
  assert.equal(service.chatGptPlanStatusKey({ ...account, verified_at: 1 }), 'chatgptPlan.status.ready');
  assert.equal(service.chatGptPlanStatusKey({ ...account, in_service: true }), 'chatgptPlan.status.joined');
  assert.equal(service.chatGptPlanStatusKey({ ...account, in_service: true, verified_at: 1 }), 'chatgptPlan.status.joined');
  assert.equal(service.chatGptPlanStatusKey({ ...account, needs_reauth: true, in_service: true }), 'chatgptPlan.status.reauthorize');
  assert.equal(service.chatGptPlanStatusKey({ ...account, connected: false }), 'chatgptPlan.status.disconnected');
});

test('concurrent account discovery shares a storage read and releases the flight after completion', async () => {
  let complete!: (value: unknown) => void;
  const pending = new Promise((resolve) => { complete = resolve; });
  const { service, calls } = harness(() => pending);
  const first = service.listChatGptPlanAccounts();
  const second = service.listChatGptPlanAccounts();
  assert.equal(first, second);
  assert.equal(calls.length, 1);
  complete([]);
  await first;
  await service.listChatGptPlanAccounts();
  assert.equal(calls.length, 2);
});

test('raw nullable authorization summaries preserve membership while the API Service member snapshot stays authoritative', () => {
  const { service } = harness();
  const saved = { id: 'registration', in_service: true, email: null, verified_at: null, verified_model: null };
  const { in_service: _member, ...raw } = saved;
  assert.equal(service.withChatGptPlanMembership(raw, undefined, saved).in_service, true);
  assert.equal(service.withChatGptPlanMembership(raw, ['chatgpt_plan_registration']).in_service, true);
  assert.equal(service.withChatGptPlanMembership(saved, []).in_service, false);
  assert.equal(service.withChatGptPlanMembership(raw, undefined, { id: 'other', in_service: true }).in_service, false);
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_USAGE_UNAVAILABLE'), 'chatgptPlan.error.usageUnavailable');
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_NOT_ELIGIBLE'), 'chatgptPlan.error.notEligible');
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_UNSUPPORTED_CAPABILITY'), 'chatgptPlan.error.unsupportedCapability');
});


test('manual callback and incognito commands use the existing pending login ID', async () => {
  const { service, calls } = harness();
  await service.openChatGptPlanIncognitoWindow('pending-login');
  await service.submitChatGptPlanCallbackUrl('pending-login', 'http://localhost:1455/auth/callback?code=one&state=two');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'chatgpt_plan_open_incognito_window', args: { loginId: 'pending-login' } },
    { command: 'chatgpt_plan_submit_callback_url', args: { loginId: 'pending-login', callbackUrl: 'http://localhost:1455/auth/callback?code=one&state=two' } },
  ]));
});


test('binding and service-mode changes retain explicit source identity and never synthesize a verification', async () => {
  const { service, calls } = harness();
  await service.bindChatGptPlanAccount('registration', 'source');
  await service.setChatGptPlanServiceMode('source', 'codex');
  await service.setChatGptPlanServiceMode('source', 'official', 'registration', 'model');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'chatgpt_plan_bind_account', args: { registrationId: 'registration', sourceAccountId: 'source' } },
    { command: 'chatgpt_plan_set_service_mode', args: { sourceAccountId: 'source', mode: 'codex', registrationId: null, intent: 'join' } },
    { command: 'chatgpt_plan_set_service_mode', args: { sourceAccountId: 'source', mode: 'official', registrationId: 'registration', intent: 'join', model: 'model' } },
  ]));
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_SOURCE_IDENTITY_UNKNOWN'), 'chatgptPlan.error.sourceIdentityUnknown');
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_BINDING_CONFLICT'), 'chatgptPlan.error.bindingConflict');
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_MODE_PREFERENCE_SAVE_FAILED'), 'chatgptPlan.error.modePreferenceSaveFailed');
});


test('unlink removes only the explicit registration link and normalizes binding errors', async () => {
  const { service, calls } = harness();
  await service.unlinkChatGptPlanAccount('registration');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'chatgpt_plan_unbind_account', args: { registrationId: 'registration' } },
  ]));
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_BINDING_MISMATCH'), 'chatgptPlan.error.bindingRequired');
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_ACCOUNT_NOT_FOUND'), 'chatgptPlan.error.missing');
});

test('service mode mutations share an unfinished transaction across dialogs and block conflicting modes', async () => {
  let finish!: (value: unknown) => void;
  const pending = new Promise((resolve) => { finish = resolve; });
  const { service, calls } = harness(() => pending);
  const first = service.setChatGptPlanServiceMode('source', 'official', 'grant', 'model');
  assert.equal(service.setChatGptPlanServiceMode('source', 'official', 'grant', 'model'), first);
  await assert.rejects(service.setChatGptPlanServiceMode('source', 'codex'), /CHATGPT_PLAN_BUSY/);
  assert.equal(calls.length, 1);
  const other = service.setChatGptPlanServiceMode('other', 'codex');
  assert.equal(calls.length, 2, 'different accounts remain independent');
  finish({ collection: { accountIds: ['chatgpt_plan_grant'] } });
  await Promise.all([first, other]);
  await service.setChatGptPlanServiceMode('source', 'codex');
  assert.equal(calls.length, 3, 'settled transactions release their source lock');
});

test('failed service mode mutations release their flight for an explicit retry', async () => {
  let attempt = 0;
  const { service, calls } = harness(async () => { if (++attempt === 1) throw new Error('failure'); return {}; });
  await assert.rejects(service.setChatGptPlanServiceMode('source', 'official', 'grant', 'model'));
  await service.setChatGptPlanServiceMode('source', 'official', 'grant', 'model');
  assert.equal(calls.length, 2);
});

test('authorization check and initial join issue no generation request and need no model argument', async () => {
  const { service, calls } = harness();
  await service.checkChatGptPlanAuthorization('grant');
  await service.setChatGptPlanServiceMode('source', 'official', 'grant');
  await service.joinChatGptPlan('grant');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'chatgpt_plan_check_authorization', args: { registrationId: 'grant' } },
    { command: 'chatgpt_plan_set_service_mode', args: { sourceAccountId: 'source', mode: 'official', registrationId: 'grant', intent: 'join' } },
    { command: 'chatgpt_plan_join', args: { registrationId: 'grant' } },
  ]));
  const account = { id: 'grant', source_account_id: 'source', source_binding_valid: true, connected: true, needs_reauth: false };
  assert.equal(service.isChatGptPlanAuthorizationUsable(account), true);
  assert.equal(service.chatGptPlanStatusKey(account), 'chatgptPlan.status.ready');
  assert.equal(service.isChatGptPlanAuthorizationUsable({ ...account, check_error: 'CHATGPT_PLAN_NETWORK' }), true);
  assert.equal(service.isChatGptPlanAuthorizationUsable({ ...account, needs_reauth: true }), false);
  assert.equal(service.isChatGptPlanAuthorizationUsable({ ...account, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }), false);
});

test('a service UI deadline remains retryable without releasing or duplicating the backend mutation', async () => {
  let resolve!: (value: unknown) => void;
  const raw = new Promise((next) => { resolve = next; });
  const deadlines: (() => void)[] = [];
  const { service, calls } = harness(() => raw, {
    setTimeout(callback: () => void, milliseconds: number) { assert.equal(milliseconds, 90_000); deadlines.push(callback); return deadlines.length; }, clearTimeout() {},
  });
  const first = service.setChatGptPlanServiceMode('source', 'official', 'grant');
  deadlines[0]();
  await assert.rejects(first, /CHATGPT_PLAN_TIMEOUT/);
  await assert.rejects(service.setChatGptPlanServiceMode('source', 'codex'), /CHATGPT_PLAN_BUSY/);
  const retry = service.setChatGptPlanServiceMode('source', 'official', 'grant');
  assert.notEqual(retry, first);
  assert.equal(calls.length, 1);
  resolve({ collection: { accountIds: ['chatgpt_plan_grant'] } });
  assert.equal((await retry).collection.accountIds[0], 'chatgpt_plan_grant');
});


test('switch intent cannot alias an unfinished join, and release is one backend transaction', async () => {
  const { service, calls } = harness();
  await service.setChatGptPlanServiceMode('source', 'official', 'grant', { intent: 'switch' });
  await service.releaseChatGptPlanAuthorization('grant', 'local_gateway');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'chatgpt_plan_set_service_mode', args: { sourceAccountId: 'source', mode: 'official', registrationId: 'grant', intent: 'switch' } },
    { command: 'chatgpt_plan_release_authorization', args: { registrationId: 'grant', disposition: 'local_gateway' } },
  ]));
  assert.equal(service.chatGptPlanErrorKey('CHATGPT_PLAN_SERVICE_MEMBER_REMOVED'), 'chatgptPlan.error.serviceMemberRemoved');
});

test('legacy detach unwraps the new atomic result while disconnect preserves its returned service state', async () => {
  const result = { authorization: { id: 'grant', source_account_id: null }, state: { collection: { accountIds: ['source'], pausedAccountIds: ['source'] } }, remote_revoked: false };
  const { service } = harness(async () => result);
  assert.equal(await service.unlinkChatGptPlanAccount('grant'), result.authorization);
  const disconnected = await service.disconnectChatGptPlan('grant');
  assert.equal(disconnected, result);
});

test('canonical source membership reads the explicit binding map and never treats every member as official', () => {
  const { service } = harness();
  const grant = { id: 'grant', source_account_id: 'source', in_service: true };
  assert.equal(service.withChatGptPlanMembership(grant, ['source'], undefined, { source: 'grant' }).in_service, true);
  assert.equal(service.withChatGptPlanMembership(grant, ['source'], undefined, {}).in_service, false);
  assert.equal(service.withChatGptPlanMembership(grant, [], undefined, { source: 'grant' }).in_service, false);
  assert.equal(service.withChatGptPlanMembership(grant, ['source'], undefined, { source: 'other' }).in_service, false);
});

test('mode success requires canonical membership, the selected binding and a non-paused route', () => {
  const { service } = harness();
  const good = { collection: { accountIds: ['source'], officialAccountBindings: { source: 'grant' } }, serviceMembers: [{ sourceAccountId: 'source', accountId: 'source', mode: 'official', paused: false }] };
  assert.equal(service.isChatGptPlanServiceModeApplied(good, 'source', 'official', 'grant'), true);
  assert.equal(service.isChatGptPlanServiceModeApplied(good, 'source', 'codex'), false);
  assert.equal(service.isChatGptPlanServiceModeApplied(good, 'source', 'official'), false);
  for (const patch of [{ accountIds: ['chatgpt_plan_grant'] }, { officialAccountBindings: {} }, { pausedAccountIds: ['source'] }]) {
    assert.equal(service.isChatGptPlanServiceModeApplied({ ...good, collection: { ...good.collection, ...patch } }, 'source', 'official', 'grant'), false);
  }
  assert.equal(service.isChatGptPlanServiceModeApplied({ ...good, serviceMembers: [{ ...good.serviceMembers[0], paused: true }] }, 'source', 'official', 'grant'), false);
});
