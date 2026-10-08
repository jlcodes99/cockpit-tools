import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule } from '../../../tests/helpers/reactHookHarness.ts';
import { codexModelConfigErrorKey } from '../../utils/codexModelConfig.ts';
import type { CodexModelConfigImportPreview } from '../../types/codex.ts';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}
function preview(): CodexModelConfigImportPreview {
  return { revision: 'expected-revision', entries: [{ section: 'models', id: 'new', action: 'added' }],
    added: ['models/new'], updated: [], conflicts: [], skipped: [], errors: [], committed: 0,
    models: [], defaultModelId: null };
}
function harness(service: Record<string, unknown>, imported = (_models: unknown, _default: unknown) => {}, options: {
  dialog?: Record<string, unknown>; fs?: Record<string, unknown>; onListen?: (listener: (event: any) => void) => void;
  restart?: () => Promise<unknown>;
} = {}) {
  let feedback: string | null = null;
  const h = loadHookModule(new URL('./CodexModelConfigTransferModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    'react-dom': { createPortal: (child: unknown) => child },
    '@tauri-apps/plugin-dialog': { open: async () => null, save: async () => null, ...options.dialog },
    '@tauri-apps/plugin-fs': { readTextFile: async () => '', stat: async () => ({ size: 0 }), writeTextFile: async () => {}, ...options.fs },
    '../../utils/tauriEventListener': { listenSafely: async (_event: string, listener: (event: any) => void) => { options.onListen?.(listener); return () => {}; } },
    '../ModalErrorMessage': { ModalErrorMessage: () => null, useModalErrorState: () => ({
      get message() { return feedback; }, scrollKey: 0, clear: () => { feedback = null; }, report: (value: string) => { feedback = value; },
    }) },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    '../../services/codexService': service,
    '../../services/codexLocalAccessService': { restartCodexLocalAccessSidecar: options.restart ?? (async () => {}) },
    '../../utils/codexModelConfig': { codexModelConfigErrorKey },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
  }, { document: { addEventListener() {}, removeEventListener() {}, body: {} } });
  let tree = h.render(() => h.exports.CodexModelConfigTransferModal({ instanceId: 'instance-b', onClose() {}, onImported: imported }));
  return { h, all: () => elements(tree), flush: () => { tree = h.flush(); },
    text: (content: string) => { elements(tree).find((node) => node.type === 'textarea').props.onChange({ target: { value: content } }); tree = h.flush(); },
    click: (key: string) => { const button = elements(tree).find((node) => node.type === 'button' && elements(node.props.children).length === 0 && node.props.children === key)
      ?? elements(tree).find((node) => node.type === 'button' && node.props.children?.includes?.(key));
      assert.ok(button, key); button.props.onClick(); tree = h.flush(); } };
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

test('a missing-fields response marks the JSON field and prevents importing an empty object', async () => {
  let previewContent: string | undefined;
  const ui = harness({ previewCodexModelConfigImport: async (input: { jsonContent: string }) => {
    previewContent = input.jsonContent;
    throw new Error('MODEL_CONFIG_FIELDS_UNSUPPORTED');
  } });
  ui.text('{}'); ui.click('codex.modelConfig.preview'); await settle(); ui.flush();
  assert.equal(previewContent, '{}');
  assert.equal(ui.all().find((node) => node.type === 'textarea').props['aria-invalid'], true);
  assert.ok(ui.all().find((node) => node.props?.role === 'alert'
    && node.props.children === 'codex.modelConfig.errors.fields'));
  assert.equal(ui.all().find((node) => node.type === 'button' && node.props.className === 'btn btn-primary').props.disabled, true);
  ui.h.unmount();
});

test('editing import content invalidates late preview and releases loading state', async () => {
  const pending = deferred<CodexModelConfigImportPreview>();
  const ui = harness({ previewCodexModelConfigImport: () => pending.promise });
  ui.text('{"models":[]}'); ui.click('codex.modelConfig.preview');
  ui.text('{"models":[{}]}');
  pending.resolve(preview()); await settle(); ui.flush();
  const importButton = ui.all().find((node) => node.type === 'button' && node.props.className === 'btn btn-primary');
  assert.equal(importButton.props.disabled, true);
  assert.equal(ui.all().some((node) => node.props?.className === 'codex-model-config-counts'), false);
  ui.h.unmount();
});

test('closing during export or file metadata read suppresses late native dialogs and reads', async () => {
  const pendingExport = deferred<string>(); let saveCalls = 0;
  const ui = harness({ exportCodexModelConfig: () => pendingExport.promise }, undefined,
    { dialog: { save: async () => { saveCalls += 1; return null; } } });
  ui.click('codex.modelConfig.export'); ui.h.unmount();
  pendingExport.resolve('{}'); await settle(); assert.equal(saveCalls, 0);
  const pendingStat = deferred<{ size: number }>(); let readCalls = 0;
  const fileUi = harness({}, undefined, { dialog: { open: async () => '/temporary/models.json' },
    fs: { stat: () => pendingStat.promise, readTextFile: async () => { readCalls += 1; return '{}'; } } });
  fileUi.click('codex.modelConfig.file'); await settle(); fileUi.h.unmount();
  pendingStat.resolve({ size: 2 }); await settle(); assert.equal(readCalls, 0);
});

test('confirmed import remains closable and late completion never fills a disposed dialog', async () => {
  const pending = deferred<CodexModelConfigImportPreview>(); let imported = 0;
  const ui = harness({ previewCodexModelConfigImport: async () => preview(), importCodexModelConfig: () => pending.promise }, () => { imported += 1; });
  ui.text('{}'); ui.click('codex.modelConfig.preview'); await settle(); ui.flush();
  ui.click('codex.modelConfig.import');
  assert.equal(ui.all().find((node) => node.type === 'button' && node.props['aria-label'] === 'common.close').props.disabled, undefined);
  ui.h.unmount(); pending.resolve({ ...preview(), committed: 1, models: [{ model_id: 'new', display_name: 'New' }] });
  await settle(); assert.equal(imported, 0);
});

test('gateway application failure stays inside the import dialog and retry only restarts the service', async () => {
  const pending = deferred<CodexModelConfigImportPreview>();
  let listener: ((event: any) => void) | undefined; let importCalls = 0; let restartCalls = 0;
  const ui = harness({ previewCodexModelConfigImport: async () => preview(), importCodexModelConfig: () => { importCalls += 1; return pending.promise; } }, undefined,
    { onListen: (value) => { listener = value; }, restart: async () => { restartCalls += 1; } });
  ui.text('{}'); ui.click('codex.modelConfig.preview'); await settle(); ui.flush();
  ui.click('codex.modelConfig.import');
  listener!({ payload: { instanceId: 'other-instance', revision: 'expected-revision', error: 'bad reload' } }); ui.flush();
  assert.equal(ui.all().some((node) => node.props?.message === 'codex.modelConfig.errors.apply'), false);
  listener!({ payload: { instanceId: 'instance-b', revision: 'expected-revision', error: 'bad reload' } }); ui.flush();
  pending.resolve({ ...preview(), committed: 1, models: [{ model_id: 'new', display_name: 'New' }] });
  await settle(); ui.flush();
  assert.ok(ui.all().find((node) => node.props?.message === 'codex.modelConfig.errors.apply'));
  assert.equal(ui.all().some((node) => node.props?.role === 'status'), false);
  ui.click('codex.localAccess.restartAction'); await settle(); ui.flush();
  assert.equal(importCalls, 1); assert.equal(restartCalls, 1);
  ui.h.unmount();
});

test('import submits target and revision; failure keeps modal open and requires a new preview', async () => {
  let submitted: any;
  const ui = harness({ previewCodexModelConfigImport: async () => preview(), importCodexModelConfig: async (input: unknown) => {
    submitted = input; throw new Error('MODEL_CONFIG_STATE_CHANGED');
  } });
  ui.text('{}'); ui.click('codex.modelConfig.preview'); await settle(); ui.flush();
  ui.click('codex.modelConfig.import'); await settle(); ui.flush();
  assert.equal(submitted.instanceId, 'instance-b'); assert.equal(submitted.expectedRevision, 'expected-revision');
  assert.equal(submitted.conflictStrategy, 'keep_existing');
  assert.ok(ui.all().find((node) => node.props?.role === 'dialog'));
  assert.ok(ui.all().find((node) => node.props?.message === 'codex.modelConfig.errors.stateChanged'));
  assert.equal(ui.all().find((node) => node.type === 'button' && node.props.className === 'btn btn-primary').props.disabled, true);
  ui.h.unmount();
});
