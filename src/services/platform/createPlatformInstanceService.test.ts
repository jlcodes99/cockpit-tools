import assert from 'node:assert/strict';
import test from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { createPlatformInstanceService } from './createPlatformInstanceService';

(globalThis as Record<string, unknown>).window ??= {};

test('updateInstance forwards launchMode so launch mode edits persist', async () => {
  const service = createPlatformInstanceService('claude');
  let seen: Record<string, unknown> | null = null;
  mockIPC((command, args) => {
    assert.equal(command, 'claude_update_instance');
    seen = args as Record<string, unknown>;
    return {};
  });
  try {
    await service.updateInstance({
      instanceId: 'default',
      launchMode: 'cli',
    });
    assert.ok(seen);
    assert.equal((seen as Record<string, unknown>).launchMode, 'cli');
  } finally {
    clearMocks();
  }
});

test('updateInstance omits launchMode when not provided', async () => {
  const service = createPlatformInstanceService('claude');
  let seen: Record<string, unknown> | null = null;
  mockIPC((_command, args) => {
    seen = args as Record<string, unknown>;
    return {};
  });
  try {
    await service.updateInstance({ instanceId: 'default', name: 'x' });
    assert.ok(seen);
    assert.equal('launchMode' in (seen as Record<string, unknown>), false);
  } finally {
    clearMocks();
  }
});
