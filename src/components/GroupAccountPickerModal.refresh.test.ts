import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

type Element = { type: unknown; props: Record<string, any> };
function nodes(tree: any): Element[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree?.props ? [tree, ...nodes(tree.props.children)] : [];
}
const t = (key: string) => key;
const variants = [
  ['GroupAccountPickerModal', './GroupAccountPickerModal.tsx'],
  ['CodexGroupAccountPickerModal', './CodexGroupAccountPickerModal.tsx'],
  ['UniversalGroupAccountPickerModal', './AccountGroupModal.tsx'],
];
function picker(name: string, file: string, empty = false) {
  let session: ReturnType<typeof loadHookModule>;
  const h = loadHookModule(new URL(file, import.meta.url), {
    '../hooks/useDialogSession': { useDialogSession: (key: string | null, initialize: () => void) => session.exports.useDialogSession(key, initialize) },
    '../hooks/useEscClose': { useEscClose() {} },
    './MultiSelectFilterDropdown': { MultiSelectFilterDropdown: 'filter' },
    './AccountTagFilterDropdown': { AccountTagFilterDropdown: 'tags' },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    'react-i18next': { useTranslation: () => ({ t }) },
    '../presentation/platformAccountPresentation': { buildCodexAccountPresentation: (a: any) => ({ displayName: a.email, planLabel: 'TEAM', planClass: '' }) },
    '../utils/account': { getAntigravityTierBadge: () => ({ label: 'PRO', className: '', tier: 'PRO' }) },
    '../utils/accountFilters': { collectAvailableAccountTags: () => [], buildAccountTierCounts: () => ({ all: 2 }), buildAccountTierFilterOptions: () => [], accountMatchesTagFilters: () => true, accountMatchesTypeFilters: () => true, normalizeAccountTag: (tag: string) => tag },
  });
  session = loadHookModule(new URL('../hooks/useDialogSession.ts', import.meta.url), { react: h.hooks });
  const accounts = [{ id: 'a', email: 'a@example.com' }, { id: 'b', email: 'b@example.com' }];
  let fail = false, closed = 0;
  const saved: any[] = [];
  const props = {
    isOpen: true, targetGroup: { id: 'group', name: 'Original', accountIds: ['a'] },
    accounts: empty ? [] : accounts, accountGroups: [], verificationStatusMap: {},
    getVerificationBadge: () => null, maskAccountText: (value: string) => value,
    onClose() { closed++; }, onConfirm: async (payload: any) => { saved.push(payload); if (fail) throw new Error('save failed'); },
  };
  h.render(() => h.exports[name](props));
  const find = (predicate: (node: Element) => boolean) => { const node = nodes(h.flush()).find(predicate); assert.ok(node); return node; };
  const search = () => find((node) => node.type === 'input' && node.props.placeholder === 'accounts.search');

  // Account rows contain their own checkbox; the first checkbox is the select-all header.
  const checkboxes = () => nodes(h.flush()).filter((node) => node.type === 'input' && node.props.type === 'checkbox');
  return { h, props, accounts, saved, search, checkboxes, find, closed: () => closed,
    fail(value: boolean) { fail = value; },
    save: () => find((node) => node.type === 'button' && node.props.className === 'btn btn-primary').props.onClick(),
  };
}

for (const [name, file] of variants) {
  test(`${name}: refresh preserves search, selections, filters and errors; retry submits the draft`, async () => {
    const p = picker(name, file);
    p.checkboxes()[1].props.onChange(); p.checkboxes()[2].props.onChange(); // Select B instead of A.
    if (name === 'GroupAccountPickerModal') {
      p.find((node) => node.props.id === 'group-account-name').props.onChange({ target: { value: 'Edited' } });
      p.find((node) => node.type === 'filter').props.onToggleValue('PRO');
      p.find((node) => node.type === 'tags').props.onToggleTag('work');
    }
    p.fail(true); await p.save();
    p.search().props.onChange({ target: { value: 'b@example.com' } });
    p.props.accounts = structuredClone(p.accounts); p.props.targetGroup = structuredClone(p.props.targetGroup); p.h.flush();
    assert.equal(p.search().props.value, 'b@example.com');
    assert.equal(p.checkboxes()[1].props.checked, true);
    assert.ok(nodes(p.h.flush()).some((node) => typeof node.props.message === 'string' && node.props.message.includes('save failed')));
    assert.equal(p.closed(), 0);
    if (name === 'GroupAccountPickerModal') {
      assert.equal(p.find((node) => node.props.id === 'group-account-name').props.value, 'Edited');
      assert.deepEqual([...p.find((node) => node.type === 'filter').props.selectedValues], ['PRO']);
      assert.deepEqual([...p.find((node) => node.type === 'tags').props.selectedTags], ['work']);
    }
    p.fail(false); await p.save();
    assert.deepEqual([...p.saved[p.saved.length - 1].accountIds], ['b']);
    assert.equal(p.closed(), 1); p.h.unmount();
  });

  test(`${name}: late account loading preserves persisted members; reopen and target change use latest data`, () => {
    const p = picker(name, file, true);
    p.props.accounts = p.accounts; p.h.flush(); assert.equal(p.checkboxes()[1].props.checked, true);
    p.checkboxes()[2].props.onChange(); p.search().props.onChange({ target: { value: 'edited' } });
    p.props.isOpen = false; p.h.flush();
    p.props.targetGroup = { id: 'group', name: 'Latest', accountIds: ['b'] }; p.props.isOpen = true; p.h.flush();
    assert.equal(p.search().props.value, '');
    assert.equal(p.checkboxes()[1].props.checked, false); assert.equal(p.checkboxes()[2].props.checked, true);
    p.props.targetGroup = { id: 'other', name: 'Other', accountIds: ['a'] }; p.h.flush();
    assert.equal(p.checkboxes()[1].props.checked, true); assert.equal(p.checkboxes()[2].props.checked, false);
    p.h.unmount();
  });
}
