import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useCodebuddyInstanceStore } from '../stores/useCodebuddyInstanceStore';
import { useCodebuddyAccountStore } from '../stores/useCodebuddyAccountStore';
import type { CodebuddyAccount } from '../types/codebuddy';
import {
  getCodebuddyAccountDisplayEmail,
  getCodebuddyPlanBadge,
  getCodebuddyUsage,
} from '../types/codebuddy';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { DosageNotifyQuotaPreview } from '../components/platform/DosageNotifyQuotaPreview';

interface CodebuddyInstancesContentProps {
  accountsForSelect?: CodebuddyAccount[];
}

export function CodebuddyInstancesContent({
  accountsForSelect,
}: CodebuddyInstancesContentProps = {}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || 'zh-CN';
  const instanceStore = useCodebuddyInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useCodebuddyAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderCodebuddyQuotaPreview = (account: CodebuddyAccount) => {
    const usage = getCodebuddyUsage(account);
    return (
      <DosageNotifyQuotaPreview
        usage={usage}
        locale={locale}
        emptyText={t('instances.quota.empty', "No cached quota")}
        normalText={t('codebuddy.usageNormal', "Normal")}
        abnormalText={t('codebuddy.usageAbnormal', "Abnormal")}
        abnormalDisplay="detail"
      />
    );
  };

  return (
    <PlatformInstancesContent<CodebuddyAccount>
      instanceStore={instanceStore}
      accounts={sourceAccounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderCodebuddyQuotaPreview}
      renderAccountBadge={(account) => {
        const planBadge = getCodebuddyPlanBadge(account);
        const normalizedClass = planBadge.toLowerCase();
        return <span className={`instance-plan-badge ${normalizedClass}`}>{planBadge}</span>;
      }}
      getAccountSearchText={(account) => `${getCodebuddyAccountDisplayEmail(account)} ${getCodebuddyPlanBadge(account)}`}
      appType="codebuddy"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="codebuddy.instances.unsupported.descPlatform"
      unsupportedDescDefault="CodeBuddy 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
