import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { registerHooks } from 'node:module';
import type { TFunction } from 'i18next';

registerHooks({ load(url, context, next) {
  return url.endsWith('.css') ? { format: 'module', source: '', shortCircuit: true } : next(url, context);
} });
const { OpenAIOfficialUsagePanel } = await import('./OpenAIOfficialUsagePanel');

const t = ((key: string) => key) as TFunction;
const summary = { mode: 'openai_official', totalCost: 2, todayCost: 1, todayRequests: 4, todayTotalTokens: 400, modelStatsCount: 0, latencyMs: 1 };

test('the official panel renders spend rather than a remaining balance', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t }));
  assert.ok(html.includes('$1.00') && !html.includes('$2.00'));
  assert.ok(!html.includes('codex.openaiUsage.monthCost'));
  assert.ok(!html.includes('codex.openaiUsage.scope'));
  assert.ok(!html.includes('accountBalance') && !html.includes('totalAvailable'));
});

test('detail panels retain monthly spend while cards keep only the three daily fields', () => {
  const detail = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t, detailed: true }));
  const card = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t }));
  assert.ok(detail.includes('codex.openaiUsage.monthCost') && detail.includes('$2.00'));
  assert.ok(detail.includes('codex.openaiUsage.scope'));
  assert.equal((detail.match(/<strong>/g) ?? []).length, 4);
  assert.equal((card.match(/<strong>/g) ?? []).length, 3);
  assert.ok(!card.includes('codex.openaiUsage.monthCost'));
});

test('permission denial replaces cached statistics with the admin permission guide', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t, error: 'OPENAI_USAGE_PERMISSION_REQUIRED' }));
  assert.ok(html.includes('codex.openaiUsage.permissionGuide'));
  assert.ok(html.includes('https://platform.openai.com/settings/organization/admin-keys'));
  assert.ok(!html.includes('$1.00'));
  const detail = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t, detailed: true, error: 'OPENAI_USAGE_PERMISSION_REQUIRED' }));
  assert.ok(!detail.includes('$1.00') && !detail.includes('$2.00'));
  assert.ok(detail.includes('codex.openaiUsage.permissionGuide'));
});

test('a network failure preserves cached spend without mislabelling it as a permission problem', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t, error: 'OPENAI_USAGE_NETWORK_FAILED' }));
  assert.ok(html.includes('$1.00') && html.includes('codex.openaiUsage.networkFailed'));
  assert.ok(!html.includes('admin-keys'));
});

test('invalid credentials do not show cached spend or a misleading permission guide', () => {
  const html = renderToStaticMarkup(createElement(OpenAIOfficialUsagePanel, { summary, t, error: 'OPENAI_USAGE_INVALID_KEY' }));
  assert.ok(html.includes('codex.openaiUsage.invalidKey'));
  assert.ok(!html.includes('$1.00') && !html.includes('admin-keys'));
});
