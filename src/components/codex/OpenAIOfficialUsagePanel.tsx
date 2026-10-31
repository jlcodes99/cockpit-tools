import type { TFunction } from 'i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ExternalLink } from 'lucide-react';
import { formatModelProviderUsageMoney, type ModelProviderUsageSummary } from '../../services/modelProviderUsageService';
import { codexDailyUsageMetrics } from './CodexDailyUsagePanel';
import { OPENAI_ADMIN_KEYS_URL, openAIUsageErrorKey } from '../../utils/openaiOfficialUsage';
import './OpenAIOfficialUsagePanel.css';

interface Props {
  summary?: ModelProviderUsageSummary;
  loading?: boolean;
  error?: string;
  variant?: 'card' | 'table';
  detailed?: boolean;
  t: TFunction;
}

/** Same credential as the account; organization statistics are never a balance. */
export function OpenAIOfficialUsagePanel({ summary, loading, error, variant = 'card', detailed = false, t }: Props) {
  const permissionRequired = error?.includes('OPENAI_USAGE_PERMISSION_REQUIRED');
  const invalidKey = error?.includes('OPENAI_USAGE_INVALID_KEY');
  const usage = summary?.mode === 'openai_official' && !permissionRequired && !invalidKey ? summary : undefined;
  const metrics = codexDailyUsageMetrics(usage, t);
  if (detailed) metrics.splice(1, 0, {
    key: 'monthCost', label: t('codex.openaiUsage.monthCost'),
    value: typeof usage?.totalCost === 'number' && Number.isFinite(usage.totalCost)
      ? formatModelProviderUsageMoney(usage.totalCost, usage.unit) : '—',
  });
  return <div className={`codex-api-key-usage-panel openai-official-usage ${variant}${detailed ? ' detailed' : ''}${usage ? '' : ' empty'}`}>
    {usage && <>
      <div className="codex-api-key-usage-grid codex-daily-usage-grid">
        {metrics.map(item => <div key={item.key}><span>{item.label}</span><strong>{item.value}</strong></div>)}
      </div>
      {detailed && <p className="openai-official-usage-note">{t('codex.openaiUsage.scope')}</p>}
    </>}
    {loading && <p className="openai-official-usage-note" role="status">{t('codex.openaiUsage.loading')}</p>}
    {error && <div className="openai-official-usage-feedback" role="status">
      <p>{t(openAIUsageErrorKey(error))}</p>
      {permissionRequired && <>
        <p>{t('codex.openaiUsage.permissionGuide')}</p>
        <a className="btn btn-secondary" href={OPENAI_ADMIN_KEYS_URL} target="_blank" rel="noreferrer"
          onClick={(event) => { event.preventDefault(); void openUrl(OPENAI_ADMIN_KEYS_URL); }}>
          <ExternalLink size={13} />{t('codex.openaiUsage.openAdminKeys')}
        </a>
      </>}
    </div>}
    {!usage && !loading && !error && <p className="openai-official-usage-note">{t('codex.openaiUsage.pending')}</p>}
  </div>;
}
