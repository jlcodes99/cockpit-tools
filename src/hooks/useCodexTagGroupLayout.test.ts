import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { mergeIdLists, USER_MEMORY_LISTS } from '../utils/userMemory';

const groups = (...keys: string[]) => keys.map((groupKey) => ({ groupKey }));

test('tag groups fold across pages and keep filtered groups in the saved drag order', () => {
  let page = groups('alpha', 'gamma');
  const availableTags = ['alpha', 'beta', 'gamma'];
  const writes: Array<[string, string[]]> = [];
  let hovered = 'alpha';
  const h = loadHookModule(new URL('./useCodexTagGroupLayout.ts', import.meta.url), {
    '../utils/userMemory': {
      hydrateUserMemory: async () => {},
      subscribeUserMemory: () => () => {},
      readUserMemoryList: () => [],
      persistUserMemoryList: (id: string, order: string[]) => writes.push([id, Array.from(order)]),
      mergeIdLists,
      USER_MEMORY_LISTS,
    },
  }, {
    document: { elementFromPoint: () => ({ closest: () => ({ getAttribute: () => hovered }) }) },
  });
  const layout = () => h.flush();
  const keys = () => Array.from(layout().visible, (group: { groupKey: string }) => group.groupKey);
  h.render(() => h.exports.useCodexTagGroupLayout(page, availableTags));
  assert.deepEqual(keys(), ['alpha', 'gamma']);

  layout().toggle('alpha');
  assert.equal(layout().isCollapsed('alpha'), true);
  assert.equal(layout().isCollapsed('gamma'), false);
  layout().toggleAll();
  assert.equal(layout().isCollapsed('beta'), true);
  layout().toggle('gamma');
  assert.equal(layout().isCollapsed('gamma'), false);
  layout().toggleAll();
  assert.equal(layout().isCollapsed('alpha'), false);
  assert.equal(layout().isCollapsed('beta'), false);

  const event = { button: 0, pointerId: 1, currentTarget: { setPointerCapture() {} }, preventDefault() {} };
  layout().startDrag(event, 'gamma');
  layout().dragOver(event);
  assert.equal(layout().dropTarget, 'alpha');
  layout().drop(event);
  assert.deepEqual(keys(), ['gamma', 'alpha']);
  assert.equal(layout().dropTarget, null);
  assert.deepEqual(writes, [[USER_MEMORY_LISTS.codexTagGroupOrder, ['__untagged__', 'gamma', 'alpha', 'beta']]]);
  page = groups('alpha', 'beta', 'gamma');
  assert.deepEqual(keys(), ['gamma', 'alpha', 'beta']);
  hovered = 'outside';
  layout().startDrag(event, 'gamma');
  layout().drop(event);
  assert.equal(writes.length, 1);
  layout().move('alpha', 'beta');
  assert.deepEqual(keys(), ['gamma', 'beta', 'alpha']);
  h.unmount();
});
