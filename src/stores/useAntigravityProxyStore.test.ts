import assert from 'node:assert/strict';
import test from 'node:test';
import { createAntigravityProxyStore } from './useAntigravityProxyStore';
import type { AntigravityProxyView } from '../services/antigravityProxyService';
import { deferred } from '../../tests/helpers/reactHookHarness';

const view = (id: string, legacyBound = false): AntigravityProxyView => ({ accountId: id, binding: null, error: null, legacyBound });

test('cards share one pending list request, and a completed list removes deleted accounts', async () => {
  const read = deferred<AntigravityProxyView[]>();
  let calls = 0;
  const store = createAntigravityProxyStore({ list: () => { calls++; return read.promise; } });
  const first = store.getState().reload();
  assert.equal(store.getState().reload(), first);
  assert.equal(calls, 1);
  read.resolve([view('a')]); await first;
  assert.deepEqual(Object.keys(store.getState().views), ['a']);
  store.getState().accept(view('deleted'));
  await store.getState().reload();
  assert.deepEqual(Object.keys(store.getState().views), ['a']);
});

test('late list reads cannot overwrite an account saved while the request was running', async () => {
  const read = deferred<AntigravityProxyView[]>();
  const store = createAntigravityProxyStore({ list: () => read.promise });
  const request = store.getState().reload();
  const saved = view('a', true);
  store.getState().accept(saved);
  read.resolve([view('a'), view('b')]); await request;
  assert.equal(store.getState().views.a, saved);
  assert.equal(store.getState().views.b.accountId, 'b');
});

test('a failed list preserves saved metadata and permits a retry', async () => {
  let fail = true;
  const store = createAntigravityProxyStore({ list: async () => { if (fail) throw new Error('secret must not render'); return [view('a')]; } });
  store.getState().accept(view('a', true));
  await store.getState().reload();
  assert.equal(store.getState().failed, true);
  assert.equal(store.getState().views.a.legacyBound, true);
  fail = false; await store.getState().reload();
  assert.equal(store.getState().failed, false);
  assert.equal(store.getState().loading, false);
});
