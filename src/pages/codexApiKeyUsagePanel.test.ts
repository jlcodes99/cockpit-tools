import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { registerHooks } from 'node:module';
import type { TFunction } from 'i18next';
import type { CodexAccount } from '../types/codex';
import type { ModelProviderUsageSummary } from '../services/modelProviderUsageService';
import { useCodexApiKeyUsageFormatting } from './codexApiKeyUsageFormatting';

registerHooks({ load(url, context, next) {
  return url.endsWith('.css') ? { format: 'module', source: '', shortCircuit: true } : next(url, context);
} });
const { renderCodexApiKeyUsagePanel, resolveCodexAccountDailyUsage } = await import('./codexApiKeyUsagePanel');
const t = ((key: string, fallback?: unknown) => typeof fallback === 'string' ? fallback : key) as TFunction;
const account: CodexAccount = {
  id: 'test', email: 'test@example.com', auth_mode: 'apikey',
  openai_api_key: 'test-key', api_base_url: 'https://provider.example',
  tokens: { id_token: '', access_token: '' }, created_at: 0, last_used: 0,
};
function renderSummary(partial: Partial<ModelProviderUsageSummary>, variant: 'card' | 'table' = 'card') {
  const summary: ModelProviderUsageSummary = { mode: 'new_api', unit: 'USD', modelStatsCount: 0, latencyMs: 0, ...partial };
  function Panel() {
    const formatting = useCodexApiKeyUsageFormatting(t, String);
    return renderCodexApiKeyUsagePanel({
      account: partial.mode === 'openai_official' ? { ...account, api_base_url: 'https://api.openai.com/v1' } : account,
      provider: null, variant, usageState: { loading: false, summary }, t, ...formatting,
    });
  }
  return renderToStaticMarkup(createElement(Panel));
}
for (const variant of ['card', 'table'] as const) {
  test(`official API ${variant} keeps daily usage`, () => {
    const html = renderSummary({ mode: 'openai_official', todayCost: 12.5, todayRequests: 7, todayTotalTokens: 123, balance: 900 }, variant);
    for (const field of ['todayCost', 'todayRequests', 'todayTokens']) assert.ok(html.includes(`usage.fields.${field}`));
    assert.ok(html.includes('$12.50') && html.includes('>7<') && html.includes('>123<'));
    assert.ok(!html.includes('$900'));
    assert.ok(!html.includes('codex.openaiUsage.scope'));
  });
  test(`DeepSeek ${variant} shows its balance instead of daily cost`, () => {
    const html = renderSummary({ mode: 'deepseek', balance: 9.59, todayCost: 12.5 }, variant);
    assert.ok(html.includes('$9.59'));
    assert.ok(!html.includes('$12.50'));
  });
  test(`Sub2API ${variant} keeps its balance and request metrics`, () => {
    const html = renderSummary({ mode: 'sub2api', remaining: 80, todayRequests: 7, todayTotalTokens: 123 }, variant);
    assert.ok(html.includes('$80.00') && html.includes('>7<') && html.includes('>123<'));
  });
  test(`New API ${variant} shows its quota`, () => {
    const html = renderSummary({ mode: 'new_api', quotaLimit: 100, quotaRemaining: 80,
      details: [{ key: 'totalGranted', label: 'Granted', value: '100' }, { key: 'totalAvailable', label: 'Available', value: '80' }] }, variant);
    assert.ok(html.includes('$80.00') && html.includes('$100'));
  });
  test(`token plan ${variant} shows the remaining quota`, () => {
    const html = renderSummary({ mode: 'token_plan', remaining: 80, planName: 'Example' }, variant);
    assert.ok(html.includes('$80.00') && html.includes('Example'));
  });
}
test('official missing daily usage stays unknown', () => {
  const html = renderSummary({ mode: 'openai_official', balance: 40, totalCost: 120 });
  assert.equal((html.match(/<strong>—<\/strong>/g) ?? []).length, 3);
  assert.ok(!html.includes('$40.00'));
});
test('failed relay quota query shows an error instead of a blank panel', () => {
  function Panel() {
    const formatting = useCodexApiKeyUsageFormatting(t, String);
    return renderCodexApiKeyUsagePanel({
      account, provider: null, usageState: { loading: false, error: 'network error' }, t, ...formatting,
    });
  }
  assert.ok(renderToStaticMarkup(createElement(Panel)).includes('配额查询失败'));
});
test('APIKEY.FUN Anthropic account renders returned usage', () => {
  function Panel() {
    const formatting = useCodexApiKeyUsageFormatting(t, String);
    return renderCodexApiKeyUsagePanel({
      account: { ...account, api_base_url: 'https://api.apikey.fan', api_wire_api: 'anthropic' },
      provider: null, usageState: { loading: false, summary: { mode: 'sub2api', remaining: 80, todayRequests: 7, modelStatsCount: 0, latencyMs: 0 } }, t, ...formatting,
    });
  }
  const html = renderToStaticMarkup(createElement(Panel));
  assert.ok(html.includes('80') && html.includes('>7<'));
});
test('legacy Cockpit daily requests survive without treating total tokens as today usage', () => {
  const value = { ...account, quota: { raw_data: { usage: { stats: { requests: { today: 4, total: 50 }, tokens: { total: 10000 } } } } } } as CodexAccount;
  const summary = resolveCodexAccountDailyUsage(value);
  assert.equal(summary?.todayRequests, 4);
  assert.equal(summary?.todayTotalTokens, undefined);
  assert.equal(summary?.todayCost, undefined);
});

test('remote Cockpit cards keep only three daily fields and move budget and warnings into details', async () => {
  const cockpit = {
    product:'cockpit-tools' as const, protocolVersion:1 as const, serviceId:'A', serviceName:'Cockpit Tools', keyLabel:'B', status:'active',
    models:['gpt-5.5'], imageModels:['gpt-image-2.5'],
    capabilities:{ responses:true, images:true, imageEdits:true, hostedImages:true, usage:true, websockets:true },
    tokenLimit:1000, tokenUsed:100, updatedAt:0,
    budget:{ policy:{ period:'monthly' as const, limitMicrousd:20_000_000, dailyLimitMicrousd:null, maxConcurrent:2, dailyImageLimit:10 },
      usedMicrousd:5_000_000, remainingMicrousd:15_000_000, dailyUsedMicrousd:0, dailyRemainingMicrousd:null,
      resetsAt:null, lifetimeMicrousd:5_000_000, unmeteredRequests:0, dailyImageRequests:1, timezone:'UTC' as const, inFlightMayExceed:true },
  };
  const summary={ mode:'cockpit', cockpit, totalCost:900, todayCost:1.25, todayRequests:7, todayTotalTokens:123, modelStatsCount:0, latencyMs:0 };
  for (const variant of ['card','table'] as const) {
    const html = renderSummary(summary,variant);
    assert.equal((html.match(/<strong>/g) ?? []).length,3);
    for (const field of ['todayCost','todayRequests','todayTokens']) assert.ok(html.includes(`usage.fields.${field}`));
    assert.ok(html.includes('$1.25') && html.includes('>7<') && html.includes('>123<'));
    for (const key of ['remote','images','unmetered','consumed']) assert.ok(!html.includes(`common.apiBudget.${key}`));
    assert.ok(!html.includes('$900') && !html.includes('$15.00'));
  }
  const { CodexRemoteServiceDetails }=await import('../components/codex/CodexRemoteServiceDetails');
  const details=renderToStaticMarkup(createElement(CodexRemoteServiceDetails,{state:{loading:false,summary},baseUrl:'http://gateway.test/v1',apiKey:'private-key',t}));
  assert.ok(details.includes('$15.00') && details.includes('$20.00'));
  assert.ok(details.includes('common.apiBudget.accountingNote') && details.includes('codex.localAccess.requestLogTitle'));
  assert.ok(!details.includes('private-key'));
});
