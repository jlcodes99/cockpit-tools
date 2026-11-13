import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useWorkbuddyInstanceStore } from '../stores/useWorkbuddyInstanceStore';
import { useWorkbuddyAccountStore } from '../stores/useWorkbuddyAccountStore';
import type { WorkbuddyAccount } from '../types/workbuddy';
import {
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
} from '../types/workbuddy';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { DosageNotifyQuotaPreview } from '../components/platform/DosageNotifyQuotaPreview';

interface WorkbuddyInstancesContentProps {
  accountsForSelect?: WorkbuddyAccount[];
}

export function WorkbuddyInstancesContent({
  accountsForSelect,
}: WorkbuddyInstancesContentProps = {}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || 'zh-CN';
  const instanceStore = useWorkbuddyInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useWorkbuddyAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderWorkbuddyQuotaPreview = (account: WorkbuddyAccount) => {
    const usage = getWorkbuddyUsage(account);
    return (
      <DosageNotifyQuotaPreview
        usage={usage}
        locale={locale}
        emptyText={t('instances.quota.empty', '暂无配额缓存')}
        normalText={t('workbuddy.usageNormal', '正常')}
        abnormalText={t('workbuddy.usageAbnormal', '异常')}
        abnormalDisplay="short"
      />
    );
  };

  return (
    <PlatformInstancesContent<WorkbuddyAccount>
      instanceStore={instanceStore}
      accounts={sourceAccounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderWorkbuddyQuotaPreview}
      renderAccountBadge={(account) => {
        const planBadge = getWorkbuddyPlanBadge(account);
        const normalizedClass = planBadge.toLowerCase();
        return <span className={`tier-badge account-picker-plan ${normalizedClass}`}>{planBadge}</span>;
      }}
      getAccountSearchText={(account) => `${getWorkbuddyAccountDisplayEmail(account)} ${getWorkbuddyPlanBadge(account)}`}
      appType="workbuddy"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="workbuddy.instances.unsupported.descPlatform"
      unsupportedDescDefault="WorkBuddy 应用多开仅支持 macOS、Windows 和 Linux。"
    />
  );
}
