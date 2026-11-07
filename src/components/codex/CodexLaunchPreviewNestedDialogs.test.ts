import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
for (const saving of [false, true]) {
  test(`image-account picker registers its actual root and preserves saving=${saving} Escape guard`, () => {
    let trap: any;
    let escape: any;
    let cancelled = 0;
    const h = loadHookModule(new URL('./CodexImageAccountPickerModal.tsx', import.meta.url), {
      'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
      '../../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean) => { trap = { ref, open }; } },
      '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void) => { escape = { open, close }; } },
      '../../hooks/usePagination': { usePagination: () => ({ pageItems: [], totalItems: 0, pageSize: 10, totalPages: 1, currentPage: 1 }) },
      '../MultiSelectFilterDropdown': { MultiSelectFilterDropdown: 'filter' },
      '../PaginationControls': { PaginationControls: 'pagination' },
      '../SingleSelectFilterDropdown': { SingleSelectFilterDropdown: 'sort' },
      '../SingleSelectDropdown': { SingleSelectDropdown: 'model-select' },
      './useCodexImageModelCatalog': { useCodexImageModelCatalog: () => ({ ready: true, deleted: [], models: ['gpt-image-2.5', 'gpt-image-2'], syncError: '' }) },
      '../../utils/privacy': { isPrivacyModeEnabledByDefault: () => false, maskSensitiveValue: (value: string) => value },
    });
    const tree = h.render(() => h.exports.CodexImageAccountPickerModal({ accounts: [], selectedIds: [], saving,
      onCancel: () => { cancelled++; }, onConfirm() {} }));
    const dialog = nodes(tree).find((node) => node.props?.role === 'dialog');
    assert.equal(dialog.props.ref, trap.ref);
    assert.equal(dialog.props.tabIndex, -1);
    assert.equal(trap.open, true);
    assert.equal(escape.open, !saving);
    assert.equal(tree.props.onClick, undefined);
    assert.equal(nodes(dialog).find((node) => node.props?.className === 'modal-close').props.disabled, saving);
    if (!saving) { escape.close(); assert.equal(cancelled, 1); }
    h.unmount();
  });
}

test('the session-repair child registers its rendered root while slow instance reads remain pending', () => {
  let trap: any;
  let escape: any;
  let closed = 0;
  const t = (key: string) => key;
  const error = { message: null, scrollKey: 0, set() {} };
  const store = { repairSessionVisibilityAcrossInstances: () => new Promise(() => {}),
    listSessionVisibilityRepairInstances: () => new Promise(() => {}),
    listSessionVisibilityRepairProviders: () => new Promise(() => {}) };
  const selected: string[] = [];
  const h = loadHookModule(new URL('./CodexSessionVisibilityRepairModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean) => { trap = { ref, open }; } },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void) => { escape = { open, close }; } },
    '../../stores/useCodexInstanceStore': { useCodexInstanceStore: (selector: any) => selector(store) },
    '../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => error },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
  });
  const tree = h.render(() => h.exports.CodexSessionVisibilityRepairModal({ open: true, selectedSessionIds: selected, onClose: () => { closed++; } }));
  const dialog = nodes(tree).find((node) => node.props?.role === 'dialog');
  assert.equal(dialog.props.ref, trap.ref);
  assert.equal(dialog.props.tabIndex, -1);
  assert.equal(trap.open, true);
  assert.equal(escape.open, true);
  assert.equal(tree.props.onClick, undefined);
  escape.close(); assert.equal(closed, 1);
  h.unmount();
});
