import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InstancesManager } from '../components/InstancesManager';
import { OverviewTabsHeader } from '../components/OverviewTabsHeader';
import { useAntigravityRuntimeTarget } from '../hooks/useAntigravityRuntimeTarget';
import { useAccountStore } from '../stores/useAccountStore';
import { useAntigravityLegacyInstanceStore } from '../stores/useAntigravityLegacyInstanceStore';
import { useInstanceStore } from '../stores/useInstanceStore';
import type { Account } from '../types/account';
import { Page } from '../types/navigation';
import { DisplayGroup, getDisplayGroups } from '../services/groupService';
import {
  buildAntigravityAccountPresentation,
  buildQuotaPreviewLines,
} from '../presentation/platformAccountPresentation';
import {
  ANTIGRAVITY_ACCOUNTS_SORT_BY_STORAGE_KEY,
  ANTIGRAVITY_ACCOUNTS_SORT_DIRECTION_STORAGE_KEY,
  createAntigravityAccountComparator,
  normalizeAntigravitySortBy,
  normalizeAntigravitySortDirection,
} from '../utils/antigravityAccountSort';

interface InstancesPageProps {
  onNavigate?: (page: Page) => void;
}

export function InstancesPage({ onNavigate }: InstancesPageProps) {
  const { t } = useTranslation();
  const runtimeTarget = useAntigravityRuntimeTarget();
  const legacyInstanceStore = useAntigravityLegacyInstanceStore();
  const ideInstanceStore = useInstanceStore();
  const instanceStore =
    runtimeTarget === 'antigravity' ? legacyInstanceStore : ideInstanceStore;
  const { accounts, currentAccountsByTarget, fetchAccounts, fetchCurrentAccount } = useAccountStore();
  const currentAccount = currentAccountsByTarget[runtimeTarget] ?? null;
  const [displayGroups, setDisplayGroups] = useState<DisplayGroup[]>([]);
  const [sortBy] = useState(() =>
    normalizeAntigravitySortBy(
      localStorage.getItem(ANTIGRAVITY_ACCOUNTS_SORT_BY_STORAGE_KEY),
    ),
  );
  const [sortDirection] = useState(() =>
    normalizeAntigravitySortDirection(
      localStorage.getItem(ANTIGRAVITY_ACCOUNTS_SORT_DIRECTION_STORAGE_KEY),
    ),
  );

  const accountSortComparator = useMemo(
    () =>
      createAntigravityAccountComparator({
        sortBy,
        sortDirection,
        displayGroups,
        currentAccountId: currentAccount?.id ?? null,
      }),
    [currentAccount?.id, displayGroups, sortBy, sortDirection],
  );
  const sortedAccountsForSelect = useMemo(
    () => [...accounts].sort(accountSortComparator),
    [accountSortComparator, accounts],
  );

  useEffect(() => {
    fetchAccounts();
    fetchCurrentAccount(runtimeTarget);
  }, [fetchAccounts, fetchCurrentAccount, runtimeTarget]);

  useEffect(() => {
    getDisplayGroups()
      .then((groups) => {
        setDisplayGroups(groups);
      })
      .catch((error) => {
        console.error('Failed to load display groups:', error);
      });
  }, []);

  useEffect(() => {
    if (runtimeTarget === 'antigravity_cli') {
      onNavigate?.('overview');
    }
  }, [onNavigate, runtimeTarget]);

  const renderAccountQuotaPreview = (account: Account) => {
    const presentation = buildAntigravityAccountPresentation(account, displayGroups, t);
    const lines = buildQuotaPreviewLines(presentation.quotaItems, 3);
    if (lines.length === 0) {
      return <span className="account-quota-empty">{t('instances.quota.empty', '暂无配额缓存')}</span>;
    }
    return (
      <div className="account-quota-preview">
        {lines.map((line) => (
          <span className="account-quota-item" key={`${account.id}-${line.key}`}>
            <span className={`quota-dot ${line.quotaClass}`} />
            <span className={`quota-text ${line.quotaClass}`}>
              {line.text}
            </span>
          </span>
        ))}
      </div>
    );
  };

  if (runtimeTarget === 'antigravity_cli') {
    return null;
  }

  return (
    <div className="instances-page">
      <OverviewTabsHeader
        active="instances"
        onNavigate={onNavigate}
        subtitle={t('instances.subtitle', '多实例独立配置，多账号并行运行。')}
      />
      <InstancesManager
        instanceStore={instanceStore}
        accounts={sortedAccountsForSelect}
        fetchAccounts={fetchAccounts}
        renderAccountQuotaPreview={renderAccountQuotaPreview}
        renderAccountBadge={(account) => {
          const presentation = buildAntigravityAccountPresentation(account, displayGroups, t);
          return (
            <span className={`instance-plan-badge ${presentation.planClass}`}>{presentation.planLabel}</span>
          );
        }}
        getAccountSearchText={(account) => {
          const presentation = buildAntigravityAccountPresentation(account, displayGroups, t);
          return `${presentation.displayName} ${presentation.planLabel} ${account.name ?? ''}`;
        }}
        appType={runtimeTarget}
      />
    </div>
  );
}
