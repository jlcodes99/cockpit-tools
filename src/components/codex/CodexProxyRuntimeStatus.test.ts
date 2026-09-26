import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { CodexProxyConnectionSummary } from './CodexProxyConnectionSummary';
import { CodexProxyRuntimeDetails, CodexProxyRuntimePort } from './CodexProxyRuntimeDetails';
import * as previewUtils from '../../utils/codexProxyPreview';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import type { CodexAccount } from '../../types/codex';
import en from '../../locales/en-US.json';

const base: CodexProxyRuntimeStatus = { account: 'running', sidecar: 'running', desktop: 'idle', accountPort: 45100, sidecarPort: 45100, desktopPort: null,
  desktopEntry: { state: 'listening', port: 45101, requestCount: 0, lastRequestState: 'none', lastError: null } };
const account: CodexAccount = { id: 'test', email: 'test@example.com', tokens: { access_token: '', id_token: '' }, created_at: 0, last_used: 0, plan_type: 'TEAM' };

async function translator() {
  const i18n = createInstance();
  await i18n.init({ lng: 'en', resources: { en: { translation: en } } });
  return i18n;
}

async function renderPanel(status: CodexProxyRuntimeStatus) {
  const i18n = await translator();
  const h = loadHookModule(new URL('./CodexProxyRuntimeStatus.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: i18n.t.bind(i18n) }) },
    '../../services/codexAccountProxyService': { getCodexProxyRuntimeStatus: async () => status },
    '../../utils/codexProxyPreview': previewUtils,
    './CodexProxyRuntimeDetails': { CodexProxyRuntimeDetails, CodexProxyRuntimePort },
  }, { setTimeout: () => 1, clearTimeout: () => {} });
  h.render(() => h.exports.CodexProxyRuntimeStatusPanel({ accountId: 'test', revision: 0 }));
  await settlePromises();
  const html = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, h.flush()));
  h.unmount();
  return html;
}

test('the runtime panel presents a listening entry as ready while the engine waits for demand', async () => {
  const html = await renderPanel(base);
  assert.match(html, /Entry ready/);
  assert.match(html, /Entry requests: 0/);
  assert.match(html, /Waiting for the first connection request/);
  assert.match(html, /the engine starts when a connection request arrives/);
  assert.match(html, /Entry<\/small><code>127\.0\.0\.1:45101/);
  assert.doesNotMatch(html, /Not started/);
  assert.match(html, /does not verify Internet access or identify the requesting client/);
});

test('running forwarding distinguishes the launch port from the engine port and avoids Internet success claims', async () => {
  const html = await renderPanel({ ...base, desktop: 'running', desktopPort: 45102,
    desktopEntry: { ...base.desktopEntry!, requestCount: 1, lastRequestState: 'forwarded' } });
  assert.match(html, /Entry<\/small><code>127\.0\.0\.1:45101/);
  assert.match(html, /Engine · Running · <code>127\.0\.0\.1:45102/);
  assert.match(html, /Last local relay established/);
  assert.match(html, /does not verify Internet access/);
  assert.doesNotMatch(html, /Proxy check passed/);
});

test('last relay errors stay visible without rendering backend error contents', async () => {
  const html = await renderPanel({ ...base, desktop: 'running', desktopPort: 45102,
    desktopEntry: { ...base.desktopEntry!, requestCount: 1, lastRequestState: 'failed', lastError: 'https://secret.invalid/?token=private' } });
  assert.match(html, /Relay failed/);
  assert.match(html, /Last local relay failed/);
  assert.doesNotMatch(html, /secret\.invalid|token=private|Entry ready|Last local relay established/);
});

test('an entry failure is not described as waiting or ready, and legacy ports are labeled as engine ports', async () => {
  const failed = await renderPanel({ ...base, desktopEntry: { ...base.desktopEntry!, state: 'failed' } });
  assert.match(failed, /Entry failed/);
  assert.doesNotMatch(failed, /Waiting for the first|Entry ready/);
  const legacy = await renderPanel({ ...base, desktop: 'running', desktopPort: 45102, desktopEntry: undefined });
  assert.match(legacy, /Engine<\/small><code>127\.0\.0\.1:45102/);
  assert.doesNotMatch(legacy, /Entry requests|Entry ready/);
  const direct = await renderPanel({ ...base, desktop: 'direct' });
  assert.match(direct, /Entry ready/);
  assert.doesNotMatch(direct, /Engine ·|the engine starts when/);
});

test('the preview summary shows effective unified routing even without an independent binding', async () => {
  const i18n = await translator();
  const render = (status: CodexProxyRuntimeStatus | null, current = account, failed = false) =>
    renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(CodexProxyConnectionSummary, { account: current, status, failed })));
  const unified = render({ ...base, proxySource: 'unified', effectiveProxy: { protocol: 'RESOURCE', name: 'United States automatic', sourceName: 'Personal subscription' } });
  assert.match(unified, /Unified proxy/);
  assert.match(unified, /United States automatic/);
  assert.match(unified, /Personal subscription/);
  assert.match(unified, /TEAM/);
  assert.doesNotMatch(unified, /RESOURCE/);
  assert.doesNotMatch(unified, /No account proxy configured|Unbound/);
  const independent = render({ ...base, proxySource: 'account', effectiveProxy: { protocol: 'http', name: 'Current independent' } },
    { ...account, egress_proxy: { protocol: 'http', name: 'Old binding' } });
  assert.match(independent, /Independent proxy|Dedicated proxy/);
  assert.match(independent, /Current independent/);
  assert.doesNotMatch(independent, /Old binding/);
  assert.match(render({ ...base, proxySource: 'none' }), /No account proxy/);
  assert.doesNotMatch(render(null), /No account proxy|Unbound/);
  assert.match(render(null, account, true), /Could not read runtime status|Unable to read status|Status unavailable|Failed to read status/);
});

test('current-node card shows the actual leaf beside the switch action and keeps automatic policy separate', async () => {
  const i18n = await translator();
  const render = (status: CodexProxyRuntimeStatus) => renderToStaticMarkup(createElement(I18nextProvider, { i18n },
    createElement(CodexProxyConnectionSummary, { account, status, failed: false, onSwitch: () => {} })));
  const status: CodexProxyRuntimeStatus = { ...base, proxySource: 'unified',
    effectiveProxy: { protocol: 'RESOURCE', name: 'US automatic policy' },
    accountNode: 'US actual leaf', sidecarNode: 'US actual leaf',
    accountSelection: { name: 'US actual leaf', delayMs: 216, checkedAt: 1750000000000 } };
  const html = render(status);
  const current = html.slice(html.indexOf('class="codex-proxy-preview-current"'));
  assert.match(html, /US automatic policy/);
  assert.match(current, /Current node/);
  assert.match(current, /US actual leaf/);
  assert.match(current, /216 ms/);
  assert.match(current, /aria-haspopup="dialog"/);
  assert.match(current, /Switch node/);
  assert.doesNotMatch(current, /US automatic policy/);
  assert.doesNotMatch(current, /aria-expanded|Collapse/);
  const stopped = render({ ...status, account: 'stopped', sidecar: 'stopped' });
  assert.match(stopped, /Current node not available yet/);
  assert.doesNotMatch(stopped, /US actual leaf|216 ms/);
});

test('current-node card distinguishes channels, deduplicates shared leaves and never attaches another node’s delay', async () => {
  const i18n = await translator();
  const render = (status: CodexProxyRuntimeStatus) => renderToStaticMarkup(createElement(I18nextProvider, { i18n },
    createElement(CodexProxyConnectionSummary, { account, status, failed: false })));
  const status: CodexProxyRuntimeStatus = { ...base, desktop: 'running', accountNode: 'API leaf', sidecarNode: 'API leaf', desktopNode: 'Desktop leaf',
    accountSelection: { name: 'Old leaf', delayMs: 999, checkedAt: 1750000000000 },
    desktopSelection: { name: 'Desktop leaf', delayMs: 123, checkedAt: 1750000000000 } };
  const html = render(status);
  assert.match(html, /API leaf/);
  assert.match(html, /Desktop leaf/);
  assert.match(html, /123 ms/);
  assert.ok(html.includes(renderToStaticMarkup(createElement('small', null, i18n.t('codex.proxy.runtimeDesktop')))));
  assert.ok(html.includes(renderToStaticMarkup(createElement('small', null, i18n.t('codex.proxy.combinedRuntime')))));
  assert.doesNotMatch(html, /Old leaf|999 ms/);
  const shared = render({ ...status, desktopNode: 'API leaf', desktopSelection: null });
  assert.equal((shared.match(/<strong>API leaf<\/strong>/g) ?? []).length, 1);
});
