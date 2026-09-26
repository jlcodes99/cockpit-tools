import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { detectProxyInputKind, emptyProxyForm, proxyFormUrl, proxySummary } from './codexProxyPresentation';

test('proxy summary uses safe backend metadata only', () => {
  assert.equal(proxySummary({ protocol: 'http', server: 'localhost', port: 8080 }), 'HTTP · localhost:8080');
  assert.equal(proxySummary({ protocol: 'vmess' }), 'VMESS');
  assert.equal(proxySummary({ protocol: 'catalog', sourceName: 'Example', name: 'Tokyo' }), 'Example · Tokyo');
  assert.equal(proxySummary({ protocol: 'ss', server: '::1', port: 443 }), 'SS · [::1]:443');
  assert.equal(proxySummary(null), '');
  assert.equal(proxySummary(undefined), '');
});

test('live proxy UI never reads legacy raw secrets and retains only backend save metadata', () => {
  // The workbench split keeps every write in one place: the shared exit editor hook.
  for (const file of ['CodexAccountProxyButton.tsx', 'CodexProxyAccountsSection.tsx', 'CodexProxyAccountDialog.tsx', 'CodexProxyAssignDialog.tsx', 'CodexProxyOverviewSection.tsx', 'useCodexProxyExitEditor.ts']) {
    const source = readFileSync(new URL(`../components/codex/${file}`, import.meta.url), 'utf8');
    assert.doesNotMatch(source, /egress_proxy_url/);
  }
  const editor = readFileSync(new URL('../components/codex/useCodexProxyExitEditor.ts', import.meta.url), 'utf8');
  assert.match(editor, /const updated = await updateAccountEgressProxy\(id, null\)/);
  assert.match(editor, /const updated = await bindProxyCatalog\(id, choice\.sourceId, choice\.itemId, selections \?\? \{\}, choice\.groupId\)/);
  assert.equal((editor.match(/setOverrides\(\(old\) => \(\{ \.\.\.old, \[id\]: updated\.egress_proxy \?\? null \}\)\)/g) ?? []).length, 2);
  assert.match(editor, /const checked = scope === 'selection'\s+\? await probeProxyCatalog\(requestId, choice\.sourceId, choice\.itemId, selections \?\? \{\}\)\s+: await testCodexAccountProxy\(id, requestId, null\)/);
  // Saving an unchanged binding stays a no-op instead of rewriting the account.
  assert.match(editor, /if \(operation\.current \|\| !account \|\| saved \|\| !selectionReady\) return;/);
});

test('simple proxy form handles IPv6, encoded authentication and default ports', () => {
  assert.equal(proxyFormUrl({ ...emptyProxyForm(), host: '::1', port: '80' }), 'http://[::1]:80');
  assert.equal(proxyFormUrl({ protocol: 'socks5h', host: 'example.com', port: '1080', username: 'a@b', password: 'p:a+ss' }), 'socks5h://a%40b:p%3Aa%2Bss@example.com:1080');
});

test('simple form rejects ambiguous hosts, absent credentials and invalid ports', () => {
  for (const update of [{ host: 'x/y' }, { host: 'x\\y' }, { port: '0' }, { port: '65536' }, { port: '80x' }, { password: 'secret' }, { protocol: 'file' }]) {
    assert.equal(proxyFormUrl({ ...emptyProxyForm(), host: 'example.com', port: '8080', ...update }), null);
  }
  assert.equal(proxyFormUrl({ ...emptyProxyForm(), host: 'example.com', port: '80', username: '\uD800' }), null);
});

test('the exit editor can cancel an in-flight egress check from every entry point', () => {
  const editor = readFileSync(new URL('../components/codex/useCodexProxyExitEditor.ts', import.meta.url), 'utf8');
  assert.match(editor, /cancelCodexAccountProxy/);
  for (const file of ['CodexProxyAccountDialog.tsx', 'CodexProxyOverviewSection.tsx', 'CodexProxyTestsSection.tsx']) {
    const source = readFileSync(new URL(`../components/codex/${file}`, import.meta.url), 'utf8');
    assert.match(source, /codex\.proxy\.cancelCheck/);
  }
  const service = readFileSync(new URL('../services/codexAccountProxyService.ts', import.meta.url), 'utf8');
  assert.match(service, /cancel_codex_account_egress_proxy/);
  assert.match(service, /PROXY_PROBE_CANCELLED: 'probeCancelled'/);
});

test('runtime status styles distinguish missing, starting and running engines', () => {
  const css = readFileSync(new URL('../styles/pages/codex-more-layout.css', import.meta.url), 'utf8');
  assert.match(css, /\.codex-proxy-runtime-item\.is-missing b/);
  assert.match(css, /\.codex-proxy-runtime-item\.is-starting b/);
  assert.match(css, /\.codex-proxy-runtime-item\.is-running b/);
});


test('subscription detection does not mistake multiline proxies or credential URLs for a subscription', () => {
  assert.equal(detectProxyInputKind(' https://example.test/sub?token=private '), 'subscription');
  for (const value of ['https://user:pass@proxy.test:443', 'https://proxy.test:8443', 'https://proxy.test', 'socks5://u:p@host:1080', 'host:1080:u:p', 'https://one.test/sub\nhttps://two.test/sub']) {
    assert.equal(detectProxyInputKind(value), 'manual');
  }
});
