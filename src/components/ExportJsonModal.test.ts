import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function renderPreview(jsonContent: string, hidden = true) {
  const h = loadHookModule(new URL('./ExportJsonModal.tsx', import.meta.url), {
    'react-i18next': {
      useTranslation: () => ({
        t: (key: string, fallback?: string) => fallback ?? key,
      }),
    },
    '../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
  });
  h.render(() => h.exports.ExportJsonPreview({ jsonContent, hidden }));
  return h;
}

test('large export content renders a truncated preview immediately', () => {
  const items = Array.from({ length: 20000 }, (_, index) => ({ id: index, value: index * 1.5, name: `item-${index}` }));
  const h = renderPreview(JSON.stringify({ items }));
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
  const h = renderPreview(prefix + '🧪 more content"}', false);
  const tree = h.flush();
  const textarea = nodes(tree).find((node) => node.type === 'textarea');
  assert.ok(textarea);
  assert.equal(textarea.props.value.length, prefix.length);
  assert.ok(textarea.props.value === prefix);
  h.unmount();
});

test('small export content keeps rendering the preview immediately', () => {
  const h = renderPreview('{"email":"user@example.com"}');
  const tree = h.flush();

  const textarea = nodes(tree).find((node) => node.type === 'textarea');
  assert.ok(textarea);
  assert.match(String(textarea.props.value), /"email": "us\*\*\*om"/);
  assert.ok(!nodes(tree).some((node) => String(node.props?.className ?? '').includes('export-json-preview-note')));
  h.unmount();
});

test('copying and saving a large export send the complete JSON', async () => {
  const json = JSON.stringify({ accounts: 'a'.repeat(400 * 1024), tail: 'complete' });
  let copied = '';
  let saved = '';
  const h = loadHookModule(new URL('../hooks/useExportJsonModal.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { save: async () => '/fixture/export.json' },
    '@tauri-apps/plugin-opener': {},
    '@tauri-apps/api/core': {
      invoke: async (command: string, args?: { content: string }) => {
        if (command === 'get_downloads_dir') return '/fixture';
        assert.equal(command, 'save_text_file');
        saved = args!.content;
      },
    },
    '../utils/windowsOperationDialog': { presentWindowsOperationError: () => false },
  }, {
    navigator: {
      userAgent: 'fixture',
      clipboard: { writeText: async (content: string) => { copied = content; } },
    },
    window: { setTimeout: () => undefined },
  });
  const options = { exportFilePrefix: 'test', exportJsonByIds: async () => json };
  const modal = h.render(() => h.exports.useExportJsonModal(options));
  await modal.startExport(['fixture']);
  const opened = h.flush();
  await opened.copyJson();
  await opened.saveJson();

  assert.ok(copied === json);
  assert.ok(saved === json);
  h.unmount();
});
