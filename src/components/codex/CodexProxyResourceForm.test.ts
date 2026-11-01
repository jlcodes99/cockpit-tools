import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}

test('remote-provider previews remain local and allow explicit import despite unknown node counts', async () => {
  const submissions: any[] = [];
  const previews: string[] = [];
  const timers: (() => void)[] = [];
  const h = loadHookModule(new URL('./CodexProxyResourceForm.tsx', import.meta.url), {
    'react-dom': { createPortal: (v: unknown) => v },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    '../../services/codexProxyCatalogService': {
      catalogErrorKey: () => 'codex.proxy.catalog.failed', catalogUnsupportedKey: () => 'unsupported',
      previewProxyImport: async (input: string) => {
        previews.push(input);
        return { structured: true, rows: [], valid: 0, invalid: 0, duplicates: 0, nativeProfile: true };
      },
    },
  }, { document: { body: {} }, setTimeout: (fn: () => void) => { timers.push(fn); return timers.length; }, clearTimeout() {} });
  h.render(() => h.exports.CodexProxyResourceForm({ busy: false, error: '', onSubmit: (v: any) => submissions.push(v), onClose() {}, onEdit() {} }));
  const input = 'proxy-providers:\n  remote: {type: http, url: https://example.invalid/sub}';
  elements(h.flush()).find((n) => n.type === 'textarea').props.onChange({ target: { value: input } });
  h.flush(); timers.splice(0).forEach((fn) => fn()); await settlePromises();
  const tree = elements(h.flush());
  assert.equal(previews.length, 1);
  assert.equal(submissions.length, 0);
  assert.ok(tree.some((n) => n.props.children === 'codex.proxy.catalog.nativePreview'));
  const confirm = tree.find((n) => n.type === 'button' && n.props.className === 'btn btn-primary');
  assert.ok(confirm);
  assert.equal(confirm.props.disabled, false);
  confirm.props.onClick();
  assert.equal(submissions.length, 1);
  assert.equal(submissions[0].input, input);
  h.unmount();
});
