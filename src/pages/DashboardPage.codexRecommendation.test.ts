import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { CodexAccount, CodexQuota } from '../types/codex';

let server: ViteDevServer;
let DashboardPage: typeof import('./DashboardPage').DashboardPage;
let allAccountsView: typeof import('./DashboardAllAccounts');
let prefetchDashboardAccounts: typeof import('./dashboardAccountPrefetch').prefetchDashboardAccounts;
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
  allAccountsView = await server.ssrLoadModule('/src/pages/DashboardAllAccounts.tsx') as typeof allAccountsView;
  ({ prefetchDashboardAccounts } = await server.ssrLoadModule('/src/pages/dashboardAccountPrefetch.ts'));
  const { applyDurableUiSnapshot } = await server.ssrLoadModule('/src/utils/durableUiSettings.ts');
  applyDurableUiSnapshot({ values: { 'agtools.privacy_mode_enabled': '0' } });
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

test('dashboard all accounts renders different platforms with colliding account IDs and paginates cards', () => {
  const { DashboardAllAccounts, dashboardAccountCards } = allAccountsView;
  const build = (entry: { id: string; label: string }) => createElement('span', null, entry.label);
  const accounts = [
    ...dashboardAccountCards('codex', [{ id: 'shared', label: 'Codex quota 0%' }], build),
    ...dashboardAccountCards('claude_manager', [{ id: 'shared', label: 'Claude quota unavailable' }], build),
    ...dashboardAccountCards('trae_cn', [{ id: 'shared', label: 'TRAE CN 200 credits' }], build),
    ...dashboardAccountCards('trae_solo_cn', [{ id: 'shared', label: 'TRAE SOLO CN 250 credits' }], build),
    ...dashboardAccountCards('qoder', Array.from({ length: 22 }, (_, index) => ({
      id: String(index), label: `Qoder quota ${index}%`,
    })), build),
  ];
  const html = renderToStaticMarkup(createElement(DashboardAllAccounts, { accounts, loading: false }));
  assert.equal([...html.matchAll(/class="dashboard-all-accounts-item"/g)].length, 20);
  assert.match(html, /Codex quota 0%/);
  assert.match(html, /Claude quota unavailable/);
  assert.match(html, /TRAE CN 200 credits/);
  assert.match(html, /TRAE SOLO CN 250 credits/);
  assert.match(html, /Qoder quota 15%/);
  assert.doesNotMatch(html, /Qoder quota 16%/);
  assert.match(html, /class="pagination-container"/);
});

test('dashboard all accounts shows progress alongside cached data and distinguishes an empty list', () => {
  const { DashboardAllAccounts, dashboardAccountCards } = allAccountsView;
  const accounts = dashboardAccountCards('codex', [{ id: 'cached' }], () => 'Cached quota');
  const partial = renderToStaticMarkup(createElement(DashboardAllAccounts, { accounts, loading: true }));
  assert.match(partial, /role="status"/);
  assert.match(partial, /Cached quota/);
  const empty = renderToStaticMarkup(createElement(DashboardAllAccounts, { accounts: [], loading: false }));
  assert.doesNotMatch(empty, /role="status"|class="pagination-container"/);
  assert.match(empty, /<p>[^<]+<\/p>/);
});

test('dashboard account prefetch stays shared and a slow platform does not delay the next platform', async () => {
  const timers: Array<{ callback: () => void; delay: number }> = [];
  const original = window.setTimeout;
  Object.defineProperty(window, 'setTimeout', {
    value: (callback: () => void, delay: number) => {
      timers.push({ callback, delay });
      return timers.length;
    },
  });
  try {
    const called: string[] = [];
    let finishFirst!: () => void;
    const first = new Promise<void>((resolve) => { finishFirst = resolve; });
    const tasks = [
      () => { called.push('first'); return first; },
      async () => { called.push('second'); },
    ];
    const prefetch = prefetchDashboardAccounts(tasks);
    assert.equal(prefetchDashboardAccounts(tasks), prefetch);
    assert.deepEqual(timers.map(({ delay }) => delay), [6000, 7200]);
    for (const { callback } of timers) callback();
    await Promise.resolve();
    assert.deepEqual(called, ['first', 'second']);
    finishFirst();
    await prefetch;
  } finally {
    window.setTimeout = original;
  }
});
