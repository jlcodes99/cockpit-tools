import { useTranslation } from 'react-i18next';
import { getCodexQuotaWindows, getCodexSubscriptionPresentationForAccount, isCodexChatgptPlanAccount, type CodexAccount } from '../../types/codex';
import { getCodexServiceAccountDisplay } from '../../utils/chatgptPlanOverview';
import { formatCodexQuotaPoolPercent, formatCodexQuotaPoolWindowLabel } from '../../utils/codexQuotaPool';

/** Account details are read-only; service actions keep the original endpoint. */
export function CodexServiceAccountDetails({ account, mask }: {
  account: CodexAccount;
  mask: (value: string) => string;
}) {
  const { t } = useTranslation();
  const display = getCodexServiceAccountDisplay(account);
  const windows = getCodexQuotaWindows(display.quota);
  const proxy = display.egress_proxy;
  const proxyLabel = proxy?.name || proxy?.selectedName || proxy?.sourceName
    || (proxy?.server ? `${proxy.server}${proxy.port ? `:${proxy.port}` : ''}` : undefined);
  const subscription = display.subscription_active_until
    ? getCodexSubscriptionPresentationForAccount(display, t) : undefined;
  return <div className="codex-api-service-account-meta" aria-label={t('chatgptPlan.officialFlow.sourceDetails')}>
    {display.account_name && display.account_name !== display.email && <span title={mask(display.email)}>{mask(display.email)}</span>}
    {display.tags?.map((tag) => <span key={tag}>{tag}</span>)}
    {display.account_note && <span title={mask(display.account_note)}>{mask(display.account_note)}</span>}
    {proxyLabel && <span>{t('codex.proxy.action')} · {mask(proxyLabel)}</span>}
    {windows.map((window) => <span key={window.id} title={isCodexChatgptPlanAccount(account) ? t('chatgptPlan.officialFlow.sourceQuota') : undefined}>
      {isCodexChatgptPlanAccount(account) ? `${t('chatgptPlan.officialFlow.sourceQuota')} · ` : ''}
      {formatCodexQuotaPoolWindowLabel(window.label, t('codex.localAccess.quotaPool.weeklyShort'))} {formatCodexQuotaPoolPercent(window.percentage)}
    </span>)}
    {subscription && <span title={subscription.titleText}>{subscription.valueText} · {subscription.detailText}</span>}
  </div>;
}
