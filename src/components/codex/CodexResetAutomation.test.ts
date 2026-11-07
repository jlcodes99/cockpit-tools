import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, deferred, settlePromises } from '../../../tests/helpers/reactHookHarness';
function nodes(tree: any): any[] {
  if (!tree || typeof tree !== 'object') return [];
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return [tree, ...nodes(tree.props?.children)];
}
function setup(loadState: any = {}, component: 'dialog' | 'status' | 'section' = 'dialog') {
  const baseRule = { enabled: false, expiry_enabled: true, before_seconds: 3600, quota_enabled: true, remaining_percent: 5 };
  let state: any = loadState === null ? null : { revision: 0, public_rule: baseRule, accounts: {}, records: [], ...loadState };
  const saveTask = deferred<any>();
  const errors = { message: null as string | null, scrollKey: 0, clear() { this.message = null; }, report(value: string) { this.message = value; this.scrollKey++; } };
  const calls = { public: [] as any[], account: [] as any[], close: 0, settings: 0 };
  let hook: any;
  const t = (key: string, args?: any) => args ? `${key}:${JSON.stringify(args)}` : key;
  const react = new Proxy({}, { get(_target, name) { return name === 'useSyncExternalStore' ? (_sub: any, get: any) => get() : hook.hooks[name]; } });
  hook = loadHookModule(new URL('./CodexResetAutomation.tsx', import.meta.url), {
    react, 'react-dom': { createPortal: (node: any) => node },
    'react-i18next': { useTranslation: () => ({ t }) },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'Dropdown' },
    '../ModalErrorMessage': { ModalErrorMessage: 'ModalError', useModalErrorState: () => errors },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../services/codexResetAutomationService': {
      defaultResetRule: baseRule, getResetAutomationSnapshot: () => state,
      subscribeResetAutomation: () => () => {}, loadResetAutomation: async () => state,
      resolveResetAutomation: (s: any, id: string) => {
        const account = s?.accounts[id], mode = account?.mode ?? 'inherit';
        const rule = account?.mode === 'custom' ? account.rule : s?.public_rule;
        return { mode, rule, enabled: Boolean(s && mode !== 'off' && rule?.enabled && (rule.expiry_enabled || rule.quota_enabled)) };
      },
      prepareResetRuleDraft: (rule: any) => rule,
      validateResetRule: (rule: any) => rule.enabled && rule.expiry_enabled && !(rule.before_seconds > 0) ? 'invalidTime' : null,
      saveResetPublicRule(...args: any[]) { calls.public.push(args); return saveTask.promise; },
      saveResetAccountRule(...args: any[]) { calls.account.push(args); return saveTask.promise; },
    },
  }, { document: { body: {} } });
  hook.render(() => {
    if (component === 'status') return hook.exports.CodexResetAutomationStatus({ accountId: 'a' });
    if (component === 'section') return hook.exports.CodexResetAutomationSection({ accountId: 'a', onSettings: () => calls.settings++ });
    const root = hook.exports.CodexResetAutomationDialog({ accountId: 'a', accountLabel: 'masked account', onClose: () => calls.close++ });
    const editor = nodes(root).find(n => n.type?.name === 'RuleEditor');
    const fields = editor?.type(editor.props);
    const preview = nodes(fields).find(n => n.type?.name === 'RulePreview');
    return [root, fields, preview?.type(preview.props)];
  });
  const tree = () => nodes(hook.flush());
  const button = (label: string) => tree().find(n => n.type === 'button' && (n.props.children === label || Array.isArray(n.props.children) && n.props.children.includes(label)));
  const editor = () => tree().find(n => n.type?.name === 'RuleEditor');
  return { hook, calls, errors, tree, button, editor, baseRule, saveTask, setState: (next: any) => { state = next; } };
}
test('saving public settings changes only that scope and returns to credit details', async () => {
  const original = { enabled: true, before_seconds: 600 };
  const h = setup({ accounts: { a: { mode: 'custom', rule: original } } });
  h.editor().props.onChange({ ...h.baseRule, before_seconds: 900 });
  h.button('codex.resetAutomation.publicTab').props.onClick();
  h.editor().props.onChange({ ...h.baseRule, enabled: true });
  h.button('common.save').props.onClick(); h.button('common.save').props.onClick();
  assert.equal(h.calls.public.length, 1); assert.equal(h.calls.account.length, 0);
  assert.equal(h.calls.public[0][1], h.baseRule);
  assert.equal(h.calls.public[0][2], false); assert.equal(h.calls.public[0][3], undefined);
  h.saveTask.resolve({ revision: 1, public_rule: { ...h.baseRule, enabled: true }, accounts: { a: { mode: 'custom', rule: original } }, records: [] });
  await settlePromises(); assert.equal(h.calls.close, 1); h.hook.unmount();
});
test('tab changes preserve unsaved custom and public drafts independently', () => {
  const h = setup({ accounts: { a: { mode: 'custom', rule: { enabled: true, before_seconds: 600 } } } });
  h.editor().props.onChange({ ...h.baseRule, before_seconds: 900 });
  h.button('codex.resetAutomation.publicTab').props.onClick();
  h.editor().props.onChange({ ...h.baseRule, before_seconds: 1800 });
  h.button('codex.resetAutomation.accountTab').props.onClick();
  assert.equal(h.editor().props.rule.before_seconds, 900);
  h.button('codex.resetAutomation.publicTab').props.onClick();
  assert.equal(h.editor().props.rule.before_seconds, 1800); h.hook.unmount();
});
test('all three switches and both numeric fields stay visible while disabled', () => {
  const h = setup();
  const switches = () => h.tree().filter(n => n.props?.role === 'switch');
  const inputs = () => h.tree().filter(n => n.type === 'input');
  assert.equal(switches().length, 3); assert.equal(inputs().length, 2);
  assert.deepEqual(switches().map(n => n.props.disabled), [false, true, true]);
  assert.ok(inputs().every(n => n.props.disabled));
  switches()[0].props.onClick();
  assert.ok(inputs().every(n => !n.props.disabled));
  switches()[1].props.onClick(); switches()[2].props.onClick();
  assert.equal(inputs().length, 2); assert.ok(inputs().every(n => n.props.disabled));
  assert.equal(h.editor().props.rule.before_seconds, 3600);
  assert.equal(h.editor().props.rule.remaining_percent, 5);
  assert.ok(h.tree().some(n => n.props?.children === 'codex.resetAutomation.previewEmpty'));
  h.button('common.save').props.onClick(); assert.equal(h.calls.account.length, 1);
  h.hook.unmount();
});
test('an unconfigured account directly edits the public default and saves its own custom rule', () => {
  const rule = { enabled: true, expiry_enabled: true, quota_enabled: true, before_seconds: 1800, remaining_percent: 20 };
  const h = setup({ public_rule: rule });
  assert.ok(h.tree().filter(n => n.props?.role === 'switch').every(n => !n.props.disabled));
  assert.equal(h.tree().filter(n => n.type === 'Dropdown').length, 1);
  assert.equal(h.editor().props.rule.before_seconds, 1800);
  h.editor().props.onChange({ ...rule, remaining_percent: 25 });
  h.button('common.save').props.onClick();
  assert.deepEqual(JSON.parse(JSON.stringify(h.calls.account[0])), ['a', { mode: 'custom', rule: { ...rule, remaining_percent: 25 } }, { mode: 'inherit' }]);
  assert.equal(h.calls.public.length, 0); h.hook.unmount();
});
test('save for all is a public-only save option, independent of the master switch', async () => {
  const accounts = { a: { mode: 'off' }, b: { mode: 'custom', rule: { enabled: true, before_seconds: 600 } } };
  const h = setup({ accounts });
  const scope = () => h.tree().find(n => n.props?.role === 'switch' && n.props['aria-label'] === 'codex.resetAutomation.applyAll');
  assert.equal(scope(), undefined);
  h.button('codex.resetAutomation.publicTab').props.onClick();
  assert.equal(scope().props['aria-checked'], false); assert.equal(scope().props.disabled, false);
  scope().props.onClick();
  h.button('common.save').props.onClick(); h.button('common.save').props.onClick();
  assert.equal(h.calls.public.length, 1);
  assert.deepEqual(h.calls.public[0], [h.baseRule, h.baseRule, true, accounts]);
  h.saveTask.reject(new Error('conflict')); await settlePromises();
  assert.equal(h.calls.close, 0); assert.equal(scope().props['aria-checked'], true);
  h.button('codex.resetAutomation.accountTab').props.onClick();
  h.button('common.save').props.onClick();
  assert.equal(h.calls.account.length, 1); assert.equal(h.calls.account[0][1].mode, 'custom');
  await settlePromises(); h.hook.unmount();
});
test('turning a custom rule off retains fields; errors stay inside the settings dialog', async () => {
  const h = setup({ accounts: { a: { mode: 'custom', rule: { enabled: true, expiry_enabled: true, before_seconds: 600, quota_enabled: true, remaining_percent: 20 } } } });
  h.tree().find(n => n.props?.role === 'switch').props.onClick();
  h.button('common.save').props.onClick();
  assert.equal(h.calls.account[0][1].mode, 'custom');
  assert.equal(h.calls.account[0][1].rule.enabled, false);
  assert.equal(h.calls.account[0][1].rule.before_seconds, 600);
  assert.equal(h.calls.account[0][1].rule.remaining_percent, 20);
  h.saveTask.reject(new Error('save failed')); await settlePromises();
  assert.equal(h.calls.close, 0); assert.ok(h.errors.message?.startsWith('codex.resetAutomation.saveFailed'));
  assert.equal(h.tree().find(n => n.type === 'ModalError').props.message, h.errors.message);
  h.button('codex.resetAutomation.publicTab').props.onClick(); assert.equal(h.errors.message, null);
  assert.equal(h.tree()[0].props.onClick, undefined); h.hook.unmount();
});
test('legacy Off can be enabled explicitly without silently following a public rule', () => {
  const h = setup({ accounts: { a: { mode: 'off' } }, public_rule: { enabled: true, expiry_enabled: true, quota_enabled: true, before_seconds: 7200, remaining_percent: 15 } });
  assert.equal(h.editor().props.rule.enabled, false);
  h.tree().find(n => n.props?.role === 'switch').props.onClick();
  h.button('common.save').props.onClick();
  assert.equal(h.calls.account[0][1].mode, 'custom'); assert.equal(h.calls.account[0][1].rule.enabled, true);
  assert.equal(h.calls.account[0][2].mode, 'off'); h.hook.unmount();
});
test('invalid time remains by its field; legal threshold zero can be saved', async () => {
  const h = setup();
  h.editor().props.onChange({ ...h.baseRule, enabled: true, before_seconds: 0 }); h.button('common.save').props.onClick();
  assert.equal(h.editor().props.fieldError, 'invalidTime'); assert.equal(h.calls.account.length, 0);
  assert.equal(h.tree().find(n => n.type === 'input').props['aria-invalid'], true);
  h.editor().props.onChange({ ...h.baseRule, before_seconds: 60, remaining_percent: 0 }); h.button('common.save').props.onClick();
  assert.equal(h.calls.account.length, 1);
  h.saveTask.resolve({ revision: 1, public_rule: h.baseRule, accounts: {}, records: [] });
  await settlePromises(); assert.equal(h.calls.close, 1); h.hook.unmount();
});
test('the details section opens settings without redeeming a card; card unknown state stays unlabeled', () => {
  const unknown = setup(null, 'status'); assert.equal(unknown.tree().length, 0); unknown.hook.unmount();
  const h = setup({ public_rule: { enabled: true, expiry_enabled: true, quota_enabled: true, before_seconds: 43200, remaining_percent: 10 } }, 'section');
  h.button('nav.settings').props.onClick();
  assert.equal(h.calls.settings, 1); assert.equal(h.calls.public.length, 0); assert.equal(h.calls.account.length, 0);
  h.hook.unmount();
});
