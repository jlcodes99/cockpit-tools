import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { canUseCodexAccountProxy } from './codexAccountProxy';
import type { CodexAccount } from '../types/codex';

const account = {
  id: 'ordinary', email: 'test@example.com', auth_mode: 'oauth',
  tokens: { access_token: 'access', id_token: 'id', refresh_token: 'refresh' },
  created_at: 0, last_used: 0,
} satisfies CodexAccount;

test('proxy eligibility does not depend on subscription tier or refresh token', () => {
  for (const plan_type of ['FREE', 'PLUS', 'PRO', 'TEAM']) {
    assert.equal(canUseCodexAccountProxy({ ...account, plan_type }), true);
  }
  assert.equal(canUseCodexAccountProxy({ ...account, tokens: { access_token: 'access', id_token: '' } }), true);
});

test('provider, pending, web-session and agent accounts cannot use independent proxy', () => {
  for (const update of [
    { auth_mode: 'apikey' }, { api_provider_mode: 'custom' as const },
    { api_provider_id: 'provider' }, { upstream_grok_account_id: 'grok' },
    { authorization_status: 'pending' }, { token_source_mode: 'chatgpt_web_session' },
  ]) assert.equal(canUseCodexAccountProxy({ ...account, ...update }), false);
});

test('card and table proxy controls follow reset controls, outside footer actions', () => {
  const source = readFileSync(new URL('../pages/useCodexAccountsRenderers.tsx', import.meta.url), 'utf8');
  assert.equal((source.match(/\{resetCreditControls\}\s*<CodexAccountProxyButton account=\{account\} \/>/g) || []).length, 2);
  const button = readFileSync(new URL('../components/codex/CodexAccountProxyButton.tsx', import.meta.url), 'utf8');
  // The shortcut shows a short egress state and routes to the shared proxy page.
  assert.match(button, /codex\.proxy\.filter_unbound/);
  assert.match(button, /codex\.proxy\.management/);
  assert.match(button, /requestCodexAccountProxy\(account\.id\)/);
  assert.match(button, /event.stopPropagation\(\)/);
  assert.doesNotMatch(button, /CodexProxyPreviewPicker|testCodexAccountProxy/);
});

test('account shortcut opens the read-only preview while the page keeps every edit action', () => {
  const view = readFileSync(new URL('../pages/CodexAccountsView.tsx', import.meta.url), 'utf8');
  const listener = view.slice(view.indexOf('const openProxy ='), view.indexOf('window.addEventListener(CODEX_OPEN_PROXY_EVENT'));
  // The card/table shortcut previews one account in place instead of jumping pages.
  assert.match(listener, /setProxyPreviewId\(accountId\)/);
  assert.doesNotMatch(listener, /setActiveTab/);
  // Leaving the preview for the workbench stays explicit and account-scoped.
  assert.match(view, /setProxyAccountId\(proxyPreviewAccount\.id\)/);
  assert.match(view, /setActiveTab\("proxy"\)/);
  // Same top-layout page registration as the header entry, with the account as context.
  assert.match(view, /activeTab === "proxy" && <CodexEgressProxyPage/);
  assert.match(view, /accountId=\{proxyAccountId\}/);
  // The preview stays a summary: selection, checks and binding never move into it.
  const preview = readFileSync(new URL('../components/codex/CodexAccountProxyPreview.tsx', import.meta.url), 'utf8');
  for (const removed of [
    'CodexProxyPreviewPicker',
    'testCodexAccountProxy',
    'cancelCodexAccountProxy',
    'egress_proxy_url',
  ]) {
    assert.doesNotMatch(preview, new RegExp(removed));
  }
  assert.match(preview, /codex\.proxy\.management/);
});

test('proxy page reports the effective exit mode instead of a saved-or-bound flag', () => {
  // The shared helper drives the table state; a configured proxy is not a health check.
  const draft = readFileSync(new URL('./codexProxyDraft.ts', import.meta.url), 'utf8');
  // Independent / unified / default / stale, with stale driven by the catalog, not by the flag alone.
  assert.match(draft, /return resolvable \? 'independent' : 'stale'/);
  assert.match(draft, /state\.loading \|\| state\.failed\) return 'independent'/);
  const accounts = readFileSync(new URL('../components/codex/CodexProxyAccountsSection.tsx', import.meta.url), 'utf8');
  assert.match(accounts, /resolveExitMode\(binding, \{ catalog, loading: catalogLoading, failed: Boolean\(catalogError\) \}, following\.has\(entry\.id\)\)/);
  assert.match(accounts, /codex-proxy-accounts-mode is-\$\{mode\}/);
  assert.match(accounts, /mode === 'stale' && <small>\{t\('codex\.proxy\.modeStale'\)\}/);
  assert.doesNotMatch(accounts, /codex-proxy-page-state\$\{saved \? ' is-bound' : ''\}/);
  // A saved binding must never be presented as a verified exit on its own.
  for (const file of ['CodexProxyAccountsSection.tsx', 'CodexProxyOverviewSection.tsx', 'CodexProxyExitRulesSection.tsx']) {
    const source = readFileSync(new URL(`../components/codex/${file}`, import.meta.url), 'utf8');
    assert.match(source, /codex\.proxy\.modeStale(?:Hint)?|codexProxyExitModeKey\(mode\)/, file);
  }
});
