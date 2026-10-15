import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function harness() {
  let sources = ['a', 'b', 'c'].map((id) => ({ id, name: id }));
  let disabled = false;
  const commits: string[][] = [];
  const listeners = new Map<string, (event?: any) => void>();
  const h = loadHookModule(new URL('./useProxyResourceOrder.ts', import.meta.url), {}, {
    window: { addEventListener: (type: string, fn: () => void) => listeners.set(type, fn), removeEventListener: (type: string) => listeners.delete(type) },
  });
  const render = () => h.exports.useProxyResourceOrder(sources, disabled, (ids: string[]) => commits.push(Array.from(ids)));
  h.render(render);
  return { h, commits, listeners, state: () => h.flush(), ids: () => Array.from(h.flush().orderedSources, (item: any) => item.id),
    sources(next: typeof sources) { sources = next; h.render(render); }, disabled(value: boolean) { disabled = value; h.render(render); } };
}

test('top-layout style dragging previews immediately but saves once on release', () => {
  const h = harness(); h.state().begin('a', 0); h.state().hover('b', 1); h.state().hover('c', 1);
  assert.deepEqual(h.ids(), ['b', 'c', 'a']); assert.equal(h.state().draggingId, 'a'); assert.equal(h.commits.length, 0);
  h.listeners.get('mouseup')!(); h.listeners.get('blur')!();
  assert.deepEqual(h.commits, [['b', 'c', 'a']]); assert.equal(h.state().draggingId, null); h.h.unmount();
});

test('unchanged order, invalid handles and disabled sorting never save', () => {
  const h = harness(); h.state().begin('a', 2); h.state().hover('c', 1); h.state().finish();
  h.state().begin('missing', 0); h.state().hover('c', 1);
  h.state().begin('a', 0); h.state().hover('b', 1); h.state().hover('b', 1); h.state().finish();
  h.disabled(true); h.state().begin('a', 0); h.state().moveBy('b', 1); h.state().finish();
  assert.deepEqual(h.commits, []); h.h.unmount();
});

test('Escape cancels the preview without persisting', () => {
  const h = harness(); h.state().begin('c', 0); h.state().hover('a', 1);
  h.listeners.get('keydown')!({ key: 'Escape' });
  assert.deepEqual(h.ids(), ['a', 'b', 'c']); h.listeners.get('mouseup')!(); assert.deepEqual(h.commits, []); h.h.unmount();
});

test('leaving the list, window blur and a lost mouseup all finish the last preview', () => {
  for (const finish of ['leave', 'blur', 'buttons']) {
    const h = harness(); h.state().begin('c', 0); h.state().hover('a', 1);
    if (finish === 'leave') h.state().finish(); else if (finish === 'blur') h.listeners.get('blur')!(); else h.state().hover('b', 0);
    assert.deepEqual(h.commits, [['c', 'a', 'b']]); h.h.unmount();
  }
});

test('membership changes or starting a filtered/busy state cancel a pending drag', () => {
  for (const reason of ['sources', 'disabled']) {
    const h = harness(); h.state().begin('a', 0); h.state().hover('c', 1);
    if (reason === 'sources') h.sources(['a', 'b', 'c', 'new'].map((id) => ({ id, name: id }))); else h.disabled(true);
    h.listeners.get('mouseup')!(); assert.deepEqual(h.commits, []); assert.equal(h.state().draggingId, null); h.h.unmount();
  }
});

test('metadata refreshes remain visible during dragging without invalidating order', () => {
  const h = harness(); h.state().begin('a', 0); h.state().hover('c', 1);
  h.sources(['a', 'b', 'c'].map((id) => ({ id, name: `updated-${id}` })));
  assert.deepEqual(h.ids(), ['b', 'c', 'a']); assert.equal(h.state().orderedSources[2].name, 'updated-a');
  h.state().finish(); assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.h.unmount();
});

test('keyboard and menu moves retain all identities and respect list boundaries', () => {
  const h = harness(); h.state().moveBy('a', -1); h.state().moveBy('c', 1); h.state().moveBy('missing', 1);
  assert.equal(h.commits.length, 0); h.state().moveBy('b', -1);
  assert.deepEqual(h.commits, [['b', 'a', 'c']]); h.h.unmount();
});

test('unmount removes global listeners and never commits an unfinished drag', () => {
  const h = harness(); h.state().begin('a', 0); h.state().hover('c', 1); h.h.unmount();
  assert.equal(h.listeners.size, 0); assert.deepEqual(h.commits, []);
});
