import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

test('createPlatformInstanceService forwards launchMode on update and keeps subsequent reads consistent', async () => {
  const calls: Array<{ command: string; payload?: Record<string, unknown> }> = [];
  let storedLaunchMode: 'app' | 'cli' = 'app';
  let storedBindAccountId: string | null = null;

  const { exports } = loadHookModule(
    new URL('./createPlatformInstanceService.ts', import.meta.url),
    {
      '@tauri-apps/api/core': {
        invoke: async (command: string, payload?: Record<string, unknown>) => {
          calls.push({ command, payload: payload ? { ...payload } : undefined });
          if (command === 'claude_update_instance') {
            if (payload?.launchMode === 'app' || payload?.launchMode === 'cli') {
              storedLaunchMode = payload.launchMode;
            }
            if (payload && 'bindAccountId' in payload) {
              storedBindAccountId = (payload.bindAccountId as string | null) ?? null;
            }
            return {
              id: payload?.instanceId ?? '__default__',
              launchMode: storedLaunchMode,
              bindAccountId: storedBindAccountId,
            };
          }
          if (command === 'claude_list_instances') {
            return [
              {
                id: '__default__',
                launchMode: storedLaunchMode,
                bindAccountId: storedBindAccountId,
              },
            ];
          }
          return null;
        },
      },
    },
  );

  const service = exports.createPlatformInstanceService('claude');
  const updated = await service.updateInstance({
    instanceId: '__default__',
    launchMode: 'cli',
    bindAccountId: 'acc-1',
  });
  assert.equal(updated.launchMode, 'cli');
  assert.equal(updated.bindAccountId, 'acc-1');
  assert.deepEqual(calls[0], {
    command: 'claude_update_instance',
    payload: {
      instanceId: '__default__',
      bindAccountId: 'acc-1',
      launchMode: 'cli',
    },
  });

  const listed = await service.listInstances();
  assert.equal(listed[0]?.launchMode, 'cli');
  assert.equal(listed[0]?.bindAccountId, 'acc-1');

  await service.updateInstance({
    instanceId: '__default__',
    extraArgs: '--verbose',
  });
  assert.equal('launchMode' in (calls[2]?.payload ?? {}), false);
});
