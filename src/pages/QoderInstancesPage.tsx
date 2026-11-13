import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useQoderInstanceStore } from '../stores/useQoderInstanceStore';
import { useQoderAccountStore } from '../stores/useQoderAccountStore';
import type { QoderAccount } from '../types/qoder';
import {
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  resolveQoderPlanBadgeClass,
  getQoderUsage,
} from '../types/qoder';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';

interface QoderInstancesContentProps {
  accountsForSelect?: QoderAccount[];
}

export function QoderInstancesContent({
  accountsForSelect,
}: QoderInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useQoderInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useQoderAccountStore();
  const accounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderQoderQuotaPreview = (account: QoderAccount) => {
    const usage = getQoderUsage(account);
    if (usage.inlineSuggestionsUsedPercent == null) {
      return (
        <span className="account-quota-empty">
          {t('instances.quota.empty', '暂无配额缓存')}
        </span>
      );
    }
    const percentage = Math.max(0, Math.min(100, Math.round(usage.inlineSuggestionsUsedPercent)));
    const quotaClass = percentage >= 90 ? 'low' : percentage >= 70 ? 'medium' : 'high';

    return (
      <div className="account-quota-preview">
        <span className="account-quota-item">
          <span className={`quota-dot ${quotaClass}`} />
          <span className={`quota-text ${quotaClass}`}>{percentage}% used</span>
        </span>
        {usage.creditsRemaining != null && usage.creditsTotal != null && (
          <span className="account-quota-extra">
            {usage.creditsRemaining.toFixed(0)} / {usage.creditsTotal.toFixed(0)}
          </span>
        )}
      </div>
    );
  };

  return (
    <PlatformInstancesContent<QoderAccount>
      instanceStore={instanceStore}
      accounts={accounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderQoderQuotaPreview}
      renderAccountBadge={(account) => {
        const plan = getQoderPlanBadge(account);
        return <span className={`tier-badge account-picker-plan raw-value ${resolveQoderPlanBadgeClass(plan)}`}>{plan}</span>;
      }}
      getAccountSearchText={(account) =>
        `${getQoderAccountDisplayEmail(account)} ${getQoderPlanBadge(account)}`
      }
      appType="qoder"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="qoder.instances.unsupported.descPlatform"
      unsupportedDescDefault="Qoder 应用多开仅支持 macOS、Windows 和 Linux。"
    />
  );
}
