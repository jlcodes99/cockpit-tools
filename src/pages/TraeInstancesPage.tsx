import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useTraeInstanceStore } from '../stores/useTraeInstanceStore';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';
import type { TraeAccount } from '../types/trae';
import {
  getTraeAccountDisplayEmail,
  getTraeAccountDisplayName,
  getTraePlanBadgeClass,
  getTraePlanDisplayName,
  getTraeUsage,
} from '../types/trae';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';

interface TraeInstancesContentProps {
  accountsForSelect?: TraeAccount[];
}

function formatUsd(value: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--';
  return `$${value.toFixed(2).replace(/\.00$/, '')}`;
}

export function TraeInstancesContent({
  accountsForSelect,
}: TraeInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useTraeInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useTraeAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderTraeQuotaPreview = (account: TraeAccount) => {
    const usage = getTraeUsage(account);
    if (usage.usedPercent == null && usage.spentUsd == null && usage.totalUsd == null) {
      return <span className="account-quota-empty">{t('instances.quota.empty', '暂无配额缓存')}</span>;
    }

    const usageText = usage.usedPercent == null ? '--' : `${usage.usedPercent}%`;
    const costText = `${formatUsd(usage.spentUsd)} / ${formatUsd(usage.totalUsd)}`;
    const quotaClass = usage.usedPercent != null && usage.usedPercent >= 90
      ? 'low'
      : usage.usedPercent != null && usage.usedPercent >= 70
      ? 'medium'
      : 'high';

    return (
      <div className="account-quota-preview">
        <span className="account-quota-item">
          <span className={`quota-dot ${quotaClass}`} />
          <span className={`quota-text ${quotaClass}`}>{usageText}</span>
        </span>
        <span className="account-quota-item">
          <span className="quota-dot high" />
          <span className="quota-text high">{costText}</span>
        </span>
      </div>
    );
  };

  return (
    <PlatformInstancesContent<TraeAccount>
      instanceStore={instanceStore}
      accounts={sourceAccounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderTraeQuotaPreview}
      renderAccountBadge={(account) => (
        <span className={`instance-plan-badge ${getTraePlanBadgeClass(getTraePlanDisplayName(account))}`}>
          {getTraePlanDisplayName(account)}
        </span>
      )}
      getAccountSearchText={(account) =>
        `${getTraeAccountDisplayName(account)} ${getTraeAccountDisplayEmail(account)} ${getTraePlanDisplayName(account)}`
      }
      appType="trae"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="common.shared.instances.unsupported.desc"
      unsupportedDescDefault="Trae 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
