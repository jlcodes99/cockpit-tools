import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('late loading and external updates hydrate untouched fields but preserve edits', () => {
  const h = loadHookModule(new URL('./useRefreshSafeDraft.ts', import.meta.url), {});
  let saved = '';
  h.render(() => h.exports.useRefreshSafeDraft(saved));
  saved = 'loaded'; assert.equal(h.flush()[0], 'loaded');
  h.flush()[1]('edited'); saved = 'new saved value';
  assert.equal(h.flush()[0], 'edited');
  saved = 'edited'; h.flush(); // The save was acknowledged; the draft is clean again.
  saved = 'updated elsewhere'; assert.equal(h.flush()[0], 'updated elsewhere');
  h.unmount();
});

test('unchanged refreshes preserve empty, false and zero edits', () => {
  for (const [saved, edit] of [['saved', ''], [true, false], [5, 0]]) {
    const h = loadHookModule(new URL('./useRefreshSafeDraft.ts', import.meta.url), {});
    h.render(() => h.exports.useRefreshSafeDraft(saved)); h.flush()[1](edit);
    for (let i = 0; i < 3; i++) assert.equal(h.flush()[0], edit);
    h.unmount();
  }
});

test('key labels merge independently and handle additions, deletion and save acknowledgement', () => {
  const h = loadHookModule(new URL('./useRefreshSafeDraft.ts', import.meta.url), {});
  let saved: Record<string, string> = { a: 'A', b: 'B', deleted: 'old' };
  h.render(() => h.exports.useRefreshSafeRecordDraft(saved));
  h.flush()[1]((drafts: object) => ({ ...drafts, a: 'edited' }));
  saved = { a: 'A', b: 'updated B', added: 'new' };
  let drafts = h.flush()[0];
  assert.equal(drafts.a, 'edited'); assert.equal(drafts.b, 'updated B');
  assert.equal(drafts.added, 'new'); assert.equal(drafts.deleted, undefined);
  saved = { ...saved }; assert.equal(h.flush()[0], drafts, 'identical refreshes must not cause a render loop');
  saved = { ...saved, a: 'edited' }; h.flush();
  saved = { ...saved, a: 'updated A' }; drafts = h.flush()[0];
  assert.equal(drafts.a, 'updated A'); h.unmount();
});
