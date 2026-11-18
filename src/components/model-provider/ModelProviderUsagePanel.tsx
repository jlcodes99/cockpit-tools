import { useTranslation } from 'react-i18next';
import {
  formatModelProviderUsageInteger,
  formatModelProviderUsageMoney,
  formatModelProviderUsageTokenCount,
  resolveModelProviderUsageMode,
  type ModelProviderUsageSummary,
} from '../../services/modelProviderUsageService';

interface ModelProviderUsagePanelProps {
  summary?: ModelProviderUsageSummary | null;
  loading?: boolean;
  error?: string;
  unavailable?: boolean;
  className?: string;
  variant?: 'card' | 'table';
}

export function ModelProviderUsagePanel({
  summary,
  loading = false,
  error,
  unavailable = false,
  className,
  variant,
}: ModelProviderUsagePanelProps) {
  const { t } = useTranslation();
  const usageMode = resolveModelProviderUsageMode(summary ?? undefined);
  const isSupportedUsage =
    usageMode === 'cockpit' ||
    usageMode === 'sub2api' ||
    usageMode === 'new_api' ||
    usageMode === 'token_plan' ||
    usageMode === 'custom';
  const classNames = [
    'codex-api-key-usage-panel',
    usageMode ?? 'sub2api',
    variant,
    className,
  ].filter(Boolean).join(' ');

  if (!summary) {
    const emptyText = loading
      ? t('codex.modelProviders.usage.loading', '正在查询额度...')
      : error
        ? error
        : unavailable
          ? t('codex.modelProviders.usage.noKey', '暂无可查询额度')
          : t('codex.modelProviders.usage.pending', '等待查询额度');
    return (
      <div className={`${classNames} empty`}>
        <div className="codex-api-key-usage-empty" title={emptyText}>
          {emptyText}
        </div>
      </div>
    );
  }

  if (!isSupportedUsage) {
    return null;
  }

  if (usageMode === 'token_plan') {
    const resetDetail = summary?.details?.find((item) =>
      ['intervalExpiresAt', 'weeklyExpiresAt', 'expiresAt'].includes(item.key),
    );
    return (
      <div className={classNames}>
        <div className="codex-api-key-usage-grid">
          <div>
            <span>
              {t('codex.modelProviders.usage.fields.remaining', 'Remaining')}
            </span>
            <strong>
              {formatModelProviderUsageMoney(
                summary?.quotaRemaining ?? summary?.remaining,
                summary?.unit,
              )}
            </strong>
          </div>
          <div>
            <span>{t('codex.modelProviders.usage.fields.planName', 'Plan')}</span>
            <strong>{summary?.planName || '-'}</strong>
          </div>
          <div>
            <span>
              {t('codex.modelProviders.usage.fields.expiresAt', 'Next Reset')}
            </span>
            <strong>{resetDetail?.value || '-'}</strong>
          </div>
        </div>
      </div>
    );
  }

  if (usageMode === 'custom') {
    const balance = summary.remaining ?? summary.balance ?? summary.quotaRemaining;
    const money = (value: number) => formatModelProviderUsageMoney(value, summary.unit ?? '');
    const metrics = [
      [t('codex.modelProviders.usage.accountBalance', '账户余额'), balance == null
        ? summary.quotaUnlimited ? t('codex.modelProviders.usage.unlimitedQuota', '无限额度') : null
        : money(balance)],
      [t('codex.modelProviders.usage.limit', '总额度'), summary.quotaLimit == null ? null : money(summary.quotaLimit)],
      [t('codex.modelProviders.usage.used', '已用额度'), summary.quotaUsed == null ? null : money(summary.quotaUsed)],
      [t('codex.modelProviders.usage.fields.todayRequests', '今日请求'), summary.todayRequests == null ? null : formatModelProviderUsageInteger(summary.todayRequests)],
      [t('codex.modelProviders.usage.fields.todayTokens', '今日 Token'), summary.todayTotalTokens == null ? null : formatModelProviderUsageTokenCount(summary.todayTotalTokens)],
      [t('codex.modelProviders.usage.fields.todayCost', '今日费用'), summary.todayCost == null ? null : money(summary.todayCost)],
      [t('codex.modelProviders.usage.fields.totalRequests', '总请求'), summary.totalRequests == null ? null : formatModelProviderUsageInteger(summary.totalRequests)],
      [t('codex.modelProviders.usage.fields.totalTokens', '总 Token'), summary.totalTotalTokens == null ? null : formatModelProviderUsageTokenCount(summary.totalTotalTokens)],
      [t('codex.modelProviders.usage.fields.totalCost', '总费用'), summary.totalCost == null ? null : money(summary.totalCost)],
      [t('codex.modelProviders.usage.fields.isAvailable', '余额可用'), summary.isValid == null ? null : t(summary.isValid ? 'common.yes' : 'common.no')],
    ].filter(([, value]) => value != null);
    return (
      <div className={classNames}>
        <div className="codex-api-key-usage-grid">
          {metrics.length === 0 && <span>{t('dashboard.noData', '暂无数据')}</span>}
          {metrics.map(([label, value]) => (
            <div key={label}><span>{label}</span><strong>{value}</strong></div>
          ))}
        </div>
      </div>
    );
  }

  const balanceText = formatModelProviderUsageMoney(
    summary.remaining ?? summary.balance ?? summary.quotaRemaining,
    summary.unit,
  );
  const todayRequests = formatModelProviderUsageInteger(summary.todayRequests);
  const todayTokens = formatModelProviderUsageTokenCount(summary.todayTotalTokens);

  return (
    <div className={classNames}>
      <div className="codex-api-key-usage-grid">
        <div>
          <span>{t('codex.modelProviders.usage.accountBalance', '账户余额')}</span>
          <strong>{balanceText}</strong>
        </div>
        <div>
          <span>{t('codex.modelProviders.usage.fields.todayRequests', '今日请求')}</span>
          <strong>{todayRequests}</strong>
        </div>
        <div>
          <span>{t('codex.modelProviders.usage.fields.todayTokens', '今日 Token')}</span>
          <strong>{todayTokens}</strong>
        </div>
      </div>
    </div>
  );
}
