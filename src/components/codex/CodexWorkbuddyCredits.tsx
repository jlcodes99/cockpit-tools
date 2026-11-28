import { useTranslation } from 'react-i18next';
import { getWorkbuddyOfficialQuotaModel, PACKAGE_CODE, type WorkbuddyAccount } from '../../types/workbuddy';
import { getWorkbuddyCredits } from '../../utils/workbuddyCredits';

/** Native WorkBuddy credits are a balance, never a Codex quota percentage. */
export function CodexWorkbuddyCredits({
  source,
  loading = false,
  readFailed = false,
  compact = false,
}: {
  source?: WorkbuddyAccount;
  loading?: boolean;
  readFailed?: boolean;
  compact?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language;
  // Enterprise usage has its own official quota model, including unlimited plans.
  // Never sum stale personal packages into an enterprise balance.
  const enterprise = source?.enterprise_id?.trim()
    ? getWorkbuddyOfficialQuotaModel(source).resources.filter(resource => resource.packageCode === PACKAGE_CODE.enterprise)
    : [];
  const unlimited = enterprise.some(resource => resource.unlimited);
  const left = enterprise.length ? enterprise.reduce((sum, resource) => sum + resource.remain, 0)
    : source && !source.enterprise_id?.trim() ? getWorkbuddyCredits(source).left : null;
  const failed = readFailed || Boolean(source?.quota_query_last_error);
  const value = unlimited ? t('common.shared.quota.unlimited') : left == null
    ? t(loading ? 'common.loading' : 'common.shared.quota.noData')
    : new Intl.NumberFormat(locale, { maximumFractionDigits: 2 }).format(left);
  const label = t('workbuddy.credits.total');
  const status = failed
    ? t(left == null ? 'common.shared.quota.queryFailed' : 'common.shared.quota.cachedRefreshFailed')
    : !loading && !source ? t('common.automation.accountNotFound') : null;
  const updatedAt = source?.usage_updated_at;
  const timestamp = updatedAt && Number.isFinite(updatedAt) && updatedAt > 0 && updatedAt <= 8.64e12
    ? new Intl.DateTimeFormat(locale, { dateStyle: 'short', timeStyle: 'short' }).format(new Date(updatedAt * 1000))
    : null;
  const updatedText = timestamp ? t('common.shared.quota.lastSuccessfulQuery', { time: timestamp }) : null;

  if (compact) {
    return (
      <span className="codex-compact-quota" title={[label, status, updatedText].filter(Boolean).join('\n')}>
        <span className="codex-compact-dot" />
        <span>{label}</span>
        <span className="codex-compact-quota-value">{value}</span>
        {status && <span className="quota-error-inline" role="status">{status}</span>}
      </span>
    );
  }
  return (
    <div className="codex-quota-mini">
      <div className="codex-quota-mini-head">
        <span className="codex-quota-mini-label">{label}</span>
        <strong>{value}</strong>
      </div>
      {status && <div className="quota-error-inline" role="status">{status}</div>}
      {updatedText && <div className="codex-quota-mini-reset">{updatedText}</div>}
    </div>
  );
}
