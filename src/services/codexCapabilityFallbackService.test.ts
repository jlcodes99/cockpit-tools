import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

test('concurrent entrances share reads and failed reads can be retried without clearing saved data', async () => {
  let calls = 0;
  let state = deferred<unknown>();
  const accounts = [{ id: 'a' }];
  const timers = new Set<() => void>();
  const h = loadHookModule(new URL('./codexCapabilityFallbackService.ts', import.meta.url), {
    './codexService': { listCodexAccounts: async () => accounts },
    './codexLocalAccessService': { getCodexLocalAccessState() { calls++; return state.promise; } },
  }, {
    setTimeout(callback: () => void) { timers.add(callback); return callback; },
    clearTimeout(callback: () => void) { timers.delete(callback); },
  });
  const first = h.exports.readCapabilityFallbackSnapshot();
  const second = h.exports.readCapabilityFallbackSnapshot();
  assert.equal(calls, 1);
  const failures = [assert.rejects(first, /read failed/), assert.rejects(second, /read failed/)];
  state.reject(new Error('read failed')); await Promise.all(failures);
  assert.equal(timers.size, 0);
  state = deferred<unknown>();
  const retry = h.exports.readCapabilityFallbackSnapshot();
  const saved = { running: true, collection: { enabled: true, capabilityFallback: { enabled: false, accountIds: [], overrides: [] } } };
  state.resolve(saved);
  const result = await retry;
  assert.equal(calls, 2);
  assert.equal(result.state, saved);
  assert.equal(result.accounts, accounts);
  assert.equal(timers.size, 0);
});

test('slow reads leave loading after a bounded timeout and retries do not multiply the unresolved work', async () => {
  let calls = 0;
  const read = deferred<unknown>();
  const timers = new Set<() => void>();
  const h = loadHookModule(new URL('./codexCapabilityFallbackService.ts', import.meta.url), {
    './codexService': { listCodexAccounts: async () => [] },
    './codexLocalAccessService': { getCodexLocalAccessState() { calls++; return read.promise; } },
  }, {
    setTimeout(callback: () => void, delay: number) { assert.equal(delay, 15_000); timers.add(callback); return callback; },
    clearTimeout(callback: () => void) { timers.delete(callback); },
  });
  const first = h.exports.readCapabilityFallbackSnapshot();
  const rejected = assert.rejects(first, /CAPABILITY_FALLBACK_TIMEOUT/);
  for (const expire of [...timers]) expire();
  await rejected;
  assert.equal(timers.size, 0);
  const retry = h.exports.readCapabilityFallbackSnapshot();
  assert.equal(calls, 1);
  read.resolve({ collection: { enabled: false } });
  await retry; await settlePromises();
  assert.equal(timers.size, 0);
});

test('automatic capabilities expose authoritative reads without retired toggle commands', async () => {
  const calls: { command: string; args: any }[] = [];
  const saved = { running: true, collection: { enabled: true } };
  const h = loadHookModule(new URL('./codexLocalAccessService.ts', import.meta.url), {
    '@tauri-apps/api/core': { async invoke(command: string, args: any) { calls.push({ command, args }); return saved; } },
  });
  assert.equal(h.exports.updateCodexLocalAccessCapabilityFallback, undefined);
  assert.equal(h.exports.updateCodexLocalAccessMultiAgent, undefined);
  assert.equal(await h.exports.getCodexLocalAccessState(), saved);
  assert.deepEqual(calls.map((call) => call.command), ['codex_local_access_get_state']);
});
