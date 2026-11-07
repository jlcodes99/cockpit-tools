import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness.ts';
import { DEFAULT_APP_BACKGROUND } from '../../utils/appBackground.ts';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function harness(library = false) {
  const calls: unknown[][] = [];
  const revoked: string[] = [];
  const listeners = new Set<unknown>();
  let created = 0;
  const props = {
    skin: { id: 'skin1', name: 'Landscape', image: new Blob(['first']), settings: { ...DEFAULT_APP_BACKGROUND, preset: 'custom' } },
    selected: true, disabled: false,
    onSelect: (...args: unknown[]) => calls.push(['select', ...args]),
    onReplace: (...args: unknown[]) => calls.push(['replace', ...args]),
    onRename: (...args: unknown[]) => calls.push(['rename', ...args]),
    onDelete: (...args: unknown[]) => calls.push(['delete', ...args]),
  };
  const module = loadHookModule(new URL('./BackgroundSkinLibrary.tsx', import.meta.url), {
    'react-dom': { createPortal: (element: unknown) => element },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../components/AppBackground': { BackgroundSurface: () => null },
  }, {
    URL: { createObjectURL: () => `blob:${++created}`, revokeObjectURL: (url: string) => revoked.push(url) },
    document: { body: {}, addEventListener: (_event: string, listener: unknown) => listeners.add(listener),
      removeEventListener: (_event: string, listener: unknown) => listeners.delete(listener) },
  });
  let tree = nodes(module.render(() => library ? module.exports.BackgroundSkinLibrary({
    skins: [props.skin], activeSkinId: 'skin1', disabled: false, onAdd: () => calls.push(['add']),
    onSelect: props.onSelect, onReplace: props.onReplace, onRename: props.onRename, onDelete: props.onDelete,
  }) : module.exports.BackgroundSkinCard(props)));
  const render = () => tree = nodes(module.flush());
  const button = (label: string) => tree.find(node => node.type === 'button' && node.props.children === label);
  const open = () => {
    tree.find(node => node.props.className === 'btn btn-secondary background-skin-menu-trigger').props.onClick();
    render();
  };
  return { props, module, render, button, open, calls, revoked, listeners };
}

test('skin card exposes separate selection, replacement, rename and delete requests', () => {
  const h = harness();
  const tree = h.render();
  tree.find(node => node.type === 'button' && node.props['aria-pressed'] === true).props.onClick();
  assert.deepEqual(h.calls.shift(), ['select', 'skin1']);
  h.open(); h.button('settings.background.replaceImage').props.onClick(); h.render();
  assert.deepEqual(h.calls.shift(), ['replace', 'skin1']);
  h.open(); h.button('settings.background.rename').props.onClick(); h.render();
  assert.deepEqual(h.calls.shift(), ['rename', 'skin1', 'Landscape']);
  h.open(); h.button('common.delete').props.onClick(); h.render();
  assert.deepEqual(h.calls.shift(), ['delete', 'skin1']);
  h.module.unmount();
});

test('library rename requires a nonempty name and deletion requires confirmation', () => {
  const h = harness(true);
  let tree = h.render();
  const card = () => tree.find(node => node.type === h.module.exports.BackgroundSkinCard).props;
  card().onRename('skin1', 'Landscape'); tree = h.render();
  let input = tree.find(node => node.type === 'input');
  assert.equal(input.props.value, 'Landscape');
  input.props.onChange({ target: { value: '  ' } }); tree = h.render();
  assert.equal(h.button('common.save').props.disabled, true);
  tree.find(node => node.type === 'form').props.onSubmit({ preventDefault() {} });
  assert.equal(h.calls.length, 0);
  input = tree.find(node => node.type === 'input');
  input.props.onChange({ target: { value: 'Mountains' } }); tree = h.render();
  tree.find(node => node.type === 'form').props.onSubmit({ preventDefault() {} }); tree = h.render();
  assert.deepEqual(h.calls.shift(), ['rename', 'skin1', 'Mountains']);
  card().onDelete('skin1'); tree = h.render();
  assert.equal(h.calls.length, 0, 'opening delete confirmation does not delete');
  h.button('common.cancel').props.onClick(); tree = h.render();
  assert.equal(h.calls.length, 0, 'cancelling delete keeps the skin');
  card().onDelete('skin1'); h.render();
  tree = h.render();
  tree.find(node => node.type === 'form').props.onSubmit({ preventDefault() {} });
  assert.deepEqual(h.calls.shift(), ['delete', 'skin1']);
  h.module.unmount();
});

test('thumbnail URLs and outside-click listeners are released on replacement and unmount', () => {
  const h = harness();
  h.open(); assert.equal(h.listeners.size, 1);
  h.button('settings.background.replaceImage').props.onClick(); h.render();
  assert.equal(h.listeners.size, 0);
  h.props.skin = { ...h.props.skin, image: new Blob(['second']) }; h.render();
  assert.deepEqual(h.revoked, ['blob:1']);
  h.open();
  h.module.unmount();
  assert.deepEqual(h.revoked, ['blob:1', 'blob:2']);
  assert.equal(h.listeners.size, 0);
});
