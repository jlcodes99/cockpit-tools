import type { ModelProviderUsageSummary } from '../services/modelProviderUsageService';
import type { UnifiedQuotaMetric } from '../presentation/platformAccountPresentation';

export function resolveCodexProxyRsUsageWindows(summary?: ModelProviderUsageSummary) {
  const detail = (key: string) => summary?.details?.find(item => item.key === key)?.value;
  return (['daily', 'weekly'] as const).map(key => {
    const amount = (field: string) => {
      const value = detail(`${key}.${field}`);
      return value != null && value.trim() !== '' && Number.isFinite(Number(value)) ? value : null;
    };
    const unlimited = detail(`${key}.total`) === '∞';
    const total = unlimited ? '∞' : amount('total');
    const remaining = detail(`${key}.remaining`) === '∞' ? '∞' : amount('remaining');
    const ratio = total != null && Number(total) > 0 && remaining != null ? Number(remaining) / Number(total) * 100 : null;
    const rawReset = detail(`${key}.resetsAt`);
    return { key, total, unlimited, used: amount('used'), remaining,
      percentage: ratio != null && Number.isFinite(ratio) ? ratio : null,
      resetsAt: rawReset && Number.isFinite(Date.parse(rawReset)) ? rawReset : null };
  });
}

export function formatCodexProxyRsAmount(value: string | null, unit?: string | null): string {
  if (value == null) return '—';
  if (value === '∞') return value;
  const amount = Number(value).toFixed(2);
  return unit === 'USD' ? `$${amount}` : `${amount}${unit ? ` ${unit}` : ''}`;
}

export function codexProxyRsQuotaMetrics(summary: ModelProviderUsageSummary | undefined, t: (key: string, fallback: string) => string): UnifiedQuotaMetric[] {
  return resolveCodexProxyRsUsageWindows(summary).map(w => {
    const money = (value: string | null) => formatCodexProxyRsAmount(value, summary?.unit);
    const label = w.key === 'daily' ? t('codexProxyRs.daily', '每日') : t('codexProxyRs.weekly', '每周');
    const reset = w.resetsAt ? new Date(w.resetsAt).toLocaleString() : t('codexProxyRs.noReset', '暂无重置时间');
    return {
      // Unlimited windows remain fully available; the bar does not represent a usage ratio.
      key: w.key, label, percentage: w.unlimited ? 100 : w.percentage ?? 0,
      quotaClass: w.unlimited ? 'high' : w.percentage == null ? 'neutral' : w.percentage <= 10 ? 'low' : w.percentage <= 30 ? 'medium' : 'high',
      valueText: w.unlimited ? '∞' : w.percentage == null ? '—' : `${Number(w.percentage.toFixed(2))}%`,
      resetText: `${money(w.remaining)} / ${money(w.total)} · ${reset}`,
      hintText: `${label} · ${t('codexProxyRs.total', '总额度')} ${money(w.total)} · ${t('codexProxyRs.used', '已使用')} ${money(w.used)} · ${t('codexProxyRs.remaining', '剩余')} ${money(w.remaining)} · ${reset}`,
    };
  });
}
