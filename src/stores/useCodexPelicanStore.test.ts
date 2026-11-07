import assert from 'node:assert/strict';
import test from 'node:test';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { useCodexPelicanStore as store } from './useCodexPelicanStore.ts';
import { CODEX_PELICAN_DEFAULT_PROMPT, isPelicanRunning, type CodexPelicanBatch } from '../types/codexPelican.ts';

const batch = (status: CodexPelicanBatch['status'] = 'completed'): CodexPelicanBatch => ({
  id: 'batch', revision: 3, createdAt: 1, status, prompt: 'Original pelican prompt',
  model: 'gpt-6-astra', effort: 'high', concurrency: 2,
  items: ['a', 'b'].map((accountId) => ({ id: accountId, accountId, accountEmail: '', status: 'completed', hasHtml: false })),
});

async function withIPC(callback: Parameters<typeof mockIPC>[0], run: () => Promise<void>) {
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {} });
  try {
    mockIPC(callback);
    await run();
  } finally {
    clearMocks();
    if (windowDescriptor) Object.defineProperty(globalThis, 'window', windowDescriptor);
    else Reflect.deleteProperty(globalThis, 'window');
  }
}

test.beforeEach(() => store.setState(store.getInitialState(), true));

test('provider and account entries preserve their own target modes', () => {
  const target = { providerId: 'provider', apiKeyId: 'key', model: 'custom' };
  store.getState().openProviders([target]);
  assert.equal(store.getState().sourceMode, 'providers');
  assert.deepEqual(store.getState().providerTargets, [target]);
  store.getState().open(['account']);
  assert.equal(store.getState().sourceMode, 'accounts');
  assert.deepEqual(store.getState().selectedAccountIds, ['account']);
});

test('provider entry cannot replace an active account batch or pending start', () => {
  for (const status of ['running', 'cancelling'] as const) {
    store.setState({ active: batch(status), sourceMode: 'accounts' });
    store.getState().openProviders([{ providerId: 'p', apiKeyId: 'k', model: 'm' }]);
    assert.equal(store.getState().view, 'results');
    assert.equal(store.getState().sourceMode, 'accounts');
    assert.deepEqual(store.getState().providerTargets, []);
  }
});

test('editing provider history restores provider, key and per-item model references', () => {
  const source = batch();
  source.items[0].provider = { providerId: 'p', apiKeyId: 'k', model: 'custom', providerName: 'Relay',
    apiKeyName: 'Key', baseUrl: 'https://example.test/v1', wireApi: 'responses' };
  store.getState().editBatch(source);
  assert.equal(store.getState().sourceMode, 'providers');
  assert.deepEqual(store.getState().providerTargets, [{ providerId: 'p', apiKeyId: 'k', model: 'custom' }]);
});

test('opening from selected accounts shows an editable setup even after a completed test', () => {
  store.setState({ active: batch() });
  store.getState().open(['a']);
  assert.equal(store.getState().view, 'setup');
  assert.deepEqual(store.getState().selectedAccountIds, ['a']);
  assert.equal(store.getState().draft.prompt, CODEX_PELICAN_DEFAULT_PROMPT);
  store.getState().open();
  assert.equal(store.getState().view, 'results', 'floating entry still reopens the active result');
});

test('editing a past prompt copies settings into a draft and leaves original results untouched', () => {
  const source = batch();
  store.setState({ active: source });
  store.getState().editBatch(source);
  assert.equal(store.getState().view, 'setup');
  assert.deepEqual(store.getState().selectedAccountIds, ['a', 'b']);
  assert.deepEqual(store.getState().draft, { prompt: source.prompt, model: source.model, effort: source.effort, concurrency: source.concurrency });
  store.getState().updateDraft({ prompt: 'Edited prompt' });
  store.getState().minimize();
  store.getState().show('history');
  store.getState().show('setup');
  assert.equal(store.getState().draft.prompt, 'Edited prompt');
  assert.equal(source.prompt, 'Original pelican prompt');
  assert.equal(store.getState().active, source);
});

test('running or cancelling tests cannot be replaced through the prompt editor', () => {
  for (const status of ['running', 'cancelling'] as const) {
    store.setState({ active: batch(status), view: 'results' });
    store.getState().editBatch(batch());
    assert.equal(store.getState().view, 'results');
    store.getState().open(['b']);
    assert.equal(store.getState().view, 'results');
  }
  store.setState({ active: null, starting: true });
  store.getState().editBatch(batch());
  assert.equal(store.getState().view, 'results');
});

test('manual retry reactivates a dismissed historical batch without losing newer progress', () => {
  const latest = { ...batch('running'), revision: 8 };
  store.setState({ active: { ...batch(), id: 'newer', createdAt: 5 }, batch: latest, dismissedIds: new Set(['batch']) });
  store.getState().resumeBatch({ ...batch('running'), revision: 7 });
  assert.equal(store.getState().active, latest);
  assert.equal(store.getState().batch, latest);
  assert.equal(store.getState().dismissedIds.has('batch'), false);
});

test('late snapshots cannot displace a resumed historical batch or its stop controls', () => {
  const later = { ...batch(), id: 'later', createdAt: 5 };
  store.getState().receive(later);
  store.getState().resumeBatch(batch('running'));
  for (const status of ['running', 'completed'] as const) {
    store.getState().receive({ ...later, status, revision: 4 });
    const state = store.getState();
    assert.equal(state.active?.id, 'batch');
    assert.equal(state.batch?.id, state.active?.id, 'the stop footer remains attached to the displayed batch');
    assert.equal(isPelicanRunning(state.active), true, 'the active batch continues to be polled');
  }
  store.getState().receive({ ...batch('running'), revision: 8 });
  store.getState().receive({ ...batch('running'), revision: 6 });
  assert.equal(store.getState().active?.revision, 8);
  assert.equal(store.getState().batch?.revision, 8);
});

test('completion does not release the explicitly resumed batch to stale foreign snapshots', () => {
  store.getState().resumeBatch(batch('running'));
  store.getState().receive({ ...batch(), revision: 8 });
  store.getState().receive({ ...batch(), id: 'later', createdAt: 5 });
  assert.equal(store.getState().active?.id, 'batch');
  assert.equal(store.getState().active?.status, 'completed');
});

test('an explicit start changes activation without losing progress that beats the invoke response', async () => {
  const previous = { ...batch(), id: 'previous', createdAt: 100 };
  const next = { ...batch('running'), id: 'next', createdAt: 2, revision: 1 };
  store.getState().resumeBatch(previous);
  await withIPC(() => {
    store.getState().receive({ ...next, revision: 8, status: 'completed' });
    store.getState().receive({ ...next, revision: 4 });
    assert.equal(store.getState().active?.id, previous.id, 'only the explicit response changes activation');
    return next;
  }, async () => {
    await store.getState().start({ ...store.getState().draft, accountIds: ['a'] });
    assert.equal(store.getState().active?.id, next.id);
    assert.equal(store.getState().active?.revision, 8);
    assert.equal(store.getState().batch?.status, 'completed');
    assert.equal(store.getState().startSnapshots.size, 0, 'temporary snapshots are released');
    store.getState().receive({ ...previous, revision: 10 });
    assert.equal(store.getState().active?.id, next.id);
    store.getState().resumeBatch({ ...previous, status: 'running', revision: 11 });
    store.getState().receive({ ...next, revision: 12 });
    assert.equal(store.getState().active?.id, previous.id, 'a later explicit resume can select another batch');
  });
});

test('a failed start leaves the previous explicit activation intact', async () => {
  store.getState().resumeBatch(batch());
  await withIPC(() => { throw new Error('start failed'); }, async () => {
    await assert.rejects(store.getState().start({ ...store.getState().draft, accountIds: ['a'] }), /start failed/);
    store.getState().receive({ ...batch(), id: 'later', createdAt: 5 });
    assert.equal(store.getState().active?.id, 'batch');
    assert.equal(store.getState().starting, false);
    assert.equal(store.getState().startSnapshots.size, 0);
  });
});

test('dismissal releases explicit activation without resurrecting dismissed snapshots', async () => {
  store.getState().resumeBatch(batch());
  await withIPC(() => undefined, async () => {
    await store.getState().dismiss('batch');
    store.getState().receive(batch());
    assert.equal(store.getState().active, null);
    assert.equal(store.getState().activatedBatchId, null);
    store.getState().receive({ ...batch('running'), id: 'next', createdAt: 2 });
    assert.equal(store.getState().active?.id, 'next');
  });
});

test('edited start sends the exact prompt and selected account to IPC', async () => {
  const calls: { command: string; args: unknown }[] = [];
  await withIPC((command, args) => { calls.push({ command, args }); return batch('running'); }, async () => {
    store.getState().updateDraft({ prompt: 'User-edited prompt\n保持完整 HTML' });
    const request = { ...store.getState().draft, accountIds: ['a'] };
    await store.getState().start(request);
    assert.deepEqual(calls, [
      { command: 'codex_pelican_start', args: { request } },
    ]);
  });
});
