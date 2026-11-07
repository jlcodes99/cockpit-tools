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
const source = { id: 'codex-source', email: 'user@example.com', auth_mode: 'chatgpt' };
const grant = { id: 'grant', label: 'Plan', email: 'USER@example.com', connected: true, needs_reauth: false, in_service: false, source_binding_valid: false };
function harness(props: Record<string, unknown> = {}, save: (...args: unknown[]) => Promise<unknown> = async () => ({})) {
  const error = { message: '', scrollKey: 0, clear() { this.message = ''; }, report(message: string) { this.message = message; this.scrollKey++; } };
  const calls = { saved: [] as unknown[][], changed: 0, closed: 0, applied: [] as unknown[][] };
  const hook = loadHookModule(new URL('./CodexAccountServiceDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage', useModalErrorState: () => error },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
    './ChatGptPlanDialog': { ChatGptPlanDialog: 'PlanDialog' },
    '../../utils/codexAccountServiceNavigation': { announceCodexServiceAuthorizationApplied(...args: unknown[]) { calls.applied.push(JSON.parse(JSON.stringify(args))); } },
    '../../services/chatgptPlanService': {
      withChatGptPlanMembership,
      chatGptPlanErrorKey: () => 'chatgptPlan.error.sourceIdentityUnknown',
      setChatGptPlanServiceMode: (...args: unknown[]) => { calls.saved.push(args); return save(...args); },
    },
  }, { document: { body: {} }, window: new EventTarget() });
  hook.render(() => hook.exports.CodexAccountServiceDialog({ sourceAccount: source, memberAccountIds: [], registrations: [], onChanged() { calls.changed++; }, onClose() { calls.closed++; }, ...props }));
  const tree = () => nodes(hook.flush());
  const button = (key: string) => tree().find((node) => node.type === 'button' && (node.props.children === key || Array.isArray(node.props.children) && node.props.children.includes(key)))!;
  const dialog = () => tree().find((node) => node.type === 'PlanDialog');
  return { hook, tree, button, dialog, calls, error };
}

test('new members default to Codex access and cancelling never saves or verifies', () => {
  const ctx = harness({ registrations: [grant] });
  assert.equal(ctx.button('chatgptPlan.serviceMode.codex').props['aria-selected'], true);
  assert.equal(ctx.dialog(), undefined);
  ctx.button('common.cancel').props.onClick();
  assert.equal(ctx.calls.closed, 1);
  assert.deepEqual(ctx.calls.saved, []);
  ctx.hook.unmount();
});

test('Codex access is an explicit deduplicated save and a failure keeps the dialog open', async () => {
  const saved = deferred<unknown>();
  const ctx = harness({}, () => saved.promise);
  ctx.button('chatgptPlan.serviceMode.applyCodex').props.onClick();
  ctx.button('chatgptPlan.serviceMode.applyCodex').props.onClick();
  assert.deepEqual(ctx.calls.saved, [['codex-source', 'codex']]);
  saved.reject(new Error('identity unknown'));
  await settlePromises();
  assert.equal(ctx.tree().find((node) => node.type === 'ErrorMessage')!.props.message, 'chatgptPlan.error.sourceIdentityUnknown');
  assert.equal(ctx.calls.closed, 0);
  assert.equal(ctx.calls.changed, 0);
  assert.equal(ctx.button('chatgptPlan.serviceMode.applyCodex').props.disabled, false);
  ctx.hook.unmount();
});

test('official access requires explicit selection even for a sole same-email unlinked grant', () => {
  const ctx = harness({ registrations: [grant, { ...grant, id: 'other-card', source_account_id: 'someone-else' }, { ...grant, id: 'other-email', email: 'other@example.com' }] });
  ctx.button('chatgptPlan.serviceMode.official').props.onClick();
  assert.equal(ctx.dialog(), undefined);
  const picker = ctx.tree().find((node) => node.type === 'Dropdown')!;
  assert.deepEqual(picker.props.options.map((item: any) => item.value), ['grant']);
  picker.props.onChange('grant');
  assert.equal(ctx.dialog()!.props.account.id, 'grant');
  assert.equal(ctx.dialog()!.props.sourceAccount.id, 'codex-source');
  assert.equal(ctx.dialog()!.props.embedded, true);
  assert.deepEqual(ctx.calls.saved, []);
  ctx.dialog()!.props.onJoined();
  assert.equal(ctx.calls.closed, 1);
  ctx.hook.unmount();
});

test('valid linked membership selects official management while invalid links require an explicit choice', () => {
  const linked = { ...grant, source_account_id: source.id, source_binding_valid: true };
  const ctx = harness({ registrations: [linked], memberAccountIds: ['chatgpt_plan_grant'] });
  assert.equal(ctx.button('chatgptPlan.serviceMode.official').props['aria-selected'], true);
  assert.equal(ctx.dialog()!.props.account.in_service, true);
  assert.deepEqual(ctx.calls.saved, []);
  ctx.hook.unmount();
  const invalid = harness({ registrations: [{ ...linked, source_binding_valid: false }], memberAccountIds: ['chatgpt_plan_grant'], initialMode: 'official' });
  assert.equal(invalid.dialog(), undefined);
  assert.equal(invalid.tree().some((node) => node.type === 'Dropdown'), true);
  invalid.hook.unmount();
});

test('new official authorization uses source ID, and a confirmed binding stays in the dialog without saving service mode', () => {
  const ctx = harness({ initialMode: 'official' });
  ctx.button('chatgptPlan.serviceMode.newAuthorization').props.onClick();
  assert.equal(ctx.dialog()!.props.account, undefined);
  assert.equal(ctx.dialog()!.props.sourceAccount.id, 'codex-source');
  ctx.dialog()!.props.onBound({ ...grant, source_account_id: source.id, source_binding_valid: true });
  assert.equal(ctx.dialog()!.props.account.source_account_id, 'codex-source');
  assert.equal(ctx.calls.closed, 0);
  assert.deepEqual(ctx.calls.saved, []);
  ctx.hook.unmount();
});

test('service choice uses one constrained shell with a scrolling body and explicit close controls', () => {
  const sourceText = readFileSync(new URL('./CodexAccountServiceDialog.tsx', import.meta.url), 'utf8');
  const css = readFileSync(new URL('./CodexAccountServiceDialog.css', import.meta.url), 'utf8');
  assert.doesNotMatch(sourceText, /className="modal-overlay[^"\n]*"\s+onClick/);
  assert.match(css, /\.modal\.codex-account-service-dialog[^}]*max-height:\s*calc\(100vh - 32px\)/);
  assert.match(css, /\.codex-account-service-dialog > \.modal-body[^}]*min-height:\s*0;[^}]*overflow-y:\s*auto/);
  assert.match(css, /\.codex-account-service-dialog > \.modal-footer[^}]*flex:\s*0 0 auto/);
});


test('successful service changes wait for the parent membership refresh before closing', async () => {
  const refresh = deferred<void>();
  let refreshed = 0;
  const ctx = harness({ onChanged() { refreshed++; return refresh.promise; } });
  ctx.button('chatgptPlan.serviceMode.applyCodex').props.onClick();
  await settlePromises();
  assert.equal(refreshed, 1);
  assert.equal(ctx.calls.closed, 0);
  assert.equal(ctx.button('chatgptPlan.serviceMode.applyCodex').props.disabled, true);
  refresh.resolve();
  await settlePromises();
  assert.equal(ctx.calls.closed, 1);
  ctx.hook.unmount();
});


test('explicitly keeping Codex mode announces exactly the related persisted selection after refresh', async () => {
  const ctx = harness({ registrations: [{ ...grant, source_account_id: source.id, source_binding_valid: true }] }, async () => ({ collection: { accountIds: [source.id, 'unrelated'] } }));
  ctx.button('chatgptPlan.serviceMode.applyCodex').props.onClick();
  await settlePromises();
  assert.deepEqual(ctx.calls.applied, [[['codex-source', 'chatgpt_plan_grant'], ['codex-source']]]);
  assert.equal(ctx.calls.closed, 1);
  ctx.hook.unmount();
});
