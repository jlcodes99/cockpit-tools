import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useMinimaxCodeInstanceStore } from '../stores/useMinimaxCodeInstanceStore';
import { useMinimaxCodeAccountStore } from '../stores/useMinimaxCodeAccountStore';
import type { MinimaxCodeAccount } from '../types/minimaxCode';
import {
  getMinimaxCodeAccountDisplayEmail,
  getMinimaxCodePlanBadge,
  getMinimaxCodeUsage,
} from '../types/minimaxCode';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { DosageNotifyQuotaPreview } from '../components/platform/DosageNotifyQuotaPreview';

interface MinimaxCodeInstancesContentProps {
  accountsForSelect?: MinimaxCodeAccount[];
}

export function MinimaxCodeInstancesContent({
  accountsForSelect,
}: MinimaxCodeInstancesContentProps = {}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || 'zh-CN';
  const instanceStore = useMinimaxCodeInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useMinimaxCodeAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderMinimaxCodeQuotaPreview = (account: MinimaxCodeAccount) => {
    const usage = getMinimaxCodeUsage(account);
    return (
      <DosageNotifyQuotaPreview
        usage={{
          isNormal: true,
          dosageNotifyCode: usage.code ?? undefined,
          dosageNotifyZh: usage.zh ?? undefined,
          dosageNotifyEn: usage.en ?? undefined,
        }}
        locale={locale}
        emptyText={t('instances.quota.empty', '暂无配额缓存')}
        normalText={t('minimaxCode.usageNormal', '正常')}
        abnormalText={t('minimaxCode.usageAbnormal', '异常')}
        abnormalDisplay="short"
      />
    );
  };

  return (
    <PlatformInstancesContent<MinimaxCodeAccount>
      instanceStore={instanceStore}
      accounts={sourceAccounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderMinimaxCodeQuotaPreview}
      renderAccountBadge={(account) => {
        const planBadge = getMinimaxCodePlanBadge(account);
        const normalizedClass = planBadge.toLowerCase();
        return <span className={`instance-plan-badge ${normalizedClass}`}>{planBadge}</span>;
      }}
      getAccountSearchText={(account) => `${getMinimaxCodeAccountDisplayEmail(account)} ${getMinimaxCodePlanBadge(account)}`}
      appType="minimax_code"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="minimaxCode.instances.unsupported.descPlatform"
      unsupportedDescDefault="MiniMax Code 应用多开仅支持 macOS、Windows 和 Linux。"
    />
  );
}
