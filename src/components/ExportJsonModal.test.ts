import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function renderModal(jsonContent: string, hidden = true) {
  const h = loadHookModule(new URL('./ExportJsonModal.tsx', import.meta.url), {
    'react-i18next': {
      useTranslation: () => ({
        t: (key: string, fallback?: string) => fallback ?? key,
      }),
    },
    '../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
  });
  const props = {
    isOpen: true,
    title: 'Export',
    jsonContent,
    hidden,
    copied: false,
    saving: false,
    savedPath: null,
    canOpenSavedDirectory: false,
    pathCopied: false,
    onClose() {},
    onToggleHidden() {},
    onCopyJson: async () => {},
    onSaveJson: async () => {},
    onOpenSavedDirectory: async () => {},
    onCopySavedPath: async () => {},
  };
  h.render(() => h.exports.ExportJsonModal(props));
  return h;
}

test('large export content renders a truncated preview immediately', () => {
  const items = Array.from({ length: 20000 }, (_, index) => ({ id: index, value: index * 1.5, name: `item-${index}` }));
  const h = renderModal(JSON.stringify({ items }));
  const tree = h.flush();

  const textarea = nodes(tree).find((node) => node.type === 'textarea');
  assert.ok(textarea);
  assert.ok(String(textarea.props.value).length <= 300 * 1024);
  assert.match(String(textarea.props.value), /\*\*\*/);

  const note = nodes(tree).find((node) => String(node.props?.className ?? '').includes('export-json-preview-note'));
  assert.ok(note);
  assert.equal(note.props.children, 'common.previewTruncated');
  assert.equal(textarea.props['aria-describedby'], note.props.id);
  h.unmount();
});

test('revealed export preview truncates without splitting an emoji', () => {
  const start = '{"value":"';
  const prefix = start + 'a'.repeat(300 * 1024 - start.length - 1);
  const h = renderModal(prefix + '🧪 more content"}', false);
  const tree = h.flush();
  const textarea = nodes(tree).find((node) => node.type === 'textarea');
  assert.ok(textarea);
  assert.equal(textarea.props.value, prefix);
  h.unmount();
});

test('small export content keeps rendering the preview immediately', () => {
  const h = renderModal('{"email":"user@example.com"}');
  const tree = h.flush();

  const textarea = nodes(tree).find((node) => node.type === 'textarea');
  assert.ok(textarea);
  assert.match(String(textarea.props.value), /"email": "us\*\*\*om"/);
  assert.ok(!nodes(tree).some((node) => String(node.props?.className ?? '').includes('export-json-preview-note')));
  h.unmount();
});
