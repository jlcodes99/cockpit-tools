import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer, type ViteDevServer } from 'vite';

const STORAGE_KEY = 'agtools.platform_layout.v1';
const ORIGINAL_PLATFORM_IDS = [
  'claude_manager',
  'codex',
  'codex_api_service',
  'antigravity',
  'antigravity_ide',
  'zed',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'grok',
  'codebuddy',
  'codebuddy_cn',
  'qoder',
  'zcode',
  'trae',
  'trae_solo',
  'trae_cn',
  'trae_solo_cn',
  'workbuddy',
] as const;

interface MemoryStorage extends Storage {
  values: Map<string, string>;
}

function createMemoryStorage(initial?: Record<string, string>): MemoryStorage {
  const values = new Map(Object.entries(initial ?? {}));
  return {
    values,
    get length() {
      return values.size;
    },
    clear() {
      values.clear();
    },
    getItem(key) {
      return values.get(key) ?? null;
    },
    key(index) {
      return Array.from(values.keys())[index] ?? null;
    },
    removeItem(key) {
      values.delete(key);
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
  };
}

async function loadStore(server: ViteDevServer, persisted?: object) {
  const storage = createMemoryStorage(
    persisted ? { [STORAGE_KEY]: JSON.stringify(persisted) } : undefined,
  );
  Object.defineProperty(globalThis, 'localStorage', {
    value: storage,
    writable: true,
    configurable: true,
  });
  server.moduleGraph.invalidateAll();
  const module = await server.ssrLoadModule('/src/stores/usePlatformLayoutStore.ts');
  return {
    storage,
    store: module.usePlatformLayoutStore as {
      getState(): {
        orderedPlatformIds: string[];
        hiddenPlatformIds: string[];
        sidebarPlatformIds: string[];
        trayPlatformIds: string[];
        orderedEntryIds: string[];
        hiddenEntryIds: string[];
        sidebarEntryIds: string[];
        platformGroups: Array<{ id: string; platformIds: string[] }>;
        openCodeGoPlatformMigrated: boolean;
        resetPlatformLayout(): void;
      };
    },
  };
}

let server: ViteDevServer;
test.before(async () => {
  server = await createServer({
    root: process.cwd(),
    appType: 'custom',
    logLevel: 'silent',
    server: { middlewareMode: true },
  });
});
test.after(async () => {
  await server.close();
});

test('defaults expose OpenCode Go as a visible top-level row with all toggles enabled', { concurrency: false }, async () => {
  const { store } = await loadStore(server);
  const state = store.getState();
  const entryId = 'group:platform-opencode_go';

  assert.deepEqual(
    state.orderedEntryIds,
    [
      'group:platform-claude_manager',
      'group:codex-suite',
      entryId,
      'group:antigravity-suite',
      'group:platform-zed',
      'group:platform-github-copilot',
      'group:platform-windsurf',
      'group:platform-kiro',
      'group:platform-cursor',
      'group:platform-grok',
      'group:codebuddy-suite',
      'group:platform-qoder',
      'group:platform-zcode',
      'group:trae-suite',
    ],
  );
  assert.deepEqual(
    state.platformGroups.find((group) => group.id === 'platform-opencode_go')?.platformIds,
    ['opencode_go'],
    `groups: ${JSON.stringify(state.platformGroups)}`,
  );
  assert.equal(state.hiddenEntryIds.includes(entryId), false, 'dashboard defaults visible');
  assert.equal(
    state.sidebarEntryIds.includes(entryId),
    true,
    `sidebar defaults enabled: ${state.sidebarEntryIds.join(', ')}`,
  );
  assert.equal(state.trayPlatformIds.includes('opencode_go'), true, 'tray defaults enabled');
  assert.equal(state.openCodeGoPlatformMigrated, true);
});

test('migration inserts OpenCode Go after Codex without reordering existing platforms', { concurrency: false }, async () => {
  const legacyOrder = [...ORIGINAL_PLATFORM_IDS];
  const legacyEntryOrder = legacyOrder.map((id) => `platform:${id}`);
  const legacySidebar = ['platform:claude_manager', 'platform:codex'];
  const { storage, store } = await loadStore(server, {
    orderedPlatformIds: legacyOrder,
    orderedEntryIds: legacyEntryOrder,
    platformGroups: [],
    hiddenPlatformIds: [],
    hiddenEntryIds: [],
    sidebarPlatformIds: ['claude_manager', 'codex'],
    sidebarEntryIds: legacySidebar,
    trayPlatformIds: ['codex'],
    traySortMode: 'manual',
    antigravityGroupFirstMigrated: true,
    traeSuiteDefaultGroupRestored: true,
    codexApiServiceSuiteMigrated: true,
  });
  const state = store.getState();

  assert.deepEqual(
    state.orderedPlatformIds.filter((id) => id !== 'opencode_go'),
    legacyOrder,
    'the existing platform order is preserved exactly',
  );
  assert.equal(
    state.orderedPlatformIds.indexOf('opencode_go'),
    state.orderedPlatformIds.indexOf('codex') + 1,
    state.orderedPlatformIds.join(', '),
  );
  assert.equal(
    state.orderedEntryIds.indexOf('group:platform-opencode_go'),
    state.orderedEntryIds.indexOf('group:platform-codex') + 1,
  );
  assert.equal(state.hiddenEntryIds.includes('group:platform-opencode_go'), false);
  assert.equal(state.sidebarEntryIds.includes('group:platform-opencode_go'), true);
  assert.equal(state.trayPlatformIds.includes('opencode_go'), true);
  assert.equal(JSON.parse(storage.getItem(STORAGE_KEY)!).openCodeGoPlatformMigrated, true);
});

test('migration marker prevents re-enabling user-disabled OpenCode Go toggles', { concurrency: false }, async () => {
  const { store } = await loadStore(server, {
    orderedPlatformIds: ['codex', 'opencode_go', ...ORIGINAL_PLATFORM_IDS.filter((id) => id !== 'codex')],
    platformGroups: [],
    orderedEntryIds: ['platform:codex', 'platform:opencode_go'],
    hiddenEntryIds: ['group:platform-opencode_go'],
    sidebarEntryIds: ['group:platform-codex'],
    trayPlatformIds: ['codex'],
    openCodeGoPlatformMigrated: true,
    antigravityGroupFirstMigrated: true,
    traeSuiteDefaultGroupRestored: true,
    codexApiServiceSuiteMigrated: true,
  });
  const state = store.getState();

  assert.equal(state.hiddenEntryIds.includes('group:platform-opencode_go'), true);
  assert.equal(state.sidebarEntryIds.includes('group:platform-opencode_go'), false);
  assert.equal(state.trayPlatformIds.includes('opencode_go'), false);
});

test('reset restores the exact OpenCode Go default row and toggle visibility', { concurrency: false }, async () => {
  const { store } = await loadStore(server, {
    orderedPlatformIds: ['opencode_go', ...ORIGINAL_PLATFORM_IDS],
    platformGroups: [],
    orderedEntryIds: ['platform:opencode_go'],
    hiddenEntryIds: ['group:platform-opencode_go'],
    sidebarEntryIds: [],
    trayPlatformIds: [],
    openCodeGoPlatformMigrated: true,
    antigravityGroupFirstMigrated: true,
    traeSuiteDefaultGroupRestored: true,
    codexApiServiceSuiteMigrated: true,
  });

  store.getState().resetPlatformLayout();
  const state = store.getState();
  const entryId = 'group:platform-opencode_go';

  assert.equal(state.orderedEntryIds.indexOf(entryId), state.orderedEntryIds.indexOf('group:codex-suite') + 1);
  assert.equal(state.hiddenEntryIds.includes(entryId), false);
  assert.equal(
    state.sidebarEntryIds.includes(entryId),
    true,
    `reset sidebar entries: ${state.sidebarEntryIds.join(', ')}`,
  );
  assert.equal(state.trayPlatformIds.includes('opencode_go'), true);
  assert.equal(state.openCodeGoPlatformMigrated, true);
});
