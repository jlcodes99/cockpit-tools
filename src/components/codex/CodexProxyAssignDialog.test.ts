import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import type { ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import type { CodexUnifiedProxyPreview } from '../../services/codexUnifiedProxyService';
import type { CodexAccount } from '../../types/codex';
import type { ProxyAssignment } from './CodexProxyAssignDialog';

function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}
const source: ProxyCatalogSource = {
  id: 'source', name: 'Subscription', kind: 'subscription', revision: '1', updatedAt: 0,
  lastAttemptAt: null, autoUpdate: false, error: null, default: null, defaultInvalidated: false,
  nodes: ['one', 'two'].map((id) => ({ id, name: id, supported: true, protocol: 'http', error: null })),
  groups: [{ id: 'select', name: 'select', kind: 'select', supported: true, error: null, members: ['one', 'two'] }],
};
const previewValue = (itemId: string) => ({ binding: { itemId }, eligibleAccountIds: ['account'], independentAccountIds: [] } as unknown as CodexUnifiedProxyPreview);

function harness(initial: Partial<ProxyAssignment> = {}) {
  const accounts = [{ id: 'account', email: 'test@example.invalid' } as CodexAccount];
  const previews: { args: any[]; pending: ReturnType<typeof deferred<CodexUnifiedProxyPreview>> }[] = [];
  const bindings: any[][] = [];
  const applied: any[][] = [];
  let preflights = 0;
  let closed = 0;
  let failure = false;
  const h = loadHookModule(new URL('./CodexProxyAssignDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => (account: CodexAccount) => account.email },
    './useCodexProxyWorkspace': {},
    './CodexProxyWorkspaceContext': { useCodexProxyWorkspace: () => ({ accounts, reloadUnified() {}, acceptUnified() {} }) },
    './CodexProxyPicker': { CodexProxyPicker: 'picker' },
    './useProxyLatency': { useProxyLatency: () => ({ results: {}, running: false, cancel() {} }) },
    '../ModalErrorMessage': { ModalErrorMessage: 'error-message' },
    '../../utils/codexPreferences': { getCodexPlanBadgeStyle: () => 'default', CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT: 'badge' },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: { getState: () => ({ applyAccountSnapshot() {} }) } },
    '../../services/codexProxyEngineService': { preflightCodexProxyEngine: async () => { preflights++; if (failure) throw new Error('engine failed'); } },
    '../../services/codexProxyCatalogService': {
      bindProxyCatalog: async (...args: any[]) => { bindings.push(args); return accounts[0]; },
      catalogErrorKey: () => 'codex.proxy.catalog.failed',
    },
    '../../services/codexUnifiedProxyService': {
      previewCodexUnifiedProxy: (...args: any[]) => { const pending = deferred<CodexUnifiedProxyPreview>(); previews.push({ args, pending }); return pending.promise; },
      applyCodexUnifiedProxy: async (...args: any[]) => { applied.push(args); return {}; },
      unifiedProxyErrorKey: () => 'codex.proxy.catalog.failed',
    },
  }, { window: { addEventListener() {}, removeEventListener() {} }, document: { body: {} } });
  const choice = { sourceId: source.id, itemId: '', groupId: '', selections: {}, catalog: { sources: [source] }, ...initial };
  h.render(() => h.exports.CodexProxyAssignDialog({ choice, onClose() { closed++; } }));
  const tree = () => elements(h.flush());
  const picker = () => tree().find((node) => node.type === 'picker')!.props;
  const confirm = () => tree().find((node) => node.type === 'footer').props.children[1].props;
  const selectAccount = () => tree().find((node) => node.props.className?.startsWith('btn codex-proxy-assign-account')).props.onClick();
  const shared = () => tree().filter((node) => node.props.className?.startsWith('btn codex-proxy-assign-scope'))[1].props.onClick();
  return { h, tree, picker, confirm, selectAccount, shared, previews, bindings, applied,
    preflights: () => preflights, closed: () => closed, fail: (value: boolean) => { failure = value; } };
}

test('unselected subscriptions open locally and cannot apply until a node or complete manual group is selected', async () => {
  const h = harness();
  h.selectAccount();
  assert.equal(h.confirm().disabled, true);
  h.confirm().onClick(); await settlePromises();
  assert.equal(h.preflights(), 0);
  assert.equal(h.bindings.length, 0);
  h.picker().choose('select', 'select');
  assert.equal(h.confirm().disabled, true);
  h.picker().chooseMember('select', 'two');
  assert.equal(h.confirm().disabled, false);
  h.confirm().onClick(); await settlePromises();
  assert.deepEqual(JSON.parse(JSON.stringify(h.bindings)), [['account', 'source', 'select', { select: 'two' }, 'select']]);
  assert.equal(h.closed(), 1);
  h.h.unmount();
});

test('prefilled subscription nodes remain selected and closing never changes account bindings', () => {
  const h = harness({ itemId: 'two' });
  assert.equal(h.picker().itemId, 'two');
  assert.equal(h.preflights(), 0);
  assert.equal(h.previews.length, 0);
  assert.equal(h.bindings.length, 0);
  h.tree().find((node) => node.type === 'header').props.children[1].props.onClick();
  assert.equal(h.closed(), 1);
  h.h.unmount();
});

test('shared preview requires a complete selection and ignores responses for previously selected nodes', async () => {
  const h = harness();
  h.shared(); h.h.flush();
  assert.equal(h.previews.length, 0);
  h.picker().choose('one', ''); h.h.flush();
  assert.equal(h.previews.length, 1);
  assert.equal(h.confirm().disabled, true);
  h.picker().choose('two', ''); h.h.flush();
  assert.equal(h.previews.length, 2);
  h.previews[0].pending.resolve(previewValue('one')); await settlePromises();
  assert.equal(h.confirm().disabled, true);
  h.previews[1].pending.resolve(previewValue('two')); await settlePromises();
  assert.equal(h.confirm().disabled, false);
  h.confirm().onClick(); await settlePromises();
  assert.equal(h.applied[0][1], 'two');
  assert.equal(h.closed(), 1);
  h.h.unmount();
});

test('clearing the selection invalidates a pending shared preview and prevents applying it', async () => {
  const h = harness({ itemId: 'one' });
  h.shared(); h.h.flush();
  h.picker().choose('', ''); h.h.flush();
  h.previews[0].pending.resolve(previewValue('one')); await settlePromises();
  assert.equal(h.confirm().disabled, true);
  h.confirm().onClick(); await settlePromises();
  assert.equal(h.applied.length, 0);
  assert.equal(h.preflights(), 0);
  h.h.unmount();
});

test('apply failures remain inside the open dialog and changing the node clears old errors before retry', async () => {
  const h = harness({ itemId: 'one' });
  h.selectAccount(); h.fail(true);
  h.confirm().onClick(); await settlePromises();
  assert.equal(h.closed(), 0);
  assert.equal(h.tree().some((node) => node.type === 'error-message' && node.props.message === 'codex.proxy.catalog.failed'), true);
  h.picker().choose('two', '');
  assert.equal(h.tree().some((node) => node.type === 'error-message' && node.props.message), false);
  h.fail(false); h.confirm().onClick(); await settlePromises();
  assert.equal(h.bindings[0][2], 'two');
  assert.equal(h.closed(), 1);
  h.h.unmount();
});
