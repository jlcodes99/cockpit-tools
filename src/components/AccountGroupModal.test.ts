import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

type Element = { type: unknown; props: Record<string, any> };
const t = (key: string, args?: unknown) => args && typeof args === 'object' ? `${key}:${JSON.stringify(args)}` : key;
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
const group = (id: string) => ({ id, name: id, accountIds: [], createdAt: 1, quotaAutoRefreshMinutes: null });
function harness(add = false) {
  let readError: Error | null = new Error('read failed');
  let reorderError: Error | null = null;
  let assignMissing = false;
  let closed = 0;
  let entries = [group('A'), group('B')];
  const calls: string[] = [];
  const quotaCalls: unknown[] = [];
  let pendingRead: Promise<typeof entries> | null = null;
  const h = loadHookModule(new URL('./AccountGroupModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    '../hooks/useEscClose': { useEscClose() {} },
    '../services/codexAccountGroupService': {
      resolveCodexGroupQuotaAutoRefreshMinutes: (value: any) => value.quotaAutoRefreshMinutes ?? null,
      normalizeCodexGroupQuotaAutoRefreshMinutes: (value: unknown) => Math.min(999, Math.max(1, Number(value))),
      async setCodexGroupQuotaAutoRefreshMinutes(id: string, minutes: number | null) {
        quotaCalls.push(minutes); return { ...group(id), quotaAutoRefreshMinutes: minutes };
      },
    },
    '../services/platformGroupService': {
      normalizePlatform: (value: string) => value,
      async getPlatformGroups() { if (pendingRead) return pendingRead; if (readError) throw readError; return entries; },
      async createPlatformGroup(_platform: string, name: string) { calls.push('create'); const value = group(name); entries = [...entries, value]; return value; },
      async reorderPlatformGroups(_platform: string, ids: string[]) {
        calls.push('reorder'); if (reorderError) throw reorderError;
        entries = ids.map((id) => ({ ...group(id), name: `${id}-saved` })); return entries;
      },
      async assignAccountsToPlatformGroup(_platform: string, id: string) { calls.push('assign'); return assignMissing ? null : group(id); },
    },
  });
  const props = { isOpen: true, platform: 'codex', onClose() { closed += 1; }, onGroupsChanged() {}, onAdded() {}, accountIds: ['account'] };
  h.render(() => add ? h.exports.AddToGroupModal(props) : h.exports.AccountGroupModal(props));
  const find = (predicate: (value: Element) => boolean) => {
    const node = nodes(h.flush()).find(predicate); assert.ok(node); return node;
  };
  const button = (label: string) => find((node) => node.type === 'button' && (text(node.props.children) === label || node.props.title === label));
  return { h, props, find, button, calls, quotaCalls, closed: () => closed,
    allowRead() { readError = null; }, failReorder(value: boolean) { reorderError = value ? new Error('write failed') : null; },
    assignMissing(value: boolean) { assignMissing = value; }, pendingRead(value: Promise<typeof entries>) { pendingRead = value; },
  };
}

for (const add of [false, true]) {
  test(`${add ? 'assignment' : 'management'} load failure stays open, blocks writes and retries`, async () => {
    const h = harness(add); await settlePromises();
    assert.match(text(h.h.flush()), /read failed/);
    assert.doesNotMatch(text(h.h.flush()), /accounts.groups.empty/);
    const input = h.find((node) => node.type === 'input');
    assert.equal(input.props.disabled, true);
    input.props.onChange({ target: { value: 'new' } });
    h.find((node) => node.type === 'input').props.onKeyDown({ key: 'Enter' });
    await settlePromises();
    assert.deepEqual(h.calls, []);
    // Re-enter the failure state after the synthetic input event cleared feedback.
    h.props.isOpen = false; h.h.flush(); h.props.isOpen = true; h.h.flush(); await settlePromises();
    h.allowRead(); h.button('common.retry').props.onClick(); await settlePromises();
    assert.doesNotMatch(text(h.h.flush()), /read failed/);
    assert.equal(h.find((node) => node.type === 'input').props.disabled, false);
    assert.equal(h.closed(), 0);
    h.h.unmount();
  });
}

test('failed reorder retains original rows and retries with latest service snapshot', async () => {
  const h = harness(); h.allowRead(); await settlePromises(); h.button('common.retry').props.onClick(); await settlePromises();
  h.failReorder(true);
  await h.button('accounts.groups.moveDown').props.onClick();
  assert.match(text(h.h.flush()), /write failed/);
  assert.deepEqual(nodes(h.h.flush()).filter((node) => node.props.className === 'group-name').map((node) => text(node)), ['A', 'B']);
  h.failReorder(false); await h.button('accounts.groups.moveDown').props.onClick();
  assert.doesNotMatch(text(h.h.flush()), /write failed/);
  assert.deepEqual(nodes(h.h.flush()).filter((node) => node.props.className === 'group-name').map((node) => text(node)), ['B-saved', 'A-saved']);
  assert.equal(h.closed(), 0); h.h.unmount();
});

test('missing target group is an assignment error and keeps the dialog open', async () => {
  const h = harness(true); await settlePromises(); h.allowRead(); h.button('common.retry').props.onClick(); await settlePromises();
  h.assignMissing(true); await h.button('A').props.onClick();
  assert.match(text(h.h.flush()), /accounts.groups.error.notFound/);
  assert.equal(h.closed(), 0);
  h.assignMissing(false); await h.button('A').props.onClick();
  assert.equal(h.closed(), 1); h.h.unmount();
});

test('Codex quota controls retain disabled, inherited and custom minutes', async () => {
  const h = harness(); await settlePromises(); h.allowRead(); h.button('common.retry').props.onClick(); await settlePromises();
  h.find((node) => node.type === 'dropdown').props.onChange('-1'); await settlePromises();
  assert.equal(h.find((node) => node.type === 'dropdown').props.value, '-1');
  h.find((node) => node.type === 'dropdown').props.onChange('inherit'); await settlePromises();
  h.find((node) => node.type === 'dropdown').props.onChange('custom');
  h.find((node) => node.type === 'input' && node.props.type === 'number').props.onChange({ target: { value: '37' } });
  h.find((node) => node.type === 'input' && node.props.type === 'number').props.onKeyDown({ key: 'Enter', preventDefault() {} });
  await settlePromises();
  assert.deepEqual(h.quotaCalls, [-1, null, 37]);
  assert.equal(h.find((node) => node.type === 'dropdown').props.value, '37');
  h.h.unmount();
});

test('a pending read is ignored after the dialog closes', async () => {
  const h = harness(); await settlePromises();
  const pending = deferred<ReturnType<typeof group>[]>(); h.pendingRead(pending.promise);
  h.button('common.retry').props.onClick();
  h.props.isOpen = false; h.h.flush();
  pending.resolve([group('stale')]); await settlePromises();
  assert.equal(h.h.flush(), null); h.h.unmount();
});
