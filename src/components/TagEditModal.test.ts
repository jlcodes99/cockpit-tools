import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule } from '../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
const t = (key: string) => key;

function setup(onSave: (tags: string[], notes?: string) => Promise<void>) {
  const escape: any[] = [];
  const traps: any[] = [];
  const props = { isOpen: true, initialTags: ['existing'], availableTags: ['existing', 'fast'],
    onClose: () => { closed++; }, onSave };
  let closed = 0;
  const h = loadHookModule(new URL('./TagEditModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '../utils/globalTagOperations': { globalRenameTag: async () => {}, globalDeleteTag: async () => {} },
    '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void, ref: any) => { escape.push({ open, close, ref }); } },
    '../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean, restore = true, active = true) => { traps.push({ ref, open, restore, active }); } },
    '../hooks/useEnterConfirm': { useEnterConfirm() {} },
    './ModalErrorMessage': { ModalErrorMessage: 'modal-error' },
  });
  const tree = () => { escape.length = 0; traps.length = 0; return h.flush(); };
  h.render(() => h.exports.TagEditModal(props));
  const find = (predicate: (node: any) => boolean) => nodes(tree()).find(predicate);
  const save = () => find((node) => node.type === 'button' && ['accounts.tagModal.save', 'common.saving'].includes(node.props.children));
  return { h, tree, find, save, escape, traps, closed: () => closed };
}

test('tag save failure keeps the draft and dialog open, clears on retry and guards duplicate saves', async () => {
  const first = deferred<void>();
  const second = deferred<void>();
  const saved: string[][] = [];
  const s = setup(async (tags) => { saved.push(Array.from(tags)); await (saved.length === 1 ? first.promise : second.promise); });
  const input = s.find((node) => node.type === 'input');
  input.props.onChange({ target: { value: ' Reviewed ' } });
  const saving = s.save().props.onClick();
  assert.equal(s.save().props.disabled, true);
  await s.save().props.onClick();
  assert.equal(saved.length, 1);
  assert.equal(s.find((node) => node.props.className === 'modal-close').props.disabled, true);
  assert.equal(s.escape[0].open, false);
  first.reject('disk write failed');
  await saving;
  assert.equal(s.closed(), 0);
  assert.equal(s.find((node) => node.type === 'modal-error').props.message, 'disk write failed');
  assert.equal(s.find((node) => node.type === 'input').props.value, ' Reviewed ');
  const retrying = s.save().props.onClick();
  assert.equal(s.find((node) => node.type === 'modal-error').props.message, '');
  second.resolve();
  await retrying;
  assert.equal(s.closed(), 1);
  assert.deepEqual(saved, [['existing', 'reviewed'], ['existing', 'reviewed']]);
  s.h.unmount();
});

test('nested tag dialogs trap focus and isolate parent keys while Escape targets only the top dialog', () => {
  const s = setup(async () => {});
  const dialog = s.find((node) => node.props.role === 'dialog');
  assert.equal(dialog.props.ref, s.traps[0].ref);
  assert.equal(dialog.props.ref, s.escape[0].ref);
  assert.equal(dialog.props.tabIndex, -1);
  assert.equal(s.traps[0].restore, true);
  assert.equal(s.escape[0].open, true);
  const overlay = s.find((node) => node.props.className === 'modal-overlay');
  assert.equal(overlay.props.onClick, undefined);
  let stopped = false;
  overlay.props.onKeyDown({ stopPropagation() { stopped = true; } });
  assert.equal(stopped, true);
  s.find((node) => node.props.className === 'tag-suggestion-delete-btn').props.onClick({ stopPropagation() {} });
  const confirm = s.find((node) => node.props.className === 'modal tag-global-delete-confirm-modal');
  assert.equal(confirm.props.ref, s.traps[1].ref);
  assert.equal(s.traps[0].active, false);
  assert.equal(s.escape[0].open, false);
  assert.equal(s.escape[1].open, true);
  s.escape[1].close();
  assert.equal(s.find((node) => node.props.className === 'modal tag-global-delete-confirm-modal'), undefined);
  assert.equal(s.closed(), 0);
  assert.equal(s.escape[0].open, true);
  s.escape[0].close();
  assert.equal(s.closed(), 1);
  s.h.unmount();
});
