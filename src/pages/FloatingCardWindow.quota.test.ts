import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import postcss from 'postcss';
import { floatingCardTargetHeight } from '../utils/floatingCardAppearance';
import { after, before, test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import type { Account } from '../types/account';

// Render the real consumer and presentation with real stores; only the native
// host and browser storage are inert. SSR does not run effects or native calls.
let server: ViteDevServer;
let FloatingCardWindow: () => ReturnType<typeof createElement>;
let accountStore: any;
const storage = new Map<string, string>();
const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
const oldStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');

before(async () => {
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    __TAURI_INTERNALS__: { metadata: { currentWindow: { label: 'floating-card' }, currentWebview: { label: 'floating-card' } } },
    addEventListener() {}, removeEventListener() {}, setTimeout() { return 0; }, clearTimeout() {},
    location: { search: '' },
  } });
  server = await createServer({ configFile: false, server: { middlewareMode: true, watch: null, hmr: false }, appType: 'custom',
    esbuild: { jsx: 'automatic' }, ssr: { external: ['react', 'react-dom', 'react-dom/server'] } });
  accountStore = (await server.ssrLoadModule('/src/stores/useAccountStore.ts')).useAccountStore;
  ({ FloatingCardWindow } = await server.ssrLoadModule('/src/pages/FloatingCardWindow.tsx'));
});

after(async () => {
  await server?.close();
  if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow); else Reflect.deleteProperty(globalThis, 'window');
  if (oldStorage) Object.defineProperty(globalThis, 'localStorage', oldStorage); else Reflect.deleteProperty(globalThis, 'localStorage');
});

function render(platform: string, names: string[], free = false) {
  storage.set('agtools.floating_card.platform', platform);
  const account: Account = {
    id: 'quota-test', email: 'quota@example.com', created_at: 0, last_used: 0,
    token: { access_token: '', refresh_token: '', expires_in: 0, expiry_timestamp: 0, token_type: '' },
    quota: { last_updated: 0, subscription_tier: free ? 'FREE' : 'PRO',
      models: names.map((name, index) => ({ name, percentage: index * 20, reset_time: '2030-01-01T00:00:00Z' })) },
  };
  const snapshot = { accounts: [account], currentAccountsByTarget: { antigravity: account, antigravity_ide: account } };
  accountStore.setState(snapshot);
  Object.assign(accountStore.getInitialState(), snapshot);
  return renderToStaticMarkup(createElement(FloatingCardWindow));
}

for (const platform of ['antigravity', 'antigravity_ide']) {
  test(`${platform} renders all five independent quota windows`, () => {
    const html = render(platform, ['3p-5h', '3p-weekly', 'gemini-5h', 'gemini-weekly', 'gemini-argon-wtu-weekly']);
    assert.equal((html.match(/class="floating-card-quota-row"/g) ?? []).length, 5);
    for (const label of ['Claude (5h)', 'Claude (Weekly)', 'Gemini (5h)', 'Gemini (Weekly)', 'Gemini 4 Argon (Weekly)']) assert.ok(html.includes(label), label);
    assert.match(html, /width:0%/);
    assert.match(html, /80%/);
  });
  test(`${platform} retains Gemini weekly without an Argon bucket`, () => {
    const html = render(platform, ['3p-5h', '3p-weekly', 'gemini-5h', 'gemini-weekly']);
    assert.equal((html.match(/class="floating-card-quota-row"/g) ?? []).length, 4);
    assert.ok(html.includes('Gemini (Weekly)'));
  });
  test(`${platform} retains all three free-tier weekly windows`, () => {
    const html = render(platform, ['3p-weekly', 'gemini-weekly', 'gemini-argon-wtu-weekly'], true);
    assert.equal((html.match(/class="floating-card-quota-row"/g) ?? []).length, 3);
    assert.ok(html.includes('Gemini 4 Argon (Weekly)'));
    assert.ok(!html.includes('(5h)'));
  });
  test(`${platform} does not synthesize missing quota windows`, () => {
    const html = render(platform, []);
    assert.equal((html.match(/class="floating-card-quota-row"/g) ?? []).length, 0);
  });
}

test('Cursor keeps its three primary bars and excludes On-Demand', async () => {
  const store = (await server.ssrLoadModule('/src/stores/useCursorAccountStore.ts')).useCursorAccountStore;
  const account = { id: 'cursor-test', email: 'cursor@example.com', access_token: '', created_at: 0, last_used: 0,
    cursor_usage_raw: { individualUsage: { plan: { totalPercentUsed: 20, autoPercentUsed: 30, apiPercentUsed: 40 }, onDemand: { used: 100, limit: 1000, enabled: true } } } };
  const snapshot = { accounts: [account], currentAccountId: account.id };
  store.setState(snapshot); Object.assign(store.getInitialState(), snapshot);
  storage.set('agtools.floating_card.platform', 'cursor');
  const html = renderToStaticMarkup(createElement(FloatingCardWindow));
  assert.equal((html.match(/class="floating-card-quota-row"/g) ?? []).length, 3);
  for (const label of ['Total Usage', 'Auto + Composer', 'API Usage']) assert.ok(html.includes(label), label);
  assert.ok(!html.includes('On-Demand'));
});

test('Codex retains its existing two-window cap', async () => {
  const store = (await server.ssrLoadModule('/src/stores/useCodexAccountStore.ts')).useCodexAccountStore;
  const account = { id: 'codex-test', email: 'codex@example.com', created_at: 0, last_used: 0,
    tokens: { access_token: '', refresh_token: '', id_token: '' },
    quota: { hourly_percentage: 23, weekly_percentage: 67,
      raw_data: { code_review_rate_limit: { primary_window: { used_percent: 30, limit_window_seconds: 604800 } } } } };
  const snapshot = { accounts: [account], currentAccount: account };
  store.setState(snapshot); Object.assign(store.getInitialState(), snapshot);
  storage.set('agtools.floating_card.platform', 'codex');
  const html = renderToStaticMarkup(createElement(FloatingCardWindow));
  assert.equal((html.match(/class="floating-card-quota-row"/g) ?? []).length, 2);
  assert.match(html, /23%/); assert.match(html, /67%/);
  assert.ok(!html.includes('Code Review'));
});


test('quota bodies remain scrollable when content exceeds the window height cap', () => {
  const stylesheet = postcss.parse(readFileSync(new URL('./FloatingCardWindow.css', import.meta.url), 'utf8'));
  for (const selector of ['.floating-card-full-body', '.floating-card-minimal-body']) {
    let overflowY: string | undefined;
    stylesheet.walkRules(selector, (rule) => {
      rule.walkDecls('overflow-y', (declaration) => { overflowY = declaration.value; });
    });
    assert.equal(overflowY, 'auto', selector);
  }
  assert.equal(floatingCardTargetHeight(false, 430, false), 430);
  assert.equal(floatingCardTargetHeight(false, 900, false), 520);
  assert.equal(floatingCardTargetHeight(true, 900, false), 520);
});
