import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises, deferred } from '../../../tests/helpers/reactHookHarness.ts';
import { createAppBackgroundStore } from '../../stores/useAppBackgroundStore.ts';
import { DEFAULT_APP_BACKGROUND } from '../../utils/appBackground.ts';
import type { AppBackgroundRecord } from '../../services/appBackgroundService.ts';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

async function harness(prepare = async (_file: File) => new Blob(['photo'])) {
  let saved: AppBackgroundRecord = { settings: { ...DEFAULT_APP_BACKGROUND, preset: 'mist' }, image: null };
  const store = createAppBackgroundStore({ load: async () => saved, save: async record => { saved = record; } });
  await store.getState().load();
  const Library = () => null;
  const module = loadHookModule(new URL('./AppBackgroundSettings.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../stores/useAppBackgroundStore': { useAppBackgroundStore: () => store.getState() },
    '../../components/AppBackground': { BackgroundSurface: () => null },
    '../../components/SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './BackgroundSkinLibrary': { BackgroundSkinLibrary: Library },
    '../../services/appBackgroundService': { prepareBackgroundImage: prepare },
  }, { Error });
  let tree = nodes(module.render(() => module.exports.AppBackgroundSettings()));
  const render = () => tree = nodes(module.flush());
  const library = () => tree.find(node => node.type === Library).props;
  const input = () => tree.find(node => node.type === 'input' && node.props.type === 'file');
  const changeFile = (name?: string) => {
    const target = { files: name ? [{ name }] : [], value: name ?? '' };
    input().props.onChange({ target });
    assert.equal(target.value, '', 'clear input so selecting the same file fires another change');
  };
  let opened = 0;
  const picker = { value: '', click: () => { opened++; } };
  input().props.ref.current = picker;
  return { store, module, render, library, changeFile, picker, opened: () => opened, saved: () => saved };
}

test('background controls remain interactive while saving and restoring defaults keeps skins', async () => {
  const h = await harness();
  h.store.setState({ saving: true });
  let tree = h.render();
  assert.equal(tree.find(node => node.type === 'fieldset').props.disabled, false);
  const footer = tree.find(node => node.props.className === 'background-settings-actions');
  const actions = nodes(footer).filter(node => node.type === 'button');
  assert.equal(actions.length, 1);
  assert.ok(JSON.stringify(actions[0]).includes('settings.background.restore'));
  tree.find(node => node.type === 'button' && node.key === 'forest').props.onClick();
  assert.equal(h.store.getState().record.settings.preset, 'forest');
  const range = tree.find(node => node.type === 'input' && node.props.type === 'range');
  range.props.onChange({ target: { value: '80' } });
  assert.equal(h.store.getState().record.settings.strength, 80);
  h.library().onAdd(); h.changeFile('Landscape.png'); await settlePromises();
  tree = h.render();
  tree.find(node => node.props.className === 'background-settings-actions').props.children[1].props.onClick();
  assert.equal(h.store.getState().record.settings.preset, 'default');
  assert.equal(h.store.getState().record.skins?.length, 1);
  h.module.unmount(); await h.store.getState().flush();
  assert.equal(h.saved().settings.preset, 'default');
});

test('picker can reopen after cancellation and repeatedly replace the same image without resetting skin settings', async () => {
  const h = await harness();
  h.library().onAdd(); h.changeFile(); await settlePromises();
  assert.equal(h.store.getState().record.settings.preset, 'mist');
  h.library().onAdd(); h.changeFile('Landscape.png'); await settlePromises(); h.render();
  const first = h.store.getState().record.activeSkinId!;
  assert.equal(h.store.getState().record.skins![0].name, 'Landscape');
  h.store.getState().update({ blur: 9, position: 'top' });
  h.library().onSelect(first);
  assert.equal(h.opened(), 2, 'selecting a saved skin does not open the picker');
  for (let i = 0; i < 2; i++) {
    h.library().onReplace(first); h.changeFile('Landscape.png'); await settlePromises(); h.render();
    assert.equal(h.store.getState().record.skins?.length, 1);
    assert.equal(h.store.getState().record.settings.blur, 9);
    assert.equal(h.store.getState().record.settings.position, 'top');
  }
  h.library().onAdd(); h.changeFile('Another.webp'); await settlePromises(); h.render();
  assert.equal(h.opened(), 5);
  assert.equal(h.store.getState().record.skins?.length, 2);
  assert.notEqual(h.store.getState().record.activeSkinId, first);
  h.module.unmount(); await h.store.getState().flush();
});

test('failed image preparation preserves the current skin and allows retry; duplicate changes are ignored while preparing', async () => {
  const firstRead = deferred<Blob>();
  let reads = 0;
  const h = await harness(async () => { if (++reads === 1) return firstRead.promise; return new Blob(['retry']); });
  h.library().onAdd(); h.changeFile('Broken.png');
  h.changeFile('Duplicate.png');
  assert.equal(reads, 1);
  assert.equal(h.render().find(node => node.type === 'fieldset').props.disabled, true);
  firstRead.reject(new Error('imageRead')); await settlePromises();
  let tree = h.render();
  assert.ok(tree.some(node => node.props.role === 'alert' && node.props.children[0] === 'settings.background.imageRead'));
  assert.equal(h.store.getState().record.settings.preset, 'mist');
  h.library().onAdd(); h.changeFile('Retry.png'); await settlePromises();
  tree = h.render();
  assert.equal(tree.some(node => node.props.role === 'alert'), false);
  assert.equal(h.store.getState().record.skins?.length, 1);
  h.module.unmount(); await h.store.getState().flush();
});
