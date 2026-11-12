import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness.ts';
import * as order from '../../utils/codexExperimentalModelOrder.ts';
import * as context from '../../utils/codexModelContext.ts';
import * as modelConfig from '../../utils/codexModelConfig.ts';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

test('reset restores the supplied model names, order, efforts and default selection', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    './CodexProviderModelFetch': { CodexProviderModelFetch: () => null },
    'react-dom': { createPortal: (child: unknown) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  const resetModels = [
    { model_id: 'default-model', display_name: 'Default', reasoning_efforts: ['medium', 'high'] },
    { model_id: 'other', display_name: 'Other' },
  ];
  let models = [
    { model_id: 'other', display_name: 'User label' },
    { model_id: 'custom', display_name: 'Custom' },
  ];
  let defaultModelId: string | null = 'custom';
  h.render(() => h.exports.CodexExperimentalModelEditor({
    models, resetModels, defaultModelId, resetDefaultModelId: 'default-model', mode: 'inline',
    onChange: (next: typeof models) => { models = next; },
    onDefaultModelChange: (id: string | null) => { defaultModelId = id; },
  }));
  const resetButton = () => elements(h.flush())
    .find((node) => node.props?.className === 'codex-experimental-model-editor__reset-btn');
  assert.equal(resetButton().props.disabled, false);
  resetButton().props.onClick();
  assert.deepEqual(JSON.parse(JSON.stringify(models)), resetModels);
  assert.equal(defaultModelId, 'default-model');
  assert.equal(resetButton().props.disabled, true);
  h.unmount();
});

test('provider discovery offers models for selection and keeps results scoped to their connection', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    './CodexProviderModelFetch': { CodexProviderModelFetch: 'provider-fetch' },
    'react-dom': { createPortal: (child: any) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  let models = [{ model_id: 'custom', display_name: 'Custom' }];
  let channel = { id: 'route', namespace: 'relay', providerName: 'Relay', models: [], discovery: {
    accountId: 'account', baseUrl: 'https://relay.invalid/v1', apiKey: 'fixture', wireApi: 'chat_completions', authScheme: 'bearer',
  } };
  const added: Array<[string, string | undefined]> = [];
  h.render(() => h.exports.CodexExperimentalModelEditor({
    models, availableChannels: [channel], mode: 'inline',
    onChange: (next: typeof models) => { models = next; },
    onModelAdded: (id: string, routeId: string | undefined) => added.push([id, routeId]),
  }));
  const nodes = () => elements(h.flush());
  nodes().find((node) => node.props?.className === 'codex-experimental-model-editor__add-btn').props.onClick();
  const fetch = () => nodes().find((node) => node.type === 'provider-fetch');
  const options = () => nodes().filter((node) => node.type === 'button' && node.key);
  assert.equal(fetch().props.wireApi, 'chat_completions');
  assert.equal(fetch().props.baseUrl, 'https://relay.invalid/v1');
  fetch().props.onModels(['vendor/model', 'vendor/model', 'model-2']);
  assert.deepEqual(models.map((model) => model.model_id), ['custom']);
  options().find((node) => node.key === 'relay/vendor/model').props.onClick();
  assert.deepEqual(Array.from(models, (model) => model.model_id), ['custom', 'relay/vendor/model']);
  assert.deepEqual(added, [['relay/vendor/model', 'route']]);
  channel = { ...channel, namespace: '', discovery: { ...channel.discovery, accountId: 'direct' } };
  assert.ok(!options().some((node) => node.key === 'model-2'));
  fetch().props.onModels(['vendor/direct']);
  options().find((node) => node.key === 'vendor/direct').props.onClick();
  assert.ok(models.some((model) => model.model_id === 'vendor/direct'));
  assert.equal(added.length, 1);
  h.unmount();
});

test('adds selected catalog models without replacing the saved order, names or default', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    'react-dom': { createPortal: (child: any) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  const existing = [
    { model_id: 'custom', display_name: 'Custom' },
    { model_id: 'old-official', display_name: 'Renamed by user' },
  ];
  const resetModels = [
    { model_id: 'OLD-OFFICIAL', display_name: 'Default name' },
    { model_id: 'new-official', display_name: 'New official' },
    { model_id: 'relay/pool-model', display_name: 'Pool model' },
  ];
  let models = existing;
  const added: string[] = [];
  const defaults: Array<string | null> = [];
  h.render(() => h.exports.CodexExperimentalModelEditor({
    models, resetModels, defaultModelId: 'custom', mode: 'inline',
    onChange: (next: typeof models) => { models = next; },
    onModelAdded: (id: string) => added.push(id),
    onDefaultModelChange: (id: string | null) => defaults.push(id),
  }));
  const nodes = () => elements(h.flush());
  nodes().find((node) => node.props?.className === 'codex-experimental-model-editor__add-btn').props.onClick();
  const candidates = () => nodes().filter((node) => node.type === 'button' && node.props?.className === 'codex-experimental-model-editor__add-menu-item');
  assert.deepEqual(candidates().map((node) => node.key), ['new-official', 'relay/pool-model']);
  candidates()[0].props.onClick();
  assert.deepEqual(candidates().map((node) => node.key), ['relay/pool-model']);
  nodes().find((node) => node.props?.className === 'codex-experimental-model-editor__channel-add-all').props.onClick();
  assert.deepEqual(Array.from(models), [...existing, ...resetModels.slice(1)]);
  assert.deepEqual(added, ['new-official', 'relay/pool-model']);
  assert.deepEqual(defaults, []);
  assert.equal(candidates().length, 0);
  h.unmount();
});

test('Ultra uses a separate, editable request effort only on supported models', async () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': {
      getCodexModelReasoningEfforts: async () => ({ 'gpt-6.1-sol': ['high', 'max', 'ultra'], 'other': ['high'] }),
    },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    'react-dom': { createPortal: (child: any) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  let models = [
    { model_id: 'gpt-6.1-sol', display_name: 'Sol', default_reasoning_effort: 'high' },
    { model_id: 'other', display_name: 'Other' },
  ];
  h.render(() => h.exports.CodexExperimentalModelEditor({ models, mode: 'inline', onChange: (next: typeof models) => { models = next; } }));
  await settlePromises();
  const overrides = () => elements(h.flush())
    .filter((node) => node.props?.ariaLabel === 'codex.experimentalModels.ultraReasoning');
  assert.equal(overrides().length, 1);
  assert.deepEqual(Array.from(overrides()[0].props.options, (option: { value: string }) => option.value), ['', 'high', 'max']);
  overrides()[0].props.onChange('max');
  assert.equal(overrides()[0].props.value, 'max');
  assert.equal(models[0].default_reasoning_effort, 'high');
  overrides()[0].props.onChange('');
  assert.equal(overrides()[0].props.value, '');
  h.unmount();
});

test('editing model IDs and names retains row identity through edits, sorting and deletion', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    'react-dom': { createPortal: (child: any) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  let models = [{ model_id: 'first', display_name: 'First' }, { model_id: 'second', display_name: 'Second' }];
  let tree = h.render(() => h.exports.CodexExperimentalModelEditor({ models, mode: 'inline', onChange: (next: typeof models) => { models = next; } }));
  const rows = () => elements(tree).filter((node) => node.props?.className?.split(' ').includes('codex-experimental-model-editor__row'));
  const initialKeys = rows().map((row) => row.key);
  for (const value of ['f', 'fi', 'final']) {
    const input = elements(rows()[0]).find((node) => node.type === 'input' && node.props.value === models[0].model_id);
    assert.ok(input);
    input.props.onChange({ target: { value } });
    tree = h.flush();
    assert.deepEqual(rows().map((row) => row.key), initialKeys);
  }
  const name = elements(rows()[0]).find((node) => node.type === 'input' && node.props.value === 'First');
  name.props.onChange({ target: { value: 'Renamed' } });
  tree = h.flush();
  assert.deepEqual(rows().map((row) => row.key), initialKeys);
  models = [models[1], models[0]];
  tree = h.flush();
  assert.deepEqual(rows().map((row) => row.key), [...initialKeys].reverse());
  models = [models[1]];
  tree = h.flush();
  assert.equal(rows()[0].key, initialKeys[0]);
  h.unmount();
});

test('editing context and reasoning keeps distinct row keys while model IDs temporarily collide', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    'react-dom': { createPortal: (child: any) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  let models = [{ model_id: 'same', display_name: 'First' }, { model_id: 'same', display_name: 'Second' }];
  let tree = h.render(() => h.exports.CodexExperimentalModelEditor({ models, mode: 'inline', onChange: (next: typeof models) => { models = next; } }));
  const rows = () => elements(tree).filter((node) => node.props?.className?.split(' ').includes('codex-experimental-model-editor__row'));
  const initialKeys = rows().map((row) => row.key);
  assert.equal(new Set(initialKeys).size, 2);
  for (const field of ['context', 'reasoning']) {
    const trigger = elements(rows()[1]).find((node) => node.props?.className === `codex-experimental-model-editor__${field}-trigger`);
    trigger.props.onClick();
    tree = h.flush();
    const option = elements(rows()[1]).filter((node) => node.props?.className?.startsWith(`codex-experimental-model-editor__${field}-option`))[1];
    assert.ok(option);
    option.props.onClick();
    tree = h.flush();
    assert.deepEqual(rows().map((row) => row.key), initialKeys);
    assert.equal(elements(rows()[1]).some((node) => node.props?.value === 'Second'), true);
  }
  h.unmount();
});

test('High then Medium, cancellation/reselection and all six levels retain canonical editor order', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    'react-dom': { createPortal: (child: any) => child },
  }, { document: { addEventListener() {}, removeEventListener() {} }, window: { addEventListener() {}, removeEventListener() {} } });
  let models: any[] = [{ model_id: 'gpt-5.6-sol', display_name: 'Model' }];
  h.render(() => h.exports.CodexExperimentalModelEditor({ models, mode: 'inline', onChange: (next: any[]) => { models = next; } }));
  const toggle = (effort: string) => {
    let tree = elements(h.flush());
    if (!tree.some((node) => node.props?.className?.startsWith('codex-experimental-model-editor__reasoning-option'))) {
      tree.find((node) => node.props?.className === 'codex-experimental-model-editor__reasoning-trigger').props.onClick();
      tree = elements(h.flush());
    }
    tree.find((node) => node.props?.className?.startsWith('codex-experimental-model-editor__reasoning-option')
      && node.key === effort)!.props.onClick();
    h.flush();
  };
  toggle('high'); toggle('medium');
  assert.deepEqual(models[0].reasoning_efforts, ['medium', 'high']);
  toggle('medium'); toggle('medium');
  assert.deepEqual(models[0].reasoning_efforts, ['medium', 'high']);
  for (const effort of ['ultra', 'max', 'xhigh', 'low']) toggle(effort);
  assert.deepEqual(models[0].reasoning_efforts, ['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
  h.unmount();
});
