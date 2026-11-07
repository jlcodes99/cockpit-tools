import { MiniMaxCodeQuota } from '../components/minimax/MiniMaxCodeQuota';
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
        const normalizedClass = planBadge.toLowerCase();
        return planBadge ? <span className={`instance-plan-badge ${normalizedClass}`}>{planBadge}</span> : null;
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
