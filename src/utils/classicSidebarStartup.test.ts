import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

const KEY = 'agtools.platform_layout.v1';
const saved = JSON.stringify({ orderedPlatformIds: ['claude_manager', 'codex', 'grok'],
  sidebarEntryIds: ['group:platform-grok', 'group:codex-suite'], _layoutUpdatedAt: 200 });
const defaults = JSON.stringify({ orderedPlatformIds: ['claude_manager', 'codex', 'grok'],
  sidebarEntryIds: ['group:platform-claude_manager', 'group:codex-suite', 'group:platform-grok'] });

async function harness(cached: string | null, disk: string | null) {
  let local = cached;
  let durable = disk;
  const read = deferred<{ values: Record<string, string> }>();
  const writes: string[] = [];
  const calls = { initialized: 0, marked: 0 };
  const work: Promise<void>[] = [];
  (globalThis as any).localStorage = {
    getItem: () => local,
    setItem: (_key: string, value: string) => { local = value; },
  };
  (globalThis as any).window = { dispatchEvent() {}, __TAURI_INTERNALS__: {
    invoke(command: string, args?: { values: Record<string, string> }) {
      if (command === 'load_ui_preferences') return read.promise;
      if (command === 'save_ui_preferences') {
        durable = args!.values[KEY]; writes.push(durable);
        return Promise.resolve({ values: args!.values });
      }
      throw new Error(command);
    },
  } };
  const prefs = await import(`./uiPreferences.ts?startup=${Date.now()}-${Math.random()}`);
  const module = loadHookModule(new URL('./classicSidebarStartup.ts', import.meta.url), { './uiPreferences': prefs });
  // Run the real App startup effect, so reintroducing a direct defaults write is a regression.
  const app = ts.createSourceFile('App.tsx', readFileSync(new URL('../App.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const callbacks: ts.ArrowFunction[] = [];
  function visit(node: ts.Node) {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'useEffect') {
      const callback = node.arguments[0];
      if (callback && ts.isArrowFunction(callback) && callback.body.getText(app).includes('sideNavClassicFirstSyncDone')) callbacks.push(callback);
    }
    ts.forEachChild(node, visit);
  }
  visit(app); assert.equal(callbacks.length, 1);
  const exports: { boot?: () => (() => void) | undefined } = {};
  vm.runInNewContext(ts.transpileModule(`exports.boot = ${callbacks[0].getText(app)};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText, {
    exports, sideNavLayoutMode: 'classic', sideNavClassicFirstSyncDone: false,
    syncSidebarEntriesFromDashboard() { calls.initialized++; prefs.persistPlatformLayout(defaults); },
    markSideNavClassicFirstSyncDone() { calls.marked++; },
    initializeClassicSidebar(options: unknown) {
      const promise = module.exports.initializeClassicSidebar(options); work.push(promise); return promise;
    },
  });
  const hydrate = prefs.hydrateUiPreferences();
  return { prefs, calls, writes, boot: exports.boot!,
    local: () => local, durable: () => durable,
    resolve: () => read.resolve({ values: disk == null ? {} : { [KEY]: disk } }),
    reject: () => read.reject(new Error('disk unavailable')),
    finish: async () => { await hydrate; await Promise.all(work); await settlePromises(); },
  };
}

test('upgrade with an empty WebView cache waits for the saved Grok sidebar without writing defaults', async () => {
  const h = await harness(null, saved); h.boot();
  assert.equal(h.calls.initialized, 0);
  assert.equal(h.calls.marked, 0);
  assert.equal(h.local(), null);
  h.resolve(); await h.finish();
  assert.equal(h.local(), saved); assert.equal(h.durable(), saved);
  assert.equal(h.calls.initialized, 0); assert.equal(h.calls.marked, 1);
  assert.equal(h.writes.length, 0);
});

test('a lost first-sync marker does not expand a saved sidebar or override an explicit empty selection', async () => {
  for (const value of [saved, JSON.stringify({ orderedPlatformIds: ['grok'], sidebarEntryIds: [], _layoutUpdatedAt: 200 })]) {
    const h = await harness(value, value); h.boot(); h.resolve(); await h.finish();
    assert.equal(h.calls.initialized, 0); assert.equal(h.calls.marked, 1);
    assert.equal(h.local(), value); assert.equal(h.durable(), value);
    assert.equal(h.writes.length, 0);
  }
});

test('a real first install initializes once despite StrictMode cleanup during hydration', async () => {
  const h = await harness(null, null); const cleanup = h.boot(); cleanup?.(); h.boot();
  h.resolve(); await h.finish();
  assert.equal(h.calls.initialized, 1); assert.equal(h.calls.marked, 1);
  assert.equal(h.writes.length, 1);
  assert.deepEqual(JSON.parse(h.durable()!).sidebarEntryIds, JSON.parse(defaults).sidebarEntryIds);
});

test('a layout edited during a slow load wins without a later automatic reset', async () => {
  const h = await harness(null, saved); h.boot();
  h.prefs.persistPlatformLayout(JSON.stringify({ orderedPlatformIds: ['grok', 'codex'], sidebarEntryIds: ['group:platform-grok'] }));
  h.resolve(); await h.finish();
  assert.equal(h.calls.initialized, 0); assert.equal(h.calls.marked, 1);
  assert.equal(h.writes.length, 1);
  assert.deepEqual(JSON.parse(h.durable()!).sidebarEntryIds, ['group:platform-grok']);
});

test('failed reads and a closed startup effect cannot replace unknown or saved preferences', async () => {
  const failed = await harness(null, saved); failed.boot(); failed.reject(); await failed.finish();
  assert.equal(failed.calls.initialized, 0); assert.equal(failed.calls.marked, 0);
  assert.equal(failed.durable(), saved); assert.equal(failed.writes.length, 0);
  const closed = await harness(null, saved); closed.boot()?.(); closed.resolve(); await closed.finish();
  assert.equal(closed.calls.initialized, 0); assert.equal(closed.calls.marked, 0);
  assert.equal(closed.durable(), saved); assert.equal(closed.writes.length, 0);
});
