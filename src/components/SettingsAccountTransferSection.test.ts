import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}
function label(value: any): string {
  if (value == null || typeof value === 'boolean') return '';
  if (Array.isArray(value)) return value.map(label).join('');
  return typeof value === 'object' ? label(value.props?.children) : String(value);
}

function harness(importError?: string) {
  const migration = deferred<any>();
  const calls: string[] = [];
  let traps: any[] = [];
  let escapes: any[] = [];
  const settings = { enabled: false, directory_path: '/fixture/old', retention_days: 15 };
  const selection = { includeAccounts: true, includeConfig: true };
  const t = (key: string, options?: Record<string, unknown>) => options?.error ? `${key}: ${options.error}` : key;
  const h = loadHookModule(new URL('./SettingsAccountTransferSection.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    'react-dom': { createPortal: (node: any) => node },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '@tauri-apps/plugin-dialog': { open: async () => '/fixture/new' },
    './ExportJsonModal': { ExportJsonModal: 'ExportJsonModal' },
    '../hooks/useExportJsonModal': { useExportJsonModal: () => ({ preparing: false, showModal: false }) },
    '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void, ref: any) => escapes.push({ open, close, ref }) },
    '../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean, restore: boolean, active: boolean) => traps.push({ ref, open, restore, active }) },
    '../services/accountTransferService': {},
    '../services/dataTransferService': { importDataTransferJson: async () => { throw new Error(importError ?? 'synthetic import failure'); } },
    '../services/scheduledBackupService': {
      getAutoBackupState: async () => ({
        settings: { status: 'fulfilled', value: settings },
        files: { status: 'fulfilled', value: [] },
        usage: { status: 'fulfilled', value: { entries: [], total_size_bytes: 0, total_file_count: 0 } },
      }),
      getSelectionFromAutoBackupSettings: () => selection,
      selectionToAutoBackupMode: () => 'full',
      normalizeAutoBackupPlatforms: () => [],
      previewBackupDirectoryChange: async () => ({ old_directory: '/fixture/old', new_directory: '/fixture/new', size_bytes: 0, file_count: 0, sources: [] }),
      changeBackupDirectory: () => { calls.push('change'); return migration.promise; },
      cancelBackupDirectoryChange: async () => { calls.push('cancel'); },
    },
    '../types/platform': { ALL_PLATFORM_IDS: [] },
    '../utils/platformMeta': { getPlatformLabel: (value: string) => value },
    '../utils/windowsOperationDialog': { presentWindowsOperationError: () => false },
  }, { window: { addEventListener() {}, removeEventListener() {} }, document: { body: {} } });
  h.render(() => {
    traps = []; escapes = [];
    return h.exports.SettingsAccountTransferSection();
  });
  return {
    h, calls, migration,
    async ready() { await settlePromises(); h.flush(); },
    click(key: string) {
      const button = nodes(h.flush()).find((node) => node.type === 'button' && label(node) === key);
      assert.ok(button, `button ${key}`);
      button.props.onClick(); h.flush();
    },
    esc() { const active = escapes.filter((entry) => entry.open); assert.equal(active.length, 1); assert.equal(active[0].ref, traps.find((trap) => trap.open && trap.active)?.ref); active[0].close(); h.flush(); },
    dialogs() { return nodes(h.flush()).filter((node) => node.props?.role === 'dialog'); },
    activeTrap() { const active = traps.filter((entry) => entry.open && entry.active); assert.equal(active.length, 1); return active[0]; },
  };
}

test('transfer options and import dialogs register their actual roots and close through their existing guards', async () => {
  const h = harness(); await h.ready();
  for (const key of ['common.shared.export.title', 'common.shared.import.label']) {
    h.click(key);
    const [dialog] = h.dialogs();
    assert.equal(dialog.props.tabIndex, -1);
    assert.equal(dialog.props['aria-modal'], 'true');
    assert.equal(h.activeTrap().ref, dialog.props.ref);
    assert.equal(h.activeTrap().restore, true);
    h.esc(); assert.equal(h.dialogs().length, 0);
  }
  assert.deepEqual(h.calls, []);
  h.h.unmount();
});

test('migration children suspend backup manager keyboard handling and Escape never aborts a running directory write', async () => {
  const h = harness(); await h.ready(); h.click('common.open');
  const parent = h.dialogs()[0];
  h.click('settings.transfer.backup.changeDirectory'); await h.ready();
  assert.equal(h.dialogs().length, 2);
  assert.notEqual(h.activeTrap().ref, parent.props.ref);
  h.esc(); assert.equal(h.dialogs().length, 1);
  assert.equal(h.activeTrap().ref, parent.props.ref);
  assert.deepEqual(h.calls, []);

  h.click('settings.transfer.backup.changeDirectory'); await h.ready();
  h.click('settings.transfer.backup.switchOnly');
  assert.deepEqual(h.calls, ['change']);
  h.esc(); assert.equal(h.dialogs().length, 2);
  assert.notEqual(h.activeTrap().ref, parent.props.ref);
  assert.deepEqual(h.calls, ['change']);
  h.migration.resolve({ new_directory: '/fixture/new', migrated_file_count: 0, migrated_size_bytes: 0, remaining_paths: [] });
  await h.ready(); h.esc();
  assert.equal(h.dialogs().length, 1);
  assert.equal(h.activeTrap().ref, parent.props.ref);
  h.esc(); assert.equal(h.dialogs().length, 0);
  h.h.unmount();
});

test('JSON preview suspends keyboard handling without closing or losing its open lifecycle', () => {
  let trap: any;
  let escapeOpen: boolean | undefined;
  const props = { isOpen: true, active: true, title: 'Export', jsonContent: '{}', onClose() {} };
  const h = loadHookModule(new URL('./ExportJsonModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean) => { escapeOpen = open; } },
    '../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean, restore: boolean, active: boolean) => { trap = { ref, open, restore, active }; } },
  });
  h.render(() => h.exports.ExportJsonModal(props));
  assert.equal(escapeOpen, true);
  props.active = false;
  const dialog = nodes(h.flush()).find((node) => node.props?.role === 'dialog');
  assert.equal(dialog.props.ref, trap.ref);
  assert.equal(dialog.props.tabIndex, -1);
  assert.equal(trap.open, true);
  assert.equal(trap.active, false);
  assert.equal(trap.restore, true);
  assert.equal(escapeOpen, false);
  h.unmount();
});


test('malformed JSON keeps the import dialog open and reports format failure instead of empty input', async () => {
  const h = harness('invalid_json'); await h.ready();
  h.click('common.shared.import.label');
  const textarea = nodes(h.h.flush()).find((node) => node.type === 'textarea');
  textarea.props.onChange({ target: { value: '{invalid-json' } }); h.h.flush();
  h.click('modals.import.importBtn'); await h.ready();
  assert.equal(h.dialogs().length, 1);
  const text = label(h.h.flush());
  assert.ok(text.includes('common.shared.import.invalidJson'));
  assert.ok(!text.includes('messages.jsonRequired'));
  h.esc(); assert.equal(h.dialogs().length, 0);
  h.h.unmount();
});
