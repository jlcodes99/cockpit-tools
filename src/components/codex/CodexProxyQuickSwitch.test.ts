import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function harness() {
  const selected: any[] = [];
  const calls = { saved: 0, closed: 0, applied: 0, cancelled: 0 };
  let escape: () => void = () => {};
  const editor = { bound: false, sourceId: '', busy: '', error: '', notice: '', selectionReady: false, saved: false,
    select: (value: unknown) => selected.push(value), save() { calls.saved++; } };
  const binding = { sourceId: 'shared', itemId: 'automatic', groupId: 'automatic', selections: {} };
  const h = loadHookModule(new URL('./CodexProxyQuickSwitch.tsx', import.meta.url), {
    'react-dom': { createPortal: (content: unknown) => content },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    './CodexProxyWorkspaceContext': { useCodexProxyWorkspace: () => ({ catalog: { sources: [{ id: 'manual', name: 'Manual' }] }, catalogLoading: false, catalogError: '', reloadCatalog() {} }) },
    './useCodexProxyExitEditor': { useCodexProxyExitEditor: () => editor },
    './useProxyLatency': { useProxyLatency: () => ({ cancel() { calls.cancelled++; } }) },
    './CodexProxyPicker': { CodexProxyPicker: () => null },
    '../SingleSelectDropdown': { SingleSelectDropdown: function SingleSelectDropdown() { return null; } },
    '../ModalErrorMessage': { ModalErrorMessage: () => null },
    '../../utils/codexProxyDraft': { restoreExitChoice: (_catalog: unknown, value: unknown) => value },
    '../../utils/codexProxySelection': { sourceDefaultDraft: () => undefined },
    '../../hooks/useEscClose': { useEscCloseTopmost: (_enabled: boolean, close: () => void) => { escape = close; } },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
  }, { document: { body: {} } });
  const show = (ready: boolean) => h.render(() => h.exports.CodexProxyQuickSwitch({ accountId: 'account', displayName: 'masked account',
    initialBinding: ready ? binding : null, bindingReady: ready, onApplied() { calls.applied++; }, onClose() { calls.closed++; } }));
  return { ...h, show, selected, binding, calls, editor, escape: () => escape() };
}
function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

test('quick switch waits for the effective binding and restores an automatic group without pinning a leaf', () => {
  const h = harness(); h.show(false); assert.equal(h.selected.length, 0);
  h.show(true); assert.equal(h.selected.length, 1); assert.equal(h.selected[0], h.binding);
  h.show(true); assert.equal(h.selected.length, 1); h.unmount();
});

test('a late effective-binding read cannot overwrite the user’s quick-switch draft', () => {
  const h = harness(); const tree = h.show(false);
  const select = elements(tree).find((value) => value.type?.name === 'SingleSelectDropdown');
  select.props.onChange('manual');
  assert.equal(h.selected[0].sourceId, 'manual');
  h.show(true); assert.equal(h.selected.length, 1); h.unmount();
});

test('switching uses a modal with a separate scroll body and always reachable action footer', () => {
  const h = harness(); const tree = h.show(true);
  assert.equal(tree.props.className, 'modal-overlay codex-proxy-switch-overlay');
  assert.equal(tree.props.onClick, undefined, 'clicking the backdrop must not dismiss the draft');
  const dialog = elements(tree).find((value) => value.props?.role === 'dialog');
  assert.equal(dialog.props['aria-modal'], 'true');
  const title = elements(dialog).find((value) => value.type === 'h2');
  assert.equal(dialog.props['aria-labelledby'], title.props.id);
  assert.equal(title.props.children, 'codex.proxy.quickSwitch');
  const body = elements(dialog).find((value) => value.props?.className === 'modal-body');
  const footer = elements(dialog).find((value) => value.type === 'footer');
  assert.ok(!elements(body).includes(footer), 'buttons must not scroll out with the form');
  assert.ok(elements(footer).some((value) => value.props?.children === 'common.cancel'));
  h.editor.selectionReady = true;
  const save = elements(h.show(true)).find((value) => value.type === 'button' && value.props.className === 'btn btn-primary');
  assert.equal(save.props.disabled, false);
  save.props.onClick(); assert.equal(h.calls.saved, 1);
  h.unmount();
});

test('cancel and Escape discard the draft and cancel checks without saving; saving keeps the dialog open', () => {
  const h = harness();
  const cancel = () => elements(h.show(true)).find((value) => value.type === 'button' && value.props.children === 'common.cancel');
  h.editor.busy = 'save';
  assert.equal(cancel().props.disabled, true);
  h.escape(); assert.equal(h.calls.closed, 0); assert.equal(h.calls.cancelled, 0);
  h.editor.busy = ''; h.show(true); h.escape();
  assert.equal(h.calls.closed, 1); assert.equal(h.calls.cancelled, 1); assert.equal(h.calls.saved, 0);
  cancel().props.onClick();
  assert.equal(h.calls.closed, 2); assert.equal(h.calls.cancelled, 2); assert.equal(h.calls.saved, 0);
  h.unmount();
});

test('a failed save stays in the dialog; only successful completion notifies the preview', () => {
  const h = harness(); h.show(true);
  h.editor.busy = 'save'; h.show(true);
  h.editor.busy = ''; h.editor.error = 'Saving failed';
  const tree = h.show(true);
  const body = elements(tree).find((value) => value.props?.className === 'modal-body');
  const error = elements(body).find((value) => value.type?.name === 'ModalErrorMessage');
  assert.equal(error.props.message, 'Saving failed');
  assert.equal(h.calls.applied, 0); assert.equal(h.calls.closed, 0);
  h.editor.busy = 'save'; h.editor.error = ''; h.show(true);
  h.editor.busy = ''; h.editor.notice = 'Saved'; h.show(true);
  assert.equal(h.calls.applied, 1);
  h.show(true); assert.equal(h.calls.applied, 1, 'rerenders must not refresh and dismiss twice');
  h.unmount();
});
