import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { CodexAccount, CodexQuota } from '../types/codex';

let server: ViteDevServer;
let DashboardPage: typeof import('./DashboardPage').DashboardPage;
let store: typeof import('../stores/useCodexAccountStore').useCodexAccountStore;
const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');

before(async () => {
  const data = new Map<string, string>();
  const localStorage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => data.set(key, value),
    removeItem: (key: string) => data.delete(key),
  };
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: localStorage });
  // SSR does not run effects. Disable startup timers so no Tauri requests occur.
  Object.defineProperty(globalThis, 'window', { configurable: true, value: Object.assign(new EventTarget(), {
    localStorage, location: { hostname: 'localhost', search: '' },
    setTimeout: () => 0, clearTimeout: () => {}, matchMedia: () => ({ matches: false }),
  }) });
  // Use the real Vite transforms for import.meta.env, CSS and image imports.
  server = await createServer({
    server: { middlewareMode: true, watch: null, hmr: false },
    optimizeDeps: { noDiscovery: true, include: [] }, appType: 'custom',
  });
  ({ DashboardPage } = await server.ssrLoadModule('/src/pages/DashboardPage.tsx'));
  ({ useCodexAccountStore: store } = await server.ssrLoadModule('/src/stores/useCodexAccountStore.ts'));
  const { usePlatformLayoutStore } = await server.ssrLoadModule('/src/stores/usePlatformLayoutStore.ts');
  Object.assign(usePlatformLayoutStore.getInitialState(), {
    orderedEntryIds: ['platform:codex'], hiddenEntryIds: [], platformGroups: [],
  });
});

after(async () => {
  await server?.close();
  for (const [key, descriptor] of [['window', originalWindow], ['localStorage', originalStorage]] as const) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else Reflect.deleteProperty(globalThis, key);
  }
});

function account(id: string, quota?: CodexQuota): CodexAccount {
  return { id, email: `${id}@example.com`, tokens: { id_token: '', access_token: '' }, created_at: 0, last_used: 0, quota };
}
function dual(value: number): CodexQuota {
  return { hourly_percentage: value, weekly_percentage: value, hourly_window_present: true, weekly_window_present: true };
}
function recommendation(candidates: CodexAccount[]): string {
  const current = account('current', dual(100));
  // Zustand's SSR snapshot is getInitialState, not the client getState snapshot.
  Object.assign(store.getInitialState(), { accounts: [current, ...candidates], currentAccount: current });
  const html = renderToStaticMarkup(createElement(DashboardPage, { onNavigate() {}, onOpenPlatformLayout() {}, onEasterEggTriggerClick() {} }));
  const recommended = html.split('class="split-half recommend-half"')[1];
  assert.ok(recommended, 'the actual dashboard renders its Codex recommendation');
  assert.doesNotMatch(recommended, /current@example.com/);
  return recommended;
}

for (const [name, quota, competitor, expected] of [
  ['exhausted seven-day primary', { hourly_percentage: 0, weekly_percentage: 100, hourly_window_minutes: 10080, hourly_window_present: true, weekly_window_present: false }, 25, 'healthy'],
  ['ninety-percent seven-day primary', { hourly_percentage: 90, weekly_percentage: 100, hourly_window_minutes: 10080, hourly_window_present: true, weekly_window_present: false }, 92, 'healthy'],
  ['hourly-only window', { hourly_percentage: 10, weekly_percentage: 100, hourly_window_minutes: 300, hourly_window_present: true, weekly_window_present: false }, 30, 'healthy'],
  ['secondary-only window', { hourly_percentage: 100, weekly_percentage: 20, hourly_window_present: false, weekly_window_present: true }, 30, 'healthy'],
  ['legacy two-window quota', { hourly_percentage: 60, weekly_percentage: 20 }, 30, 'candidate'],
  ['both flags false primary fallback', { hourly_percentage: 10, weekly_percentage: 100, hourly_window_present: false, weekly_window_present: false }, 30, 'healthy'],
  ['partial false flag', { hourly_percentage: 100, weekly_percentage: 100, hourly_window_present: false }, 10, 'healthy'],
  ['existing weekly-exhaustion gate', { hourly_percentage: 100, weekly_percentage: 0, hourly_window_present: true, weekly_window_present: true }, 25, 'healthy'],
  ['equal displayed quota preserves first candidate', { hourly_percentage: 20, weekly_percentage: 100, hourly_window_present: true, weekly_window_present: false }, 20, 'candidate'],
] as Array<[string, CodexQuota, number, string]>) {
  test(`Codex dashboard recommendation uses displayed windows: ${name}`, () => {
    const html = recommendation([account('candidate', quota), account('healthy', dual(competitor))]);
    assert.match(html, new RegExp(`${expected}@example.com`));
    assert.doesNotMatch(html, new RegExp(`${expected === 'healthy' ? 'candidate' : 'healthy'}@example.com`));
  });
}

test('zero quota remains eligible, while missing quota and unsupported API accounts stay excluded', () => {
  const unsupported = { ...account('unsupported', dual(100)), auth_mode: 'apikey', api_wire_api: 'chat_completions' as const };
  const html = recommendation([account('missing'), unsupported, account('zero', dual(0))]);
  assert.match(html, /zero@example.com/);
  assert.doesNotMatch(html, /missing@example.com|unsupported@example.com/);
});

test('dashboard still has no recommendation without an eligible alternative', () => {
  assert.match(recommendation([account('missing')]), /暂无更好推荐/);
});
