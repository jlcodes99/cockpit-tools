import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { withChatGptPlanMembership } from '../../services/chatgptPlanService';

type Node = { type: unknown; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}
const account = { id: 'plan', label: 'Example', email: 'user@example.com', connected: true, needs_reauth: false, in_service: false };

function harness(service: Record<string, unknown>, existing: unknown = account, expectedEmail?: string, memberAccountIds?: string[], discoveryError?: string, embedded = false, extra: Record<string, unknown> = {}) {
  const failures = { message: '', scrollKey: 0, clear() { this.message = ''; }, report(message: string) { this.message = message; this.scrollKey++; } };
  const timers = new Map<number, () => void>();
  const calls = { closed: 0, changed: 0, opened: [] as string[], applied: [] as unknown[][] };
  let nextTimer = 0;
  const hook = loadHookModule(new URL('./ChatGptPlanDialog.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    'react-dom': { createPortal: (value: unknown) => value },
    '@tauri-apps/plugin-opener': { async openUrl(url: string) { calls.opened.push(url); } },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage', useModalErrorState: () => failures },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
    '../../utils/reducedMotion': { scrollElementIntoView() {} },
    '../../utils/codexAccountServiceNavigation': { announceCodexServiceAuthorizationApplied(...args: unknown[]) { calls.applied.push(JSON.parse(JSON.stringify(args))); } },
    '../../services/chatgptPlanService': {
      chatGptPlanAuthorizationUrl: (url: string) => url,
      chatGptPlanStatusKey: () => 'chatgptPlan.status.unverified',
      withChatGptPlanMembership,
      chatGptPlanErrorKey: () => 'chatgptPlan.error.generic',
      async listChatGptPlanModels() { return [{ id: 'model', display_name: 'Model' }]; },
      async listChatGptPlanAccounts() { return [account]; },
      async cancelChatGptPlanLogin() {},
      async startChatGptPlanLogin() { return { login_id: 'default', authorize_url: 'https://auth.openai.com/', expires_at: Date.now() / 1000 + 60 }; },
      async pollChatGptPlanLogin() { return { status: 'pending' }; },
      ...service,
    },
  }, {
    window: new EventTarget(), document: { body: {} },
    setTimeout(callback: () => void) { timers.set(++nextTimer, callback); return nextTimer; },
    clearTimeout(id: number) { timers.delete(id); },
    navigator: { clipboard: { async writeText() {} } },
  });
  hook.render(() => hook.exports.ChatGptPlanDialog({ embedded, account: existing, expectedEmail, memberAccountIds, discoveryError, onClose() { calls.closed++; }, onChanged() { calls.changed++; }, ...extra }));
  const tree = () => nodes(hook.flush());
  const button = (key: string) => tree().find((node) => node.type === 'button' && (node.props.children === key || nodes(node.props.children).some((child) => child.props.children === key) || Array.isArray(node.props.children) && node.props.children.includes(key)))!;
  return { hook, tree, button, failures, calls, timers };
}

test('dialog opens while models load, deduplicates join, and retries failures inside the dialog', async () => {
  const models = deferred<unknown>();
  const join = deferred<void>();
  let joins = 0;
  let tests = 0;
  const ctx = harness({
    listChatGptPlanModels: () => models.promise,
    joinChatGptPlan() { joins++; return join.promise; },
    async verifyChatGptPlan() { tests++; return account; },
  });
  assert.equal(ctx.tree().some((node) => node.props.role === 'dialog'), true);
  assert.equal(ctx.button('chatgptPlan.verifyAndJoin').props.disabled, true);
  models.resolve([{ id: 'model' }]);
  await settlePromises();
  const action = ctx.button('chatgptPlan.verifyAndJoin');
  action.props.onClick(); action.props.onClick();
  assert.equal(joins, 1);
  assert.equal(tests, 0);
  join.reject(new Error('quota limit'));
  await settlePromises();
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.generic');
  assert.equal(ctx.tree().some((node) => node.props.role === 'dialog'), true);
  assert.equal(ctx.calls.closed, 0);
  ctx.hook.unmount();
});

test('closing during login preparation cancels its late-created session', async () => {
  const login = deferred<unknown>();
  const cancelled: string[] = [];
  const fresh = harness({ startChatGptPlanLogin: () => login.promise, async cancelChatGptPlanLogin(id: string) { cancelled.push(id); } }, null);
  fresh.tree().find((node) => node.type === 'button' && node.props['aria-label'] === 'common.close')!.props.onClick();
  fresh.hook.unmount();
  login.resolve({ login_id: 'late', authorize_url: 'https://auth.openai.com/', expires_at: Date.now() / 1000 + 60 });
  await settlePromises();
  assert.deepEqual(cancelled, ['late']);
  assert.equal(fresh.calls.opened.length, 0);
});

test('authorization saves without joining, model requests or automatic service start', async () => {
  let modelRequests = 0;
  let joins = 0;
  const ctx = harness({
    async startChatGptPlanLogin() { return { login_id: 'login', authorize_url: 'https://auth.openai.com/', expires_at: Date.now() / 1000 + 60 }; },
    async pollChatGptPlanLogin() { return { status: 'complete', account }; },
    async listChatGptPlanModels() { modelRequests++; return []; },
    async joinChatGptPlan() { joins++; },
  }, null);
  await settlePromises(); ctx.tree(); await settlePromises();
  assert.equal(ctx.calls.opened.length, 0);
  assert.equal(modelRequests, 0);
  ctx.button('chatgptPlan.saveOnly').props.onClick();
  assert.equal(ctx.calls.closed, 1);
  assert.equal(joins, 0);
  ctx.hook.unmount();
});

test('failed remote revocation is reported explicitly after local disconnect', async () => {
  const ctx = harness({ async disconnectChatGptPlan() { return { remote_revoked: false }; } });
  await settlePromises();
  ctx.button('chatgptPlan.disconnect').props.onClick();
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.disconnectTitle'), true);
  ctx.button('chatgptPlan.disconnect').props.onClick();
  await settlePromises();
  assert.equal(ctx.tree().some((node) => Array.isArray(node.props.children) && node.props.children.includes('chatgptPlan.localDisconnectedNotice')), true);
  assert.equal(ctx.calls.closed, 0);
  ctx.hook.unmount();
});

test('a different authorized email cannot continue or bill verification but may save the separate access', async () => {
  let joins = 0;
  let reads = 0;
  const ctx = harness({
    async startChatGptPlanLogin() { return { login_id: 'login', authorize_url: 'https://auth.openai.com/', expires_at: Date.now() / 1000 + 60 }; },
    async pollChatGptPlanLogin() { return { status: 'complete', account }; },
    async listChatGptPlanModels() { reads++; return [{ id: 'model' }]; },
    async joinChatGptPlan() { joins++; },
  }, null, 'original@example.com');
  ctx.button('codex.oauth.timeoutRetry').props.onClick();
  await settlePromises(); ctx.tree(); await settlePromises();
  assert.equal(ctx.button('chatgptPlan.continue').props.disabled, true);
  // Exercise the handler directly too: a disabled UI must not be the only guard.
  ctx.button('chatgptPlan.continue').props.onClick();
  assert.equal(reads, 0);
  assert.equal(joins, 0);
  assert.equal(ctx.tree().some((node) => node.type === 'ErrorMessage' && node.props.message === 'chatgptPlan.error.mismatch'), true);
  ctx.button('chatgptPlan.saveOnly').props.onClick();
  assert.equal(ctx.calls.closed, 1);
  ctx.hook.unmount();
});

test('token exchange failure retains the issued registration for the next authorization attempt', async () => {
  const registrations: unknown[] = [];
  const draft = { ...account, id: 'issued-registration', connected: false, needs_reauth: true };
  const ctx = harness({
    async startChatGptPlanLogin(id?: string) { registrations.push(id); return { login_id: 'login', authorize_url: 'https://auth.openai.com/', expires_at: Date.now() / 1000 + 60 }; },
    async pollChatGptPlanLogin() { return { status: 'error', account: draft, error: 'CHATGPT_PLAN_CONFIGURATION' }; },
  }, null);
  await settlePromises(); ctx.tree(); await settlePromises(); ctx.tree();
  assert.equal(ctx.calls.changed, 1);
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.generic');
  ctx.button('codex.oauth.timeoutRetry').props.onClick();
  await settlePromises();
  assert.deepEqual(registrations, [undefined, 'issued-registration']);
  ctx.hook.unmount();
});

test('retrying a failed join sends exactly one test per explicit attempt', async () => {
  let attempts = 0;
  let extraTests = 0;
  let joined = false;
  const ctx = harness({
    async joinChatGptPlan() { attempts++; if (attempts === 1) throw new Error('temporary'); joined = true; },
    async verifyChatGptPlan() { extraTests++; return account; },
    async listChatGptPlanAccounts() { return [{ ...account, in_service: joined, verified_at: joined ? 1 : undefined }]; },
  });
  await settlePromises();
  ctx.button('chatgptPlan.verifyAndJoin').props.onClick();
  await settlePromises();
  assert.equal(ctx.calls.closed, 0);
  assert.equal(ctx.button('chatgptPlan.verifyAndJoin').props.disabled, false);
  ctx.button('chatgptPlan.verifyAndJoin').props.onClick();
  await settlePromises();
  assert.equal(attempts, 2);
  assert.equal(extraTests, 0);
  assert.equal(ctx.tree().some((node) => node.type === 'button' && node.props.children === 'chatgptPlan.verifyAndJoin'), false);
  ctx.hook.unmount();
});

test('an expired access failure exposes reauthorization while retaining the saved account identity', async () => {
  const registrations: string[] = [];
  const ctx = harness({
    chatGptPlanErrorKey: () => 'chatgptPlan.error.reauthorize',
    async listChatGptPlanModels() { throw new Error('CHATGPT_PLAN_REAUTH_REQUIRED'); },
    async startChatGptPlanLogin(id: string) { registrations.push(id); return { login_id: 'reauth', authorize_url: 'https://auth.openai.com/', expires_at: Date.now() / 1000 + 60 }; },
    async pollChatGptPlanLogin() { return { status: 'pending' }; },
  });
  await settlePromises();
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.reauthorize');
  ctx.button('common.reauthorize').props.onClick();
  await settlePromises();
  assert.deepEqual(registrations, ['plan']);
  ctx.hook.unmount();
});

test('raw verification summaries without in_service cannot remove existing membership from the dialog', async () => {
  const ctx = harness({
    async verifyChatGptPlan() { return { ...account, in_service: undefined, verified_at: 123, verified_model: 'model' }; },
  }, { ...account, in_service: true }, undefined, ['chatgpt_plan_plan']);
  await settlePromises();
  ctx.button('chatgptPlan.verify').props.onClick();
  await settlePromises();
  assert.equal(Boolean(ctx.button('chatgptPlan.remove')), true);
  assert.equal(ctx.tree().some((node) => node.type === 'button' && node.props.children === 'chatgptPlan.verifyAndJoin'), false);
  ctx.hook.unmount();
});

test('overlay never dismisses the dialog and constrained body keeps actions outside scrolling content', () => {
  const source = readFileSync(new URL('./ChatGptPlanDialog.tsx', import.meta.url), 'utf8');
  const css = readFileSync(new URL('./ChatGptPlan.css', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /className="modal-overlay[^"\n]*"\s+onClick/);
  assert.match(css, /\.modal\.chatgpt-plan-dialog[\s\S]*?max-height:\s*calc\(100vh - 32px\)/);
  assert.match(css, /\.chatgpt-plan-dialog \.modal-body\s*\{[^}]*min-height:\s*0;[^}]*overflow-y:\s*auto/);
  assert.match(css, /\.chatgpt-plan-overlay\s*\{[^}]*overflow-y:\s*auto;[^}]*align-items:\s*safe center/);
});

test('account discovery errors use the existing in-dialog error area without dismissing the authorization flow', () => {
  const ctx = harness({}, null, 'user@example.com', undefined, 'chatgptPlan.error.storage');
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.storage');
  assert.equal(ctx.tree().some((node) => node.props.role === 'dialog'), true);
  assert.equal(Boolean(ctx.button('codex.oauth.timeoutRetry')), true);
  assert.equal(ctx.calls.closed, 0);
  ctx.hook.unmount();
});


test('embedded authorization has one host dialog and explains official authorization and API-only use before login', () => {
  const ctx = harness({}, null, undefined, undefined, undefined, true);
  assert.equal(ctx.tree().some((node) => node.props.role === 'dialog'), false);
  assert.equal(ctx.tree().some((node) => node.props.className === 'modal-header'), false);
  assert.equal(ctx.tree().some((node) => Array.isArray(node.props.children) && node.props.children.includes('chatgptPlan.oauthSummary')), true);
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.oauthLimits'), true);
  assert.equal(ctx.tree().some((node) => node.props.className === 'modal-footer'), true);
  assert.equal(ctx.calls.opened.length, 0);
  ctx.hook.unmount();
});


test('initial authorization prepares a link without opening a browser, and each browser action uses that session', async () => {
  const incognito: string[] = [];
  const ctx = harness({ async openChatGptPlanIncognitoWindow(id: string) { incognito.push(id); } }, null);
  await settlePromises();
  assert.equal(ctx.calls.opened.length, 0);
  assert.equal(ctx.tree().some((node) => node.type === 'input' && node.props.readOnly && node.props.value === 'https://auth.openai.com/'), true);
  ctx.button('common.shared.oauth.incognitoWindow').props.onClick();
  await settlePromises();
  assert.deepEqual(incognito, ['default']);
  assert.equal(ctx.calls.opened.length, 0);
  ctx.button('common.shared.oauth.openBrowser').props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.opened, ['https://auth.openai.com/']);
  ctx.hook.unmount();
});

test('invalid manual callbacks stay near the field, retry once, and completion remains owned by polling', async () => {
  const callback = deferred<void>();
  const submitted: unknown[] = [];
  let accepted = false;
  const ctx = harness({
    chatGptPlanErrorKey: () => 'chatgptPlan.error.callback',
    submitChatGptPlanCallbackUrl(id: string, url: string) { submitted.push({ id, url }); return accepted ? Promise.resolve() : callback.promise; },
  }, null);
  await settlePromises();
  const field = () => ctx.tree().find((node) => node.type === 'input' && node.props.placeholder === 'chatgptPlan.callbackPlaceholder')!;
  let focused = 0;
  field().props.ref.current = { focus() { focused++; } };
  field().props.onChange({ target: { value: ' http://localhost:1455/auth/callback?code=code&state=state ' } });
  ctx.button('accounts.oauth.continue').props.onClick();
  ctx.button('accounts.oauth.continue').props.onClick();
  assert.equal(submitted.length, 1);
  callback.reject(new Error('invalid state'));
  await settlePromises();
  assert.equal(field().props['aria-invalid'], true);
  assert.equal(focused, 1);
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, '');
  assert.equal(ctx.calls.closed, 0);
  field().props.onChange({ target: { value: 'http://localhost:1455/auth/callback?code=other&state=other' } });
  assert.equal(field().props['aria-invalid'], false);
  accepted = true;
  ctx.button('accounts.oauth.continue').props.onClick();
  await settlePromises();
  assert.equal(submitted.length, 2);
  assert.equal(ctx.calls.changed, 0, 'submission itself does not invent a saved account');
  assert.equal(ctx.button('accounts.oauth.continue').props.disabled, true);
  assert.equal(field().props.value, '');
  ctx.hook.unmount();
});


test('same-email unlinked access requires explicit card binding before models or paid verification', async () => {
  const pending = deferred<unknown>();
  const binds: unknown[] = [];
  let modelReads = 0;
  let joins = 0;
  let boundEvents = 0;
  const sourceAccount = { id: 'source', email: account.email };
  const ctx = harness({
    bindChatGptPlanAccount(id: string, source: string) { binds.push({ id, source }); return pending.promise; },
    async listChatGptPlanModels() { modelReads++; return [{ id: 'model' }]; },
    async joinChatGptPlan() { joins++; },
  }, account, undefined, [], undefined, true, { sourceAccount, onBound() { boundEvents++; } });
  await settlePromises();
  assert.equal(modelReads, 0);
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.serviceMode.bindTitle'), true);
  assert.equal(ctx.tree().some((node) => node.type === 'button' && node.props.children === 'chatgptPlan.verifyAndJoin'), false);
  ctx.button('chatgptPlan.serviceMode.bind').props.onClick();
  ctx.button('chatgptPlan.serviceMode.bind').props.onClick();
  assert.deepEqual(binds, [{ id: 'plan', source: 'source' }]);
  pending.resolve({ ...account, source_account_id: 'source', source_binding_valid: true });
  await settlePromises(); ctx.tree(); await settlePromises();
  assert.equal(boundEvents, 1);
  assert.equal(modelReads, 1);
  assert.equal(joins, 0);
  assert.equal(ctx.calls.closed, 0);
  ctx.button('chatgptPlan.verifyAndJoin').props.onClick();
  await settlePromises();
  assert.equal(joins, 1);
  assert.deepEqual(ctx.calls.applied, [[['source', 'chatgpt_plan_plan'], ['chatgpt_plan_plan']]]);
  ctx.hook.unmount();
});

test('invalid binding metadata does not bypass explicit confirmation and binding failure does not change membership', async () => {
  let bindCalls = 0;
  let reads = 0;
  const ctx = harness({
    async bindChatGptPlanAccount() { bindCalls++; throw new Error('source changed'); },
    async listChatGptPlanModels() { reads++; return []; },
  }, { ...account, source_account_id: 'source', source_binding_valid: false, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }, undefined, [], undefined, true, {
    sourceAccount: { id: 'source', email: account.email },
  });
  assert.equal(reads, 0);
  ctx.button('chatgptPlan.serviceMode.bind').props.onClick();
  await settlePromises();
  assert.equal(bindCalls, 1);
  assert.equal(ctx.calls.changed, 0);
  assert.equal(ctx.calls.closed, 0);
  assert.equal(Boolean(ctx.button('chatgptPlan.serviceMode.bind')), true);
  ctx.hook.unmount();
});

test('binding a different email or another card is blocked before invoking the backend', () => {
  let binds = 0;
  for (const candidate of [{ ...account, email: 'different@example.com' }, { ...account, source_account_id: 'different-card' }]) {
    const ctx = harness({ async bindChatGptPlanAccount() { binds++; return candidate; } }, candidate, undefined, [], undefined, true, {
      sourceAccount: { id: 'source', email: account.email },
    });
    assert.equal(ctx.button('chatgptPlan.serviceMode.bind').props.disabled, true);
    ctx.button('chatgptPlan.serviceMode.bind').props.onClick();
    assert.equal(binds, 0);
    ctx.hook.unmount();
  }
});


test('official join waits for refreshed parent state before closing and never closes a later dialog after unmount', async () => {
  for (const unmount of [false, true]) {
    const refresh = deferred<void>();
    let joins = 0;
    let completed = 0;
    const ctx = harness({ async joinChatGptPlan() { joins++; } }, {
      ...account, source_account_id: 'source', source_binding_valid: true,
    }, undefined, [], undefined, true, {
      sourceAccount: { id: 'source', email: account.email },
      onChanged: () => refresh.promise,
      onJoined() { completed++; },
    });
    await settlePromises();
    ctx.button('chatgptPlan.verifyAndJoin').props.onClick();
    await settlePromises();
    assert.equal(joins, 1);
    assert.equal(completed, 0);
    assert.equal(ctx.tree().some((node) => node.type === 'button' && node.props.children === 'chatgptPlan.verifyAndJoin'), false);
    if (unmount) ctx.hook.unmount();
    refresh.resolve();
    await settlePromises();
    assert.equal(completed, unmount ? 0 : 1);
    if (!unmount) ctx.hook.unmount();
  }
});


test('an invalid linked registration can be unlinked explicitly while preserving credentials and waiting for refreshed state', async () => {
  const refresh = deferred<void>();
  const unlinks: string[] = [];
  const ctx = harness({
    async unlinkChatGptPlanAccount(id: string) { unlinks.push(id); return { ...account, source_account_id: null, source_binding_valid: false }; },
  }, { ...account, source_account_id: 'missing-source', source_binding_valid: false }, undefined, [], undefined, true, {
    sourceAccount: { id: 'missing-source', email: account.email }, onChanged: () => refresh.promise,
  });
  ctx.button('chatgptPlan.serviceMode.unlink').props.onClick();
  assert.equal(ctx.tree().some((node) => node.props.children === 'chatgptPlan.serviceMode.unlinkHint'), true);
  assert.deepEqual(unlinks, []);
  ctx.button('common.cancel').props.onClick();
  assert.deepEqual(unlinks, []);
  ctx.button('chatgptPlan.serviceMode.unlink').props.onClick();
  ctx.button('chatgptPlan.serviceMode.unlink').props.onClick();
  await settlePromises();
  assert.deepEqual(unlinks, ['plan']);
  assert.equal(ctx.calls.closed, 0);
  assert.equal(ctx.calls.applied.length, 0, 'unlink never silently applies Codex access');
  refresh.resolve();
  await settlePromises();
  assert.equal(ctx.calls.closed, 1);
  ctx.hook.unmount();
});

test('asynchronous refresh failures after remove and disconnect are handled inside the dialog', async () => {
  for (const operation of ['remove', 'disconnect']) {
    const ctx = harness({
      async removeChatGptPlanFromService() {},
      async disconnectChatGptPlan() { return { remote_revoked: false }; },
    }, { ...account, in_service: true }, undefined, undefined, undefined, false, {
      async onChanged() { throw new Error('refresh failed'); },
    });
    await settlePromises();
    ctx.button(`chatgptPlan.${operation}`).props.onClick();
    if (operation === 'disconnect') ctx.button('chatgptPlan.disconnect').props.onClick();
    await settlePromises();
    assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.generic');
    assert.equal(ctx.calls.closed, 0);
    if (operation === 'disconnect') assert.equal(Boolean(ctx.button('common.reauthorize')), true);
    else assert.equal(ctx.tree().some((node) => node.type === 'button' && node.props.children === 'chatgptPlan.remove'), false);
    ctx.hook.unmount();
  }
});

test('background completion refresh rejection is caught without losing the saved authorization identity', async () => {
  const ctx = harness({ async pollChatGptPlanLogin() { return { status: 'complete', account }; } }, null, undefined, undefined, undefined, false, {
    async onChanged() { throw new Error('refresh failed'); },
  });
  await settlePromises(); ctx.tree(); await settlePromises();
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.generic');
  assert.equal(Boolean(ctx.button('chatgptPlan.saveOnly')), true);
  assert.equal(ctx.calls.closed, 0);
  ctx.hook.unmount();
});
