import assert from 'node:assert/strict';
import test from 'node:test';

test('existing layouts attach Antigravity CLI to the Antigravity group once', async () => {
  const stored = new Map<string, string>([[
    'agtools.platform_layout.v1',
    JSON.stringify({
      platformGroups: [{
        id: 'antigravity-suite',
        name: 'Antigravity',
        platformIds: ['antigravity', 'antigravity_ide'],
        defaultPlatformId: 'antigravity_ide',
        iconKind: 'platform',
        iconPlatformId: 'antigravity_ide',
      }],
      codexApiServiceSuiteMigrated: true,
      traeSuiteDefaultGroupRestored: true,
      antigravityGroupFirstMigrated: true,
    }),
  ]]);
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => { stored.set(key, value); },
    removeItem: (key: string) => { stored.delete(key); },
  } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    addEventListener() {}, removeEventListener() {}, dispatchEvent() { return true; },
    setTimeout: () => 0, clearTimeout() {},
  } });
  // The store keeps background persistence hooks on these globals; this file
  // runs in its own test process, so they are intentionally left installed.
  const { usePlatformLayoutStore } = await import('./usePlatformLayoutStore');
  const state = usePlatformLayoutStore.getState();
  const group = state.platformGroups.find((item) => item.id === 'antigravity-suite');
  assert.deepEqual(group?.platformIds, ['antigravity', 'antigravity_ide', 'antigravity_cli']);
  assert.equal(group?.defaultPlatformId, 'antigravity_ide');
  assert.ok(!state.orderedEntryIds.includes('platform:antigravity_cli'));
  assert.equal(state.antigravityCliSuiteMigrated, true);
});
