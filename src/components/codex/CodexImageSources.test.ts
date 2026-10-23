import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
const t = (key: string) => `translated:${key}`;
const accounts = [
  { id: 'api2', auth_mode: 'apikey', email: 'API 2', api_model_catalog: ['gpt-image-2'] },
  { id: 'api25', auth_mode: 'apikey', email: 'API 2.5', api_model_catalog: ['gpt-image-2.5'] },
  { id: 'oauth', auth_mode: 'oauth', email: 'OAuth', plan_type: 'plus' },
];
function pickerHarness(registry: any = { ready: true, deleted: [], models: ['gpt-image-2.5', 'gpt-image-2'], syncError: '', retry() {} }, pagination?: any) {
  return loadHookModule(new URL('./CodexImageAccountPickerModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/usePagination': { usePagination: pagination ?? (({ items }: any) => ({ pageItems: items })) },
    '../ModalErrorMessage': { ModalErrorMessage: 'error' },
    '../MultiSelectFilterDropdown': { MultiSelectFilterDropdown: 'filter' },
    '../PaginationControls': { PaginationControls: 'pagination' },
    '../SingleSelectFilterDropdown': { SingleSelectFilterDropdown: 'sort' },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'model-select' },
    './useCodexImageModelCatalog': { useCodexImageModelCatalog: () => registry },
    '../../presentation/platformAccountPresentation': { buildCodexAccountPresentation: (account: any) => ({ displayName: account.email, planLabel: 'API_KEY' }) },
    '../../utils/privacy': { isPrivacyModeEnabledByDefault: () => false, maskSensitiveValue: (value: string) => value },
  });
}

test('missing models stay visible in the fixed footer and locate clears filters, changes page and focuses the source', () => {
  let page = 1;
  let h: ReturnType<typeof pickerHarness>;
  h = pickerHarness(undefined, ({ items }: any) => ({ pageSize: 10, currentPage: page,
    pageItems: items.slice((page - 1) * 10, page * 10), setCurrentPage(value: number) { page = value; } }));
  const sources = Array.from({ length: 11 }, (_, index) => ({ id: `source-${index}`, auth_mode: 'apikey', email: `Source ${index}`,
    last_used: 11 - index, plan_type: index === 10 ? 'suffix' : 'bare', api_model_catalog: [index === 10 ? 'gpt-image-2.5-high' : 'gpt-image-2.5'] }));
  let focused = 0, scrolled = 0, confirmed: any;
  const render = () => h.exports.CodexImageAccountPickerModal({ accounts: sources, selectedIds: [], imageGenerationModel: 'gpt-image-2.5',
    onCancel() {}, onConfirm(ids: string[], models: any) { confirmed = { ids, models }; } });
  let tree = h.render(render);
  nodes(tree).find((node) => node.type === 'button' && node.props.children === t('codex.deepSeek.start.pickerSelectAll')).props.onClick(); tree = h.flush();
  assert.equal(nodes(tree).find((node) => node.props?.className === 'btn btn-primary').props.disabled, true);
  assert.equal(nodes(tree).filter((node) => node.props?.className === 'codex-image-account-model-error').length, 0);
  assert.equal(nodes(tree).some((node) => node.props?.className === 'codex-image-account-picker-model-hint'), true);
  // A search and a plan filter hide the incomplete selected source, but its fixed hint remains.
  nodes(tree).find((node) => node.type === 'input' && node.props?.type === 'text').props.onChange({ target: { value: 'Source 0' } });
  nodes(tree).find((node) => node.type === 'filter').props.onToggleValue('bare'); tree = h.flush();
  const dialog = nodes(tree).find((node) => node.props?.className?.includes('codex-image-account-picker') && node.props?.role === 'dialog');
  dialog.props.ref.current = { querySelectorAll() { return page === 2 ? [{ dataset: { imageSourceId: 'source-10' },
    scrollIntoView() { scrolled++; }, querySelector() { return { focus() { focused++; } }; } }] : []; } };
  nodes(tree).find((node) => node.type === 'button' && node.props.children === t('codex.localAccess.imageAccounts.locateModel')).props.onClick();
  tree = h.flush(); tree = h.flush();
  assert.equal(page, 2); assert.equal(focused, 1); assert.equal(scrolled, 1);
  assert.equal(nodes(tree).find((node) => node.type === 'input' && node.props?.type === 'text').props.value, '');
  assert.equal(nodes(tree).find((node) => node.type === 'filter').props.selectedValues.length, 0);
  assert.equal(nodes(tree).filter((node) => node.props?.className === 'codex-image-account-model-error').length, 1);
  nodes(tree).find((node) => node.type === 'model-select').props.onChange('gpt-image-2.5-high'); tree = h.flush();
  assert.equal(nodes(tree).some((node) => node.props?.className === 'codex-image-account-picker-model-hint'), false);
  nodes(tree).find((node) => node.props?.className === 'btn btn-primary').props.onClick();
  assert.equal(confirmed.ids.length, 11); assert.equal(confirmed.models['source-10'], 'gpt-image-2.5-high');
  h.unmount();
});

test('picker preserves stored API selection until model loads and removes it when the model changes', () => {
  let model: string | null = null;
  let confirmed: any;
  const h = pickerHarness();
  const render = () => h.exports.CodexImageAccountPickerModal({ accounts, selectedIds: ['api25', 'oauth'], imageGenerationModel: model,
    onCancel() {}, onConfirm(ids: string[]) { confirmed = ids; } });
  const confirm = (tree: any) => nodes(tree).find((item) => item.props?.className === 'btn btn-primary');
  let tree = h.render(render);
  assert.equal(confirm(tree).props.disabled, true);
  confirm(tree).props.onClick(); assert.equal(confirmed, undefined);
  model = 'gpt-image-2.5'; tree = h.render(render);
  const api25 = nodes(tree).find((item) => item.props?.['aria-label'] === 'API 2.5');
  assert.equal(nodes(api25).find((item) => item.type === 'input').props.checked, true);
  confirm(tree).props.onClick(); assert.deepEqual(confirmed, ['api25', 'oauth']);
  model = 'gpt-image-2'; tree = h.render(render);
  assert.equal(nodes(tree).some((item) => item.props?.['aria-label'] === 'API 2.5'), false);
  confirm(tree).props.onClick(); assert.deepEqual(confirmed, ['oauth']);
  h.unmount();
});

test('image model read times out without API eligibility and retry ignores stale responses', async () => {
  const requests = [deferred<any>(), deferred<any>()];
  let requestCount = 0;
  let timeout: (() => void) | undefined;
  const h = loadHookModule(new URL('./useCodexImageModelConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '../../services/codexLocalAccessService': { getCodexLocalAccessState: () => requests[requestCount++].promise },
    './useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ ready: true, deleted: [], models: ['gpt-image-2.5', 'gpt-image-2'] }) },
    '../CodexImageModelConfig': { CodexImageModelSelect: 'model-config' },
    '../ModalErrorMessage': { ModalErrorMessage: 'error' },
  }, { setTimeout(callback: () => void) { timeout = callback; return 1; }, clearTimeout() {} });
  let state = h.render(() => h.exports.useCodexImageModelConfig(false));
  assert.equal(state.model, null); assert.equal(state.loaded, false);
  timeout!(); state = h.flush(); assert.equal(state.model, null);
  assert.equal(state.error, 'translated:codex.localAccess.imageAccounts.loadFailed');
  assert.equal(nodes(state.control).some((item) => item.type === 'model-config'), false);
  nodes(state.control).find((item) => item.type === 'button').props.onClick(); h.flush();
  requests[0].resolve({ collection: { imageGenerationModel: 'gpt-image-2' } });
  await settlePromises(); state = h.flush(); assert.equal(state.model, null);
  requests[1].resolve({ collection: { imageGenerationModel: 'gpt-image-2.5' } });
  await settlePromises(); state = h.flush(); assert.equal(state.model, 'gpt-image-2.5'); assert.equal(state.loaded, true);
  h.unmount();
});

test('API service image save keeps the picker open on failure and closes only on success', async () => {
  let fail = true;
  let saved: string[] | undefined;
  const h = loadHookModule(new URL('../CodexImageForwardConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    'react-dom': { createPortal: (node: any) => node },
    './codex/CodexImageAccountPickerModal': { CodexImageAccountPickerModal: 'picker' },
    './codex/useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ ready: true, deleted: [], models: ['gpt-image-2.5', 'gpt-image-2'] }) },
  }, { document: { body: {} } });
  let state = h.render(() => h.exports.useCodexImageForwardConfig({ accounts, accountIds: ['api2'], imageGenerationModel: 'gpt-image-2', disabled: false,
    onSave: async (ids: string[]) => { if (fail) throw new Error('codex.localAccess.imageAccounts.sourceUnavailable'); saved = ids; } }));
  state.renderPickButton('btn').props.onClick(); state = h.flush();
  state.overlay.props.onConfirm(['api2']); await settlePromises(); state = h.flush();
  assert.equal(state.overlay.type, 'picker');
  assert.equal(state.overlay.props.errorMessage, 'translated:codex.localAccess.imageAccounts.sourceUnavailable');
  state.overlay.props.onClearError(); state = h.flush(); assert.equal(state.overlay.props.errorMessage, '');
  fail = false; state.overlay.props.onConfirm(['api2']); await settlePromises(); state = h.flush();
  assert.equal(state.overlay, null); assert.deepEqual(saved, ['api2']);
  h.unmount();
});

test('a failed model save restores the last saved dropdown selection', async () => {
  const h = loadHookModule(new URL('../CodexImageModelConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    './codex/useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ models: ['gpt-image-2.5', 'gpt-image-2'], syncError: '', update: async () => {} }) },
    './ModalErrorMessage': { ModalErrorMessage: 'error' },
  });
  const ToolModelSelect = h.exports.CodexImageToolModelSelect;
  let tree = h.render(() => ToolModelSelect({ model: 'gpt-image-2.5', disabled: false, onSave: async () => { throw new Error('upstream failure'); } }));
  nodes(tree).find((item) => item.type === 'dropdown').props.onChange('gpt-image-2');
  await settlePromises(); tree = h.flush();
  assert.equal(nodes(tree).find((item) => item.type === 'dropdown').props.value, 'gpt-image-2.5');
  assert.match(nodes(tree).find((item) => item.type === 'error').props.message, /upstream failure/);
  h.unmount();
});

test('global and instance services persist complete per-source models and return the saved mapping', async () => {
  const models = { 'source-api': 'gpt-image-2.5-high' };
  const requests: any[] = [];
  const invoke = async (command: string, args: any) => {
    requests.push({ command, args });
    return command === 'update_codex_account_instance_access'
      ? { id: 'chat-api', api_image_generation_account_models: args.imageGenerationAccountModels }
      : { collection: { imageGenerationAccountModels: args.imageGenerationAccountModels } };
  };
  const globalService = loadHookModule(new URL('../../services/codexLocalAccessService.ts', import.meta.url), { '@tauri-apps/api/core': { invoke } });
  const globalState = await globalService.exports.updateCodexLocalAccessImageGenerationAccounts(['source-api'], models);
  assert.deepEqual(globalState.collection.imageGenerationAccountModels, models);
  assert.equal(requests[0].command, 'codex_local_access_update_image_generation_accounts');
  assert.deepEqual(requests[0].args.imageGenerationAccountModels, models);
  const instanceService = loadHookModule(new URL('../../services/codexService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke },
    '../utils/codexProxyEnginePrerequisite': {},
    '../utils/codexSwitchAuthFailure': {},
    './codexLaunchPreviewConfigService': {},
  });
  const account = await instanceService.exports.updateCodexAccountInstanceAccess('chat-api', 'gateway', null, ['source-api'], models);
  assert.deepEqual(account.api_image_generation_account_models, models);
  assert.equal(requests[1].command, 'update_codex_account_instance_access');
  assert.deepEqual(requests[1].args.imageGenerationAccountModels, models);
  await instanceService.exports.updateCodexAccountInstanceAccess('chat-api', 'gateway', null, ['source-api']);
  assert.equal(requests[2].args.imageGenerationAccountModels, null);
});




test('family picker requires an explicit suffix, persists it, and keeps controls outside the checkbox label', () => {
  const h = pickerHarness();
  let family = 'gpt-image-2.5';
  const api = { id: 'source', auth_mode: 'apikey', email: 'Source', api_model_catalog: ['gpt-image-2.5-high', 'gpt-image-2-low'] };
  let result: any;
  const render = () => h.exports.CodexImageAccountPickerModal({ accounts: [api], selectedIds: ['source'], imageGenerationModel: family,
    onCancel() {}, onConfirm(ids: string[], models: any) { result = { ids, models }; } });
  const dropdown = (tree: any) => nodes(tree).find((node) => node.type === 'model-select');
  const confirm = (tree: any) => nodes(tree).find((node) => node.props?.className === 'btn btn-primary');
  let tree = h.render(render);
  assert.equal(dropdown(tree).props.value, '');
  assert.equal(confirm(tree).props.disabled, true);
  assert.equal(nodes(tree).filter((node) => node.type === 'label').some((label) => nodes(label).some((node) => node.type === 'model-select')), false);
  dropdown(tree).props.onChange('gpt-image-2.5-high'); tree = h.flush();
  confirm(tree).props.onClick(); assert.deepEqual(result, { ids: ['source'], models: { source: 'gpt-image-2.5-high' } });
  family = 'gpt-image-2'; tree = h.flush();
  assert.equal(dropdown(tree).props.value, ''); assert.equal(confirm(tree).props.disabled, true);
  dropdown(tree).props.onChange('gpt-image-2-low'); tree = h.flush();
  confirm(tree).props.onClick(); assert.deepEqual(result, { ids: ['source'], models: { source: 'gpt-image-2-low' } });
  h.unmount();
});

test('invalid saved same-family suffix is retained and never silently replaced with an available bare model', () => {
  const registry: any = { ready: false, deleted: [], models: [], syncError: '' };
  const h = pickerHarness(registry);
  let catalog = ['gpt-image-2.5'];
  let result: any;
  const render = () => h.exports.CodexImageAccountPickerModal({ accounts: [{ id: 'source', auth_mode: 'apikey', email: 'Source', api_model_catalog: catalog }],
    selectedIds: ['source'], selectedModels: { source: 'gpt-image-2.5-high' }, imageGenerationModel: 'gpt-image-2.5',
    onCancel() {}, onConfirm(ids: string[], models: any) { result = { ids, models }; } });
  const dropdown = (tree: any) => nodes(tree).find((node) => node.type === 'model-select');
  const confirm = (tree: any) => nodes(tree).find((node) => node.props?.className === 'btn btn-primary');
  let tree = h.render(render); assert.equal(confirm(tree).props.disabled, true);
  registry.ready = true; tree = h.flush();
  assert.equal(dropdown(tree).props.value, ''); assert.equal(confirm(tree).props.disabled, true);
  catalog = ['gpt-image-2.5', 'gpt-image-2.5-high']; tree = h.flush();
  assert.equal(dropdown(tree).props.value, 'gpt-image-2.5-high');
  registry.deleted = ['gpt-image-2.5-high']; tree = h.flush();
  assert.equal(dropdown(tree).props.value, ''); assert.equal(confirm(tree).props.disabled, true);
  assert.equal(dropdown(tree).props.options.some((option: any) => option.value === 'gpt-image-2.5-high'), false);
  dropdown(tree).props.onChange('gpt-image-2.5'); tree = h.flush(); confirm(tree).props.onClick();
  assert.deepEqual(result, { ids: ['source'], models: { source: 'gpt-image-2.5' } });
  h.unmount();
});
