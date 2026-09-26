import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

function harness() {
  const calls = { closed: 0, refreshed: 0, focused: 0, escapeEnabled: false };
  const frames: (() => void)[] = [];
  const account = { id: 'account', egress_proxy: { sourceId: 'subscription', itemId: 'automatic' } };
  const data = { status: null, statusError: false, requestsError: false, history: [], requests: [], loading: false,
    refresh() { calls.refreshed++; } };
  const h = loadHookModule(new URL('./CodexAccountProxyPreview.tsx', import.meta.url), {
    'react-dom': { createPortal: (content: unknown) => content },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexProxyPreview': { proxyRuntimeRows: () => [], proxyPreviewBinding: (summary: unknown) => ({ summary }) },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (select: (state: any) => unknown) => select({ accounts: [account] }) },
    './useCodexProxyPreview': { useCodexProxyPreview: () => data },
    './CodexProxyWorkspaceContext': { CodexProxyWorkspaceProvider: function Provider() { return null; } },
    './CodexProxyQuickSwitch': { CodexProxyQuickSwitch: function SwitchDialog() { return null; } },
    './CodexProxyConnectionSummary': { CodexProxyConnectionSummary: function Summary() { return null; } },
    './CodexProxyRuntimeDetails': { CodexProxyRuntimeDetails: () => null, CodexProxyRuntimePort: () => null },
    './CodexProxyActivityPreview': { CodexProxyActivityPreview: () => null },
    '../ModalErrorMessage': { ModalErrorMessage: () => null },
    '../../hooks/useEscClose': { useEscCloseTopmost: (enabled: boolean) => { calls.escapeEnabled = enabled; } },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
  }, { document: { body: {} }, requestAnimationFrame: (callback: () => void) => frames.push(callback) });
  const show = () => h.render(() => h.exports.CodexAccountProxyPreview({ account, displayName: 'masked account',
    onClose() { calls.closed++; }, onManage() {} }));
  const named = (name: string) => elements(show()).find((value) => value.type?.name === name);
  const overlay = () => elements(show()).find((value) => value.props?.className === 'modal-overlay codex-proxy-preview-overlay');
  return { ...h, show, named, overlay, calls, frames };
}

test('switching opens outside the preview body, with the preview inert until the child closes', () => {
  const h = harness();
  assert.equal(h.named('SwitchDialog'), undefined);
  assert.equal(h.calls.escapeEnabled, true);
  h.named('Summary').props.switchButtonRef.current = { focus() { h.calls.focused++; } };
  h.named('Summary').props.onSwitch();
  const child = h.named('SwitchDialog');
  assert.ok(child);
  assert.equal(child.props.displayName, 'masked account');
  assert.equal(child.props.accountId, 'account');
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
  assert.equal(h.named('SwitchDialog'), undefined);
  assert.ok(h.overlay());
  assert.equal(h.calls.closed, 0); assert.equal(h.calls.refreshed, 1);
  h.unmount();
});
