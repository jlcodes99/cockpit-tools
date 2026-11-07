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
