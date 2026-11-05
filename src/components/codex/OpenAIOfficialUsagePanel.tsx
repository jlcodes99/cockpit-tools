import type { TFunction } from 'i18next';
import { RefreshCw } from 'lucide-react';
import type { ModelProviderUsageSummary } from '../../services/modelProviderUsageService';
import type { CodexModelProvider } from '../../services/codexModelProviderService';
import { codexDailyUsageMetrics } from './CodexDailyUsagePanel';
import { formatOpenAIUsageMoney, resolveOpenAIUsageConfig, openAIUsageErrorKey } from '../../utils/openaiOfficialUsage';
import { OpenAIUsageSettingsButton } from './OpenAIUsageSettingsDialog';
import './OpenAIOfficialUsagePanel.css';

interface Props {
  provider?: CodexModelProvider | null;
  apiKey: string;
  sourceBinding?: string;
  summary?: ModelProviderUsageSummary;
  loading?: boolean;
  error?: string;
  variant?: 'card' | 'table';
  detailed?: boolean;
  onRefresh?: () => void;
  t: TFunction;
}

/** Organization statistics use an independent credential and never represent remaining credit. */
export function OpenAIOfficialUsagePanel({ provider, apiKey, sourceBinding, summary, loading, error, variant = 'card', detailed = false, onRefresh, t }: Props) {
  const config = resolveOpenAIUsageConfig(provider, apiKey);
  const current = Boolean(config && sourceBinding === config.binding);
  const currentError = current ? error : undefined;
  const invalid = currentError?.includes('OPENAI_USAGE_PERMISSION_REQUIRED') || currentError?.includes('OPENAI_USAGE_INVALID_KEY');
  const usage = current && summary?.mode === 'openai_official' && !invalid ? summary : undefined;
  const metrics = codexDailyUsageMetrics(usage, t);
  const knownCost = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
  metrics[0].value = knownCost(usage?.todayCost)
    ? formatOpenAIUsageMoney(usage.todayCost) : t('codex.openaiUsage.costUnavailable');
  if (detailed) metrics.splice(1, 0, {
    key: 'monthCost', label: t('codex.openaiUsage.monthCost'),
    value: knownCost(usage?.totalCost)
      ? formatOpenAIUsageMoney(usage.totalCost) : t('codex.openaiUsage.costUnavailable'),
  });
  return <div className={`codex-api-key-usage-panel openai-official-usage ${variant}${detailed ? ' detailed' : ''}${usage ? '' : ' empty'}`}>
    <span className="openai-official-usage-label">{t('codex.openaiUsage.organization')}</span>
    {!config && <p className="openai-official-usage-note">{t('codex.openaiUsage.notConfigured')}</p>}
    {usage && <>
      <div className="codex-api-key-usage-grid codex-daily-usage-grid">
        {metrics.map(item => <div key={item.key}><span>{item.label}</span><strong>{item.value}</strong></div>)}
      </div>
      {(!knownCost(usage.todayCost) || (detailed && !knownCost(usage.totalCost))) &&
        <p className="openai-official-usage-note">{t('codex.openaiUsage.costUnavailableHint')}</p>}
      {detailed && <p className="openai-official-usage-note">{t('codex.openaiUsage.scope')}</p>}
    </>}
    {config && loading && <p className="openai-official-usage-note" role="status">{t('codex.openaiUsage.loading')}</p>}
    {currentError && <p className="openai-official-usage-note" role="status">{t(openAIUsageErrorKey(currentError))}</p>}
    {config && !usage && !loading && !currentError && <p className="openai-official-usage-note">{t('codex.openaiUsage.pending')}</p>}
    <div className="openai-usage-actions">
      <OpenAIUsageSettingsButton provider={provider} apiKey={apiKey} t={t} />
      {config && onRefresh && <button type="button" className="btn btn-secondary" disabled={loading} onClick={onRefresh}>
        <RefreshCw size={13} />{t(currentError ? 'common.retry' : 'common.refresh')}
      </button>}
    </div>
  </div>;
}
