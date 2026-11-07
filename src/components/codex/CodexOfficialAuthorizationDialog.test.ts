import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

type Node = { type: unknown; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}
const source = { id: 'source', email: 'source@example.com' };
const grant = { id: 'grant', email: source.email, label: 'Source', connected: true, needs_reauth: false, in_service: false };
const bound = { ...grant, source_account_id: source.id, source_binding_valid: true };
const key = (value: string) => `chatgptPlan.officialFlow.${value}`;
function harness(overrides: Record<string, unknown> = {}, registrations: unknown[] = [], intent = 'join', reauthorize = false) {
  const failures = { message: '', scrollKey: 0, clear() { this.message = ''; }, report(message: string) { this.message = message; this.scrollKey++; } };
  const calls = { binds: [] as unknown[][], modes: [] as unknown[][], models: [] as string[], verified: [] as unknown[][], started: [] as unknown[], cancelled: [] as string[], opened: [] as string[], complete: [] as unknown[], applied: [] as unknown[], changed: 0, closed: 0 };
  const timers = new Map<number, { fn: () => void; delay: number }>();
  let timerId = 0;
  const hook = loadHookModule(new URL('./CodexOfficialAuthorizationDialog.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    'react-dom': { createPortal: (value: unknown) => value },
    '@tauri-apps/plugin-opener': { async openUrl(url: string) { calls.opened.push(url); } },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage', useModalErrorState: () => failures },
    '../../utils/reducedMotion': { scrollElementIntoView() {} },
    '../../utils/codexAccountServiceNavigation': { announceCodexServiceAuthorizationApplied(...args: unknown[]) { calls.applied.push(args); } },
    '../../services/chatgptPlanService': {
      chatGptPlanAuthorizationUrl: (url: string) => url,
      chatGptPlanErrorKey: (error: unknown) => String(error).includes('INVALID_CALLBACK') ? 'chatgptPlan.error.callback' : 'chatgptPlan.error.generic',
      async startChatGptPlanLogin(id: unknown) { calls.started.push(id); return { login_id: 'login', authorize_url: 'https://auth.openai.com/authorize', expires_at: Date.now() / 1000 + 300 }; },
      async pollChatGptPlanLogin() { return { status: 'pending' }; },
      async cancelChatGptPlanLogin(id: string) { calls.cancelled.push(id); },
      async bindChatGptPlanAccount(...args: unknown[]) { calls.binds.push(args); return bound; },
      async listChatGptPlanModels(id: string) { calls.models.push(id); return [{ id: 'first' }, { id: 'preferred' }]; },
      async verifyChatGptPlan(...args: unknown[]) { calls.verified.push(args); return { ...bound, verified_at: 42 }; },
      async setChatGptPlanServiceMode(...args: unknown[]) { calls.modes.push(args); return { collection: { accountIds: [`chatgpt_plan_${args[2]}`] } }; },
      async submitChatGptPlanCallbackUrl() {},
      ...overrides,
    },
  }, { document: { body: {} }, window: new EventTarget(), console,
    navigator: { clipboard: { async writeText() {} } },
    setTimeout(fn: () => void, delay: number) { timers.set(++timerId, { fn, delay }); return timerId; },
    clearTimeout(id: number) { timers.delete(id); },
  });
  hook.render(() => hook.exports.CodexOfficialAuthorizationDialog({ sourceAccount: source, registrations, intent, reauthorize, onClose() { calls.closed++; }, onComplete(value: unknown) { calls.complete.push(value); }, onChanged() { calls.changed++; } }));
  const tree = () => nodes(hook.flush());
  const button = (label: string) => tree().find((node) => node.type === 'button' && (node.props.children === label || Array.isArray(node.props.children) && node.props.children.includes(label)))!;
  const close = () => tree().find((node) => node.type === 'button' && node.props.children === 'common.cancel')!.props.onClick();
  return { hook, tree, button, close, failures, calls, timers };
}

test('opening prepares the address and manual callback without opening a browser or generating', async () => {
  const ctx = harness();
  assert.ok(ctx.tree().some((node) => node.props.role === 'dialog'));
  await settlePromises(); ctx.tree(); await settlePromises();
  assert.deepEqual(ctx.calls.started, [undefined]);
  assert.deepEqual(ctx.calls.opened, []);
  assert.deepEqual(ctx.calls.models, []);
  assert.ok(ctx.tree().some((node) => node.type === 'input' && node.props.value === 'https://auth.openai.com/authorize'));
  assert.ok(ctx.tree().some((node) => node.type === 'input' && node.props.placeholder === key('callbackPlaceholder')));
  assert.ok(ctx.button(key('openAuthorization')));
  ctx.hook.unmount();
});

test('valid bound authorization completes binding without any service or model mutation', async () => {
  const ctx = harness({}, [{ ...bound, verified_model: 'preferred' }]);
  const action = ctx.button(key('verifyAndBind'));
  action.props.onClick(); action.props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.started, []);
  assert.deepEqual(ctx.calls.binds, []);
  assert.deepEqual(ctx.calls.modes, []);
  assert.deepEqual(ctx.calls.verified, []);
  assert.deepEqual(ctx.calls.models, [], 'joining never fetches or auto-selects a model');
  assert.equal(ctx.calls.complete.length, 1);
  assert.equal(ctx.calls.applied.length, 0);
  ctx.hook.unmount();
});

test('same-email saved authorization is explicitly associated and backend rejection cannot bill or switch', async () => {
  const ctx = harness({ async bindChatGptPlanAccount() { throw new Error('CHATGPT_PLAN_SOURCE_EMAIL_MISMATCH'); } }, [grant]);
  assert.deepEqual(ctx.calls.binds, []);
  assert.deepEqual(ctx.calls.models, []);
  ctx.button(key('associate')).props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.models, []);
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 0);
  assert.equal(ctx.failures.message, 'chatgptPlan.error.generic');
  assert.equal(ctx.calls.closed, 0);
  ctx.hook.unmount();
});

test('bind-only saves the validated association without any model or service request', async () => {
  const ctx = harness({}, [grant], 'bind');
  ctx.button(key('associate')).props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.binds, [['grant', 'source']]);
  assert.deepEqual(ctx.calls.verified, []);
  assert.deepEqual(ctx.calls.models, []);
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 1);
  ctx.hook.unmount();
});

test('closing during binding suppresses every later model and service action', async () => {
  const binding = deferred<unknown>();
  const ctx = harness({ bindChatGptPlanAccount: () => binding.promise }, [grant]);
  ctx.button(key('associate')).props.onClick();
  ctx.close();
  binding.resolve(bound);
  await settlePromises();
  assert.deepEqual(ctx.calls.models, []);
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 0);
  assert.equal(ctx.calls.closed, 1);
  ctx.hook.unmount();
});

test('late login after close or preparation timeout is cancelled without opening browser', async () => {
  for (const timeout of [false, true]) {
    const login = deferred<unknown>();
    const ctx = harness({ startChatGptPlanLogin: () => login.promise });
    ctx.button(key('openAuthorization')).props.onClick();
    if (timeout) {
      [...ctx.timers.values()].find((timer) => timer.delay === 30_000)!.fn();
      await settlePromises();
      assert.equal(ctx.failures.message, 'chatgptPlan.error.generic');
    } else ctx.close();
    login.resolve({ login_id: 'late', authorize_url: 'https://auth.openai.com/authorize', expires_at: Date.now() / 1000 + 60 });
    await settlePromises();
    assert.deepEqual(ctx.calls.cancelled, ['late']);
    assert.deepEqual(ctx.calls.opened, []);
    ctx.hook.unmount();
  }
});

test('callback field errors stay in the dialog and accepted callback does not invent authorization', async () => {
  let attempts = 0;
  const ctx = harness({ async submitChatGptPlanCallbackUrl() { attempts++; throw new Error('CHATGPT_PLAN_INVALID_CALLBACK'); } });
  ctx.button(key('openAuthorization')).props.onClick();
  await settlePromises(); ctx.tree(); await settlePromises();
  const field = () => ctx.tree().find((node) => node.type === 'input' && node.props.placeholder === key('callbackPlaceholder'))!;
  field().props.onChange({ target: { value: 'invalid callback' } });
  ctx.button(key('submitCallback')).props.onClick();
  await settlePromises();
  assert.equal(attempts, 1);
  assert.equal(field().props['aria-invalid'], true);
  assert.equal(ctx.failures.message, '');
  assert.equal(ctx.calls.complete.length, 0);
  assert.equal(ctx.calls.closed, 0);
  ctx.hook.unmount();
});

test('completed login binds to the explicit source without joining or generating', async () => {
  const ctx = harness({ async pollChatGptPlanLogin() { return { status: 'complete', account: grant }; } });
  ctx.button(key('openAuthorization')).props.onClick();
  await settlePromises(); ctx.tree(); await settlePromises(); ctx.tree();
  assert.deepEqual(ctx.calls.binds, [['grant', 'source']]);
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 1);
  ctx.hook.unmount();
});

test('binding does not depend on service membership or call service mutations', async () => {
  const ctx = harness({ setChatGptPlanServiceMode() { throw new Error('must not run'); } }, [bound]);
  ctx.button(key('verifyAndBind')).props.onClick();
  await settlePromises();
  assert.equal(ctx.calls.complete.length, 1);
  assert.deepEqual(ctx.calls.modes, []);
  assert.deepEqual(ctx.calls.applied, []);
  assert.equal(ctx.failures.message, '');
  ctx.hook.unmount();
});

test('independent dialog has constrained scrolling and never dismisses on overlay click', () => {
  const ctx = harness();
  const overlay = ctx.tree().find((node) => node.props.className === 'modal-overlay codex-official-auth-overlay')!;
  assert.equal(overlay.props.onClick, undefined);
  assert.equal(ctx.tree().filter((node) => node.props.role === 'dialog').length, 1);
  const css = readFileSync(new URL('./CodexOfficialAuthorizationDialog.css', import.meta.url), 'utf8');
  assert.match(css, /\.modal\.codex-official-auth-dialog\s*\{[^}]*max-height:\s*calc\(100vh - 32px\)/);
  assert.match(css, /\.codex-official-auth-dialog \.modal-body\s*\{[^}]*min-height:\s*0;[^}]*overflow-y:\s*auto/);
  assert.match(css, /\.codex-official-auth-overlay\s*\{[^}]*overflow-y:\s*auto;[^}]*align-items:\s*safe center/);
  ctx.hook.unmount();
});

test('binding timeout restores explicit retry and its late result never joins by itself', async () => {
  const binding = deferred<unknown>();
  const ctx = harness({ bindChatGptPlanAccount: () => binding.promise }, [grant]);
  ctx.button(key('associate')).props.onClick();
  [...ctx.timers.values()].find((timer) => timer.delay === 30_000)!.fn();
  await settlePromises();
  assert.equal(ctx.button(key('associate')).props.disabled, false);
  binding.resolve(bound);
  await settlePromises();
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 0);
  ctx.hook.unmount();
});

test('binding finishing after close only notifies refreshed metadata and never joins', async () => {
  const binding = deferred<unknown>();
  const ctx = harness({ bindChatGptPlanAccount: () => binding.promise }, [grant]);
  ctx.button(key('associate')).props.onClick();
  ctx.close();
  binding.resolve(bound); await settlePromises();
  assert.equal(ctx.calls.changed, 1);
  assert.equal(ctx.calls.complete.length, 0);
  assert.deepEqual(ctx.calls.applied, []);
  assert.deepEqual(ctx.calls.modes, []);
  ctx.hook.unmount();
});

test('an expired login clears the waiting session and leaves explicit reauthorization available', async () => {
  const ctx = harness({ async startChatGptPlanLogin() { return { login_id: 'expired', authorize_url: 'https://auth.openai.com/', expires_at: 1 }; } });
  ctx.button(key('openAuthorization')).props.onClick();
  await settlePromises(); ctx.tree(); await settlePromises();
  assert.deepEqual(ctx.calls.cancelled, ['expired']);
  assert.ok(ctx.button(key('openAuthorization')));
  assert.equal(ctx.failures.message, 'chatgptPlan.error.generic');
  assert.equal(ctx.calls.complete.length, 0);
  ctx.hook.unmount();
});

test('multiple authorizations linked to the source require explicit selection and never select the first silently', async () => {
  const ctx = harness({}, [bound, { ...bound, id: 'second' }, { ...bound, id: 'foreign', source_account_id: 'other-source' }]);
  assert.equal(ctx.button(key('verifyAndBind')), undefined);
  assert.deepEqual(ctx.calls.models, []);
  assert.deepEqual(ctx.calls.modes, []);
  const candidates = ctx.tree().filter((node) => node.props.className === 'official-auth-existing-row');
  assert.equal(candidates.length, 2, 'both same-source authorizations are available, foreign source excluded');
  nodes(candidates[1]).find((node) => node.type === 'button')!.props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.models, []);
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 1);
  ctx.hook.unmount();
});

test('multiple saved authorizations include expired same-source entries for explicit reauthorization', async () => {
  const ctx = harness({}, [bound, { ...bound, id: 'expired-grant', connected: false, needs_reauth: true }]);
  const candidates = ctx.tree().filter((node) => node.props.className === 'official-auth-existing-row');
  assert.equal(candidates.length, 2);
  nodes(candidates[1]).find((node) => node.type === 'button')!.props.onClick();
  assert.deepEqual(ctx.calls.models, []);
  ctx.button(key('openAuthorization')).props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.started, ['expired-grant']);
  ctx.hook.unmount();
});

test('returning a bound account preserves its membership metadata without synthesizing a route', async () => {
  const ctx = harness({}, [{ ...bound, in_service: false }]);
  ctx.button(key('verifyAndBind')).props.onClick(); await settlePromises();
  assert.equal((ctx.calls.complete[0] as any).in_service, false);
  assert.deepEqual(ctx.calls.applied, []);
  assert.deepEqual(ctx.calls.modes, []);
  ctx.hook.unmount();
});

test('retry after binding timeout reuses the unfinished association instead of duplicating it', async () => {
  const binding = deferred<unknown>();
  let binds = 0;
  const ctx = harness({ bindChatGptPlanAccount() { binds++; return binding.promise; } }, [grant]);
  ctx.button(key('associate')).props.onClick();
  [...ctx.timers.values()].find((timer) => timer.delay === 30_000)!.fn();
  await settlePromises();
  ctx.button(key('associate')).props.onClick();
  assert.equal(binds, 1);
  binding.resolve(bound);
  await settlePromises();
  assert.deepEqual(ctx.calls.modes, []);
  assert.equal(ctx.calls.complete.length, 1);
  assert.deepEqual(ctx.calls.verified, []);
  ctx.hook.unmount();
});

test('a completed binding does not restore a paused service or replace last saved mode', async () => {
  const ctx = harness({}, [{ ...bound, in_service: true, last_service_mode: 'codex' }]);
  ctx.button(key('verifyAndBind')).props.onClick(); await settlePromises();
  assert.equal((ctx.calls.complete[0] as any).in_service, true);
  assert.equal((ctx.calls.complete[0] as any).last_service_mode, 'codex');
  assert.deepEqual(ctx.calls.modes, []);
  assert.deepEqual(ctx.calls.applied, []);
  ctx.hook.unmount();
});

test('reauthorization explicitly starts OAuth for the existing registration and never silently reuses it', async () => {
  const ctx = harness({}, [bound], 'bind', true);
  assert.equal(ctx.button(key('verifyAndBind')), undefined);
  ctx.button(key('openAuthorization')).props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.started, ['grant']);
  assert.deepEqual(ctx.calls.models, []);
  assert.deepEqual(ctx.calls.verified, []);
  assert.deepEqual(ctx.calls.modes, []);
  ctx.close();
  assert.deepEqual(ctx.calls.cancelled, ['login']);
  ctx.hook.unmount();
});

test('automatic address preparation is cancelled when closed and late startup cannot reopen a browser', async () => {
  const pending = deferred<unknown>(); let starts = 0;
  const ctx = harness({ startChatGptPlanLogin() { starts++; return pending.promise; } });
  await settlePromises(); assert.equal(starts, 1);
  assert.ok(ctx.tree().some((node) => node.type === 'input' && node.props.placeholder === key('callbackPlaceholder')));
  ctx.close(); pending.resolve({ login_id: 'auto-late', authorize_url: 'https://auth.openai.com/authorize', expires_at: Date.now() / 1000 + 60 });
  await settlePromises(); assert.deepEqual(ctx.calls.cancelled, ['auto-late']); assert.deepEqual(ctx.calls.opened, []);
  assert.deepEqual(ctx.calls.complete, []); ctx.hook.unmount();
});
