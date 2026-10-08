import assert from 'node:assert/strict';
import test from 'node:test';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';

test('cold startup and preference hydration retain the selected Antigravity group client', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const key = 'agtools.platform_layout.v1';
  const values = new Map<string, string>();
  const storage = {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => { values.set(name, value); },
    removeItem: (name: string) => { values.delete(name); },
  };
  const windowMock = Object.assign(new EventTarget(), {
    localStorage: storage,
    setTimeout: () => 0,
    clearTimeout: () => {},
  });
  const layout = (client: 'antigravity' | 'antigravity_ide', includeIde: boolean) => JSON.stringify({
    orderedPlatformIds: ['antigravity', 'antigravity_ide'],
    platformGroups: [{
      id: 'antigravity-suite', name: 'Antigravity',
      platformIds: includeIde ? ['antigravity', 'antigravity_ide'] : ['antigravity'],
      defaultPlatformId: client, iconKind: 'platform', iconPlatformId: client,
    }],
  });
  try {
    Object.defineProperty(globalThis, 'window', { configurable: true, value: windowMock });
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
    mockIPC(() => null);
    const savedAppLayout = layout('antigravity', false);
    storage.setItem(key, savedAppLayout);
    const { usePlatformLayoutStore } = await import('./usePlatformLayoutStore.ts');
    const group = () => usePlatformLayoutStore.getState().platformGroups.find(
      (item) => item.id === 'antigravity-suite',
    )!;
    assert.deepEqual(group().platformIds, ['antigravity', 'antigravity_ide']);
    assert.equal(group().defaultPlatformId, 'antigravity');
    assert.equal(group().iconPlatformId, 'antigravity');
    assert.equal(storage.getItem(key), savedAppLayout, 'startup normalization must not rewrite preferences');
    for (const client of ['antigravity_ide', 'antigravity'] as const) {
      storage.setItem(key, layout(client, true));
      windowMock.dispatchEvent(new Event('agtools:platform-layout-hydrated'));
      assert.equal(group().defaultPlatformId, client);
      assert.equal(group().iconPlatformId, client);
    }
    // Allow the sidebar store's deferred legacy-key cleanup to finish before
    // restoring global storage; tray timers are deliberately disabled above.
    await new Promise((resolve) => setTimeout(resolve, 0));
  } finally {
    clearMocks();
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (previousStorage) Object.defineProperty(globalThis, 'localStorage', previousStorage);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  }
});
