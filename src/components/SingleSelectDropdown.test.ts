import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('portaled menus skip disabled options and restore focus after selection or Escape', () => {
  const document: any = { body: {}, activeElement: null, addEventListener() {}, removeEventListener() {} };
  const chosen: string[] = [];
  let escape: (() => void) | undefined;
  let escapeRef: any;
  let triggerFocus = 0;
  const h = loadHookModule(new URL('./SingleSelectDropdown.tsx', import.meta.url), {
    'react-dom': { createPortal: (tree: unknown) => tree },
    '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void, ref: any) => { escape = open ? close : undefined; escapeRef = ref; } },
  }, { document, Node: class {}, window: {
    innerWidth: 900, innerHeight: 600,
    getComputedStyle: (element: any) => ({ zIndex: element.layer ?? 'auto' }),
    setTimeout: () => 1, clearTimeout() {}, addEventListener() {}, removeEventListener() {},
  } });
  const options = [
    { value: 'selected-disabled', label: 'Disabled selection', disabled: true },
    { value: '0', label: 'Client' },
    { value: 'middle-disabled', label: 'Disabled middle option', disabled: true },
    { value: '1', label: 'Upstream' },
    { value: 'last-disabled', label: 'Disabled last option', disabled: true },
  ];
  let tree = h.render(() => h.exports.SingleSelectDropdown({ value: 'selected-disabled', options, onChange: (value: string) => chosen.push(value) }));
  const trigger = tree.props.children[0];
  trigger.props.ref.current = {
    parentElement: { layer: '12040', parentElement: null },
    getBoundingClientRect: () => ({ left: 200, top: 510, bottom: 556, width: 300 }),
    focus: () => { triggerFocus++; },
  };
  const items = options.map((option) => ({
    ...option,
    focus() { if (!option.disabled) document.activeElement = this; },
  }));
  const matches = (selector: string) => items.filter(item =>
    (!selector.includes(':not(:disabled)') || !item.disabled)
      && (!selector.includes('aria-selected') || item.value === 'selected-disabled'));
  escapeRef.current = { querySelector: (selector: string) => matches(selector)[0], querySelectorAll: matches };

  trigger.props.onKeyDown({ key: 'ArrowDown', preventDefault() {} }); tree = h.flush();
  const menu = tree.props.children[1];
  assert.equal(escapeRef, menu.props.ref);
  assert.equal(menu.props.style.zIndex, 12041);
  assert.equal(menu.props.style.bottom, '100px');
  assert.equal(document.activeElement, items[1]);
  for (const [key, index] of [['ArrowDown', 3], ['ArrowUp', 1], ['ArrowUp', 3], ['ArrowDown', 1], ['End', 3], ['Home', 1]] as const) {
    menu.props.onKeyDown({ key, preventDefault() {} });
    assert.equal(document.activeElement, items[index], key);
  }
  menu.props.children[1].props.children[3].props.onClick(); tree = h.flush();
  assert.deepEqual(chosen, ['1']);
  assert.equal(tree.props.children[1], null);
  assert.equal(triggerFocus, 1);
  tree.props.children[0].props.onClick(); tree = h.flush();
  escape?.(); tree = h.flush();
  assert.equal(tree.props.children[1], null);
  assert.equal(triggerFocus, 2);
  h.unmount();
});

test('search filters visible labels while preserving the current value and resetting on reopen', () => {
  const h = loadHookModule(new URL('./SingleSelectDropdown.tsx', import.meta.url), {
    'react-dom': { createPortal: (tree: unknown) => tree },
    '../hooks/useEscClose': { useEscCloseTopmost() {} },
  }, {
    document: { body: {}, addEventListener() {}, removeEventListener() {} },
    window: {
      innerWidth: 900, innerHeight: 600,
      getComputedStyle: () => ({ zIndex: 'auto' }),
      setTimeout: () => 1, clearTimeout() {}, addEventListener() {}, removeEventListener() {},
    },
  });
  const chosen: string[] = [];
  const props = {
    value: 'daily',
    options: [
      { value: '', label: 'Enter a new key' },
      { value: 'daily', label: 'Daily (sk-d…ab12)' },
      { value: 'work', label: 'Work (sk-w…cd34)' },
    ],
    onChange: (value: string) => chosen.push(value),
    search: { label: 'Search keys', emptyLabel: 'No results' },
  };
  let tree = h.render(() => h.exports.SingleSelectDropdown(props));
  const trigger = tree.props.children[0];
  trigger.props.ref.current = {
    getBoundingClientRect: () => ({ left: 40, top: 30, bottom: 76, width: 300 }),
    focus() {},
  };
  trigger.props.onClick();
  tree = h.flush();
  const filter = (value: string) => {
    tree.props.children[1].props.children[0].props.children.props.onChange({ target: { value } });
    tree = h.flush();
  };
  const shown = () => Array.from(tree.props.children[1].props.children[1].props.children,
    (option: any) => option.props.children[0].props.children);
  filter(' CD34 ');
  assert.deepEqual(shown(), ['Work (sk-w…cd34)']);
  assert.equal(tree.props.children[0].props.children[0].props.children, 'Daily (sk-d…ab12)');
  assert.deepEqual(chosen, []);
  filter('missing');
  assert.deepEqual(shown(), []);
  assert.equal(tree.props.children[1].props.children[2].props.children, 'No results');
  filter('work');
  tree.props.children[1].props.children[1].props.children[0].props.onClick();
  tree = h.flush();
  assert.deepEqual(chosen, ['work']);
  tree.props.children[0].props.onClick();
  tree = h.flush();
  assert.equal(shown().length, 3);
  h.unmount();
});
