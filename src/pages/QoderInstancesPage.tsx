import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { QODER_INSTANCE_STORES } from '../stores/useQoderInstanceStore';
import { useQoderChannelAccountStore } from '../stores/useQoderAccountStore';
import type { QoderAccount, QoderPlatformId } from '../types/qoder';
import {
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  getQoderUsage,
} from '../types/qoder';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';

interface QoderInstancesContentProps {
  platformId?: QoderPlatformId;
  accountsForSelect?: QoderAccount[];
}

export function QoderInstancesContent({
  platformId = 'qoder',
  accountsForSelect,
}: QoderInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = QODER_INSTANCE_STORES[platformId]();
  const accountStore = useQoderChannelAccountStore(platformId);
  const accounts = accountsForSelect ?? accountStore.accounts;
  const fetchAccounts = accountStore.fetchAccounts;
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
        const normalizedClass = plan.toLowerCase().replace(/[^a-z0-9]+/g, '-');
        return <span className={`instance-plan-badge ${normalizedClass}`}>{plan}</span>;
      }}
      getAccountSearchText={(account) =>
        `${getQoderAccountDisplayEmail(account)} ${getQoderPlanBadge(account)}`
      }
      appType={platformId}
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="qoder.instances.unsupported.descPlatform"
      unsupportedDescDefault="Qoder 应用多开仅支持 macOS、Windows 和 Linux。"
    />
  );
}

export function QoderInstancesPage({ platformId = 'qoder' }: { platformId?: QoderPlatformId } = {}) {
  return (
    <div className="ghcp-instances-page">
      <QoderInstancesContent platformId={platformId} />
    </div>
  );
}

