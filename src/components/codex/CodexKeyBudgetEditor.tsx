import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import type { CodexLocalAccessApiKey } from '../../types/codexLocalAccess';
import { budgetCurrentUsed, formatBudgetUsd, parseBudgetValue, type CodexKeyBudgetDraft } from '../../utils/codexKeyBudget';
import type { CodexBudgetPeriod } from '../../types/codexRemoteService';
import './CodexKeyBudget.css';

export function CodexKeyBudgetEditor({ value, onChange, disabled }: {
  value: CodexKeyBudgetDraft; onChange: (value: CodexKeyBudgetDraft) => void; disabled: boolean;
}) {
  const { t } = useTranslation();
  const fields = [
    ['amount', 'budget', true, 1_000_000_000_000_000],
    ['dailyAmount', 'dailyLimit', true, 1_000_000_000_000_000],
    ['maxConcurrent', 'concurrency', false, 1024],
    ['dailyImages', 'dailyImages', false, 4_294_967_295],
  ] as const;
  return <section className="codex-key-budget-editor">
    <label><span>{t('common.apiBudget.period')}</span>
      <SingleSelectDropdown value={value.period} options={(['lifetime', 'daily', 'weekly', 'monthly'] as const).map(period => ({ value: period, label: t(`common.apiBudget.${period}`) }))}
        onChange={period => onChange({ ...value, period: period as CodexBudgetPeriod })} disabled={disabled} ariaLabel={t('common.apiBudget.period')} />
    </label>
    {fields.map(([field, label, money, max]) => {
      const invalid = parseBudgetValue(value[field], money, max) === undefined;
      return <label key={field}><span>{t(`common.apiBudget.${label}`)}{money ? ' (USD)' : ''}</span>
        <input type="text" inputMode={money ? 'decimal' : 'numeric'} value={value[field]} disabled={disabled} aria-invalid={invalid}
          placeholder={t('codex.apiService.keys.tokenLimitUnlimited')}
          onChange={event => onChange({ ...value, [field]: event.target.value })} />
        {invalid && <small role="alert" className="codex-key-budget-error">{t('common.apiBudget.invalid')}</small>}
      </label>;
    })}
    <p className="codex-key-budget-hint">{t('common.apiBudget.hint')}</p>
  </section>;
}

export function CodexKeyBudgetSummary({ apiKey }: { apiKey: CodexLocalAccessApiKey }) {
  const { t } = useTranslation();
  const policy = apiKey.budget;
  if (!policy?.limitMicrousd && !policy?.dailyLimitMicrousd) return null;
  const effective = policy.limitMicrousd == null && policy.dailyLimitMicrousd != null
    ? { ...policy, period: 'daily' as const, limitMicrousd: policy.dailyLimitMicrousd } : policy;
  const used = budgetCurrentUsed(effective, apiKey.budgetUsage);
  const limit = effective.limitMicrousd;
  return <div className="api-key-token-limit-summary codex-key-budget-summary">
    <div><span>{t('common.apiBudget.budget')} · {t(`common.apiBudget.${effective.period}`)}</span>
      <strong>{limit == null ? t('codex.apiService.keys.tokenLimitUnlimited') : `${formatBudgetUsd(Math.max(0, limit - used))} / ${formatBudgetUsd(limit)}`}</strong>
      {limit != null && <div className="api-key-token-limit-progress" role="progressbar" aria-valuenow={Math.min(used, limit)} aria-valuemin={0} aria-valuemax={limit} aria-label={t('common.apiBudget.budget')}><span style={{ width: `${Math.min(100, used / limit * 100)}%` }} /></div>}
      {(apiKey.budgetUsage?.unmeteredRequests ?? 0) > 0 && <small>{t('common.apiBudget.unmetered')}</small>}
    </div>
  </div>;
}
