import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import { singleFlightRead } from '../utils/codexProxyPreview';

function setup() {
  const reads: { command: string; args: any; task: ReturnType<typeof deferred<any>> }[] = [];
  const upstream = { id: 'api', auth_mode: 'apikey', egress_proxy: { name: 'upstream' }, openai_api_key: 'preserve-existing-key' };
  const applied: any[] = [];
  const updates: any[] = [];
  const h = loadHookModule(new URL('./codexProxyTargetService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke(command: string, args: unknown) { const task = deferred(); reads.push({ command, args, task }); return task.promise; } },
    '../stores/useCodexAccountStore': { useCodexAccountStore: { getState: () => ({ accounts: [upstream], applyAccountSnapshot(value: any) { applied.push(value); },
      updateAccountEgressProxy: async (...args: unknown[]) => { updates.push(args); return { id: 'oauth' }; } }) } },
    './codexService': { updateCodexAccountEgressProxy: async (...args: unknown[]) => { updates.push(args); return { id: args[0], auth_mode: 'apikey', egress_proxy_disabled: args[2] }; } },
    '../utils/codexProxyPreview': { singleFlightRead },
    '../utils/codexProxyStatusEvents': { codexProxyStatusEvents: { publish() {} } },
  }, { setTimeout, clearTimeout });
  return { service: h.exports, reads, applied, upstream, updates };
}

test('API/service snapshots share ordinary target IDs without overwriting real API account credentials or upstream', async () => {
  const s = setup();
  for (const id of ['api', '__api_service__']) {
    s.service.applyCodexProxyAccountSnapshot({ id, auth_mode: 'apikey', egress_proxy: { name: 'desktop' } });
    assert.equal(s.service.peekCodexProxyTarget(id).egress_proxy.name, 'desktop');
    await s.service.updateCodexProxyAccount(id, true);
    assert.equal(s.service.peekCodexProxyTarget(id).egress_proxy_disabled, true);
  }
  assert.deepEqual(s.applied, []);
  assert.equal(s.upstream.openai_api_key, 'preserve-existing-key');
  assert.equal(s.upstream.egress_proxy.name, 'upstream');
  s.service.applyCodexProxyAccountSnapshot({ id: 'oauth', auth_mode: 'oauth' });
  assert.deepEqual(s.applied, [{ id: 'oauth', auth_mode: 'oauth' }]);
});

for (const fail of [false, true]) test(`saved target projection wins over an older read ${fail ? 'failure' : 'success'}`, async () => {
  const s = setup();
  const read = s.service.getCodexProxyTarget('api');
  await settlePromises();
  assert.equal(s.reads[0].command, 'codex_proxy_target_get');
  assert.equal(s.reads[0].args.targetId, 'api');
  s.service.acceptCodexProxyTarget({ id: 'api', auth_mode: 'apikey', egress_proxy_disabled: true });
  if (fail) {
    s.reads[0].task.reject('CLIENT_PROXY_STORAGE');
    await assert.rejects(read);
  } else {
    s.reads[0].task.resolve({ id: 'api', auth_mode: 'apikey', egress_proxy_disabled: false });
    assert.equal((await read).egress_proxy_disabled, true);
  }
  assert.equal(s.service.peekCodexProxyTarget('api').egress_proxy_disabled, true);
  assert.equal(s.service.peekCodexProxyTarget('api').proxyTargetError, false);
});

test('failed initial read is marked unavailable, retries restore readiness and preserve old manual safe binding', async () => {
  const s = setup();
  const failed = s.service.getCodexProxyTarget('__api_service__'); await settlePromises();
  s.reads[0].task.reject('CLIENT_PROXY_STORAGE'); await assert.rejects(failed);
  assert.equal(s.service.peekCodexProxyTarget('__api_service__').proxyTargetError, true);
  const retry = s.service.getCodexProxyTarget('__api_service__'); await settlePromises();
  s.reads[1].task.resolve({ id: '__api_service__', auth_mode: 'apikey', egress_proxy: { protocol: 'http', server: '127.0.0.1', port: 8080, authenticated: true } });
  await retry;
  assert.equal(s.service.peekCodexProxyTarget('__api_service__').proxyTargetError, false);
  assert.equal(s.service.peekCodexProxyTarget('__api_service__').egress_proxy.server, '127.0.0.1');
});


test('confirmed follow operations preserve ordinary OAuth store writes and API/service target dispatch', async () => {
  const s = setup();
  for (const id of ['oauth', 'api', '__api_service__']) await s.service.updateCodexProxyAccount(id);
  assert.equal(JSON.stringify(s.updates), JSON.stringify([
    ['oauth', null, false], ['api', null, false], ['__api_service__', null, false],
  ]));
  assert.equal(s.service.peekCodexProxyTarget('oauth'), undefined, 'ordinary OAuth does not become a shadow target');
  assert.equal(s.service.peekCodexProxyTarget('api').egress_proxy_disabled, false);
  assert.equal(s.service.peekCodexProxyTarget('__api_service__').egress_proxy_disabled, false);
});
