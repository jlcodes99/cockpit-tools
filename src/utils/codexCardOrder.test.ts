import assert from 'node:assert/strict';
import test from 'node:test';
import { mergeCodexCardOrder, reorderCodexCardIds } from './codexCardOrder.ts';

test('moves accounts to the first or last position without including the fixed API service card', () => {
  const accounts = ['a', 'b', 'c'];
  const fixedCard = 'api-service';
  assert.deepEqual(
    [fixedCard, ...reorderCodexCardIds(accounts, accounts, 'a', 'c')],
    [fixedCard, 'b', 'c', 'a'],
  );
  assert.deepEqual(
    [fixedCard, ...reorderCodexCardIds(accounts, accounts, 'c', 'a')],
    [fixedCard, 'c', 'a', 'b'],
  );
  assert.deepEqual(reorderCodexCardIds(accounts, accounts, fixedCard, 'a'), accounts);
  assert.deepEqual(reorderCodexCardIds(accounts, accounts, 'a', fixedCard), accounts);
});

test('moves across grid rows using account order rather than row coordinates', () => {
  const accounts = ['a', 'b', 'c', 'd', 'e', 'f'];
  assert.deepEqual(reorderCodexCardIds(accounts, accounts, 'b', 'e'), ['a', 'c', 'd', 'e', 'b', 'f']);
  assert.deepEqual(reorderCodexCardIds(accounts, accounts, 'e', 'b'), ['a', 'e', 'b', 'c', 'd', 'f']);
});

test('filtered accounts move only within their existing global slots', () => {
  const accounts = ['hidden-before', 'a', 'hidden-middle', 'b', 'c', 'hidden-after'];
  assert.deepEqual(
    reorderCodexCardIds(accounts, ['a', 'b', 'c'], 'c', 'a'),
    ['hidden-before', 'c', 'hidden-middle', 'a', 'b', 'hidden-after'],
  );
});

test('reordering a page leaves every account on other pages in place', () => {
  const accounts = ['page1-a', 'page1-b', 'a', 'b', 'c', 'page3-a'];
  assert.deepEqual(
    reorderCodexCardIds(accounts, ['a', 'b', 'c'], 'a', 'c'),
    ['page1-a', 'page1-b', 'b', 'c', 'a', 'page3-a'],
  );
  assert.deepEqual(reorderCodexCardIds(accounts, ['a', 'b', 'c'], 'a', 'page3-a'), accounts);
});

test('a tag group reorder does not move accounts outside that group', () => {
  const accounts = ['a', 'other-tag', 'b', 'c'];
  assert.deepEqual(reorderCodexCardIds(accounts, ['a', 'c'], 'c', 'a'), ['c', 'other-tag', 'b', 'a']);
});

test('stale or duplicate visible IDs cannot insert missing accounts or scramble the current order', () => {
  const accounts = ['a', 'b', 'c', 'd'];
  assert.deepEqual(
    reorderCodexCardIds(accounts, ['c', 'deleted', 'a', 'a', 'b'], 'a', 'b'),
    ['b', 'a', 'c', 'd'],
  );
  assert.deepEqual(reorderCodexCardIds(accounts, ['a', 'deleted', 'b'], 'a', 'deleted'), accounts);
});

test('empty, single-card, missing-source and unchanged drops preserve account order', () => {
  assert.deepEqual(reorderCodexCardIds([], [], 'a', 'b'), []);
  assert.deepEqual(reorderCodexCardIds(['a'], ['a'], 'a', 'a'), ['a']);
  const accounts = ['a', 'b', 'c'];
  for (const [source, target] of [['a', 'a'], ['missing', 'b'], ['a', 'missing'], ['c', 'b']]) {
    assert.deepEqual(reorderCodexCardIds(accounts, ['a', 'b'], source, target), accounts);
  }
});

test('reordering never mutates account or visible input arrays', () => {
  const accounts = Object.freeze(['a', 'hidden', 'b', 'c']);
  const visible = Object.freeze(['a', 'b', 'c']);
  assert.deepEqual(reorderCodexCardIds(accounts, visible, 'a', 'c'), ['b', 'hidden', 'c', 'a']);
  assert.deepEqual(accounts, ['a', 'hidden', 'b', 'c']);
  assert.deepEqual(visible, ['a', 'b', 'c']);
});

test('saving a reordered view preserves IDs for temporarily unloaded or deleted accounts', () => {
  assert.deepEqual(
    mergeCodexCardOrder(['a', 'unloaded', 'b', 'deleted'], ['b', 'new', 'a']),
    ['b', 'new', 'a', 'unloaded', 'deleted'],
  );
});

test('merging saved and displayed orders deduplicates IDs and supports empty views', () => {
  assert.deepEqual(mergeCodexCardOrder(['a', 'old', 'old'], ['b', 'b', 'a']), ['b', 'a', 'old']);
  assert.deepEqual(mergeCodexCardOrder(['old', 'old'], []), ['old']);
  assert.deepEqual(mergeCodexCardOrder([], ['new']), ['new']);
  assert.deepEqual(mergeCodexCardOrder([], []), []);
});

test('a saved order survives serialization and can include a restored account in the next reorder', () => {
  const saved = mergeCodexCardOrder(['a', 'restored', 'b'], ['b', 'a']);
  const reloaded = JSON.parse(JSON.stringify(saved)) as string[];
  assert.deepEqual(reloaded, ['b', 'a', 'restored']);
  assert.deepEqual(reorderCodexCardIds(reloaded, reloaded, 'restored', 'b'), ['restored', 'b', 'a']);
});
