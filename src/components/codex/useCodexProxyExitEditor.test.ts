import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function editorHarness() {
  const requests: ReturnType<typeof deferred<any>>[] = [];
  const cancelled: string[] = [];
  const writes: { args: unknown[]; result: ReturnType<typeof deferred<any>> }[] = [];
  const applied: unknown[] = [];
  const accounts = ['A', 'B'].map((id) => ({ id, egress_proxy: { protocol: 'http' } }));
  const store = Object.assign((select: (state: unknown) => unknown) => select({ updateAccountEgressProxy: async () => {} }), {
    getState: () => ({ applyAccountSnapshot(value: unknown) { applied.push(value); } }),
  });
  const h = loadHookModule(new URL('./useCodexProxyExitEditor.ts', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: store },
    '../../utils/codexAccountProxy': { canUseCodexAccountProxy: () => true },
    '../../utils/privacy': {},
    '../../services/codexProxyCatalogService': {
      bindProxyCatalog: (...args: unknown[]) => { const result = deferred(); writes.push({ args, result }); return result.promise; },
      probeProxyCatalog: () => { const request = deferred(); requests.push(request); return request.promise; },
      cancelProxyCatalog: async () => { cancelled.push('catalog'); },
      catalogErrorKey: (error: unknown) => String(error).includes('PROXY_SAVE_FAILED') ? 'catalogSaveFailed' : 'catalogProbeFailed',
    },
    '../../services/codexAccountProxyService': {
      testCodexAccountProxy: () => { const request = deferred(); requests.push(request); return request.promise; },
      cancelCodexAccountProxy: async (id: string) => { cancelled.push(id); },
      proxyErrorKey: () => 'probeFailed',
    },
    '../../utils/codexProxySelection': { defaultProxySelections: () => ({}) },
    '../../utils/codexProxyDraft': {
      restoreExitChoice: () => ({ sourceId: '', itemId: '', groupId: '', selections: {} }),
      exitDraftState: (_binding: unknown, _restored: unknown, choice: { itemId: string }) => ({
        bound: true, saved: !choice.itemId, dirty: Boolean(choice.itemId),
      }),
    },
    './CodexProxyWorkspaceContext': { useCodexProxyWorkspace: () => ({ accounts, catalog: {
      sources: [{ id: 'source', nodes: [{ id: 'node', supported: true }], groups: [] }],
    } }) },
  });
  return { ...h, requests, cancelled, writes, applied, select: (id: string) => h.render(() => h.exports.useCodexProxyExitEditor(id)) };
}

for (const fail of [false, true]) {
  test(`switching accounts cancels the old probe and ignores its late ${fail ? 'failure' : 'success'}`, async () => {
    const h = editorHarness();
    h.select('A').test();
    assert.equal(h.flush().busy, 'test');
    assert.equal(h.select('B').busy, '');
    assert.deepEqual(h.cancelled, ['A']);
    h.flush().test();
    assert.equal(h.requests.length, 2);
    if (fail) h.requests[0].reject(new Error('old failure'));
    else h.requests[0].resolve({ ip: 'old' });
    await settlePromises();
    assert.equal(h.flush().busy, 'test');
    assert.equal(h.flush().error, '');
    h.flush().test();
    assert.equal(h.requests.length, 2, 'old finally must not unlock B');
    h.requests[1].resolve({ ip: 'new' });
    await settlePromises();
    assert.equal(h.flush().busy, '');
    assert.equal(h.flush().result.ip, 'new');
    assert.equal(h.select('A').result, undefined);
  });
}

test('unmount cancels a pending probe and reopening starts cleanly', async () => {
  const old = editorHarness(); old.select('A').test(); old.unmount();
  assert.deepEqual(old.cancelled, ['A']);
  const next = editorHarness(); assert.equal(next.select('A').busy, '');
  old.requests[0].reject(new Error('cancelled')); await settlePromises();
  next.flush().test(); assert.equal(next.requests.length, 1);
  next.requests[0].resolve({ ip: 'new' }); await settlePromises();
  assert.equal(next.flush().busy, '');
});

test('switching accounts cancels a draft probe through the catalog and drops its late failure', async () => {
  const h = editorHarness();
  h.select('A').select({ sourceId: 'source', itemId: 'node', groupId: '', selections: {} });
  assert.equal(h.flush().selectionReady, true);
  h.flush().test();
  assert.equal(h.flush().busy, 'test');
  assert.equal(h.select('B').busy, '');
  assert.deepEqual(h.cancelled, ['catalog']);
  h.flush().test();
  h.requests[0].reject(new Error('old draft failed'));
  await settlePromises();
  assert.equal(h.flush().error, '');
  assert.equal(h.flush().busy, 'test');
  h.requests[1].resolve({ ip: 'saved' });
  await settlePromises();
  assert.equal(h.flush().result.ip, 'saved');
  assert.equal(h.flush().resultScope, 'saved');
});

test('returning to the same account still ignores probes from its previous visit', async () => {
  const h = editorHarness();
  h.select('A').test();
  h.select('B').test();
  assert.equal(h.select('A').busy, '');
  h.flush().test();
  assert.deepEqual(h.cancelled, ['A', 'B']);
  assert.equal(h.requests.length, 3);
  h.requests[0].resolve({ ip: 'first A' });
  h.requests[1].resolve({ ip: 'B' });
  await settlePromises();
  assert.equal(h.flush().busy, 'test');
  assert.equal(h.flush().result, undefined);
  h.requests[2].resolve({ ip: 'current A' });
  await settlePromises();
  assert.equal(h.flush().busy, '');
  assert.equal(h.flush().result.ip, 'current A');
});


test('failed persistence keeps the draft and saved account; successful retry applies only that account', async () => {
  const h = editorHarness();
  h.select('A').select({ sourceId: 'source', itemId: 'node', groupId: 'group', selections: {} });
  const previous = h.flush().savedBinding;
  h.flush().save();
  assert.equal(h.flush().busy, 'save');
  assert.deepEqual(JSON.parse(JSON.stringify(h.writes[0].args)), ['A', 'source', 'node', {}, 'group']);
  h.writes[0].result.reject(new Error('PROXY_SAVE_FAILED')); await settlePromises();
  assert.equal(h.flush().busy, ''); assert.equal(h.flush().error, 'catalogSaveFailed');
  assert.equal(h.flush().itemId, 'node'); assert.equal(h.flush().savedBinding, previous);
  assert.equal(h.applied.length, 0); assert.equal(h.flush().dirty, true);
  h.flush().save();
  const account = { id: 'A', egress_proxy: { protocol: 'resource', sourceId: 'source', itemId: 'node' } };
  h.writes[1].result.resolve(account); await settlePromises();
  assert.equal(h.applied.length, 1); assert.equal(h.applied[0], account);
  assert.equal(h.flush().error, ''); assert.equal(h.flush().savedBinding, account.egress_proxy);
});

test('a failed optional exit check leaves a valid draft saveable without another probe', async () => {
  const h = editorHarness();
  h.select('A').select({ sourceId: 'source', itemId: 'node', groupId: 'group', selections: {} });
  h.flush().test();
  h.requests[0].reject(new Error('PROXY_CONNECT_TIMEOUT')); await settlePromises();
  assert.equal(h.flush().error, 'catalogProbeFailed');
  assert.equal(h.flush().selectionReady, true);
  h.flush().save();
  assert.equal(h.flush().busy, 'save'); assert.equal(h.flush().error, '');
  assert.equal(h.requests.length, 1, 'saving must not trigger or wait for a second exit check');
  assert.deepEqual(JSON.parse(JSON.stringify(h.writes[0].args)), ['A', 'source', 'node', {}, 'group']);
  const account = { id: 'A', egress_proxy: { protocol: 'resource', sourceId: 'source', itemId: 'node' } };
  h.writes[0].result.resolve(account); await settlePromises();
  assert.equal(h.flush().error, ''); assert.equal(h.flush().notice, 'codex.proxy.saved');
  assert.equal(h.flush().savedBinding, account.egress_proxy);
  assert.deepEqual(h.applied, [account]);
  h.unmount();
});
