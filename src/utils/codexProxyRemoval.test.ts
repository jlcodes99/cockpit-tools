import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function removalHarness(fetchAccounts = async (_options: unknown) => {}, fetchCurrentAccount = async (_options: unknown) => {}) {
  return loadHookModule(new URL('./codexProxyRemoval.ts', import.meta.url), {
    '../stores/useCodexAccountStore': { useCodexAccountStore: { getState: () => ({ fetchAccounts, fetchCurrentAccount }) } },
  }).exports;
}

test('removing a source rereads both account stores before completion', async () => {
  const calls: unknown[] = [];
  const current = deferred<void>();
  const api = removalHarness(async (options) => { calls.push(['accounts', options]); }, async (options) => { calls.push(['current', options]); await current.promise; });
  const catalog = { sources: [] };
  let completed = false;
  const pending = api.removeProxySourceWithRefresh({}, async () => catalog, api.refreshCodexProxyAccounts).then((value: unknown) => { completed = true; return value; });
  await settlePromises();
  assert.equal(completed, false);
  assert.equal(JSON.stringify(calls), JSON.stringify([['accounts', { throwOnError: true }], ['current', { throwOnError: true }]]));
  current.resolve(); assert.equal(await pending, catalog);
});

test('partial deletion rereads accounts before propagating the original delete failure', async () => {
  const api = removalHarness(); const events: string[] = [];
  const failure = new Error('CATALOG_PARTIAL_UNBIND');
  await assert.rejects(api.removeProxySourceWithRefresh({}, async () => { events.push('remove'); throw failure; },
    async () => { events.push('refresh'); }), (error) => error === failure);
  assert.deepEqual(events, ['remove', 'refresh']);
});

test('a committed deletion with failed refresh is retried without deleting twice', async () => {
  const api = removalHarness(); const progress = {};
  const catalog = { sources: [] }; let removals = 0; let reads = 0;
  const remove = async () => { removals++; return catalog; };
  const refresh = async () => { if (++reads === 1) throw new Error('disk read failed'); };
  await assert.rejects(api.removeProxySourceWithRefresh(progress, remove, refresh), (error) => {
    assert.equal(api.proxyRemovalErrorKey(error, () => 'other'), 'codex.proxy.catalog.accountRefreshFailed'); return true;
  });
  assert.equal(await api.removeProxySourceWithRefresh(progress, remove, refresh), catalog);
  assert.equal(removals, 1); assert.equal(reads, 2);
});

test('refresh failure after partial deletion is visible and retry attempts the remaining removal', async () => {
  const api = removalHarness(); const progress = {}; let removals = 0;
  const remove = async () => { if (++removals === 1) throw new Error('CATALOG_PARTIAL_UNBIND'); return { sources: [] }; };
  await assert.rejects(api.removeProxySourceWithRefresh(progress, remove, async () => { throw new Error('read'); }), api.ProxyAccountRefreshError);
  await api.removeProxySourceWithRefresh(progress, remove, async () => {});
  assert.equal(removals, 2);
});

for (const failed of ['accounts', 'current']) {
  test(`a ${failed} read failure is propagated after both reads settle`, async () => {
    const finish = deferred<void>(); const failure = new Error(`${failed} failed`);
    const api = removalHarness(
      async () => { if (failed === 'accounts') throw failure; await finish.promise; },
      async () => { if (failed === 'current') throw failure; await finish.promise; },
    );
    let rejected = false;
    const pending = api.refreshCodexProxyAccounts().catch((error: unknown) => { rejected = true; throw error; });
    const assertion = assert.rejects(pending, (error) => error === failure);
    await settlePromises(); assert.equal(rejected, false);
    finish.resolve(); await assertion;
  });
}
