import { useTranslation } from 'react-i18next';
import { AlertCircle } from 'lucide-react';
import { getMinimaxCodeQuotaSnapshot, type MinimaxCodeAccount } from '../../types/minimaxCode';
import './MiniMaxCodeQuota.css';

export function MiniMaxCodeQuota({ account, compact = false }: { account: MinimaxCodeAccount; compact?: boolean }) {
  const { t, i18n } = useTranslation();
  const quota = getMinimaxCodeQuotaSnapshot(account);
  const format = (value: number | null) => value === null ? '—'
    : new Intl.NumberFormat(i18n.language || 'en', { maximumFractionDigits: 2 }).format(value);
  const error = account.quota_query_last_error?.trim();
  return <div className={`minimax-credit-summary${compact ? ' compact' : ''}`}>
    {error && <div className="minimax-credit-error" role="status">
      <AlertCircle size={14} />
      <span>{t(quota.total === null ? 'minimaxCode.quotaQuery.failedRefreshCompact' : 'common.shared.quota.cachedRefreshFailed')}<small>{error}</small></span>
    </div>}
    <div className="minimax-credit-total">
      <span>{t('minimaxCode.creditsBalance')}</span><strong>{format(quota.total)}</strong>
    </div>
    {quota.unsubscribed && <p className="minimax-plan-status">{t('minimaxCode.quota.noPlan')}</p>}
    {!quota.unsubscribed && quota.plan && <p className="minimax-plan-status">{quota.plan}</p>}
    {!compact && (quota.purchased !== null || quota.gifted !== null) && <div className="minimax-credit-breakdown">
      <div><span>{t('minimaxCode.quota.purchased')}</span><b>{format(quota.purchased)}</b></div>
      <div><span>{t('minimaxCode.quota.gifted')}</span><b>{format(quota.gifted)}</b></div>
    </div>}
    {quota.total === null && !error && <p className="minimax-plan-status">{t('minimaxCode.quotaQuery.empty')}</p>}
  </div>;
}
