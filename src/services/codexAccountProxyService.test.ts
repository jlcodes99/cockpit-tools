import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import * as previewUtils from '../utils/codexProxyPreview';
import * as prerequisites from '../utils/codexProxyEnginePrerequisite';
import { createCodexProxyStatusEvents, type CodexProxyStatusUpdate } from '../utils/codexProxyStatusEvents';
import type { CodexProxyRuntimeStatus } from './codexAccountProxyService';

const measured: CodexProxyRuntimeStatus = { account: 'running', desktop: 'running', accountPort: 1234, desktopPort: 1234,
  accountSelection: { name: 'node', delayMs: 200, checkedAt: 100 } };
function harness() {
  const events = createCodexProxyStatusEvents();
  const updates: CodexProxyStatusUpdate[] = [];
  events.subscribe((event) => updates.push(event));
  const calls: { command: string; args: unknown; work: ReturnType<typeof deferred<CodexProxyRuntimeStatus>> }[] = [];
  const h = loadHookModule(new URL('./codexAccountProxyService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke(command: string, args: unknown) {
      const work = deferred<CodexProxyRuntimeStatus>(); calls.push({ command, args, work }); return work.promise;
    } },
    '../utils/codexProxyPreview': previewUtils,
    '../utils/codexProxyStatusEvents': { codexProxyStatusEvents: events },
    '../utils/codexProxyEnginePrerequisite': prerequisites,
  });
  return { service: h.exports, calls, updates };
}

test('manual latency checks share one native invocation and publish one result', async () => {
  const h = harness();
  const first = h.service.measureCodexAccountProxyLatency('A');
  const same = h.service.measureCodexAccountProxyLatency('A');
  await settlePromises();
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0].command, 'measure_codex_account_proxy_latency');
  assert.equal((h.calls[0].args as { accountId: string }).accountId, 'A');
  h.calls[0].work.resolve(measured);
  assert.equal(await first, measured); assert.equal(await same, measured);
  assert.equal(h.updates.length, 1);
  assert.equal(h.updates[0].status?.accountSelection?.delayMs, 200);
});

for (const readFails of [false, true]) test(`failed measurement refreshes shared status, including status-read failure=${readFails}`, async () => {
  const h = harness();
  const first = h.service.measureCodexAccountProxyLatency('A');
  const rejected = assert.rejects(first, /PROXY_PROBE_TIMEOUT/);
  await settlePromises();
  h.calls[0].work.reject(new Error('PROXY_PROBE_TIMEOUT')); await settlePromises();
  assert.equal(h.calls[1].command, 'get_codex_account_proxy_status');
  if (readFails) h.calls[1].work.reject(new Error('status failed'));
  else h.calls[1].work.resolve({ ...measured, accountSelection: { name: 'node', delayMs: null, checkedAt: 101 } });
  await rejected;
  assert.equal(h.updates.length, 1);
  assert.equal(h.updates[0].status?.accountSelection?.delayMs ?? null, null);
  assert.equal(h.updates[0].status?.accountSelection?.checkedAt ?? null, readFails ? null : 101);
  assert.equal(h.service.proxyErrorKey('PROXY_LATENCY_NOT_RUNNING', 'probeFailed'), 'codex.proxy.currentLatencyNotRunning');
});
