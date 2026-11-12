import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { InstancesManager } from '../components/InstancesManager';
import { OverviewTabsHeader } from '../components/OverviewTabsHeader';
import { useAntigravityRuntimeTarget } from '../hooks/useAntigravityRuntimeTarget';
import { useDurableUiSetting } from '../hooks/useDurableUiSetting';
import { useAccountsOverviewFilterState } from '../hooks/useAccountsOverviewFilterState';
import { readAccountsOverviewFilterField, readAccountsOverviewFilterPersistenceEnabled, resolveAccountSortMode } from '../utils/accountsOverviewFilterPersistence';
import { subscribeUserMemory } from '../utils/userMemory';
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
  createAntigravityAccountComparator,
  DEFAULT_ANTIGRAVITY_SORT_BY,
  normalizeAntigravitySortBy,
  normalizeAntigravitySortDirection,
} from '../utils/antigravityAccountSort';
import {
  ANTIGRAVITY_CUSTOM_SORT_ACTIVE_KEY,
  ANTIGRAVITY_FILTER_FIELD_SORT_BY,
  ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
  ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
  readAntigravityCustomSortOrder,
} from './antigravityAccountOverviewModel';

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
  const [rememberedSortBy] = useAccountsOverviewFilterState(
    ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE, ANTIGRAVITY_FILTER_FIELD_SORT_BY,
    () => readAccountsOverviewFilterPersistenceEnabled(ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE)
      ? normalizeAntigravitySortBy(readAccountsOverviewFilterField<string | null>(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE, ANTIGRAVITY_FILTER_FIELD_SORT_BY, null,
        ))
      : DEFAULT_ANTIGRAVITY_SORT_BY,
  );
  const sortBy = resolveAccountSortMode(
    rememberedSortBy, useDurableUiSetting(ANTIGRAVITY_CUSTOM_SORT_ACTIVE_KEY), DEFAULT_ANTIGRAVITY_SORT_BY,
  );
  const [sortDirection] = useAccountsOverviewFilterState(
    ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE, ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
    () => normalizeAntigravitySortDirection(readAccountsOverviewFilterPersistenceEnabled(ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterField<string | null>(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE, ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION, null,
        )
      : null),
  );
  const getOrderSnapshot = () => JSON.stringify(readAntigravityCustomSortOrder());
  const order = useSyncExternalStore(subscribeUserMemory, getOrderSnapshot, getOrderSnapshot);
  const customSortOrderIndex = useMemo(
    () => new Map((JSON.parse(order) as string[]).map((id, index) => [id, index])),
    [order],
  );

  const accountSortComparator = useMemo(
    () =>
      createAntigravityAccountComparator({
        sortBy,
        sortDirection,
        displayGroups,
        currentAccountId: currentAccount?.id ?? null,
        customSortOrderIndex,
      }),
    [currentAccount?.id, customSortOrderIndex, displayGroups, sortBy, sortDirection],
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
