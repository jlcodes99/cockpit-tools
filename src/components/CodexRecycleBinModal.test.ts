import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { CodexRecycledAccount } from '../services/codexRecycleBinService';

type Element = { type: unknown; props: Record<string, any> };
const entry = (id: string): CodexRecycledAccount => ({ id, account_id: `account-${id}`, email: `${id}@example.com`, deleted_at: 100 });
const t = (key: string, args?: unknown) => args ? `${key}:${JSON.stringify(args)}` : key;
function nodes(tree: unknown): Element[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  if (!tree || typeof tree !== 'object' || !('props' in tree)) return [];
  const node = tree as Element;
  return [node, ...nodes(node.props.children)];
}
function text(tree: unknown): string {
  if (typeof tree === 'string') return tree;
  if (Array.isArray(tree)) return tree.map(text).join('');
  return tree && typeof tree === 'object' && 'props' in tree ? text((tree as Element).props.children) : '';
}
function harness() {
  let entries = [entry('A'), entry('B')];
  const pending = deferred<void>();
  const exportPending = deferred<boolean>();
  const calls: { kind: string; value: unknown }[] = [];
  let listError: Error | null = null;
  let restoreRefreshError: Error | null = null;
  let closed = 0;
  let restored = 0;
  const error = { message: null as string | null, scrollKey: 0,
    clear() { error.message = null; }, report(message: string) { error.message = message; error.scrollKey += 1; } };
  const h = loadHookModule(new URL('./CodexRecycleBinModal.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t }) },
    './ModalErrorMessage': { ModalErrorMessage: 'modal-error', useModalErrorState: () => error },
    '../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../services/codexRecycleBinService': {
      async listCodexRecycledAccounts() { if (listError) throw listError; return entries; },
      restoreCodexRecycledAccount(id: string) { calls.push({ kind: 'restore', value: id }); return pending.promise; },
      deleteCodexRecycledAccount(id: string) { calls.push({ kind: 'delete', value: id }); return pending.promise; },
      emptyCodexRecycleBin(ids: string[]) { calls.push({ kind: 'empty', value: ids }); return pending.promise; },
      saveCodexRecycledAccounts(ids: string[]) { calls.push({ kind: 'export', value: ids }); return exportPending.promise; },
    },
  }, { document: { body: {} } });
  h.render(() => h.exports.CodexRecycleBinModal({ onClose() { closed += 1; }, async onRestored() { restored += 1; if (restoreRefreshError) throw restoreRefreshError; }, maskAccountText: () => 'MASKED' }));
  const button = (label: string) => {
    const match = nodes(h.flush()).find((node) => node.type === 'button' && text(node.props.children) === `common.${label}`);
    assert.ok(match, `Missing ${label}`); return match;
  };
  return { h, button, calls, pending, exportPending, error, failReads() { listError = new Error("read failure"); restoreRefreshError = new Error("refresh failure"); }, setEntries(value: CodexRecycledAccount[]) { entries = value; }, closed: () => closed, restored: () => restored };
}

test('clear requires confirmation and only removes the confirmed snapshot', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.emptyBin').props.onClick();
  assert.equal(h.calls.length, 0);
  h.setEntries([entry('A'), entry('B'), entry('new')]);
  const confirm = h.button('recycleBin.emptyBin');
  confirm.props.onClick(); confirm.props.onClick();
  assert.equal(h.calls.length, 1, 'same-tick double click is deduplicated');
  assert.equal(h.calls[0].kind, 'empty');
  assert.deepEqual(Array.from(h.calls[0].value as string[]), ['A', 'B']);
  h.pending.resolve(); await settlePromises();
  assert.equal(h.error.message, null);
  h.h.unmount();
});

test('permanent deletion failures stay in the modal and retry clears the old error', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.deletePermanently').props.onClick();
  assert.equal(h.calls.length, 0);
  assert.match(text(h.h.flush()), /MASKED/);
  assert.doesNotMatch(text(h.h.flush()), /A@example.com/);
  h.button('recycleBin.deletePermanently').props.onClick();
  h.pending.reject(new Error('disk failure')); await settlePromises();
  assert.match(h.error.message!, /disk failure/);
  assert.equal(h.closed(), 0);
  assert.equal(nodes(h.h.flush()).filter((node) => node.type === 'modal-error' && node.props.message).length, 1);
  h.button('cancel').props.onClick();
  assert.equal(h.error.message, null);
  h.h.unmount();
});

test('restore remains closeable while pending and refreshes accounts after the dialog closes', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.restore').props.onClick();
  assert.equal(h.calls[0].kind, 'restore');
  assert.equal(h.button('recycleBin.restore').props.disabled, true);
  const overlay = nodes(h.h.flush()).find((node) => node.props.className === 'modal-overlay codex-recycle-bin-overlay');
  assert.equal(overlay?.props.onClick, undefined);
  h.button('close').props.onClick(); h.h.unmount();
  assert.equal(h.closed(), 1);
  h.pending.resolve(); await settlePromises();
  assert.equal(h.restored(), 1);
});


test('successful restoration removes its row even if subsequent reads fail', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.restore').props.onClick();
  h.failReads();
  h.pending.resolve(); await settlePromises();
  assert.equal(h.restored(), 1);
  assert.equal(h.error.message, 'common.recycleBin.refreshFailed');
  const remainingRows = nodes(h.h.flush()).filter((node) => node.type === 'li');
  assert.equal(remainingRows.length, 1);
  h.h.unmount();
});

test('export then empty waits for a saved file and keeps the confirmed snapshot', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.emptyBin').props.onClick();
  const confirm = h.button('recycleBin.exportAndEmpty');
  confirm.props.onClick(); confirm.props.onClick();
  assert.deepEqual(h.calls.map((call) => call.kind), ['export']);
  h.setEntries([entry('new')]);
  h.exportPending.resolve(true); await settlePromises();
  assert.deepEqual(h.calls.map((call) => call.kind), ['export', 'empty']);
  assert.deepEqual(Array.from(h.calls[1].value as string[]), ['A', 'B']);
  h.pending.resolve(); await settlePromises();
  assert.equal(nodes(h.h.flush()).filter((node) => node.type === 'li').length, 1);
  assert.match(text(h.h.flush()), /common.recycleBin.exported/);
  h.h.unmount();
});

test('cancelling file save leaves the confirmation and all accounts untouched', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.emptyBin').props.onClick();
  h.button('recycleBin.exportAndEmpty').props.onClick();
  h.exportPending.resolve(false); await settlePromises();
  assert.deepEqual(h.calls.map((call) => call.kind), ['export']);
  assert.equal(h.button('recycleBin.exportAndEmpty').props.disabled, false);
  h.button('cancel').props.onClick();
  assert.equal(nodes(h.h.flush()).filter((node) => node.type === 'li').length, 2);
  assert.equal(h.error.message, null);
  assert.doesNotMatch(text(h.h.flush()), /common.recycleBin.exported/);
  h.h.unmount();
});

test('failed export never deletes an account and displays the failure inside the dialog', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.deletePermanently').props.onClick();
  h.button('recycleBin.exportAndDelete').props.onClick();
  h.exportPending.reject(new Error('export disk full')); await settlePromises();
  assert.deepEqual(h.calls.map((call) => call.kind), ['export']);
  assert.match(h.error.message!, /export disk full/);
  assert.equal(h.closed(), 0);
  assert.equal(h.button('recycleBin.exportAndDelete').props.disabled, false);
  h.h.unmount();
});

test('single-account export then delete targets only that snapshot; deletion failure retains export feedback', async () => {
  const h = harness(); await settlePromises();
  h.button('recycleBin.deletePermanently').props.onClick();
  h.button('recycleBin.exportAndDelete').props.onClick();
  assert.deepEqual(Array.from(h.calls[0].value as string[]), ['A']);
  h.exportPending.resolve(true); await settlePromises();
  assert.equal(h.calls[1].kind, 'delete');
  assert.equal(h.calls[1].value, 'A');
  h.pending.reject(new Error('delete failed')); await settlePromises();
  assert.match(h.error.message!, /delete failed/);
  assert.match(text(h.h.flush()), /common.recycleBin.exported/);
  assert.equal(h.closed(), 0);
  h.h.unmount();
});

for (const all of [false, true]) {
  test(`standalone ${all ? 'all' : 'single'} export does not remove or restore accounts`, async () => {
    const h = harness(); await settlePromises();
    h.button(all ? 'recycleBin.exportAll' : 'recycleBin.export').props.onClick();
    assert.deepEqual(Array.from(h.calls[0].value as string[]), all ? ['A', 'B'] : ['A']);
    h.exportPending.resolve(true); await settlePromises();
    assert.deepEqual(h.calls.map((call) => call.kind), ['export']);
    assert.equal(nodes(h.h.flush()).filter((node) => node.type === 'li').length, 2);
    assert.equal(h.restored(), 0);
    assert.match(text(h.h.flush()), /common.recycleBin.exported/);
    h.h.unmount();
  });
}
