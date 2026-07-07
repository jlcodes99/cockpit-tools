import { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus } from 'lucide-react';
import type { InstanceStoreState } from '../../stores/createInstanceStore';
import type { InstanceLaunchMode, InstanceProfile } from '../../types/instance';
import { InstancesManager } from '../InstancesManager';

type AccountLike = {
  id: string;
  email: string;
};

type InstancesAppType =
  | 'antigravity'
  | 'antigravity_ide'
  | 'codex'
  | 'claude'
  | 'vscode'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'workbuddy';

interface PlatformInstancesContentProps<TAccount extends AccountLike> {
  instanceStore: InstanceStoreState;
  accounts: TAccount[];
  fetchAccounts: () => Promise<void>;
  renderAccountQuotaPreview: (account: TAccount) => ReactNode;
  renderAccountBadge?: (account: TAccount) => ReactNode;
  getAccountDisplayText?: (account: TAccount) => string;
  getAccountSearchText: (account: TAccount) => string;
  appType: InstancesAppType;
  isSupported: boolean;
  unsupportedTitleKey: string;
  unsupportedTitleDefault: string;
  unsupportedDescKey: string;
  unsupportedDescDefault: string;
  onInstanceStarted?: (instance: InstanceProfile) => void | Promise<void>;
  resolveStartSuccessMessage?: (instance: InstanceProfile) => string;
  isAccountAllowedForLaunchMode?: (account: TAccount, launchMode: InstanceLaunchMode) => boolean;
  toolbarExtraActions?: ReactNode;
}

export function PlatformInstancesContent<TAccount extends AccountLike>({
  instanceStore,
  accounts,
  fetchAccounts,
  renderAccountQuotaPreview,
  renderAccountBadge,
  getAccountDisplayText,
  getAccountSearchText,
  appType,
  isSupported,
  unsupportedTitleKey,
  unsupportedTitleDefault,
  unsupportedDescKey,
  unsupportedDescDefault,
  onInstanceStarted,
  resolveStartSuccessMessage,
  isAccountAllowedForLaunchMode,
  toolbarExtraActions,
}: PlatformInstancesContentProps<TAccount>) {
  const { t } = useTranslation();

  if (!isSupported) {
    return (
      <div className="instances-page">
        <div className="empty-state">
          <h3>{t(unsupportedTitleKey, unsupportedTitleDefault)}</h3>
          <p>{t(unsupportedDescKey, unsupportedDescDefault)}</p>
          <button className="btn btn-primary" disabled>
            <Plus size={16} />
            {t('instances.actions.create', '新建实例')}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="instances-page">
      <InstancesManager<TAccount>
        instanceStore={instanceStore}
        accounts={accounts}
        fetchAccounts={fetchAccounts}
        renderAccountQuotaPreview={renderAccountQuotaPreview}
        renderAccountBadge={renderAccountBadge}
        getAccountDisplayText={getAccountDisplayText}
        getAccountSearchText={getAccountSearchText}
        appType={appType}
        onInstanceStarted={onInstanceStarted}
        resolveStartSuccessMessage={resolveStartSuccessMessage}
        isAccountAllowedForLaunchMode={isAccountAllowedForLaunchMode}
        toolbarExtraActions={toolbarExtraActions}
      />
    </div>
  );
}
