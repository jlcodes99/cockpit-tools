import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useWindsurfInstanceStore } from '../stores/useWindsurfInstanceStore';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import type { WindsurfAccount } from '../types/windsurf';
import { getWindsurfAccountDisplayEmail } from '../types/windsurf';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import {
  buildQuotaPreviewLines,
  buildWindsurfAccountPresentation,
} from '../presentation/platformAccountPresentation';

/**
 * Windsurf 多开实例内容组件（不包含 header）
 * 用于嵌入到 WindsurfAccountsPage 中
 */
interface WindsurfInstancesContentProps {
  accountsForSelect?: WindsurfAccount[];
}

export function WindsurfInstancesContent({
  accountsForSelect,
}: WindsurfInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useWindsurfInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useWindsurfAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  type AccountForSelect = WindsurfAccount & { email: string };
  const mappedAccountsForSelect = useMemo(
    () =>
      sourceAccounts.map((acc) => ({
        ...acc,
        email: acc.email || getWindsurfAccountDisplayEmail(acc),
      })) as AccountForSelect[],
    [sourceAccounts],
  );
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderWindsurfQuotaPreview = (account: AccountForSelect) => {
    const presentation = buildWindsurfAccountPresentation(account, t);
    const lines = buildQuotaPreviewLines(presentation.quotaItems, 3);
    if (lines.length === 0) {
      return <span className="account-quota-empty">{t('instances.quota.empty', '暂无配额缓存')}</span>;
    }
    return (
      <div className="account-quota-preview">
        {lines.map((line) => (
          <span className="account-quota-item" key={line.key}>
            <span className={`quota-dot ${line.quotaClass}`} />
            <span className={`quota-text ${line.quotaClass}`}>{line.text}</span>
          </span>
        ))}
      </div>
    );
  };

  return (
    <PlatformInstancesContent<AccountForSelect>
      instanceStore={instanceStore}
      accounts={mappedAccountsForSelect}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderWindsurfQuotaPreview}
      renderAccountBadge={(account) => {
        const presentation = buildWindsurfAccountPresentation(account, t);
        return <span className={`instance-plan-badge ${presentation.planClass}`}>{presentation.planLabel}</span>;
      }}
      getAccountSearchText={(account) => {
        const presentation = buildWindsurfAccountPresentation(account, t);
        return `${presentation.displayName} ${presentation.planLabel}`;
      }}
      appType="windsurf"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="windsurf.instances.unsupported.descPlatform"
      unsupportedDescDefault="Devin 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
