import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness.ts';
import * as order from '../../utils/codexExperimentalModelOrder.ts';
import * as context from '../../utils/codexModelContext.ts';
import * as modelConfig from '../../utils/codexModelConfig.ts';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

test('editing model IDs and names retains row identity through edits, sorting and deletion', () => {
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
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
