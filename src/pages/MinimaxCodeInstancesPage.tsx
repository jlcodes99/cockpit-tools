import { MiniMaxCodeQuota } from '../components/minimax/MiniMaxCodeQuota';
import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useMinimaxCodeInstanceStore } from '../stores/useMinimaxCodeInstanceStore';
import { useMinimaxCodeAccountStore } from '../stores/useMinimaxCodeAccountStore';
import type { MinimaxCodeAccount } from '../types/minimaxCode';
import {
  getMinimaxCodeAccountDisplayEmail,
  getMinimaxCodePlanBadge,
} from '../types/minimaxCode';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';

interface MinimaxCodeInstancesContentProps {
  accountsForSelect?: MinimaxCodeAccount[];
}

export function MinimaxCodeInstancesContent({
  accountsForSelect,
}: MinimaxCodeInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useMinimaxCodeInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useMinimaxCodeAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderMinimaxCodeQuotaPreview = (account: MinimaxCodeAccount) => <MiniMaxCodeQuota account={account} compact />;

  return (
    <PlatformInstancesContent<MinimaxCodeAccount>
      instanceStore={instanceStore}
      accounts={sourceAccounts}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderMinimaxCodeQuotaPreview}
      renderAccountBadge={(account) => {
        const planBadge = getMinimaxCodePlanBadge(account);
        return <span className="tier-badge account-picker-plan free">{planBadge || t('common.unknown')}</span>;
      }}
      getAccountDisplayText={getMinimaxCodeAccountDisplayEmail}
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
