import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

for (const component of ['SingleSelectFilterDropdown', 'MultiSelectFilterDropdown', 'AccountTagFilterDropdown']) {
  test(`${component}: Escape closes the menu without changing selection and restores its trigger`, () => {
    let escape: (() => void) | undefined;
    let focused = 0;
    let changes = 0;
    const changed = () => { changes++; };
    const h = loadHookModule(new URL(`./${component}.tsx`, import.meta.url), {
      '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void) => { escape = open ? close : undefined; } },
      '../hooks/useDropdownPanelPlacement': { useDropdownPanelPlacement: () => ({ panelPlacement: 'bottom', panelRef: { current: null }, scrollContainerStyle: {} }) },
      'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    }, {
      document: { addEventListener() {}, removeEventListener() {} },
      window: { setTimeout: () => 1, clearTimeout() {} },
    });
    const props = {
      value: 'one', options: [{ value: 'one', label: 'One' }], onChange: changed,
      selectedValues: ['one'], onToggleValue: changed, onClear: changed,
      availableTags: ['one'], selectedTags: ['one'], onToggleTag: changed,
      ariaLabel: 'Filter', allLabel: 'All', filterLabel: 'Filter', clearLabel: 'Clear', emptyLabel: 'Empty',
    };
    let tree = h.render(() => h.exports[component](props));
    tree.props.ref.current = { querySelector: () => ({ focus: () => { focused++; } }) };
    assert.equal(escape, undefined);
    tree.props.children[0].props.onClick(); tree = h.flush();
    assert.ok(tree.props.children[1]);
    assert.equal(typeof escape, 'function');
    escape!(); tree = h.flush();
    assert.equal(tree.props.children[1], false);
    assert.equal(escape, undefined);
    assert.equal(focused, 1);
    assert.equal(changes, 0);
    h.unmount();
  });
}

test('AccountTagFilterDropdown: select all selects every unselected tag and disables when all selected', () => {
  let selected: string[] = ['alpha'];
  const toggled: string[] = [];
  let cleared = 0;
  const h = loadHookModule(new URL('./AccountTagFilterDropdown.tsx', import.meta.url), {
    '../hooks/useEscClose': { useEscCloseTopmost: () => {} },
    '../hooks/useDropdownPanelPlacement': { useDropdownPanelPlacement: () => ({ panelPlacement: 'bottom', panelRef: { current: null }, scrollContainerStyle: {} }) },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  }, {
    document: { addEventListener() {}, removeEventListener() {} },
    window: { setTimeout: () => 1, clearTimeout() {} },
  });
  const renderDropdown = () => h.render(() => h.exports.AccountTagFilterDropdown({
    availableTags: ['alpha', 'beta', 'gamma'],
    selectedTags: selected,
    onToggleTag: (tag: string) => {
      toggled.push(tag);
      selected = selected.includes(tag) ? selected.filter((item) => item !== tag) : [...selected, tag];
    },
    onClear: () => {
      cleared++;
      selected = [];
    },
  }));
  const findButtons = (node: any): any[] => {
    if (!node || typeof node !== 'object') return [];
    const results: any[] = [];
    if (node.type === 'button') results.push(node);
    const children = Array.isArray(node.props?.children) ? node.props.children : [node.props?.children];
    for (const child of children) results.push(...findButtons(child));
    return results;
  };
  let tree = renderDropdown();
  tree.props.children[0].props.onClick();
  tree = h.flush();
  const selectAllBtn = findButtons(tree.props.children[1]).find((btn) => btn.props.className === 'tag-filter-action-btn');
  assert.ok(selectAllBtn, 'select all button should render');
  assert.equal(selectAllBtn.props.disabled, false);
  selectAllBtn.props.onClick();
  assert.deepEqual(toggled, ['beta', 'gamma']);
  assert.deepEqual(selected, ['alpha', 'beta', 'gamma']);

  tree = renderDropdown();
  const selectAllBtnAfter = findButtons(tree.props.children[1]).find((btn) => btn.props.className === 'tag-filter-action-btn');
  assert.equal(selectAllBtnAfter.props.disabled, true);
  const clearBtn = findButtons(tree.props.children[1]).find((btn) => btn.props.className === 'tag-filter-clear');
  assert.ok(clearBtn, 'clear button should render when tags are selected');
  clearBtn.props.onClick();
  assert.equal(cleared, 1);
  assert.deepEqual(selected, []);
  h.unmount();
});
