import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function setup() {
  const snapshots = new Map<string, any>();
  const reads: { id: string; task: ReturnType<typeof deferred<any>> }[] = [];
  const listeners = new Set<() => void>();
  const notify = () => { for (const listener of listeners) listener(); };
  const h = loadHookModule(new URL('./useCodexProxyTargets.ts', import.meta.url), {
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (select: (state: any) => unknown) => select({ accounts: [] }) },
    '../../services/codexProxyTargetService': {
      isCodexProxyTarget: (account: any) => account.auth_mode === 'apikey',
      peekCodexProxyTarget: (id: string) => snapshots.get(id),
      subscribeCodexProxyTargets: (listener: () => void) => { listeners.add(listener); return () => listeners.delete(listener); },
      getCodexProxyTarget: (id: string) => {
        const task = deferred(); reads.push({ id, task });
        return task.promise.then((value: any) => { snapshots.set(id, value); notify(); }, () => { snapshots.set(id, { id, proxyTargetError: true }); notify(); });
      },
    },
  });
  return { ...h, reads, snapshots, notify };
}

test('API card targets resolve asynchronously and changing targets cannot expose a previous binding; OAuth remains untouched', async () => {
  const s = setup();
  const oauth = { id: 'oauth', auth_mode: 'oauth', egress_proxy: { name: 'ordinary' } };
  let accounts = [{ id: 'api', auth_mode: 'apikey', egress_proxy: { name: 'model-upstream' }, account_note: 'saved note', plan_type: 'Original API' }, oauth];
  const show = () => s.render(() => s.exports.useCodexProxyTargets(accounts));
  assert.equal(show()[0].proxyTargetLoading, true);
  assert.equal(show()[0].egress_proxy, null, 'upstream binding must not become desktop policy');
  assert.equal(show()[1], oauth);
  accounts = [{ id: '__api_service__', auth_mode: 'apikey', egress_proxy: { name: 'unused' } }, oauth];
  assert.equal(show()[0].id, '__api_service__'); assert.equal(show()[0].proxyTargetLoading, true);
  s.reads[0].task.resolve({ id: 'api', auth_mode: 'apikey', egress_proxy: { name: 'old desktop' } }); await settlePromises();
  assert.equal(show()[0].id, '__api_service__'); assert.equal(show()[0].egress_proxy, null);
  s.reads[1].task.resolve({ id: '__api_service__', auth_mode: 'apikey', egress_proxy_disabled: true }); await settlePromises();
  assert.equal(show()[0].egress_proxy_disabled, true);
  s.unmount();
});


test('proxy projection overlays only proxy fields and preserves card metadata without mutating the account', async () => {
  const s = setup();
  const account = { id: 'api', auth_mode: 'apikey', account_name: 'DeepSeek', account_note: 'saved note', plan_type: 'API raw', egress_proxy: { name: 'upstream' } };
  const show = () => s.render(() => s.exports.useCodexProxyTargets([account]));
  show();
  s.reads[0].task.resolve({ id: 'api', auth_mode: 'apikey', egress_proxy: { name: 'desktop' }, egress_proxy_disabled: false }); await settlePromises();
  const target = show()[0];
  assert.equal(target.account_note, 'saved note'); assert.equal(target.plan_type, 'API raw'); assert.equal(target.account_name, 'DeepSeek');
  assert.equal(target.egress_proxy.name, 'desktop'); assert.equal(account.egress_proxy.name, 'upstream');
  assert.equal('proxyTargetLoading' in account, false);
  s.unmount();
});
