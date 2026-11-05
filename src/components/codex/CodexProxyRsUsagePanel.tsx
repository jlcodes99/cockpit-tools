import { useTranslation } from 'react-i18next';
import type { ModelProviderUsageSummary } from '../../services/modelProviderUsageService';
import { codexProxyRsQuotaMetrics, formatCodexProxyRsAmount, resolveCodexProxyRsUsageWindows } from '../../utils/codexProxyRsUsage';
import { CodexQuotaMiniRows } from './CodexQuotaMiniRows';

/** Keep the official two-window layout while retaining the provider's daily/weekly meaning. */
export function CodexProxyRsUsagePanel({ summary, loading, error, detailed = false }: {
  summary?: ModelProviderUsageSummary; loading?: boolean; error?: string; detailed?: boolean;
}) {
  const { t } = useTranslation();
  const windows = resolveCodexProxyRsUsageWindows(summary);
  return <div>
    {loading && <p className="codex-daily-usage-note" role="status">{t('common.loading', '加载中…')}</p>}
    {error && <p className="codex-daily-usage-note" role="alert">{error}</p>}
    <CodexQuotaMiniRows items={codexProxyRsQuotaMetrics(summary, t).map((item, index) => ({
      ...item,
      label: `${item.label}（${item.key === 'daily'
        ? t('codex.proxyRsUsage.todayUsed', '今日已用')
        : t('codex.proxyRsUsage.weekUsed', '本周已用')}：${formatCodexProxyRsAmount(windows[index].used, summary?.unit)}）`,
    }))} t={t} />
    {detailed && <>
      {windows.map(w => <section key={w.key}>
        <h3>{w.key === 'daily' ? t('codex.proxyRsUsage.daily', '每日') : t('codex.proxyRsUsage.weekly', '每周')}</h3>
        <div className="codex-api-key-usage-grid">
          {(['total', 'used', 'remaining'] as const).map(field => <div key={field}>
            <span>{t(`codex.proxyRsUsage.${field}`, field === 'total' ? '总额度' : field === 'used' ? '已使用' : '剩余')}</span>
            <strong>{formatCodexProxyRsAmount(w[field], summary?.unit)}</strong>
          </div>)}
        </div>
      </section>)}
    </>}
  </div>;
}
