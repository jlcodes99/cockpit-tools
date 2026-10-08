import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { proxyErrorKey, proxyEntryRecoveryErrorKey, type CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

function harness(restore = async (_accountId: string) => {}, measure = async (_accountId: string) => {}) {
  const calls = { closed: 0, refreshed: 0, focused: 0, managed: 0, escapeEnabled: false, restored: [] as string[], measured: [] as string[] };
  const frames: (() => void)[] = [];
  const account = { id: 'account', egress_proxy: { sourceId: 'subscription', itemId: 'automatic' } };
  const data = { status: null as CodexProxyRuntimeStatus | null, statusError: false, requestsError: false, history: [], requests: [], loading: false,
    refresh() { calls.refreshed++; } };
  const h = loadHookModule(new URL('./CodexAccountProxyPreview.tsx', import.meta.url), {
    'react-dom': { createPortal: (content: unknown) => content },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexProxyPreview': { proxyRuntimeRows: () => [], proxyPreviewBinding: (summary: unknown) => ({ summary }) },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (select: (state: any) => unknown) => select({ accounts: [account] }) },
    './useCodexProxyPreview': { useCodexProxyPreview: () => data },
    './CodexProxyWorkspaceContext': { CodexProxyWorkspaceProvider: function Provider() { return null; } },
    './CodexProxyAccountDialog': { CodexProxyAccountDialog: function SwitchDialog() { return null; } },
    './CodexProxyDisplayControl': { CodexProxyDisplayControl: () => null },
    '../../services/codexAccountProxyService': { proxyEntryRecoveryErrorKey, proxyErrorKey,
      measureCodexAccountProxyLatency(accountId: string) { calls.measured.push(accountId); return measure(accountId); },
      restoreCodexAccountProxyEntry(accountId: string) { calls.restored.push(accountId); return restore(accountId); } },
    './CodexProxyConnectionSummary': { CodexProxyConnectionSummary: function Summary() { return null; } },
    './CodexProxyRuntimeDetails': { CodexProxyRuntimeDetails: () => null, CodexProxyRuntimePort: () => null },
    './CodexProxyActivityPreview': { CodexProxyActivityPreview: () => null },
    '../ModalErrorMessage': { ModalErrorMessage: function ErrorMessage() { return null; } },
    '../../hooks/useEscClose': { useEscCloseTopmost: (enabled: boolean) => { calls.escapeEnabled = enabled; } },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
  }, { document: { body: {} }, requestAnimationFrame: (callback: () => void) => frames.push(callback) });
  const show = () => h.render(() => h.exports.CodexAccountProxyPreview({ account, displayName: 'masked account',
    onClose() { calls.closed++; }, onManage() { calls.managed++; } }));
  const named = (name: string) => elements(show()).find((value) => value.type?.name === name);
  const overlay = () => elements(show()).find((value) => value.props?.className === 'modal-overlay codex-proxy-preview-overlay');
  const button = (key: string) => elements(show()).find((value) => value.type === 'button' && elements(value).some((node) =>
    Array.isArray(node.props?.children) ? node.props.children.includes(key) : node.props?.children === key));
  return { ...h, show, named, overlay, button, calls, frames, data, account };
}

test('preview edits remain explicit and resource management uses the shared page callback', () => {
  const h = harness();
  assert.equal(h.named('SwitchDialog'), undefined);
  assert.deepEqual(h.calls.measured, []);
  assert.deepEqual(h.calls.restored, []);
  assert.equal(h.calls.managed, 0);
  h.button('codex.proxy.management').props.onClick();
  assert.equal(h.calls.managed, 1);
  for (const [action, mode] of [['onFollow', 'follow'], ['onDisable', 'disabled']] as const) {
    h.named('Summary').props[action]();
    const child = h.named('SwitchDialog');
    assert.equal(child.props.accountId, 'account');
    assert.equal(child.props.initialMode, mode);
    child.props.onResources();
    child.props.onClose();
    assert.equal(h.named('SwitchDialog'), undefined);
  }
  assert.equal(h.calls.managed, 3);
  assert.equal(h.calls.refreshed, 0, 'canceling an edit does not apply a new binding');
  assert.equal(h.calls.closed, 0);
  h.unmount();
});

test('switching opens outside the preview body, with the preview inert until the child closes', () => {
  const h = harness();
  assert.equal(h.named('SwitchDialog'), undefined);
  assert.equal(h.calls.escapeEnabled, true);
  h.named('Summary').props.switchButtonRef.current = { focus() { h.calls.focused++; } };
  h.named('Summary').props.onSwitch();
  const child = h.named('SwitchDialog');
  assert.ok(child);
  assert.equal(child.props.accountId, 'account');
  assert.equal(child.props.initialTab, 'edit');
  assert.equal(child.props.initialMode, 'independent');
  assert.equal(h.overlay().props.inert, true);
  assert.equal(h.overlay().props['aria-hidden'], true);
  assert.equal(h.calls.escapeEnabled, false, 'Escape must not close the preview under the switch dialog');
  assert.ok(!elements(h.overlay()).some((value) => value.type?.name === 'SwitchDialog'), 'the new dialog must not be nested in the inert preview');
  assert.equal(h.overlay().props.onClick, undefined);
  child.props.onClose();
  assert.equal(h.named('SwitchDialog'), undefined);
  assert.equal(h.overlay().props.inert, false);
  assert.equal(h.calls.escapeEnabled, true);
  assert.equal(h.calls.closed, 0); assert.equal(h.calls.refreshed, 0);
  h.frames.forEach((frame) => frame()); assert.equal(h.calls.focused, 1);
  h.unmount();
});

test('saving dismisses only the switch dialog and refreshes the retained preview', () => {
  const h = harness(); h.named('Summary').props.onSwitch();
  h.named('SwitchDialog').props.onApplied();
  h.named('SwitchDialog').props.onClose();
  assert.equal(h.named('SwitchDialog'), undefined);
  assert.ok(h.overlay());
  assert.equal(h.calls.closed, 0); assert.equal(h.calls.refreshed, 1);
  h.unmount();
});

test('relay recovery prevents duplicate submits and waits for an active read before refreshing', async () => {
  const pending = deferred<void>();
  const h = harness(() => pending.promise);
  const restore = h.button('codex.proxy.restoreEntry');
  assert.equal(restore.props.disabled, false);
  restore.props.onClick();
  restore.props.onClick();
  assert.deepEqual(h.calls.restored, ['account']);
  assert.equal(h.button('codex.proxy.restoringEntry').props.disabled, true);
  assert.equal(h.button('common.refresh').props.disabled, true);
  h.data.loading = true;
  pending.resolve();
  await settlePromises(); h.show();
  assert.equal(h.calls.refreshed, 0);
  h.data.loading = false;
  h.show();
  assert.equal(h.calls.refreshed, 1);
  assert.equal(h.calls.closed, 0);
  h.unmount();
});

test('recovery failure stays in the preview and is cleared on retry or mode change', async () => {
  const attempts = [deferred<void>(), deferred<void>()];
  let index = 0;
  const h = harness(() => attempts[index++].promise);
  h.button('codex.proxy.restoreEntry').props.onClick();
  attempts[0].reject('PROXY_ENTRY_PORT_UNAVAILABLE');
  await settlePromises();
  assert.equal(h.named('ErrorMessage').props.message, 'codex.proxy.restoreEntryPortBusy');
  assert.equal(h.calls.closed, 0);
  h.button('codex.proxy.restoreEntry').props.onClick();
  assert.equal(h.named('ErrorMessage').props.message, '');
  attempts[1].reject('PROXY_ENTRY_NOT_RUNNING');
  await settlePromises();
  assert.equal(h.named('ErrorMessage').props.message, 'codex.proxy.restoreEntryNotRunning');
  h.named('Summary').props.onFollow();
  assert.equal(h.named('ErrorMessage').props.message, '');
  h.unmount();
});

test('a listening entry hides recovery and a previous account cannot receive recovery results', async () => {
  const pending = deferred<void>();
  const h = harness(() => pending.promise);
  h.data.status = { desktopEntry: { state: 'listening' } } as CodexProxyRuntimeStatus;
  assert.equal(h.button('codex.proxy.restoreEntry'), undefined);
  h.data.status = { desktopEntry: { state: 'failed' } } as CodexProxyRuntimeStatus;
  h.button('codex.proxy.restoreEntry').props.onClick();
  h.account.id = 'another-account';
  h.show();
  pending.reject('PROXY_ENTRY_PORT_UNAVAILABLE');
  await settlePromises();
  assert.equal(h.named('ErrorMessage').props.message, '');
  assert.equal(h.calls.refreshed, 0);
  h.unmount();
});

test('recovery errors use specific localized messages without exposing raw host errors', () => {
  const codes = {
    PROXY_ENTRY_NOT_RUNNING: 'restoreEntryNotRunning',
    PROXY_ENTRY_PORT_UNAVAILABLE: 'restoreEntryPortBusy',
    PROXY_ENTRY_RECOVERY_BUSY: 'restoreEntryBusy',
    PROXY_ENTRY_RECOVERY_TIMEOUT: 'restoreEntryTimeout',
    PROXY_ENTRY_RECOVERY_FAILED: 'restoreEntryFailed',
  };
  for (const [code, key] of Object.entries(codes)) {
    assert.equal(proxyEntryRecoveryErrorKey(new Error(code)), `codex.proxy.${key}`);
  }
  assert.equal(proxyEntryRecoveryErrorKey('PROXY_ENGINE_MISSING'), 'codex.proxy.engineMissing');
  assert.equal(proxyEntryRecoveryErrorKey('untrusted error details'), 'codex.proxy.restoreEntryFailed');
});

test('latency checks run only on explicit click, reject duplicate submits and never block closing', async () => {
  const pending = deferred<void>();
  const h = harness(undefined, () => pending.promise);
  const summary = h.named('Summary');
  assert.deepEqual(h.calls.measured, []);
  summary.props.onMeasure(); summary.props.onMeasure();
  assert.deepEqual(h.calls.measured, ['account']);
  assert.equal(h.named('Summary').props.measuring, true);
  assert.equal(h.button('common.refresh').props.disabled, true);
  h.button('codex.proxy.restoreEntry').props.onClick();
  assert.deepEqual(h.calls.restored, []);
  const close = elements(h.show()).find((node) => node.type === 'button' && node.props['aria-label'] === 'common.close');
  assert.equal(close.props.disabled, undefined);
  assert.equal(h.calls.escapeEnabled, true);
  close.props.onClick();
  assert.equal(h.calls.closed, 1);
  h.unmount();
  pending.reject('PROXY_PROBE_TIMEOUT');
  await settlePromises();
  assert.equal(h.calls.refreshed, 0);
});

test('measurement errors remain in the dialog and clear before retry or switching', async () => {
  const attempts = [deferred<void>(), deferred<void>()];
  let index = 0;
  const h = harness(undefined, () => attempts[index++].promise);
  h.named('Summary').props.onMeasure();
  attempts[0].reject('PROXY_PROBE_TIMEOUT'); await settlePromises();
  assert.equal(h.named('ErrorMessage').props.message, 'codex.proxy.probeTimeout');
  assert.equal(h.calls.closed, 0);
  h.named('Summary').props.onMeasure();
  assert.equal(h.named('ErrorMessage').props.message, '');
  attempts[1].reject('PROXY_BINDING_CHANGED'); await settlePromises();
  assert.equal(h.named('ErrorMessage').props.message, 'codex.proxy.currentLatencyChanged');
  h.named('Summary').props.onFollow();
  assert.equal(h.named('ErrorMessage').props.message, '');
  h.unmount();
});

for (const change of ['account-roundtrip', 'binding']) test(`old measurement cannot clear new loading or report errors after ${change}`, async () => {
  const attempts = [deferred<void>(), deferred<void>()];
  let index = 0;
  const h = harness(undefined, () => attempts[index++].promise);
  h.named('Summary').props.onMeasure();
  if (change === 'account-roundtrip') {
    h.account.id = 'other'; h.show(); h.account.id = 'account';
  } else h.account.egress_proxy.itemId = 'new-route';
  assert.equal(h.named('Summary').props.measuring, false);
  h.named('Summary').props.onMeasure();
  attempts[0].reject('PROXY_PROBE_TIMEOUT'); await settlePromises();
  assert.equal(h.named('ErrorMessage').props.message, '');
  assert.equal(h.named('Summary').props.measuring, true);
  attempts[1].resolve(); await settlePromises();
  assert.equal(h.named('Summary').props.measuring, false);
  h.unmount();
});
