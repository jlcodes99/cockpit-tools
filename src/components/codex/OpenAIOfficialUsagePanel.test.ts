import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { registerHooks } from 'node:module';
import type { TFunction } from 'i18next';
import type { CodexModelProvider } from '../../services/codexModelProviderService';

registerHooks({ load(url, context, next) {
  return url.endsWith('.css') ? { format: 'module', source: '', shortCircuit: true } : next(url, context);
} });
const { OpenAIOfficialUsagePanel } = await import('./OpenAIOfficialUsagePanel');

const provider: CodexModelProvider = { id: 'p', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', supportsWebsockets: false, createdAt: 1, updatedAt: 1, apiKeys: [{ id: 'k', name: '', apiKey: 'model-key', usageApiKey: 'admin-key', usageKeyVersion: 1, createdAt: 1, updatedAt: 1 }] };
const configured = { provider, apiKey: 'model-key', sourceBinding: 'p:k:1:costs-v2' };
const t = ((key: string) => key) as TFunction;
const summary = { mode: 'openai_official', totalCost: 2, todayCost: 1, todayRequests: 4, todayTotalTokens: 400, modelStatsCount: 0, latencyMs: 1 };

test('the official panel renders spend rather than a remaining balance', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t }));
  assert.ok(html.includes('$1.00') && !html.includes('$2.00'));
  assert.ok(!html.includes('codex.openaiUsage.monthCost'));
  assert.ok(!html.includes('codex.openaiUsage.scope'));
  assert.ok(!html.includes('accountBalance') && !html.includes('totalAvailable'));
});

test('detail panels retain monthly spend while cards keep only the three daily fields', () => {
  const detail = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t, detailed: true }));
  const card = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t }));
  assert.ok(detail.includes('codex.openaiUsage.monthCost') && detail.includes('$2.00'));
  assert.ok(detail.includes('codex.openaiUsage.scope'));
  assert.equal((detail.match(/<strong>/g) ?? []).length, 4);
  assert.equal((card.match(/<strong>/g) ?? []).length, 3);
  assert.ok(!card.includes('codex.openaiUsage.monthCost'));
});

test('permission denial replaces cached statistics with query settings and retry', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t, error: 'OPENAI_USAGE_PERMISSION_REQUIRED' }));
  assert.ok(html.includes('codex.openaiUsage.permissionRequired'));
  assert.ok(html.includes('codex.openaiUsage.settings'));
  assert.ok(!html.includes('admin-key') && !html.includes('model-key'));
  assert.ok(!html.includes('$1.00'));
  const detail = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t, detailed: true, error: 'OPENAI_USAGE_PERMISSION_REQUIRED' }));
  assert.ok(!detail.includes('$1.00') && !detail.includes('$2.00'));
  assert.ok(detail.includes('codex.openaiUsage.permissionRequired'));
});

test('a network failure preserves cached spend without mislabelling it as a permission problem', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t, error: 'OPENAI_USAGE_NETWORK_FAILED' }));
  assert.ok(html.includes('$1.00') && html.includes('codex.openaiUsage.networkFailed'));
  assert.ok(!html.includes('admin-keys'));
});

test('invalid credentials do not show cached spend or a misleading permission guide', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, summary, t, error: 'OPENAI_USAGE_INVALID_KEY' }));
  assert.ok(html.includes('codex.openaiUsage.invalidKey'));
  assert.ok(!html.includes('$1.00') && !html.includes('admin-keys'));
});


test('unconfigured or removed query keys show setup without old permission errors or spend', () => {
  const empty = { ...provider, apiKeys: provider.apiKeys.map(key => ({ ...key, usageApiKey: undefined })) };
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, provider: empty, summary, error: 'OPENAI_USAGE_PERMISSION_REQUIRED', t }));
  assert.ok(html.includes('codex.openaiUsage.notConfigured'));
  assert.ok(html.includes('codex.openaiUsage.configure'));
  assert.ok(!html.includes('$1.00') && !html.includes('codex.openaiUsage.permissionRequired'));
});

test('changing query credentials hides stale statistics until the new query completes', () => {
  const newer = { ...provider, apiKeys: provider.apiKeys.map(key => ({ ...key, usageKeyVersion: 2 })) };
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { ...configured, provider: newer, summary, t }));
  assert.ok(!html.includes('$1.00'));
  assert.ok(html.includes('codex.openaiUsage.organization'));
  assert.ok(html.includes('codex.openaiUsage.pending'));
});

test('missing costs retain request and token counts without inventing zero dollars', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, {
    ...configured, summary: { ...summary, todayCost: null, totalCost: null }, t, detailed: true,
  }));
  assert.ok(html.includes('codex.openaiUsage.costUnavailable'));
  assert.ok(html.includes('codex.openaiUsage.costUnavailableHint'));
  assert.ok(html.includes('<strong>4</strong>') && html.includes('<strong>400</strong>'));
  assert.ok(!html.includes('$0.00'));
});

test('reported zero charges remain zero without a missing-cost warning', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, {
    ...configured, summary: { ...summary, todayCost: 0, totalCost: 0 }, t, detailed: true,
  }));
  assert.equal((html.match(/\$0\.00/g) ?? []).length, 2);
  assert.ok(!html.includes('codex.openaiUsage.costUnavailableHint'));
});

test('small nonzero costs are visible instead of rounding to zero', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, {
    ...configured, summary: { ...summary, todayCost: 0.000321957, totalCost: 123.45 }, t, detailed: true,
  }));
  assert.ok(html.includes('$0.000321957') && html.includes('$123.45'));
  assert.ok(!html.includes('$0.00<'));
});

test('snapshots from the old zero-default parser are hidden until refreshed', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, {
    ...configured, sourceBinding: 'p:k:1', summary: { ...summary, todayCost: 0, totalCost: 0 }, t,
  }));
  assert.ok(!html.includes('$0.00'));
  assert.ok(html.includes('codex.openaiUsage.pending'));
});
