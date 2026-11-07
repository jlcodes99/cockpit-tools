import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

function harness(onSave: (model: string | null) => Promise<unknown> | unknown) {
  const h = loadHookModule(new URL('../CodexImageModelConfig.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    './ModalErrorMessage': { ModalErrorMessage: 'error' },
    './codex/useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ models: [], syncError: '' }) },
  });
  let model: string | null | undefined;
  const render = () => {
    const control = h.exports.CodexImageModelSelect({ mainModel: model, disabled: false, onSave() {}, onSaveMainModel: onSave });
    const field = nodes(control).find((node) => node.type?.name === 'CodexImageMainModelInput');
    return field.type(field.props);
  };
  return { h, render, read(next: string | null | undefined) { model = next; return h.render(render); } };
}

const input = (tree: any) => nodes(tree).find((node) => node.type === 'input');
const saveButton = (tree: any) => nodes(tree).find((node) => node.type === 'button');

test('shared image main-model field displays the new default and authoritative backend values', () => {
  const saved: (string | null)[] = [];
  const { h, read } = harness((model) => { saved.push(model); });
  let tree = read(undefined);
  assert.equal(input(tree).props.placeholder, 'gpt-5.6-luna');
  assert.equal(input(tree).props.value, '');
  assert.equal(saveButton(tree).props.disabled, true);
  // Migration belongs to the backend; a returned value must never be silently replaced in the UI.
  for (const model of ['gpt-5.5', 'custom-image-driver', 'gpt-5.6-luna']) {
    tree = read(model);
    assert.equal(input(tree).props.value, model);
    assert.equal(saveButton(tree).props.disabled, true);
  }
  tree = read(null);
  assert.equal(input(tree).props.value, '');
  assert.deepEqual(saved, []);
  h.unmount();
});

test('image main-model saves custom IDs and clears to the backend default without writing a literal fallback', async () => {
  const saved: (string | null)[] = [];
  const { h, read } = harness(async (model) => { saved.push(model); });
  let tree = read('gpt-5.6-luna');
  input(tree).props.onChange({ target: { value: ' custom-driver ' } });
  tree = h.flush();
  await input(tree).props.onKeyDown({ key: 'Enter', nativeEvent: {}, preventDefault() {} });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(saved, ['custom-driver']);
  tree = read('custom-driver');
  assert.equal(input(tree).props.value, 'custom-driver');
  input(tree).props.onChange({ target: { value: '   ' } });
  tree = h.flush();
  saveButton(tree).props.onClick();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(saved, ['custom-driver', null]);
  tree = read(null);
  assert.equal(input(tree).props.value, '');
  assert.equal(input(tree).props.placeholder, 'gpt-5.6-luna');
  assert.equal(saveButton(tree).props.disabled, true);
  h.unmount();
});
