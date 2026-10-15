import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
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

test('High then Medium, cancellation/reselection and all six levels retain canonical editor order', () => {
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

test('opening a context or reasoning menu must not inflate the model list with in-flow padding', () => {
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
  h.render(() => h.exports.CodexExperimentalModelEditor({ models, mode: 'inline', onChange: (next: typeof models) => { models = next; } }));
  const listClassName = () => {
    const list = elements(h.flush()).find((node) => typeof node.props?.className === 'string'
      && node.props.className.includes('codex-experimental-model-editor__list'));
    assert.ok(list);
    return list.props.className as string;
  };
  const openMenu = (field: 'context' | 'reasoning') => {
    const row = elements(h.flush()).filter((node) => node.props?.className?.split(' ').includes('codex-experimental-model-editor__row')).pop();
    const trigger = elements(row).find((node) => node.props?.className === `codex-experimental-model-editor__${field}-trigger`);
    assert.ok(trigger);
    trigger.props.onClick();
  };

  assert.equal(listClassName().includes('has-open-menu'), false);
  openMenu('context');
  assert.equal(listClassName().includes('has-open-menu'), false, 'context menu must not add in-flow padding to the list');
  openMenu('reasoning');
  assert.equal(listClassName().includes('has-open-menu'), false, 'reasoning menu must not add in-flow padding to the list');
  h.unmount();

  // The blanket `.has-open-menu` padding rule is what grew the host modal
  // (issue #2804); it must stay gone from the stylesheet.
  const css = readFileSync(new URL('./CodexExperimentalModelEditor.css', import.meta.url), 'utf8');
  assert.ok(!css.includes('has-open-menu'), 'has-open-menu padding rule must be removed from the stylesheet');
  // Menus are revealed by scrolling the opened menu into view instead.
  const source = readFileSync(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), 'utf8');
  assert.ok(source.includes('scrollIntoView'), 'opened menus must be revealed via scrollIntoView');
});
