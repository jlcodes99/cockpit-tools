import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('portaled dialog menus stay above their overlay and return focus after keyboard or selection', () => {
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
  let tree = h.render(() => h.exports.SingleSelectDropdown({ value: '0', options: [{ value: '0', label: 'Client' }, { value: '1', label: 'Upstream' }], onChange: (value: string) => chosen.push(value) }));
  const trigger = tree.props.children[0];
  trigger.props.ref.current = {
    parentElement: { layer: '12040', parentElement: null },
    getBoundingClientRect: () => ({ left: 200, top: 510, bottom: 556, width: 300 }),
    focus: () => { triggerFocus++; },
  };
  trigger.props.onKeyDown({ key: 'ArrowDown', preventDefault() {} }); tree = h.flush();
  let menu = tree.props.children[1];
  assert.equal(escapeRef, menu.props.ref);
  assert.equal(menu.props.style.zIndex, 12041);
  assert.equal(menu.props.style.bottom, '100px');
  const items = [0, 1].map((id) => ({ focus: () => { document.activeElement = items[id]; } }));
  menu.props.ref.current = { querySelectorAll: () => items, querySelector: () => items[0] };
  document.activeElement = items[0];
  menu.props.onKeyDown({ key: 'ArrowDown', preventDefault() {} });
  assert.equal(document.activeElement, items[1]);
  menu.props.children[1].props.onClick(); tree = h.flush();
  assert.deepEqual(chosen, ['1']);
  assert.equal(tree.props.children[1], null);
  assert.equal(triggerFocus, 1);
  tree.props.children[0].props.onClick(); tree = h.flush();
  escape?.(); tree = h.flush();
  assert.equal(tree.props.children[1], null);
  assert.equal(triggerFocus, 2);
  h.unmount();
});
