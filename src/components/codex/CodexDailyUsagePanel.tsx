import type { TFunction } from 'i18next';
import { formatOpenAIUsageMoney } from '../../utils/openaiOfficialUsage';
import {
  formatModelProviderUsageInteger,
  formatModelProviderUsageMoney,
  formatModelProviderUsageTokenCount,
  type ModelProviderUsageSummary,
} from '../../services/modelProviderUsageService';
import './CodexDailyUsagePanel.css';

export function codexDailyUsageMetrics(summary: ModelProviderUsageSummary | undefined, t: TFunction) {
  const known = (value: number | null | undefined) => typeof value === 'number' && Number.isFinite(value);
  return [
    { key: 'todayCost', label: t('codex.modelProviders.usage.fields.todayCost'),
      value: known(summary?.todayCost)
        ? summary?.mode === 'openai_official'
          ? formatOpenAIUsageMoney(summary.todayCost!)
          : formatModelProviderUsageMoney(summary!.todayCost, summary?.unit)
        : '—' },
    { key: 'todayRequests', label: t('codex.modelProviders.usage.fields.todayRequests'),
      value: known(summary?.todayRequests) ? formatModelProviderUsageInteger(summary!.todayRequests) : '—' },
    { key: 'todayTokens', label: t('codex.modelProviders.usage.fields.todayTokens'),
      value: known(summary?.todayTotalTokens) ? formatModelProviderUsageTokenCount(summary!.todayTotalTokens) : '—' },
  ];
}

export function CodexDailyUsageMetrics({ summary, t }: { summary?: ModelProviderUsageSummary; t: TFunction }) {
  return <div className="codex-api-key-usage-grid codex-daily-usage-grid">
    {codexDailyUsageMetrics(summary, t).map(item => <div key={item.key}>
      <span>{item.label}</span><strong>{item.value}</strong>
    </div>)}
  </div>;
}

export function CodexDailyUsagePanel({ summary, loading, error, variant = 'card', t }: {
  summary?: ModelProviderUsageSummary;
  loading?: boolean;
  error?: string;
  variant?: 'card' | 'table';
  t: TFunction;
}) {
  return <div className={`codex-api-key-usage-panel codex-daily-usage ${variant}`}>
    <CodexDailyUsageMetrics summary={summary} t={t} />
    {loading && <p className="codex-daily-usage-note" role="status">{t('common.loading')}</p>}
    {error && <p className="codex-daily-usage-note" role="status">{t('common.shared.quota.queryFailed')}</p>}
    {!summary && !loading && !error && <p className="codex-daily-usage-note">{t('codex.cockpitApi.noStats')}</p>}
  </div>;
}
