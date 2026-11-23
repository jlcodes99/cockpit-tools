import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}
function dialog(windows = true) {
  const noop = () => {};
  const editor = { account: { id: 'fixture' }, sourceId: '', itemId: '', groupId: '', selections: {},
    clientDefaults: false, proxyDisabled: false, bound: false, select: noop, clearError: noop };
  const imports: Record<string, unknown> = {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/windowsOperationError': { isWindowsRuntime: () => windows },
    '../../utils/codexProxyPresentation': { proxySummary: () => '' },
    '../../utils/codexProxySelection': {},
    '../../utils/codexProxyPickerModel': { proxySourceInspectable: () => true },
    '../../utils/codexProxyBatch': {},
    '../../utils/codexProxyDraft': { unifiedProxyActive: () => false },
    '../../services/codexAccountProxyService': {},
    '../../services/codexProxyTargetService': { isCodexProxyTarget: () => false },
    '../../stores/useCodexAccountStore': {},
    '../../hooks/useEscClose': { useEscCloseTopmost: noop },
    '../../hooks/useModalScrollLock': { useModalScrollLock: noop },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: noop },
    './useProxyLatency': { useProxyLatency: () => ({}) },
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => () => 'Account', useCodexProxyExitEditor: () => editor },
    './CodexProxyWorkspaceContext': { useCodexProxyWorkspace: () => ({ catalog: { sources: [] }, unified: {} }) },
    './CodexProxyRuntimeStatus': { useCodexProxyRuntimeStatus: () => ({}) },
  };
  for (const [path, name] of [
    ['../SingleSelectDropdown', 'SingleSelectDropdown'], ['../ModalErrorMessage', 'ModalErrorMessage'],
    ['./CodexProxyPicker', 'CodexProxyPicker'], ['./CodexProxyEnvEditor', 'CodexProxyEnvEditor'],
    ['./CodexProxyCountryGuard', 'CodexProxyCountryGuard'], ['./CodexProxyRecentRequests', 'CodexProxyRecentRequests'],
    ['./CodexProxyActivityPanel', 'CodexProxyActivityPanel'],
  ]) imports[path] = { [name]: name };
  const h = loadHookModule(new URL('./CodexProxyAccountDialog.tsx', import.meta.url), imports, { document: { body: {} } });
  h.render(() => h.exports.CodexProxyAccountDialog({ accountId: 'fixture', initialTab: 'edit', onClose: noop, onApplied: noop }));
  const tree = () => elements(h.flush());
  return { h, tree, modes: () => tree().filter(node => node.props.role === 'radio') };
}
test('client ENV mounts only for the selected original-client mode and disappears on every other mode', () => {
  const d = dialog();
  const visible = () => d.tree().some(node => node.type === 'CodexProxyEnvEditor');
  assert.equal(visible(), false);
  assert.equal(d.modes().length, 4);
  d.modes()[2].props.onClick();
  assert.equal(visible(), true);
  for (const index of [0, 1, 3]) {
    d.modes()[index].props.onClick(); assert.equal(visible(), false);
    d.modes()[2].props.onClick(); assert.equal(visible(), true);
  }
  d.h.unmount();
});
test('Windows launch policy is not offered as an unsupported desktop feature on other platforms', () => {
  const d = dialog(false);
  assert.equal(d.modes().length, 3);
  assert.equal(d.tree().some(node => node.type === 'CodexProxyCountryGuard'), false);
  assert.equal(d.tree().some(node => node.type === 'CodexProxyEnvEditor'), false);
  d.h.unmount();
});

test('account editors use distinct sibling identities so switching modes removes the ENV editor', () => {
  const d = dialog();
  d.modes()[2].props.onClick();
  const env = d.tree().find(node => node.type === 'CodexProxyEnvEditor');
  const country = d.tree().find(node => node.type === 'CodexProxyCountryGuard');
  assert.ok(env && country);
  assert.notEqual(env.key, country.key);
  d.modes()[0].props.onClick();
  assert.equal(d.tree().some(node => node.type === 'CodexProxyEnvEditor'), false);
  assert.equal(d.tree().find(node => node.type === 'CodexProxyCountryGuard').key, country.key);
  d.h.unmount();
});
