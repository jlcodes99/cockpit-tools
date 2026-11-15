import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

test('initialization waits for the selected instance and grants permission to continue manually', async () => {
  const commands: string[] = [];
  const checks = new Map<string, boolean>([['default', true], ['other', false]]);
  let scheduled: (() => void) | undefined;
  const { exports, render, flush, unmount } = loadHookModule(
    new URL('./useAntigravityIdeInitialization.ts', import.meta.url),
    { '@tauri-apps/api/core': { invoke: async (command: string, { instanceId }: { instanceId: string }) => {
      commands.push(command);
      return checks.get(instanceId);
    } } },
    {
      setTimeout: (callback: () => void) => { scheduled = callback; return 1; },
      clearTimeout: () => { scheduled = undefined; },
    },
  );

  let instanceId = 'default';
  render(() => exports.useAntigravityIdeInitialization(instanceId));
  await settlePromises();
  assert.equal(flush().phase, 'required');

  flush().waitForInitialization();
  flush();
  await settlePromises();
  assert.equal(flush().phase, 'waiting');
  checks.set('default', false);
  assert.ok(scheduled);
  scheduled();
  await settlePromises();
  assert.equal(flush().phase, 'ready');
  assert.deepEqual([...new Set(commands)], ['antigravity_ide_needs_initialization']);

  instanceId = 'other';
  assert.equal(flush().phase, undefined);
  await settlePromises();
  assert.equal(flush().phase, 'ready');
  unmount();
});

test('a late initialization check cannot replace the newly selected instance status', async () => {
  const old = deferred<boolean>();
  const { exports, render, flush, unmount } = loadHookModule(
    new URL('./useAntigravityIdeInitialization.ts', import.meta.url),
    { '@tauri-apps/api/core': { invoke: (_command: string, { instanceId }: { instanceId: string }) =>
      instanceId === 'previous' ? old.promise : Promise.resolve(false) } },
  );

  let instanceId = 'previous';
  render(() => exports.useAntigravityIdeInitialization(instanceId));
  instanceId = 'current';
  flush();
  await settlePromises();

  old.resolve(true);
  await settlePromises();
  assert.equal(flush().phase, 'ready');
  unmount();
});
