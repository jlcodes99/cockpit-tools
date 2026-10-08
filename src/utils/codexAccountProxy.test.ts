import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
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
  // The shortcut shows a short egress state and requests the account preview.
  assert.match(button, /codex\.proxy\.filter_unbound/);
  assert.match(button, /codex\.proxy\.management/);
  assert.match(button, /requestCodexAccountProxy\(account\.id\)/);
  assert.match(button, /event.stopPropagation\(\)/);
  assert.doesNotMatch(button, /CodexProxyPreviewPicker|testCodexAccountProxy/);
});

test('account shortcut opens the preview while resource management stays on the shared page', () => {
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
  // Account-only edits reuse the shared editor (covered by preview behavior tests).
  // The retired picker and its direct probe/cancel paths must not return.
  const preview = readFileSync(new URL('../components/codex/CodexAccountProxyPreview.tsx', import.meta.url), 'utf8');
  for (const removed of [
    'CodexProxyPreviewPicker',
    'testCodexAccountProxy',
    'cancelCodexAccountProxy',
  ]) {
    assert.doesNotMatch(preview, new RegExp(removed));
  }
  assert.match(preview, /codex\.proxy\.management/);
});

test('proxy page reports the effective exit mode instead of a saved-or-bound flag', () => {
  const workspace = {
    accounts: [
      { ...account, id: 'bound', egress_proxy: { sourceId: 'source', itemId: 'node' } },
      { ...account, id: 'missing', egress_proxy: { sourceId: 'source', itemId: 'gone' } },
      { ...account, id: 'follower' },
      { ...account, id: 'disabled', egress_proxy_disabled: true },
    ],
    catalog: { sources: [{ id: 'source', nodes: [{ id: 'node' }], groups: [] }] },
    catalogLoading: false, catalogError: '',
    unified: { mode: 'all_accounts', binding: { name: 'Shared' } },
  };
  const h = loadHookModule(new URL('../components/codex/CodexProxyAccountsSection.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexPreferences': { getCodexPlanBadgeStyle: () => 'default' },
    '../../utils/codexProxyPresentation': { proxySummary: () => 'Saved proxy' },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: {} },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexProxyBatchBindDialog': { CodexProxyBatchBindDialog: () => null },
    './CodexProxyAccountDialog': { CodexProxyAccountDialog: () => null, CodexProxyFollowDialog: () => null },
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => (entry: CodexAccount) => entry.id },
    './CodexProxyWorkspaceContext': { useCodexProxyWorkspace: () => workspace },
  }, { window: { addEventListener() {}, removeEventListener() {} } });
  function elements(value: any): any[] {
    if (!value || typeof value !== 'object') return [];
    if (Array.isArray(value)) return value.flatMap(elements);
    return [value, ...elements(value.props?.children)];
  }
  const rows = () => elements(h.render(() => h.exports.CodexProxyAccountsSection())).filter((node) => node.type === 'tr' && node.key);
  const modes = () => rows().map((row) => elements(row).find((node) => node.props?.className?.startsWith('codex-proxy-accounts-mode ')).props.className);
  assert.deepEqual(modes(), ['independent', 'stale', 'unified', 'disabled'].map((mode) => `codex-proxy-accounts-mode is-${mode}`));
  const staleRow = rows().find((row) => row.key === 'missing');
  assert.ok(elements(staleRow).some((node) => node.type === 'small' && node.props.children === 'codex.proxy.modeStale'));
  const disabledRow = rows().find((row) => row.key === 'disabled');
  assert.ok(elements(disabledRow).some((node) => node.props?.children === 'codex.proxy.modeDisabled'));
  workspace.catalogLoading = true;
  assert.equal(modes()[1], 'codex-proxy-accounts-mode is-independent');
  workspace.catalogLoading = false; workspace.catalogError = 'catalog unavailable';
  assert.equal(modes()[1], 'codex-proxy-accounts-mode is-independent');
  workspace.catalogError = ''; workspace.unified = { ...workspace.unified, mode: 'off' };
  assert.equal(modes()[2], 'codex-proxy-accounts-mode is-default');
  assert.equal(modes()[3], 'codex-proxy-accounts-mode is-disabled');
  h.unmount();
  // A saved binding must never be presented as a verified exit on its own.
  for (const file of ['CodexProxyAccountsSection.tsx', 'CodexProxyOverviewSection.tsx', 'CodexProxyExitRulesSection.tsx']) {
    const source = readFileSync(new URL(`../components/codex/${file}`, import.meta.url), 'utf8');
    assert.match(source, /codex\.proxy\.modeStale(?:Hint)?|codexProxyExitModeKey\(mode\)/, file);
  }
});
