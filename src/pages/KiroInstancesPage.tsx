import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useKiroInstanceStore } from '../stores/useKiroInstanceStore';
import { useKiroAccountStore } from '../stores/useKiroAccountStore';
import type { KiroAccount } from '../types/kiro';
import { getKiroAccountDisplayEmail } from '../types/kiro';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import {
  buildKiroAccountPresentation,
  buildQuotaPreviewLines,
} from '../presentation/platformAccountPresentation';

/**
 * Kiro 多开实例内容组件（不包含 header）
 * 用于嵌入到 KiroAccountsPage 中
 */
interface KiroInstancesContentProps {
  accountsForSelect?: KiroAccount[];
}

export function KiroInstancesContent({
  accountsForSelect,
}: KiroInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useKiroInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useKiroAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  type AccountForSelect = KiroAccount & { email: string };
  const mappedAccountsForSelect = useMemo(
    () =>
      sourceAccounts.map((acc) => ({
        ...acc,
        email: acc.email || getKiroAccountDisplayEmail(acc),
      })) as AccountForSelect[],
    [sourceAccounts],
  );
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderKiroQuotaPreview = (account: AccountForSelect) => {
    const presentation = buildKiroAccountPresentation(account, t);
    const lines = buildQuotaPreviewLines(presentation.quotaItems, 3);
    if (lines.length === 0) {
      return <span className="account-quota-empty">{t('instances.quota.empty', "No cached quota")}</span>;
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
      renderAccountQuotaPreview={renderKiroQuotaPreview}
      renderAccountBadge={(account) => {
        const presentation = buildKiroAccountPresentation(account, t);
        return <span className={`instance-plan-badge ${presentation.planClass}`}>{presentation.planLabel}</span>;
      }}
      getAccountSearchText={(account) => {
        const presentation = buildKiroAccountPresentation(account, t);
        return `${presentation.displayName} ${presentation.planLabel}`;
      }}
      appType="kiro"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="kiro.instances.unsupported.descPlatform"
      unsupportedDescDefault="Kiro 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
