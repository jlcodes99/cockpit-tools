import assert from 'node:assert/strict';
import test from 'node:test';
import { codexProxyRsQuotaMetrics, resolveCodexProxyRsUsageWindows } from './codexProxyRsUsage';
import { resolveModelProviderUsageMode, type ModelProviderUsageSummary } from '../services/modelProviderUsageService';

function fixture(): ModelProviderUsageSummary {
  return { mode: 'codex_proxy_rs', unit: 'USD', modelStatsCount: 0, latencyMs: 1,
    details: Object.entries({ 'daily.total': '250', 'daily.used': '0', 'daily.remaining': '250', 'daily.resetsAt': '-',
      'weekly.total': '250', 'weekly.used': '3.09679626', 'weekly.remaining': '246.90320374', 'weekly.resetsAt': '2026-11-10T16:00:00Z' })
      .map(([key, value]) => ({ key, value, label: key })) };
}

test('codex-proxy-rs budgets use official quota rows with daily/weekly labels and precise cached amounts', () => {
  const summary = fixture();
  const before = JSON.stringify(summary);
  const windows = resolveCodexProxyRsUsageWindows(summary);
  const rows = codexProxyRsQuotaMetrics(summary, (_key, fallback) => fallback);
  assert.equal(resolveModelProviderUsageMode(summary), 'codex_proxy_rs');
  assert.deepEqual(rows.map(w => w.label), ['每日', '每周']);
  assert.deepEqual(rows.map(w => w.valueText), ['100%', '98.76%']);
  assert.equal(windows[1].remaining, '246.90320374');
  assert.equal(windows[1].used, '3.09679626');
  assert.ok(rows[1].resetText?.includes('$246.90 / $250.00'));
  assert.ok(rows[1].hintText?.includes('$3.10'));
  assert.equal(windows[0].resetsAt, null);
  assert.ok(rows[0].resetText?.includes('暂无重置时间'));
  assert.ok(rows[1].resetText?.includes(new Date('2026-11-10T16:00:00Z').toLocaleString()));
  assert.equal(JSON.stringify(summary), before);
});

test('null total marker means unlimited only for its own window', () => {
  const summary = fixture();
  summary.details!.find(d => d.key === 'daily.total')!.value = '∞';
  const rows = codexProxyRsQuotaMetrics(summary, (_key, fallback) => fallback);
  assert.equal(rows[0].valueText, '∞');
  assert.ok(rows[0].resetText?.includes('$250.00 / ∞'));
  assert.equal(rows[0].percentage, 100);
  assert.equal(rows[0].quotaClass, 'high');
  assert.equal(rows[1].valueText, '98.76%');
  summary.details!.find(d => d.key === 'weekly.total')!.value = '∞';
  summary.details!.find(d => d.key === 'weekly.remaining')!.value = '∞';
  const weekly = codexProxyRsQuotaMetrics(summary, (_key, fallback) => fallback)[1];
  assert.ok(weekly.resetText?.includes('∞ / ∞'));
  assert.equal(weekly.percentage, 100);
  assert.equal(weekly.quotaClass, 'high');
  assert.equal(codexProxyRsQuotaMetrics(undefined, (_key, fallback) => fallback)[0].valueText, '—');
});

test('unknown amounts and zero budgets never become unlimited, NaN or invented percentages', () => {
  const summary = fixture();
  summary.details!.find(d => d.key === 'daily.total')!.value = '0';
  summary.details!.find(d => d.key === 'weekly.remaining')!.value = '-';
  const windows = resolveCodexProxyRsUsageWindows(summary);
  const rows = codexProxyRsQuotaMetrics(summary, (_key, fallback) => fallback);
  assert.deepEqual(windows.map(w => w.percentage), [null, null]);
  assert.deepEqual(rows.map(w => w.valueText), ['—', '—']);
  assert.ok(rows.every(w => !w.resetText?.includes('NaN') && !w.resetText?.includes('无限')));
});
