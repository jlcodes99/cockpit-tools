import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useGitHubCopilotInstanceStore } from '../stores/useGitHubCopilotInstanceStore';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import type { GitHubCopilotAccount } from '../types/githubCopilot';
import { getGitHubCopilotAccountDisplayEmail } from '../types/githubCopilot';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import {
  buildGitHubCopilotAccountPresentation,
  buildQuotaPreviewLines,
} from '../presentation/platformAccountPresentation';

/**
 * GitHub Copilot 多开实例内容组件（不包含 header）
 * 用于嵌入到 GitHubCopilotAccountsPage 中
 */
interface GitHubCopilotInstancesContentProps {
  accountsForSelect?: GitHubCopilotAccount[];
}

export function GitHubCopilotInstancesContent({
  accountsForSelect,
}: GitHubCopilotInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useGitHubCopilotInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useGitHubCopilotAccountStore();
  const sourceAccounts = accountsForSelect ?? storeAccounts;
  type AccountForSelect = GitHubCopilotAccount & { email: string };
  const mappedAccountsForSelect = useMemo(
    () =>
      sourceAccounts.map((acc) => ({
        ...acc,
        email: acc.email || getGitHubCopilotAccountDisplayEmail(acc),
      })) as AccountForSelect[],
    [sourceAccounts],
  );
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const renderGitHubCopilotQuotaPreview = (account: AccountForSelect) => {
    const presentation = buildGitHubCopilotAccountPresentation(account, t);
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
      renderAccountQuotaPreview={renderGitHubCopilotQuotaPreview}
      renderAccountBadge={(account) => {
        const presentation = buildGitHubCopilotAccountPresentation(account, t);
        return <span className={`instance-plan-badge ${presentation.planClass}`}>{presentation.planLabel}</span>;
      }}
      getAccountSearchText={(account) => {
        const presentation = buildGitHubCopilotAccountPresentation(account, t);
        return `${presentation.displayName} ${presentation.planLabel}`;
      }}
      appType="vscode"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="githubCopilot.instances.unsupported.descPlatform"
      unsupportedDescDefault="GitHub Copilot 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
