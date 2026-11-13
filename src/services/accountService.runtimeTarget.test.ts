import assert from 'node:assert/strict';
import test from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { importFromLocal, refreshAllQuotas } from './accountService.ts';

test('background quota refresh sends the selected APP or IDE context to automatic switching', async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  try {
    for (const target of ['antigravity', 'antigravity_ide']) {
      Object.defineProperty(globalThis, 'window', { configurable: true, value: {
        localStorage: { getItem: () => target },
      } });
      let called = false;
      mockIPC((command, args) => {
        assert.equal(command, 'refresh_all_quotas');
        assert.deepEqual(args, { trigger: 'auto', runtimeTarget: target });
        called = true;
        return { success: 1, failed: 0, total: 1 };
      });
      await refreshAllQuotas('auto');
      assert.equal(called, true);
      clearMocks();
    }
  } finally {
    clearMocks();
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});

test('local import sends its explicit client target rather than the saved preference', async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  try {
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
      localStorage: { getItem: () => 'antigravity_ide' },
    } });
    mockIPC((command, args) => {
      assert.equal(command, 'import_from_local');
      assert.deepEqual(args, { runtimeTarget: 'antigravity' });
      return { id: 'desktop-account' };
    });
    assert.equal((await importFromLocal('antigravity')).id, 'desktop-account');
  } finally {
    clearMocks();
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
