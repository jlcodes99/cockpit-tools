import * as pathRecovery from '../../stores/useCodexPathRecoveryStore';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { isChatGptPlanServiceModeApplied } from '../../services/chatgptPlanService';
import * as navigation from '../../utils/codexAccountServiceNavigation';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { codexOfficialAuthorizationRows, codexOfficialAuthorizationRelatedIds, needsCodexOfficialAuthorizationReview, codexOfficialAuthorizationState, codexOfficialAuthorizationStateKey } from '../../utils/codexOfficialAuthorization';

const sources = [{ id: 'one', email: 'one@example.com' }, { id: 'two', email: 'two@example.com' }];
const grant = { id: 'grant', source_account_id: 'one', source_binding_valid: true, connected: true, needs_reauth: false, verified_model: 'model', in_service: false };
function nodes(node: any): any[] { return !node ? [] : Array.isArray(node) ? node.flatMap(nodes) : typeof node !== 'object' ? [] : [node, ...nodes(node.props?.children)]; }
function harness(options: { timers?: Record<string, unknown>; state?: () => Promise<any>; grants?: () => Promise<any[]>; apply?: (...args: any[]) => Promise<any>; activePage?: boolean; realNavigation?: boolean; check?: () => Promise<any>; unlink?: () => Promise<any>; disconnect?: () => Promise<any>; release?: (disposition: string) => Promise<any>; deadline?: (operation: Promise<any>, ms?: number) => Promise<any> } = {}) {
  const window = new EventTarget();
  const calls = { read: 0, fetch: 0, applied: [] as any[], notices: [] as any[], managed: [] as any[] };
  let pending: any[] = [];
  let managePending = false;
  let activePage = options.activePage ?? true;
  const error = { message: '', scrollKey: 0, clear() { this.message = ''; }, report(value: string) { this.message = value; this.scrollKey++; } };
  const t = (key: string) => key;
  const hook = loadHookModule(new URL('./CodexAccountServiceHost.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    'react-dom': { createPortal: (node: unknown) => node },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: { getState: () => ({ accounts: sources, fetchAccounts: async () => { calls.fetch++; } }) } },
    '../../services/codexLocalAccessService': { getCodexLocalAccessState: async () => { calls.read++; const state = await (options.state?.() ?? Promise.resolve({ collection: { accountIds: ['one'] } })); return { running: true, ...state, collection: { enabled: true, ...state.collection }, serviceMembers: state.serviceMembers ?? (state.collection?.accountIds ?? []).map((id: string) => ({ sourceAccountId: id === 'chatgpt_plan_grant' ? 'one' : id, accountId: id, mode: state.collection?.officialAccountBindings?.[id] || id.startsWith('chatgpt_plan_') ? 'official' : 'codex', paused: Boolean(state.collection?.pausedAccountIds?.includes(id)) })) }; } },
    '../../services/chatgptPlanService': {
      isChatGptPlanServiceModeApplied,
      listChatGptPlanAccounts: () => options.grants?.() ?? Promise.resolve([]), chatGptPlanErrorKey: () => 'safe-error',
      setChatGptPlanServiceMode: async (...args: any[]) => { calls.applied.push(args.map((arg) => arg && typeof arg === 'object' ? JSON.parse(JSON.stringify(arg)) : arg)); return options.apply?.(...args) ?? { collection: { accountIds: [args[0]], officialAccountBindings: args[1] === 'official' ? { [args[0]]: args[2] } : {} }, serviceMembers: [{ sourceAccountId: args[0], accountId: args[0], mode: args[1], paused: false }] }; },
      checkChatGptPlanAuthorization: async (id: string) => { calls.managed.push(['check', id]); return options.check?.() ?? { ...grant, last_checked_at: 42, check_error: null }; },
      unlinkChatGptPlanAccount: async (id: string) => { calls.managed.push(['unlink', id]); return options.unlink?.() ?? { ...grant, source_account_id: null }; },
      releaseChatGptPlanAuthorization: async (id: string, disposition: string) => { calls.managed.push(['release', id, disposition]); return options.release?.(disposition) ?? { authorization: { ...grant, connected: false, source_account_id: null }, state: { collection: { accountIds: ['one'], pausedAccountIds: disposition === 'pause' ? ['one'] : [] } }, remote_revoked: true }; },
    },
    '../../utils/codexAccountServiceNavigation': options.realNavigation ? navigation : {
      CODEX_ACCOUNT_SERVICE_DRAFT_CANCEL: 'cancel-draft',
      cancelCodexServiceDraftAuthorizations() {}, isCodexServiceDraftRequestActive: () => true, announceCodexServiceDraftResult: () => true,
      requeueCodexServiceAuthorizationRequests: (items: any[]) => { pending = items; window.dispatchEvent(new Event('open-service')); },
      CODEX_ACCOUNT_SERVICE_REQUEST: 'open-service',
      CODEX_OFFICIAL_AUTHORIZATION_MANAGE: 'manage-service',
      takeCodexOfficialAuthorizationManagement: () => { const next = managePending; managePending = false; return next; },
      requestCodexOfficialAuthorizationManagement: () => { managePending = true; window.dispatchEvent(new Event('manage-service')); },
      takeCodexServiceAuthorizationRequests: () => { const next = pending; pending = []; return next; },
      requestCodexServiceAuthorizations: (ids: string[]) => { pending = ids.map((accountId) => ({ accountId })); window.dispatchEvent(new Event('open-service')); },
      announceCodexServiceAuthorizationApplied: (...args: any[]) => calls.notices.push(JSON.parse(JSON.stringify(args))),
    },
    '../../utils/codexOfficialAuthorization': { codexOfficialAuthorizationRows, codexOfficialAuthorizationRelatedIds, needsCodexOfficialAuthorizationReview, codexOfficialAuthorizationState, codexOfficialAuthorizationStateKey, codexOfficialAuthorizationDeadline: (operation: Promise<any>, ms?: number) => options.deadline?.(operation, ms) ?? operation },
    './CodexOfficialManagementDialog': { CodexOfficialManagementDialog: 'Management' },
    './CodexOfficialAuthorizationDialog': { CodexOfficialAuthorizationDialog: 'Authorization' },
    './CodexOfficialBadge': { CodexOfficialBadge: 'Badge' },
    '../ModalErrorMessage': { ModalErrorMessage: 'Error', useModalErrorState: () => error },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
  }, { window, document: { body: {} }, Event, console, ...options.timers });
  const management = loadHookModule(new URL('./CodexOfficialManagementDialog.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) }, 'react-dom': { createPortal: (value: unknown) => value },
    '../../utils/reducedMotion': { scrollElementIntoView() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' }, './CodexOfficialBadge': { CodexOfficialBadge: 'Badge' },
    '../../utils/codexOfficialAuthorization': { codexOfficialAuthorizationState, codexOfficialAuthorizationStateKey },
    '../../services/chatgptPlanService': { chatGptPlanErrorKey: () => 'safe-error' },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} }, '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
  }, { window, document: { body: {} } });
  const parentFlush = hook.flush;
  hook.flush = () => {
    const tree = parentFlush();
    return tree?.type === 'Management' ? management.render(() => management.exports.CodexOfficialManagementDialog(tree.props)) : tree;
  };
  const parentUnmount = hook.unmount; hook.unmount = () => { parentUnmount(); management.unmount(); };
  hook.render(() => hook.exports.CodexAccountServiceHost({ activePage }));
  const click = (key: string, index = 0) => {
    const button = nodes(hook.flush()).filter((node) => node.type === 'button' && (!node.props.disabled || key.endsWith('officialTab')) && (node.props['aria-label'] === key || node.props.children === key
      || Array.isArray(node.props.children) && node.props.children.includes(key)
      || nodes(node.props.children).some((child) => child.props?.children === key)))[index];
    if (!button && ['chatgptPlan.officialFlow.officialTab', 'chatgptPlan.officialFlow.localTab'].includes(key)) {
      const dropdown = nodes(hook.flush()).find((node) => node.type === 'Dropdown');
      if (dropdown) { dropdown.props.onChange(key.endsWith('officialTab') ? 'official' : 'codex'); hook.flush(); return; }
    }
    assert.ok(button, `Missing button ${key}`);
    button.props.onClick(); hook.flush();
  };
  return { hook, calls, error, click, window,
    manage() { managePending = true; window.dispatchEvent(new Event('manage-service')); return hook.flush(); },
    open(...ids: string[]) { pending = ids.map((accountId) => ({ accountId })); window.dispatchEvent(new Event('open-service')); return hook.flush(); },
    inspect() { window.dispatchEvent(new Event('codex-official-authorization-check')); return hook.flush(); },
    activate() { activePage = true; return hook.flush(); },
    hide() { activePage = false; return hook.flush(); },
  };
}

test('host does no startup I/O; a batch is one list and cancel clears it', async () => {
  const ctx = harness();
  assert.equal(ctx.calls.read, 0);
  ctx.open('one', 'two'); await settlePromises();
  assert.equal(nodes(ctx.hook.flush()).filter((node) => node.type === 'li').length, 2);
  ctx.click('common.cancel');
  assert.equal(ctx.hook.flush(), null);
  assert.equal(ctx.calls.applied.length, 0);
  ctx.hook.unmount();
});

test('legacy startup events and page changes never read accounts or open authorization', async () => {
  for (const state of [
    { collection: { accountIds: ['one', 'two'] } },
    { collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } },
    { running: false, collection: { accountIds: ['one'] } },
  ]) {
    const ctx = harness({ state: async () => state, grants: async () => [{ ...grant, needs_reauth: true }], activePage: false });
    ctx.inspect(); ctx.activate(); ctx.inspect(); ctx.hide(); ctx.activate(); await settlePromises();
    assert.equal(ctx.hook.flush(), null);
    assert.equal(ctx.calls.read, 0);
    assert.deepEqual(ctx.calls.applied, []);
    ctx.hook.unmount();
  }
});

test('manual management remains available without reminders or preference controls', async () => {
  const ctx = harness({ state: async () => ({ running: false, collection: { accountIds: ['one', 'two'] } }) });
  ctx.manage(); await settlePromises();
  const tree = nodes(ctx.hook.flush());
  assert.equal(tree.filter((node) => node.type === 'li').length, 2);
  assert.equal(tree.some((node) => node.props.className === 'codex-official-reminder-preference'), false);
  assert.equal(tree.some((node) => node.type === 'input' && node.props.type === 'checkbox'), false);
  assert.deepEqual(ctx.calls.applied, []);
  ctx.click('common.close'); assert.equal(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('a revoked authorization is available for manual management without an automatic prompt', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }), grants: async () => [{ ...grant, needs_reauth: true }] });
  ctx.inspect(); await settlePromises(); assert.equal(ctx.hook.flush(), null);
  ctx.manage(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.authorizeAgain');
  assert.equal(ctx.hook.flush().props.sourceAccount.id, 'one');
  assert.equal(ctx.hook.flush().props.intent, undefined);
  ctx.hook.unmount();
});

test('an explicit official choice immediately reuses existing authorization without reopening OAuth', async () => {
  const ctx = harness({ grants: async () => [{ ...grant, verified_at: undefined, verified_model: undefined }] });
  ctx.open('one'); await settlePromises(); ctx.hook.flush();
  assert.deepEqual(ctx.calls.applied, []);
  ctx.click('chatgptPlan.officialFlow.useOfficial');
  assert.notEqual(ctx.hook.flush()?.type, 'Authorization');
  await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['one', 'official', 'grant', { intent: 'join' }]]);
  assert.deepEqual(ctx.calls.notices[0], [['one'], ['one']]);
  assert.equal(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('a pending connected binding does not interrupt startup and explicit reuse delegates validation to the backend', async () => {
  const pending = { ...grant, source_binding_valid: false, source_binding_error: null };
  const ctx = harness({ grants: async () => [pending], state: async () => ({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }) });
  ctx.inspect(); await settlePromises();
  assert.equal(ctx.hook.flush(), null);
  ctx.open('one'); await settlePromises();
  ctx.click('chatgptPlan.officialFlow.authorizeJoin'); await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['one', 'official', 'grant', { intent: 'join' }]]);
  assert.equal(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('declining authorization joins only the selected source through the local gateway', async () => {
  const ctx = harness();
  ctx.open('one', 'two'); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.localTab');
  ctx.click('chatgptPlan.officialFlow.useLocal', 1); await settlePromises();
  assert.equal(ctx.calls.applied[0][0], 'two');
  assert.equal(ctx.calls.applied[0][1], 'codex');
  assert.notEqual(ctx.hook.flush(), null, 'remaining batch stays in the same list');
  ctx.hook.unmount();
});

test('authorization for an existing member only binds; an explicit join failure stays in the dialog', async () => {
  const ctx = harness({ apply: async () => { throw new Error('private upstream details'); } });
  ctx.manage(); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.authorize');
  assert.equal(ctx.hook.flush().props.intent, undefined);
  ctx.hook.flush().props.onClose(); ctx.hook.flush();
  ctx.open('one'); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.localTab');
  ctx.click('chatgptPlan.officialFlow.useLocal'); await settlePromises();
  assert.equal(ctx.error.message, 'safe-error');
  assert.notEqual(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('closing metadata loading fences late results and startup events do not load metadata', async () => {
  const state = deferred<any>();
  const ctx = harness({ state: () => state.promise });
  ctx.open('one'); ctx.click('common.cancel');
  state.resolve({ collection: { accountIds: [] } }); await settlePromises();
  assert.equal(ctx.hook.flush(), null);
  ctx.hook.unmount();
  const slow = deferred<any>();
  const startup = harness({ state: () => slow.promise });
  startup.inspect(); slow.resolve({ collection: { accountIds: [] } }); await settlePromises();
  assert.equal(startup.hook.flush(), null);
  startup.hook.unmount();
});

test('a closed confirmed local join still publishes the backend member fact when it completes late', async () => {
  const mutation = deferred<any>();
  const ctx = harness({ grants: async () => [grant], apply: () => mutation.promise });
  ctx.open('one'); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.localTab'); ctx.click('chatgptPlan.officialFlow.useLocal');
  ctx.click('common.cancel');
  mutation.resolve({ collection: { accountIds: ['one'] } }); await settlePromises();
  assert.deepEqual(ctx.calls.notices[0], [['one'], ['one']]);
  assert.equal(ctx.hook.flush(), null, 'late backend fact does not reopen UI');
  ctx.hook.unmount();
});

test('a request arriving during a mutation waits then opens without a stuck busy state', async () => {
  const mutation = deferred<any>();
  const ctx = harness({ grants: async () => [grant], apply: () => mutation.promise });
  ctx.open('one'); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.localTab'); ctx.click('chatgptPlan.officialFlow.useLocal');
  assert.ok(nodes(ctx.hook.flush()).filter((node) => node.props.role === 'tab').every((node) => node.props.disabled));
  ctx.click('chatgptPlan.officialFlow.officialTab');
  assert.equal(nodes(ctx.hook.flush()).find((node) => node.props.id === 'codex-service-tab-codex')!.props['aria-selected'], true);
  ctx.open('two');
  assert.equal(ctx.calls.applied.length, 1);
  mutation.resolve({ collection: { accountIds: ['one'] } });
  await settlePromises(); ctx.hook.flush(); await settlePromises();
  const rows = nodes(ctx.hook.flush()).filter((node) => node.type === 'li');
  assert.equal(rows.length, 1);
  assert.equal(rows[0].key, 'two');
  const action = nodes(rows[0]).find((node) => node.type === 'button' && node.props.children === 'chatgptPlan.officialFlow.authorizeJoin');
  assert.equal(action.props.disabled, false);
  assert.equal(ctx.calls.applied.length, 1, 'completed account is not re-verified');
  ctx.hook.unmount();
});

test('a binding saved before validation failure is refreshed when returning to the guide', async () => {
  let grants: any[] = [];
  const ctx = harness({ grants: async () => grants });
  ctx.open('one'); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.authorizeJoin');
  grants = [{ ...grant, verified_model: undefined }];
  await ctx.hook.flush().props.onChanged(); ctx.hook.flush();
  ctx.hook.flush().props.onClose(); await settlePromises(); ctx.hook.flush();
  ctx.click('chatgptPlan.officialFlow.useOfficial');
  await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['one', 'official', 'grant', { intent: 'join' }]]);
  assert.equal(ctx.hook.flush(), null);
  ctx.hook.unmount();
});


test('starting or restarting the API service cannot request authorization', async () => {
  const previous = globalThis.window;
  const ctx = harness({ activePage: false, realNavigation: true });
  Object.assign(globalThis, { window: ctx.window });
  try {
    const service: Record<string, any> = {};
    const compiled = ts.transpileModule(readFileSync(new URL('../../services/codexLocalAccessService.ts', import.meta.url), 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText;
    vm.runInNewContext(compiled, { exports: service, performance, console: { info() {} }, require(name: string) {
      if (name === '@tauri-apps/api/core') return { invoke: async () => ({ running: true, collection: { enabled: true, accountIds: ['one'] } }) };
      if (name === '../stores/useCodexPathRecoveryStore') return pathRecovery;
      throw new Error(name);
    } });
    await service.activateCodexLocalAccess('default');
    await service.restartCodexLocalAccessSidecar();
    await service.setCodexLocalAccessEnabled(true);
    ctx.activate(); await settlePromises();
    assert.equal(ctx.hook.flush(), null);
    assert.equal(ctx.calls.read, 0);
    assert.deepEqual(ctx.calls.applied, []);
    navigation.requestCodexOfficialAuthorizationManagement(); await settlePromises();
    assert.notEqual(ctx.hook.flush(), null, 'the explicit management entry still opens');
  } finally { ctx.hook.unmount(); Object.assign(globalThis, { window: previous }); }
});

test('tabs show only their own explanation and action, clear errors and never mutate membership', async () => {
  const ctx = harness();
  ctx.open('one'); await settlePromises();
  const labels = () => nodes(ctx.hook.flush()).filter((node) => typeof node.props.children === 'string').map((node) => node.props.children);
  assert.ok(labels().includes('chatgptPlan.officialFlow.benefit'));
  assert.ok(!labels().includes('chatgptPlan.officialFlow.useLocal'));
  ctx.error.report('old failure');
  ctx.click('chatgptPlan.officialFlow.localTab');
  assert.equal(ctx.error.message, '');
  assert.ok(labels().includes('chatgptPlan.officialFlow.localBenefit'));
  assert.ok(!labels().includes('chatgptPlan.officialFlow.authorizeJoin'));
  assert.ok(!labels().includes('chatgptPlan.officialFlow.notAuthorized'));
  ctx.click('chatgptPlan.officialFlow.officialTab');
  assert.ok(labels().includes('chatgptPlan.officialFlow.authorizeJoin'));
  assert.deepEqual(ctx.calls.applied, []);
  ctx.click('common.cancel'); ctx.open('two'); await settlePromises();
  assert.equal(nodes(ctx.hook.flush()).find((node) => node.props.id === 'codex-service-tab-official')!.props['aria-selected'], true);
  ctx.hook.unmount();
});

test('access tabs are usable during slow metadata loading and support keyboard selection', async () => {
  const state = deferred<any>();
  const ctx = harness({ state: () => state.promise });
  ctx.open('one');
  const tab = nodes(ctx.hook.flush()).find((node) => node.props.id === 'codex-service-tab-official')!;
  let focused = ''; let prevented = false;
  tab.props.onKeyDown({ key: 'Home', preventDefault() { prevented = true; }, currentTarget: { parentElement: { querySelector(selector: string) { return { focus() { focused = selector; } }; } } } });
  assert.equal(prevented, true);
  assert.equal(focused, '#codex-service-tab-codex');
  assert.equal(nodes(ctx.hook.flush()).find((node) => node.props.id === 'codex-service-tab-codex')!.props.tabIndex, 0);
  assert.deepEqual(ctx.calls.applied, []);
  ctx.click('common.cancel');
  state.resolve({ collection: { accountIds: [] } }); await settlePromises();
  assert.equal(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('the access dialog keeps tabs and explicit close controls outside its constrained scrolling body', () => {
  const source = readFileSync(new URL('./CodexAccountServiceHost.tsx', import.meta.url), 'utf8');
  const css = readFileSync(new URL('./CodexAccountServiceHost.css', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /className="modal-overlay[^"\n]*"\s+onClick/);
  assert.match(css, /\.modal\.codex-official-guide\s*\{[^}]*max-height:\s*calc\(100vh - 32px\)/);
  assert.match(css, /\.codex-official-guide \.modal-body\s*\{[^}]*overflow-y:\s*auto;[^}]*min-height:\s*0/);
  assert.match(css, /\.codex-official-mode-tabs\s*\{[^}]*flex:\s*0 0 auto/);
  assert.match(css, /@media \(max-height: 600px\)/);
});

test('management checks authorization without generation and network errors do not mark it revoked', async () => {
  for (const check_error of [null, 'CHATGPT_PLAN_NETWORK', 'CHATGPT_PLAN_CONFIGURATION']) {
    const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }),
      grants: async () => [grant], check: async () => ({ ...grant, last_checked_at: 42, check_error }) });
    ctx.manage(); await settlePromises();
    ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.checkAuthorization'); await settlePromises();
    const tree = nodes(ctx.hook.flush());
    assert.deepEqual(ctx.calls.managed, [['check', 'grant']]);
    assert.deepEqual(ctx.calls.applied, []);
    assert.equal(tree.find((node) => node.type === 'Badge')!.props.state, 'used');
    assert.equal(ctx.error.message, check_error ? 'chatgptPlan.officialFlow.checkFailed' : '');
    assert.ok(tree.some((node) => node.props.className === 'codex-official-checked-at'));
    assert.notEqual(ctx.hook.flush().type, 'Authorization');
    ctx.hook.unmount();
  }
});

test('a confirmed invalid authorization is red and reauthorization opens a fresh OAuth flow', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }), grants: async () => [grant],
    check: async () => ({ ...grant, needs_reauth: true, check_error: 'CHATGPT_PLAN_REAUTH_REQUIRED', last_checked_at: 42 }) });
  ctx.manage(); await settlePromises();
  ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.checkAuthorization'); await settlePromises();
  assert.equal(nodes(ctx.hook.flush()).find((node) => node.type === 'Badge')!.props.state, 'reauthorize');
  ctx.click('chatgptPlan.officialFlow.authorizeAgain');
  assert.equal(ctx.hook.flush().type, 'Authorization');
  assert.equal(ctx.hook.flush().props.reauthorize, true);
  assert.equal(ctx.hook.flush().props.intent, undefined);
  assert.deepEqual(ctx.calls.applied, []);
  ctx.hook.unmount();
});

test('release offers local or pause, commits only on confirmation and preserves the member', async () => {
  for (const disposition of ['local_gateway', 'pause']) {
    const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }), grants: async () => [grant] });
    ctx.manage(); await settlePromises();
    ctx.click('chatgptPlan.officialFlow.manageAccount');
    ctx.click('chatgptPlan.officialFlow.releaseAuthorization');
    assert.deepEqual(ctx.calls.managed, []);
    assert.equal(nodes(ctx.hook.flush()).filter((node) => node.props.role === 'dialog').length, 1);
    ctx.click('common.cancel');
    assert.notEqual(ctx.hook.flush(), null);
    ctx.click('chatgptPlan.officialFlow.releaseAuthorization');
    if (disposition === 'pause') ctx.click('chatgptPlan.officialFlow.releasePause');
    ctx.click('chatgptPlan.officialFlow.releaseAuthorization'); await settlePromises();
    assert.deepEqual(ctx.calls.managed, [['release', 'grant', disposition]]);
    assert.deepEqual(ctx.calls.notices[ctx.calls.notices.length - 1], [['one'], ['one']]);
    assert.deepEqual(ctx.calls.applied, [], 'release is a single backend mutation, not a frontend fallback sequence');
    const tree = nodes(ctx.hook.flush());
    assert.equal(tree.some((node) => node.type === 'Badge'), false);
    assert.equal(tree.some((node) => node.props.children === 'codex.localAccess.accountPoolHealth.paused'), disposition === 'pause');
    ctx.hook.unmount();
  }
});

test('release reports an unconfirmed remote revoke without removing the paused member', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }), grants: async () => [grant],
    release: async () => ({ authorization: { ...grant, connected: false, source_account_id: null }, state: { collection: { accountIds: ['one'], pausedAccountIds: ['one'] } }, remote_revoked: false }) });
  ctx.manage(); await settlePromises();
  ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.releaseAuthorization');
  ctx.click('chatgptPlan.officialFlow.releasePause');
  ctx.click('chatgptPlan.officialFlow.releaseAuthorization'); await settlePromises();
  const tree = nodes(ctx.hook.flush());
  assert.deepEqual(ctx.calls.managed, [['release', 'grant', 'pause']]);
  assert.ok(tree.some((node) => node.props.className === 'codex-official-notice is-warning' && node.props.children === 'chatgptPlan.localDisconnectedNotice'));
  assert.ok(tree.some((node) => node.props.children === 'codex.localAccess.accountPoolHealth.paused'));
  assert.equal(tree.some((node) => node.type === 'Badge'), false);
  assert.notEqual(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('failed release retains confirmation and closing a slow check fences late UI results', async () => {
  const failed = harness({ grants: async () => [grant], release: async () => { throw new Error('storage failure'); } });
  failed.manage(); await settlePromises(); failed.click('chatgptPlan.officialFlow.manageAccount');
  failed.click('chatgptPlan.officialFlow.releaseAuthorization'); failed.click('chatgptPlan.officialFlow.releaseAuthorization'); await settlePromises();
  assert.equal(failed.error.message, 'safe-error');
  assert.ok(nodes(failed.hook.flush()).some((node) => node.props.className === 'codex-official-confirm'));
  assert.ok(nodes(failed.hook.flush()).some((node) => node.type === 'Badge'));
  failed.hook.unmount();
  const check = deferred<any>();
  const slow = harness({ grants: async () => [grant], check: () => check.promise });
  slow.manage(); await settlePromises(); slow.click('chatgptPlan.officialFlow.manageAccount');
  slow.click('chatgptPlan.officialFlow.checkAuthorization'); slow.click('common.close');
  check.resolve({ ...grant, last_checked_at: 42, check_error: null }); await settlePromises();
  assert.equal(slow.hook.flush(), null);
  slow.hook.unmount();
});

test('a timed-out management check can retry the same unfinished operation without duplicate requests', async () => {
  const check = deferred<any>();
  let waits = 0;
  const ctx = harness({ grants: async () => [grant], check: () => check.promise,
    deadline: (operation, ms) => ms === 45_000 && ++waits === 1 ? Promise.reject(new Error('CHATGPT_PLAN_TIMEOUT')) : operation });
  ctx.manage(); await settlePromises();
  ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.checkAuthorization'); await settlePromises();
  assert.equal(ctx.error.message, 'safe-error');
  assert.deepEqual(ctx.calls.managed, [['check', 'grant']]);
  ctx.click('chatgptPlan.officialFlow.checkAuthorization');
  assert.deepEqual(ctx.calls.managed, [['check', 'grant']]);
  check.resolve({ ...grant, last_checked_at: 42, check_error: null }); await settlePromises();
  assert.equal(ctx.error.message, '');
  assert.ok(nodes(ctx.hook.flush()).some((node) => node.props.children === 'chatgptPlan.officialFlow.checkPassed'));
  assert.deepEqual(ctx.calls.applied, []);
  ctx.hook.unmount();
});


test('unjoined account management only binds and cannot silently add a member', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: [] } }), grants: async () => [grant] });
  ctx.manage(); await settlePromises();
  assert.equal(nodes(ctx.hook.flush()).some((node) => node.props.children === 'chatgptPlan.officialFlow.checkAuthorization'), false);
  ctx.click('chatgptPlan.officialFlow.manageAccount');
  const tree = nodes(ctx.hook.flush());
  assert.ok(tree.some((node) => node.props.children === 'chatgptPlan.officialFlow.notMemberHint'));
  assert.equal(tree.some((node) => node.props.children === 'chatgptPlan.officialFlow.switchAccessMode'), false);
  ctx.click('chatgptPlan.officialFlow.authorizeAgain');
  ctx.hook.flush().props.onComplete(grant); await settlePromises();
  assert.deepEqual(ctx.calls.applied, []);
  ctx.hook.unmount();
});

test('explicit management switches use switch intent and a removed-member failure cannot join it again', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'] } }), grants: async () => [grant],
    apply: async () => { throw new Error('CHATGPT_PLAN_SERVICE_MEMBER_REMOVED'); } });
  ctx.manage(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.officialTab');
  ctx.click('chatgptPlan.officialFlow.switchAccessMode'); await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['one', 'official', 'grant', { intent: 'switch' }]]);
  assert.equal(ctx.error.message, 'safe-error');
  assert.notEqual(ctx.hook.flush(), null);
  ctx.hook.unmount();
});

test('initial join continues after bind once, whereas reauthorization preserves the current member channel', async () => {
  const ctx = harness();
  ctx.open('two'); await settlePromises(); ctx.click('chatgptPlan.officialFlow.authorizeJoin');
  assert.equal(ctx.hook.flush().props.intent, undefined, 'OAuth dialog only binds');
  assert.equal(ctx.hook.flush().props.completionMode, 'service');
  ctx.hook.flush().props.onComplete({ ...grant, source_account_id: 'two' }); await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['two', 'official', 'grant', { intent: 'join' }]]);
  assert.equal(ctx.hook.flush(), null);
  assert.deepEqual(ctx.calls.notices[0], [['two'], ['two']]);
  ctx.hook.unmount();
});

test('paused original member remains manageable after release and only explicit resume uses switch', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], pausedAccountIds: ['one'] },
    serviceMembers: [{ sourceAccountId: 'one', accountId: 'one', mode: 'codex', paused: true }] }) });
  ctx.inspect(); await settlePromises(); assert.equal(ctx.hook.flush(), null);
  ctx.manage(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.manageAccount');
  assert.deepEqual(ctx.calls.applied, []);
  ctx.click('chatgptPlan.officialFlow.resumeAccess'); await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['one', 'codex', undefined, { intent: 'switch' }]]);
  assert.equal(nodes(ctx.hook.flush()).some((node) => node.props.children === 'codex.localAccess.accountPoolHealth.paused'), false);
  ctx.hook.unmount();
});

test('switching to missing official authorization binds first and resumes switch, never join', async () => {
  const ctx = harness({ state: async () => ({ collection: { accountIds: ['one'], pausedAccountIds: ['one'] } }) });
  ctx.manage(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.officialTab'); ctx.click('chatgptPlan.officialFlow.resumeAccess');
  assert.equal(ctx.hook.flush().type, 'Authorization');
  assert.deepEqual(ctx.calls.applied, []);
  ctx.hook.flush().props.onComplete(grant); await settlePromises();
  assert.deepEqual(ctx.calls.applied, [['one', 'official', 'grant', { intent: 'switch' }]]);
  ctx.hook.unmount();
});

test('a local or unjoined release explains unchanged membership and does not offer hidden fallback', async () => {
  for (const members of [[], ['one']]) {
    const ctx = harness({ state: async () => ({ collection: { accountIds: members } }), grants: async () => [grant],
      release: async () => ({ authorization: { ...grant, source_account_id: null, connected: false }, state: { collection: { accountIds: members } }, remote_revoked: true }) });
    ctx.manage(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.manageAccount'); ctx.click('chatgptPlan.officialFlow.releaseAuthorization');
    const tree = nodes(ctx.hook.flush());
    assert.ok(tree.some((node) => node.props.children === 'chatgptPlan.officialFlow.releaseUnchangedHint'));
    assert.equal(tree.some((node) => node.props.className === 'codex-official-release-options'), false);
    ctx.click('chatgptPlan.officialFlow.releaseAuthorization'); await settlePromises();
    assert.deepEqual(ctx.calls.notices[ctx.calls.notices.length - 1], [['one'], members]);
    assert.deepEqual(ctx.calls.applied, []);
    ctx.hook.unmount();
  }
});

test('closing an explicit switch fences late UI while publishing committed members', async () => {
  const mode = deferred<any>();
  const ctx = harness({ grants: async () => [grant], apply: () => mode.promise });
  ctx.manage(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.manageAccount');
  ctx.click('chatgptPlan.officialFlow.officialTab'); ctx.click('chatgptPlan.officialFlow.switchAccessMode');
  ctx.click('common.close'); mode.resolve({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } }); await settlePromises();
  assert.equal(ctx.hook.flush(), null);
  assert.deepEqual(ctx.calls.notices[0], [['one'], ['one']]);
  ctx.hook.unmount();
});

async function draftHarness(options: Parameters<typeof harness>[0] = {}) {
  const previous = globalThis.window;
  const ctx = harness({ ...options, realNavigation: true });
  Object.assign(globalThis, { window: ctx.window });
  const results: any[] = [];
  ctx.window.addEventListener(navigation.CODEX_ACCOUNT_SERVICE_DRAFT_RESULT, (event) => results.push(JSON.parse(JSON.stringify((event as CustomEvent).detail))));
  return { ...ctx, results, dispose() { ctx.hook.unmount(); Object.assign(globalThis, { window: previous }); } };
}

test('a row dropdown draft opens OAuth directly and returns only a source-scoped choice', async () => {
  const ctx = await draftHarness();
  try {
    const requestId = navigation.requestCodexServiceDraftAuthorizations(['two']); ctx.hook.flush(); await settlePromises();
    const dialog = ctx.hook.flush(); assert.equal(dialog.type, 'Authorization'); assert.equal(dialog.props.sourceAccount.id, 'two'); assert.equal(dialog.props.completionMode, 'draft');
    dialog.props.onComplete({ ...grant, source_account_id: 'two' }); await settlePromises();
    assert.deepEqual(ctx.results, [{ requestId, sourceAccountId: 'two', status: 'selected', mode: 'official', registrationId: 'grant' }]);
    assert.equal(ctx.hook.flush(), null); assert.deepEqual(ctx.calls.applied, []); assert.deepEqual(ctx.calls.notices, []);
  } finally { ctx.dispose(); }
});
test('cancelling row OAuth retains the route and parent cancellation fences late completion', async () => {
  for (const parentCancel of [false, true]) {
    const ctx = await draftHarness();
    try {
      const requestId = navigation.requestCodexServiceDraftAuthorizations(['two']); ctx.hook.flush(); await settlePromises();
      const dialog = ctx.hook.flush();
      if (parentCancel) navigation.cancelCodexServiceDraftAuthorizations(requestId); else dialog.props.onClose(); ctx.hook.flush();
      dialog.props.onComplete({ ...grant, source_account_id: 'two' }); await settlePromises();
      assert.deepEqual(ctx.results, [{ requestId, sourceAccountId: 'two', status: 'cancelled' }]);
      assert.equal(ctx.hook.flush(), null); assert.deepEqual(ctx.calls.applied, []);
    } finally { ctx.dispose(); }
  }
});
test('a queued row dropdown keeps draft intent after an immediate join completes', async () => {
  const mutation = deferred<any>(); const ctx = await draftHarness({ grants: async () => [grant], apply: () => mutation.promise });
  try {
    navigation.requestCodexServiceAuthorization('one'); ctx.hook.flush(); await settlePromises(); ctx.click('chatgptPlan.officialFlow.useOfficial');
    const requestId = navigation.requestCodexServiceDraftAuthorizations(['two']); ctx.hook.flush();
    mutation.resolve({ collection: { accountIds: ['one'], officialAccountBindings: { one: 'grant' } } });
    await settlePromises(); ctx.hook.flush(); await settlePromises(); const dialog = ctx.hook.flush(); assert.equal(dialog.type, 'Authorization');
    dialog.props.onComplete({ ...grant, source_account_id: 'two' }); await settlePromises();
    assert.deepEqual(ctx.results, [{ requestId, sourceAccountId: 'two', status: 'selected', mode: 'official', registrationId: 'grant' }]);
    assert.deepEqual(ctx.calls.applied, [['one', 'official', 'grant', { intent: 'join' }]]);
  } finally { ctx.dispose(); }
});


test('open authorization management reconciles a pending binding without an event or a model check', async () => {
  const timers = new Map<number, () => void>(); let id = 0, reads = 0;
  const ctx = harness({ grants: async () => [{ ...grant, source_binding_valid: ++reads > 1 }], timers: {
    setTimeout: (callback: () => void) => { timers.set(++id, callback); return id; }, clearTimeout: (id: number) => timers.delete(id),
  } });
  ctx.manage(); await settlePromises();
  assert.equal(nodes(ctx.hook.flush()).find((node) => node.type === 'Badge')?.props.state, 'pending');
  assert.equal(timers.size, 1);
  const retry = [...timers.values()][0]; timers.clear(); retry(); await settlePromises();
  assert.equal(nodes(ctx.hook.flush()).find((node) => node.type === 'Badge')?.props.state, 'unused');
  assert.equal(reads, 2); assert.equal(timers.size, 0);
  assert.equal(ctx.calls.managed.length, 0); assert.equal(ctx.calls.applied.length, 0);
  ctx.hook.unmount();
});

test('closing management fences a late pending-binding reconciliation', async () => {
  const timers = new Map<number, () => void>(); let id = 0, reads = 0;
  const late = deferred<any[]>();
  const ctx = harness({ grants: () => ++reads === 1 ? Promise.resolve([{ ...grant, source_binding_valid: false }]) : late.promise, timers: {
    setTimeout: (callback: () => void) => { timers.set(++id, callback); return id; }, clearTimeout: (id: number) => timers.delete(id),
  } });
  ctx.manage(); await settlePromises(); ctx.hook.flush();
  const retry = [...timers.values()][0]; assert.ok(retry); timers.clear(); retry();
  ctx.click('common.close'); late.resolve([grant]); await settlePromises();
  assert.equal(ctx.hook.flush(), null); assert.equal(ctx.error.message, '');
  assert.equal(timers.size, 0); assert.equal(ctx.calls.applied.length, 0);
  ctx.hook.unmount();
});
