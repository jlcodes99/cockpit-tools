import assert from 'node:assert/strict';
import test from 'node:test';
import { createPelicanWriteQueue, mergePelicanSnapshot } from './pelicanState.ts';
import { isPelicanRunning, pelicanCompletedCount, type CodexPelicanBatch } from '../../../types/codexPelican.ts';

const batch = (id = 'one', revision = 1, createdAt = 1): CodexPelicanBatch => ({
  id, revision, createdAt, status: 'running', prompt: 'test', model: 'model', effort: 'high', concurrency: 3, items: [],
});

test('a late invoke snapshot cannot roll back a streamed batch', () => {
  const latest = batch('one', 10);
  assert.equal(mergePelicanSnapshot(latest, batch('one', 2), new Set()), latest);
  assert.equal(mergePelicanSnapshot(latest, batch('one', 11), new Set())?.revision, 11);
});

test('a dismissed batch cannot be resurrected by an already queued event', () => {
  assert.equal(mergePelicanSnapshot(null, batch(), new Set(['one'])), null);
  const latest = batch('two', 1, 2);
  assert.equal(mergePelicanSnapshot(latest, batch(), new Set()), latest);
});

test('progress counts all terminal results, not just successful HTML', () => {
  const value = batch();
  value.items = ['queued', 'running', 'completed', 'failed', 'cancelled', 'interrupted'].map((status, index) => ({
    id: String(index), accountId: String(index), accountEmail: '', status: status as CodexPelicanBatch['items'][number]['status'], hasHtml: false,
  }));
  assert.equal(pelicanCompletedCount(value), 4);
  assert.equal(isPelicanRunning({ ...value, status: 'cancelling' }), true);
  assert.equal(isPelicanRunning({ ...value, status: 'cancelled' }), false);
});

test('organization writes are serialized and failures do not poison later edits', async () => {
  const queue = createPelicanWriteQueue();
  const events: string[] = [];
  let release!: () => void;
  const first = queue(async () => {
    events.push('first-start');
    await new Promise<void>((resolve) => { release = resolve; });
    events.push('first-end');
  });
  const second = queue(async () => { events.push('second'); throw new Error('disk'); });
  const secondFailure = assert.rejects(second, /disk/);
  const third = queue(async () => { events.push('third'); });
  await Promise.resolve();
  assert.deepEqual(events, ['first-start']);
  release();
  await Promise.all([first, secondFailure, third]);
  assert.deepEqual(events, ['first-start', 'first-end', 'second', 'third']);
});
