import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useCodebuddyCnInstanceStore } from '../stores/useCodebuddyCnInstanceStore';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import type { CodebuddyCnAccount } from '../types/codebuddy';
import {
  getCodebuddyAccountDisplayEmail,
  getCodebuddyPlanBadge,
  getCodebuddyUsage,
} from '../types/codebuddy';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { DosageNotifyQuotaPreview } from '../components/platform/DosageNotifyQuotaPreview';
import { CheckCircle } from 'lucide-react';

interface CodebuddyCnInstancesContentProps {
  accountsForSelect?: CodebuddyCnAccount[];
}

export function CodebuddyCnInstancesContent({
  accountsForSelect,
}: CodebuddyCnInstancesContentProps = {}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || 'zh-CN';
  const instanceStore = useCodebuddyCnInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useCodebuddyCnAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderCodebuddyCnQuotaPreview = (account: CodebuddyCnAccount) => {
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
    <PlatformInstancesContent<CodebuddyCnAccount>
      instanceStore={instanceStore}
      accounts={sourceAccounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderCodebuddyCnQuotaPreview}
      renderAccountBadge={(account) => {
        const planBadge = getCodebuddyPlanBadge(account);
        const normalizedClass = planBadge.toLowerCase();

        // 检查今日是否已签到
        const isCheckedIn = account.last_checkin_time
          ? (() => {
              const today = new Date();
              const lastCheckinDate = new Date(account.last_checkin_time!);
              return (
                today.getFullYear() === lastCheckinDate.getFullYear() &&
                today.getMonth() === lastCheckinDate.getMonth() &&
                today.getDate() === lastCheckinDate.getDate()
              );
            })()
          : false;

        return (
          <div className="badge-group inline-flex items-center gap-1">
            <span className={`instance-plan-badge ${normalizedClass}`}>{planBadge}</span>
            {isCheckedIn && (
              <span className="checkin-badge badge badge-success badge-sm" title={t('codebuddyCn.checkin.checkedIn', "Checked In")}>
                <CheckCircle size={12} />
              </span>
            )}
          </div>
        );
      }}
      getAccountSearchText={(account) => `${getCodebuddyAccountDisplayEmail(account)} ${getCodebuddyPlanBadge(account)}`}
      appType="codebuddy_cn"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="codebuddyCn.instances.unsupported.descPlatform"
      unsupportedDescDefault="CodeBuddy CN 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
