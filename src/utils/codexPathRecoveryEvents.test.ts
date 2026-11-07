import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import * as recovery from '../stores/useCodexPathRecoveryStore';

function harness() {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const { exports } = loadHookModule(new URL('./codexPathRecoveryEvents.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args: JSON.parse(JSON.stringify(args)) });
      return { id: 'original-target' };
    } },
    '../stores/useCodexPathRecoveryStore': recovery,
  });
  return { recover: exports.recoverLegacyCodexPath, calls };
}

test('native API events resume the original instance and permit binding without desktop startup', async () => {
  const h = harness();
  h.recover({ kind: 'codexCommand', command: 'codex_local_access_activate', args: {
    instanceId: 'chosen-instance', autoRepairMode: 'current', launchAfterSwitch: true,
  } });
  assert.equal(h.calls.length, 0, 'the event must not start anything before user action');
  await recovery.useCodexPathRecoveryStore.getState().request!.continueWithoutDesktop!();
  assert.deepEqual(h.calls, [{ command: 'codex_local_access_activate', args: {
    instanceId: 'chosen-instance', autoRepairMode: 'current', launchAfterSwitch: false, pathRecoveryManaged: true,
  } }]);
});

test('native switch events preserve authorization generation and original no-launch intent', async () => {
  const h = harness();
  const args = { accountId: 'account', reauthTokenGeneration: 41, autoRepairMode: 'current', launchAfterSwitch: false };
  h.recover({ kind: 'codexCommand', command: 'switch_codex_account', args });
  await recovery.useCodexPathRecoveryStore.getState().request!.retry();
  assert.deepEqual(h.calls, [{ command: 'switch_codex_account', args: { ...args, pathRecoveryManaged: true } }]);
});

test('legacy events cannot replace a managed recovery or invoke an arbitrary command', async () => {
  const h = harness();
  h.recover({ kind: 'codexCommand', command: 'delete_codex_account', args: { accountId: 'account' } });
  assert.equal(recovery.useCodexPathRecoveryStore.getState().request, null);
  const pending = recovery.withCodexPathRecovery(async () => { throw 'APP_PATH_NOT_FOUND:codex'; }, { key: 'owned' });
  const cancelled = assert.rejects(pending, /CODEX_START_CANCELLED/);
  await settlePromises();
  const first = recovery.useCodexPathRecoveryStore.getState().request!;
  h.recover({ kind: 'instance', instanceId: 'other' });
  assert.equal(recovery.useCodexPathRecoveryStore.getState().request, first);
  first.cancel();
  await cancelled;
  assert.equal(h.calls.length, 0);
});

test('legacy instance recovery retries that instance rather than default', async () => {
  const h = harness();
  h.recover({ kind: 'instance', instanceId: 'managed' });
  await recovery.useCodexPathRecoveryStore.getState().request!.retry();
  assert.deepEqual(h.calls, [{ command: 'codex_start_instance', args: { instanceId: 'managed' } }]);
});
