import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function activityHarness() {
  const writes: { accountId: string; pending: ReturnType<typeof deferred> }[] = [];
  const reads: { accountId: string; pending: ReturnType<typeof deferred> }[] = [];
  const write = (accountId: string) => {
    const pending = deferred(); writes.push({ accountId, pending }); return pending.promise;
  };
  const h = loadHookModule(new URL('./useCodexProxyActivity.ts', import.meta.url), {
    '../../services/codexProxyActivityService': {
      setProxyActivityEnabled: write,
      clearProxyActivity: write,
      getProxyActivity: (accountId: string) => {
        const pending = deferred(); reads.push({ accountId, pending }); return pending.promise;
      },
    },
  }, { setTimeout: () => 0, clearTimeout() {} });
  const select = (id: string, active = true) => h.render(() => h.exports.useCodexProxyActivity(id, active));
  return { ...h, writes, reads, select };
}

for (const action of ['changeEnabled', 'clear']) {
  for (const fail of [false, true]) {
    test(`${action}: old account ${fail ? 'failure' : 'success'} cannot overwrite or unlock the next account`, async () => {
      const h = activityHarness();
      h.select('A')[action](false); h.flush();
      h.select('B');
      h.reads[1].pending.resolve({ accountId: 'B' }); await settlePromises();
      h.flush()[action](true); h.flush();
      if (fail) h.writes[0].pending.reject(new Error('old failure'));
      else h.writes[0].pending.resolve(undefined);
      await settlePromises();
      assert.equal(h.flush().snapshot.accountId, 'B');
      assert.equal(h.flush().busy, true);
      assert.equal(h.flush().error, false);
      assert.equal(h.reads.length, 2, 'old write must not start an old-account read');
      h.flush()[action](false);
      assert.equal(h.writes.length, 2, 'old finally must not release the new operation lock');
      h.writes[1].pending.resolve(undefined); await settlePromises();
      h.reads[2].pending.resolve({ accountId: 'B', updated: true }); await settlePromises();
      assert.equal(h.flush().snapshot.updated, true);
      assert.equal(h.flush().busy, false);
      h.unmount();
    });
  }

  test(`${action}: switching during its snapshot read ignores late results`, async () => {
    const h = activityHarness();
    h.select('A')[action](false); h.flush();
    h.writes[0].pending.resolve(undefined); await settlePromises();
    h.select('B');
    h.reads[2].pending.resolve({ accountId: 'B' }); await settlePromises();
    h.reads[1].pending.resolve({ accountId: 'A' }); await settlePromises();
    assert.equal(h.flush().snapshot.accountId, 'B');
    assert.equal(h.flush().busy, false);
    h.unmount();
  });

  test(`${action}: unmount prevents follow-up reads`, async () => {
    const h = activityHarness();
    h.select('A')[action](false); h.flush(); h.unmount();
    h.writes[0].pending.resolve(undefined); await settlePromises();
    assert.equal(h.reads.length, 1);
  });
}

test('a stale pre-mutation poll cannot overwrite the completed mutation', async () => {
  const h = activityHarness();
  h.select('A').clear(); h.flush();
  h.writes[0].pending.resolve(undefined); await settlePromises();
  h.reads[1].pending.resolve({ accountId: 'A', logs: [] }); await settlePromises();
  h.reads[0].pending.resolve({ accountId: 'A', logs: ['old'] }); await settlePromises();
  assert.deepEqual(h.flush().snapshot.logs, []);
  h.unmount();
});
