import { useState, useMemo, useCallback, useEffect, Fragment } from 'react';
import {
  Plus,
  RefreshCw,
  Download,
  Upload,
  Trash2,
  X,
  Globe,
  Database,
  Copy,
  Check,
  ChevronLeft,
  KeyRound,
  Play,
  RotateCw,
  CircleAlert,
  ChevronDown,
  LayoutGrid,
  List,
  Search,
  ArrowDownWideNarrow,
  Tag,
  Eye,
  EyeOff,
  BookOpen,
} from 'lucide-react';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { PaginationControls } from '../components/PaginationControls';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import {
  PlatformOverviewTab,
  PlatformOverviewTabsHeader,
} from '../components/platform/PlatformOverviewTabsHeader';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import { TraeInstancesContent } from './TraeInstancesPage';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';
import * as traeService from '../services/traeService';
import type { TraeAccount } from '../types/trae';
import {
  getTraeAccountDisplayEmail,
  getTraeAccountDisplayName,
  getTraeLoginProvider,
  getTraePlanBadge,
  getTraePlanBadgeClass,
  getTraePlanDisplayName,
  getTraeUsage,
  hasTraeQuotaData,
  TRAE_PRODUCT_TYPE,
} from '../types/trae';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
  VALID_ACCOUNTS_FILTER_VALUE,
} from '../utils/accountValidityFilter';
import {
  buildPaginatedGroups,
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from '../hooks/usePagination';
import {
  normalizeAccountsOverviewScope,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from '../utils/accountsOverviewFilterPersistence';

const TRAE_CURRENT_ACCOUNT_ID_KEY = 'agtools.trae.current_account_id';
const TRAE_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.trae.flow_notice_collapsed';
const TRAE_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('Trae');
const FILTER_TYPES_FIELD = 'filter_types';
const TRAE_KNOWN_SORT_KEYS = ['created_at', 'plan', 'quota'] as const;

type TraeQuotaSummary = {
  percentage: number | null;
  percentageText: string;
  quotaClass: 'high' | 'medium' | 'critical';
  costText: string;
  statusText: string;
  statusTone: 'normal' | 'warning' | 'unknown';
  bonusText: string;
  resetText: string;
  packageText: string;
  payAsYouGoText: string;
};

function formatNumber(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return '--';
  const hasDecimal = Math.abs(value - Math.trunc(value)) > 0.001;
  return new Intl.NumberFormat('en-US', {
    maximumFractionDigits: hasDecimal ? 2 : 0,
  }).format(value);
}

function computeQuotaClass(percent: number | null): 'high' | 'medium' | 'critical' {
  if (percent == null) return 'high';
  if (percent >= 90) return 'critical';
  if (percent >= 70) return 'medium';
  return 'high';
}

function formatTraeResetAt(timestamp: number): string {
  const date = new Date(timestamp * 1000);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}/${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}`;
}

function formatTraeMoney(value: number | null | undefined): string {
  return `$${formatNumber(value)}`;
}

export function TraeAccountsPage() {
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    readAccountsOverviewFilterPersistenceEnabled(TRAE_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterStringArray(TRAE_FILTER_PERSISTENCE_SCOPE, FILTER_TYPES_FIELD)
      : [],
  );
  const untaggedKey = '__untagged__';

  const store = useTraeAccountStore();

  const page = useProviderAccountsPage<TraeAccount>({
    platformKey: 'Trae',
    oauthLogPrefix: 'TraeOAuth',
    flowNoticeCollapsedKey: TRAE_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: TRAE_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'trae_accounts',
    store: {
      accounts: store.accounts,
      currentAccountId: store.currentAccountId,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      fetchCurrentAccountId: store.fetchCurrentAccountId,
      deleteAccounts: store.deleteAccounts,
      refreshToken: store.refreshToken,
      refreshAllTokens: store.refreshAllTokens,
      setCurrentAccountId: store.setCurrentAccountId,
      updateAccountTags: store.updateAccountTags,
    },
    oauthService: {
      startLogin: traeService.traeOauthLoginStart,
      completeLogin: (loginId: string) => traeService.traeOauthLoginComplete(loginId),
      cancelLogin: (loginId?: string) => traeService.traeOauthLoginCancel(loginId),
      submitCallbackUrl: (loginId: string, callbackUrl: string) =>
        traeService.traeOauthSubmitCallbackUrl(loginId, callbackUrl),
    },
    dataService: {
      importFromJson: traeService.importTraeFromJson,
      importFromLocal: traeService.importTraeFromLocal,
      exportAccounts: traeService.exportTraeAccounts,
      injectToVSCode: traeService.injectTraeAccount,
    },
    getDisplayEmail: (account) => getTraeAccountDisplayEmail(account),
  });

  const {
    t,
    maskAccountText,
    privacyModeEnabled,
    togglePrivacyMode,
    viewMode,
    setViewMode,
    searchQuery,
    setSearchQuery,
    filterPersistenceEnabled,
    filterPersistenceScope,
    sortBy,
    setSortBy,
    sortDirection,
    setSortDirection,
    selected,
    toggleSelect,
    toggleSelectAll,
    tagFilter,
    groupByTag,
    setGroupByTag,
    showTagFilter,
    setShowTagFilter,
    showTagModal,
    setShowTagModal,
    tagFilterRef,
    availableTags,
    toggleTagFilterValue,
    clearTagFilter,
    tagDeleteConfirm,
    tagDeleteConfirmError,
    tagDeleteConfirmErrorScrollKey,
    setTagDeleteConfirm,
    deletingTag,
    requestDeleteTag,
    confirmDeleteTag,
    openTagModal,
    handleSaveTags,
    refreshing,
    refreshingAll,
    injecting,
    handleRefresh,
    handleRefreshAll,
    handleDelete,
    handleBatchDelete,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    setDeleteConfirm,
    deleting,
    confirmDelete,
    message,
    setMessage,
    exporting,
    handleExport,
    handleExportByIds,
    getScopedSelectedCount,
    showExportModal,
    closeExportModal,
    exportJsonContent,
    exportJsonHidden,
    toggleExportJsonHidden,
    exportJsonCopied,
    copyExportJson,
    savingExportJson,
    saveExportJson,
    exportSavedPath,
    canOpenExportSavedDirectory,
    openExportSavedDirectory,
    copyExportSavedPath,
    exportPathCopied,
    showAddModal,
    addTab,
    addStatus,
    addMessage,
    tokenInput,
    setTokenInput,
    importing,
    openAddModal,
    closeAddModal,
    handleTokenImport,
    handleImportJsonFile,
    handleImportFromLocal,
    handlePickImportFile,
    importFileInputRef,
    handleInjectToVSCode,
    oauthUrl,
    oauthUrlCopied,
    oauthMeta,
    oauthPrepareError,
    oauthCompleteError,
    oauthPolling,
    oauthTimedOut,
    oauthManualCallbackInput,
    setOauthManualCallbackInput,
    oauthManualCallbackSubmitting,
    oauthManualCallbackError,
    oauthSupportsManualCallback,
    handleCopyOauthUrl,
    handleRetryOauth,
    handleOpenOauthUrl,
    handleSubmitOauthCallbackUrl,
    isFlowNoticeCollapsed,
    setIsFlowNoticeCollapsed,
    currentAccountId,
    formatDate,
    normalizeTag,
  } = page;

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_TYPES_FIELD);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_TYPES_FIELD, filterTypes);
  }, [filterPersistenceEnabled, filterPersistenceScope, filterTypes]);

  const toggleFilterTypeValue = useCallback((value: string) => {
    setFilterTypes((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearFilterTypes = useCallback(() => {
    setFilterTypes([]);
  }, []);

  const accounts = store.accounts;
  const loading = store.loading;

  const isAbnormalAccount = useCallback(
    (account: TraeAccount) => (account.status || '').toLowerCase() === 'error',
    [],
  );

  const tierSummary = useMemo(() => {
    const counts = new Map<string, number>();
    accounts.forEach((account) => {
      const plan = getTraePlanBadge(account);
      counts.set(plan, (counts.get(plan) ?? 0) + 1);
    });
    const validCount = accounts.reduce(
      (count, account) => (isAbnormalAccount(account) ? count : count + 1),
      0,
    );

    return {
      all: accounts.length,
      validCount,
      entries: Array.from(counts.entries()).sort(([left], [right]) => left.localeCompare(right)),
    };
  }, [accounts, isAbnormalAccount]);

  useEffect(() => {
    const allowed = new Set(tierSummary.entries.map(([plan]) => plan));
    allowed.add(VALID_ACCOUNTS_FILTER_VALUE);
    setFilterTypes((prev) => {
      const next = prev.filter((value) => allowed.has(value));
      return next.length === prev.length ? prev : next;
    });
  }, [tierSummary.entries]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => [
      ...tierSummary.entries.map(([plan, count]) => ({
        value: plan,
        label: `${plan} (${count})`,
      })),
      buildValidAccountsFilterOption(t, tierSummary.validCount),
    ],
    [t, tierSummary.entries, tierSummary.validCount],
  );

  const compareAccountsBySort = useCallback(
    (left: TraeAccount, right: TraeAccount) => {
      const currentFirstDiff = compareCurrentAccountFirst(left.id, right.id, currentAccountId);
      if (currentFirstDiff !== 0) {
        return currentFirstDiff;
      }

      if (sortBy === 'plan') {
        const diff = getTraePlanBadge(left).localeCompare(getTraePlanBadge(right));
        return sortDirection === 'desc' ? -diff : diff;
      }

      if (sortBy === 'quota') {
        const leftPercent = getTraeUsage(left).usedPercent ?? -1;
        const rightPercent = getTraeUsage(right).usedPercent ?? -1;
        const diff = leftPercent - rightPercent;
        return sortDirection === 'desc' ? -diff : diff;
      }

      const diff = left.created_at - right.created_at;
      return sortDirection === 'desc' ? -diff : diff;
    },
    [currentAccountId, sortBy, sortDirection],
  );

  const sortedAccountsForInstances = useMemo(
    () => [...accounts].sort(compareAccountsBySort),
    [accounts, compareAccountsBySort],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    const query = searchQuery.trim().toLowerCase();

    if (query) {
      result = result.filter((account) => {
        const searchText = [
          getTraeAccountDisplayName(account),
          getTraeAccountDisplayEmail(account),
          getTraeLoginProvider(account) ?? '',
          account.user_id ?? '',
          account.nickname ?? '',
          account.id,
          getTraePlanBadge(account),
        ]
          .join(' ')
          .toLowerCase();
        return searchText.includes(query);
      });
    }

    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        result = result.filter((account) => !isAbnormalAccount(account));
      }
      if (selectedTypes.size > 0) {
        result = result.filter((account) => selectedTypes.has(getTraePlanBadge(account)));
      }
    }

    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((account) =>
        (account.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag)),
      );
    }

    result.sort(compareAccountsBySort);
    return result;
  }, [accounts, compareAccountsBySort, filterTypes, isAbnormalAccount, normalizeTag, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('Trae'),
  });
  const paginatedAccounts = pagination.pageItems;
  const paginatedIds = useMemo(() => paginatedAccounts.map((account) => account.id), [paginatedAccounts]);
  const isAllPaginatedSelected = useMemo(
    () => isEveryIdSelected(selected, paginatedIds),
    [paginatedIds, selected],
  );

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, TraeAccount[]]>;

    const groups = new Map<string, TraeAccount[]>();
    const selectedTags = new Set(tagFilter.map(normalizeTag));

    filteredAccounts.forEach((account) => {
      const normalizedTags = (account.tags || []).map(normalizeTag).filter(Boolean);
      const matchedTags =
        selectedTags.size > 0
          ? normalizedTags.filter((tag) => selectedTags.has(tag))
          : normalizedTags;

      if (matchedTags.length === 0) {
        if (!groups.has(untaggedKey)) groups.set(untaggedKey, []);
        groups.get(untaggedKey)?.push(account);
        return;
      }

      matchedTags.forEach((tag) => {
        if (!groups.has(tag)) groups.set(tag, []);
        groups.get(tag)?.push(account);
      });
    });

    return Array.from(groups.entries()).sort(([left], [right]) => {
      if (left === untaggedKey) return -1;
      if (right === untaggedKey) return 1;
      return left.localeCompare(right);
    });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter]);

  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts],
  );

  const resolveGroupLabel = useCallback(
    (groupKey: string) =>
      groupKey === untaggedKey ? t('accounts.defaultGroup', '默认分组') : groupKey,
    [t],
  );

  const resolveQuotaSummary = useCallback(
    (account: TraeAccount): TraeQuotaSummary => {
      const usage = getTraeUsage(account);
      const percentage =
        typeof usage.usedPercent === 'number' && Number.isFinite(usage.usedPercent)
          ? Math.max(0, Math.min(100, Math.round(usage.usedPercent)))
          : null;

      const isFreePlan = (usage.identityStr ?? account.plan_type ?? '')
        .trim()
        .toLowerCase()
        .includes('free');

      const statusTone: TraeQuotaSummary['statusTone'] = !account.trae_usage_raw
        ? 'unknown'
        : usage.usageExhausted
          ? usage.payAsYouGoOpen && !isFreePlan
            ? 'normal'
            : 'warning'
          : 'normal';

      const statusText = !account.trae_usage_raw
        ? t('trae.quota.statusUnknown', 'Status: --')
        : usage.usageExhausted
          ? isFreePlan
            ? t(
                'trae.quota.statusExhaustedFree',
                'Status: Usage exhausted, upgrade recommended',
              )
            : usage.payAsYouGoOpen
              ? t('trae.quota.statusNormal', 'Status: Normal')
              : t(
                  'trae.quota.statusExhaustedPro',
                  'Status: Usage exhausted, upgrade or enable on-demand usage',
                )
          : t('trae.quota.statusNormal', 'Status: Normal');

      return {
        percentage,
        percentageText: percentage == null ? '--' : `${percentage}%`,
        quotaClass: computeQuotaClass(percentage),
        costText:
          usage.spentUsd != null && usage.totalUsd != null
            ? t('trae.quota.usedOfTotal', {
                used: formatNumber(usage.spentUsd),
                total: formatNumber(usage.totalUsd),
                defaultValue: '${{used}} / ${{total}}',
              })
            : t('trae.quota.usageUnknown', 'Usage: --'),
        statusText,
        statusTone,
        bonusText:
          (usage.bonusUsage ?? 0) > 0
            ? t('trae.quota.bonusUsed', {
                amount: formatTraeMoney(usage.bonusUsage),
                defaultValue: 'Bonus: +{{amount}}',
              })
            : (usage.bonusQuota ?? 0) > 0
              ? t('trae.quota.bonusIncluded', 'Bonus: Included')
              : t('trae.quota.bonusEmpty', 'Bonus: --'),
        resetText:
          (usage.resetAt ?? account.plan_reset_at ?? null) != null
            ? t('trae.quota.resetAt', {
                date: formatTraeResetAt(usage.resetAt ?? account.plan_reset_at ?? 0),
                defaultValue: '重置时间：{{date}}',
              })
            : t('trae.quota.resetUnknown', '重置时间未知'),
        packageText: usage.hasPackage
          ? usage.consumingProductType === TRAE_PRODUCT_TYPE.PACKAGE
            ? t('trae.quota.packageConsuming', 'Package: Consuming')
            : t('trae.quota.packageAvailable', 'Package: Available')
          : t('trae.quota.packageEmpty', 'Package: --'),
        payAsYouGoText: usage.payAsYouGoOpen
          ? usage.consumingProductType === TRAE_PRODUCT_TYPE.PAY_GO
            ? t('trae.quota.payAsYouGoConsuming', 'On-Demand Usage: Consuming')
            : t('trae.quota.payAsYouGoEnabled', 'On-Demand Usage: Enabled')
          : t('trae.quota.payAsYouGoEmpty', 'On-Demand Usage: --'),
      };
    },
    [t],
  );

  const resolveDisplayName = useCallback(
    (account: TraeAccount) => getTraeAccountDisplayName(account),
    [],
  );

  const resolveDisplayEmail = useCallback(
    (account: TraeAccount) => getTraeAccountDisplayEmail(account),
    [],
  );

  const resolveSignedInWithText = useCallback(
    (account: TraeAccount) => {
      const provider = getTraeLoginProvider(account) ?? t('kiro.account.providerUnknown', 'Unknown');
      return t('kiro.account.signedInWith', {
        provider,
        defaultValue: 'Signed in with {{provider}}',
      });
    },
    [t],
  );

  const resolveSingleExportBaseName = useCallback(
    (account: TraeAccount) => {
      const display = resolveDisplayEmail(account);
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolveDisplayEmail],
  );

  const resolvePlanLabel = useCallback(
    (account: TraeAccount) => getTraePlanDisplayName(account),
    [],
  );

  const renderCompactQuota = useCallback(
    (quota: TraeQuotaSummary, variant: 'card' | 'table' = 'card') => {
      const labelClass = variant === 'table' ? 'quota-name' : 'quota-label';
      const valueClass = variant === 'table' ? 'quota-value' : 'quota-pct';
      const trackClass = variant === 'table' ? 'quota-progress-track' : 'quota-bar-track';
      const barClass = variant === 'table' ? 'quota-progress-bar' : 'quota-bar';
      const metaPills = [quota.bonusText, quota.packageText, quota.payAsYouGoText];

      return (
        <div className={`quota-item trae-compact-quota ${variant === 'table' ? 'is-table windsurf-table-credit-item' : 'is-card'}`}>
          <div className="quota-header trae-compact-quota-header">
            <span className={labelClass}>{t('instances.labels.quota', '配额')}</span>
            <div className="trae-compact-quota-main">
              <span className="trae-compact-quota-total" title={quota.costText}>
                {quota.costText}
              </span>
              <span className={`${valueClass} ${quota.quotaClass}`}>{quota.percentageText}</span>
            </div>
          </div>
          <div className={trackClass}>
            <div
              className={`${barClass} ${quota.quotaClass}`}
              style={{ width: `${Math.min(quota.percentage ?? 0, 100)}%` }}
            />
          </div>
          <div className="trae-compact-quota-meta-row">
            <span className={`trae-compact-quota-status ${quota.statusTone}`} title={quota.statusText}>
              {quota.statusText}
            </span>
            <span className="trae-compact-quota-reset" title={quota.resetText}>
              {quota.resetText}
            </span>
          </div>
          <div className="trae-compact-quota-pills">
            {metaPills.map((text, index) => (
              <span key={`${variant}-meta-${index}`} className="pill pill-secondary trae-compact-quota-pill" title={text}>
                {text}
              </span>
            ))}
          </div>
        </div>
      );
    },
    [t],
  );

  const renderGridCards = useCallback(
    (items: TraeAccount[], groupKey?: string) =>
      items.map((account) => {
        const displayName = resolveDisplayName(account);
        const displayEmail = resolveDisplayEmail(account);
        const showDisplayEmail = displayEmail !== 'unknown' && displayEmail !== displayName;
        const quota = resolveQuotaSummary(account);
        const planLabel = resolvePlanLabel(account);
        const planClass = getTraePlanBadgeClass(planLabel);
        const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
        const visibleTags = accountTags.slice(0, 2);
        const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
        const isSelected = selected.has(account.id);
        const isCurrent = currentAccountId === account.id;
        const hasStatusError = (account.status || '').toLowerCase() === 'error';
        const statusTitle = account.status_reason || t('accounts.status.refreshFailed', '刷新失败');
        const signedInWithText = resolveSignedInWithText(account);
        const userIdText = account.user_id || '--';
        const quotaError = account.quota_query_last_error?.trim();
        const hasQuotaData = hasTraeQuotaData(account);

        return (
          <div
            key={groupKey ? `${groupKey}-${account.id}` : account.id}
            className={`ghcp-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}
          >
            <div className="card-top">
              <div className="card-select">
                <input
                  type="checkbox"
                  checked={isSelected}
                  onChange={() => toggleSelect(account.id)}
                />
              </div>
              <span className="account-email" title={maskAccountText(displayName)}>
                {maskAccountText(displayName)}
              </span>
              {planLabel && planLabel !== 'UNKNOWN' && (
                <span className={`tier-badge ${planClass} raw-value`}>{planLabel}</span>
              )}
              {isCurrent && (
                <span className="current-tag">{t('accounts.status.current', '当前')}</span>
              )}
              {hasStatusError && (
                <span className="status-pill warning" title={statusTitle}>
                  <CircleAlert size={12} />
                  {t('accounts.status.refreshFailed', '刷新失败')}
                </span>
              )}
              {quotaError && (
                <span className="status-pill warning" title={quotaError}>
                  <CircleAlert size={12} />
                  {t('common.shared.quota.queryFailed', '配额查询失败')}
                </span>
              )}
            </div>

            <div className="account-sub-line">
              <span className="kiro-table-subline">
                {showDisplayEmail && (
                  <>
                    {maskAccountText(displayEmail)}
                    {' | '}
                  </>
                )}
                {signedInWithText} | {t('kiro.account.userId', 'User ID')}: {maskAccountText(userIdText)}
              </span>
            </div>

            {accountTags.length > 0 && (
              <div className="card-tags">
                {visibleTags.map((tag, index) => (
                  <span key={`${account.id}-${tag}-${index}`} className="tag-pill">
                    {tag}
                  </span>
                ))}
                {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
              </div>
            )}

            <div className="ghcp-quota-section">
              {hasQuotaData ? (
                renderCompactQuota(quota, 'card')
              ) : (
                <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
              )}
            </div>

            <div className="card-footer">
              <span className="card-date">{formatDate(account.created_at)}</span>
              <div className="card-actions">
                <button
                  className="card-action-btn success"
                  onClick={() => handleInjectToVSCode?.(account.id)}
                  disabled={injecting === account.id}
                  title={t('dashboard.switch', '切换')}
                >
                  {injecting === account.id ? (
                    <RefreshCw size={14} className="loading-spinner" />
                  ) : (
                    <Play size={14} />
                  )}
                </button>
                <button
                  className="card-action-btn"
                  onClick={() => openTagModal(account.id)}
                  title={t('accounts.editTags', '编辑标签')}
                >
                  <Tag size={14} />
                </button>
                <button
                  className="card-action-btn"
                  onClick={() => handleRefresh(account.id)}
                  disabled={refreshing === account.id}
                  title={t('common.refresh', '刷新')}
                >
                  <RotateCw
                    size={14}
                    className={refreshing === account.id ? 'loading-spinner' : ''}
                  />
                </button>
                <button
                  className="card-action-btn export-btn"
                  onClick={() =>
                    handleExportByIds([account.id], resolveSingleExportBaseName(account))
                  }
                  title={t('common.shared.export.title', '导出')}
                >
                  <Upload size={14} />
                </button>
                <button
                  className="card-action-btn danger"
                  onClick={() => handleDelete(account.id)}
                  title={t('common.delete', '删除')}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
          </div>
        );
      }),
    [
      currentAccountId,
      formatDate,
      handleDelete,
      handleExportByIds,
      handleInjectToVSCode,
      handleRefresh,
      injecting,
      maskAccountText,
      openTagModal,
      refreshing,
      renderCompactQuota,
      resolveDisplayName,
      resolveDisplayEmail,
      resolveSignedInWithText,
      resolvePlanLabel,
      resolveQuotaSummary,
      resolveSingleExportBaseName,
      selected,
      t,
      toggleSelect,
    ],
  );

  const renderTableRows = useCallback(
    (items: TraeAccount[], groupKey?: string) =>
      items.map((account) => {
        const displayName = resolveDisplayName(account);
        const displayEmail = resolveDisplayEmail(account);
        const showDisplayEmail = displayEmail !== 'unknown' && displayEmail !== displayName;
        const quota = resolveQuotaSummary(account);
        const planLabel = resolvePlanLabel(account);
        const planClass = getTraePlanBadgeClass(planLabel);
        const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
        const visibleTags = accountTags.slice(0, 3);
        const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
        const isCurrent = currentAccountId === account.id;
        const hasStatusError = (account.status || '').toLowerCase() === 'error';
        const statusTitle = account.status_reason || t('accounts.status.refreshFailed', '刷新失败');
        const signedInWithText = resolveSignedInWithText(account);
        const userIdText = account.user_id || '--';
        const quotaError = account.quota_query_last_error?.trim();
        const hasQuotaData = hasTraeQuotaData(account);

        return (
          <tr key={groupKey ? `${groupKey}-${account.id}` : account.id} className={isCurrent ? 'current' : ''}>
            <td>
              <input
                type="checkbox"
                checked={selected.has(account.id)}
                onChange={() => toggleSelect(account.id)}
              />
            </td>
            <td>
              <div className="account-cell">
                <div className="account-main-line">
                  <span className="account-email-text" title={maskAccountText(displayName)}>
                    {maskAccountText(displayName)}
                  </span>
                  {planLabel && planLabel !== 'UNKNOWN' && (
                    <span className={`tier-badge ${planClass} raw-value`}>{planLabel}</span>
                  )}
                  {isCurrent && (
                    <span className="mini-tag current">{t('accounts.status.current', '当前')}</span>
                  )}
                </div>
                {hasStatusError && (
                  <div className="account-sub-line">
                    <span className="status-pill warning" title={statusTitle}>
                      <CircleAlert size={12} />
                      {t('accounts.status.refreshFailed', '刷新失败')}
                    </span>
                  </div>
                )}
                {quotaError && (
                  <div className="account-sub-line">
                    <span className="status-pill warning" title={quotaError}>
                      <CircleAlert size={12} />
                      {t('common.shared.quota.queryFailed', '配额查询失败')}
                    </span>
                  </div>
                )}
                <div className="account-sub-line">
                  <span className="kiro-table-subline">
                    {showDisplayEmail && (
                      <>
                        {maskAccountText(displayEmail)}
                        {' | '}
                      </>
                    )}
                    {signedInWithText} | {t('kiro.account.userId', 'User ID')}: {maskAccountText(userIdText)}
                  </span>
                </div>
                {accountTags.length > 0 && (
                  <div className="account-tags-inline">
                    {visibleTags.map((tag, index) => (
                      <span key={`${account.id}-tag-${tag}-${index}`} className="tag-pill">
                        {tag}
                      </span>
                    ))}
                    {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                  </div>
                )}
              </div>
            </td>
            <td>
              {hasQuotaData ? (
                renderCompactQuota(quota, 'table')
              ) : (
                <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
              )}
            </td>
            <td>{formatDate(account.created_at)}</td>
            <td className="sticky-action-cell table-action-cell">
              <div className="action-buttons">
                <button
                  className="action-btn success"
                  onClick={() => handleInjectToVSCode?.(account.id)}
                  disabled={injecting === account.id}
                  title={t('dashboard.switch', '切换')}
                >
                  {injecting === account.id ? (
                    <RefreshCw size={14} className="loading-spinner" />
                  ) : (
                    <Play size={14} />
                  )}
                </button>
                <button
                  className="action-btn"
                  onClick={() => openTagModal(account.id)}
                  title={t('accounts.editTags', '编辑标签')}
                >
                  <Tag size={14} />
                </button>
                <button
                  className="action-btn"
                  onClick={() => handleRefresh(account.id)}
                  disabled={refreshing === account.id}
                  title={t('common.refresh', '刷新')}
                >
                  <RotateCw
                    size={14}
                    className={refreshing === account.id ? 'loading-spinner' : ''}
                  />
                </button>
                <button
                  className="action-btn"
                  onClick={() =>
                    handleExportByIds([account.id], resolveSingleExportBaseName(account))
                  }
                  title={t('common.shared.export.title', '导出')}
                >
                  <Upload size={14} />
                </button>
                <button
                  className="action-btn danger"
                  onClick={() => handleDelete(account.id)}
                  title={t('common.delete', '删除')}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </td>
          </tr>
        );
      }),
    [
      currentAccountId,
      formatDate,
      handleDelete,
      handleExportByIds,
      handleInjectToVSCode,
      handleRefresh,
      injecting,
      maskAccountText,
      openTagModal,
      refreshing,
      renderCompactQuota,
      resolveDisplayName,
      resolveDisplayEmail,
      resolveSignedInWithText,
      resolvePlanLabel,
      resolveQuotaSummary,
      resolveSingleExportBaseName,
      selected,
      t,
      toggleSelect,
    ],
  );

  return (
    <div className="ghcp-accounts-page trae-accounts-page">
      <PlatformOverviewTabsHeader platform="trae" active={activeTab} onTabChange={setActiveTab} />

      <div
        className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`}
        role="note"
        aria-live="polite"
      >
        <button
          type="button"
          className="ghcp-flow-notice-toggle"
          onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)}
          aria-expanded={!isFlowNoticeCollapsed}
        >
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('trae.flowNotice.title', 'Trae 账号接入说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown
            size={16}
            className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`}
          />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t(
                'trae.flowNotice.desc',
                '支持官方 OAuth 授权、本机导入、JSON 导入与本地注入切号；切号过程按 Trae 客户端真实落盘规则写回。',
              )}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>
                {t(
                  'trae.flowNotice.permission',
                  '权限范围：读取并写入本机 Trae 配置目录中的 storage.json 登录相关字段，用于账号导入、切号注入与套餐信息展示；所有数据仅在本机处理。',
                )}
              </li>
              <li>
                {t(
                  'trae.flowNotice.network',
                  '网络范围：OAuth 登录、令牌刷新和套餐查询会请求 Trae 官方接口；不会向第三方服务上传本地账号文件或原始存储内容。',
                )}
              </li>
            </ul>
          </div>
        )}
      </div>

      {activeTab === 'overview' && (
        <>
          {message && (
            <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
              {message.text}
              <button onClick={() => setMessage(null)} aria-label={t('common.close', '关闭')}>
                <X size={14} />
              </button>
            </div>
          )}

          <div className="toolbar">
            <div className="toolbar-left">
              <div className="search-box">
                <Search size={16} className="search-icon" />
                <input
                  type="text"
                  placeholder={t('common.shared.search')}
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                />
              </div>

              <div className="view-switcher">
                <button
                  className={`view-btn ${viewMode === 'list' ? 'active' : ''}`}
                  onClick={() => setViewMode('list')}
                  title={t('common.shared.view.list', '列表视图')}
                >
                  <List size={16} />
                </button>
                <button
                  className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`}
                  onClick={() => setViewMode('grid')}
                  title={t('common.shared.view.grid', '卡片视图')}
                >
                  <LayoutGrid size={16} />
                </button>
              </div>

              <MultiSelectFilterDropdown
                options={tierFilterOptions}
                selectedValues={filterTypes}
                allLabel={t('common.shared.filter.all', '全部 ({{count}})', { count: tierSummary.all })}
                filterLabel={t('common.shared.filterLabel', '筛选')}
                clearLabel={t('accounts.clearFilter', '清空筛选')}
                emptyLabel={t('common.none', '暂无')}
                ariaLabel={t('common.shared.filterLabel', '筛选')}
                onToggleValue={toggleFilterTypeValue}
                onClear={clearFilterTypes}
              />

              <div className="tag-filter" ref={tagFilterRef}>
                <button
                  type="button"
                  className={`tag-filter-btn ${tagFilter.length > 0 ? 'active' : ''}`}
                  onClick={() => setShowTagFilter((prev) => !prev)}
                  aria-label={t('accounts.filterTags', '标签筛选')}
                >
                  <Tag size={14} />
                  {tagFilter.length > 0
                    ? `${t('accounts.filterTagsCount', '标签')}(${tagFilter.length})`
                    : t('accounts.filterTags', '标签筛选')}
                </button>
                {showTagFilter && (
                  <div
                    ref={page.tagFilterPanelRef}
                    className={`tag-filter-panel ${page.tagFilterPanelPlacement === 'top' ? 'open-top' : ''}`}
                  >
                    {availableTags.length === 0 ? (
                      <div className="tag-filter-empty">
                        {t('accounts.noAvailableTags', '暂无可用标签')}
                      </div>
                    ) : (
                      <div className="tag-filter-options" style={page.tagFilterScrollContainerStyle}>
                        {availableTags.map((tag) => (
                          <label
                            key={tag}
                            className={`tag-filter-option ${tagFilter.includes(tag) ? 'selected' : ''}`}
                          >
                            <input
                              type="checkbox"
                              checked={tagFilter.includes(tag)}
                              onChange={() => toggleTagFilterValue(tag)}
                            />
                            <span className="tag-filter-name">{tag}</span>
                            <button
                              type="button"
                              className="tag-filter-delete"
                              onClick={(event) => {
                                event.preventDefault();
                                event.stopPropagation();
                                requestDeleteTag(tag);
                              }}
                              aria-label={t('accounts.deleteTagAria', {
                                tag,
                                defaultValue: '删除标签 {{tag}}',
                              })}
                            >
                              <X size={12} />
                            </button>
                          </label>
                        ))}
                      </div>
                    )}
                    <div className="tag-filter-divider" />
                    <label className="tag-filter-group-toggle">
                      <input
                        type="checkbox"
                        checked={groupByTag}
                        onChange={(event) => setGroupByTag(event.target.checked)}
                      />
                      <span>{t('accounts.groupByTag', '按标签分组展示')}</span>
                    </label>
                    {tagFilter.length > 0 && (
                      <button type="button" className="tag-filter-clear" onClick={clearTagFilter}>
                        {t('accounts.clearFilter', '清空筛选')}
                      </button>
                    )}
                  </div>
                )}
              </div>

              <SingleSelectFilterDropdown
                value={TRAE_KNOWN_SORT_KEYS.includes(sortBy as (typeof TRAE_KNOWN_SORT_KEYS)[number]) ? sortBy : 'created_at'}
                options={[
                  { value: 'created_at', label: t('accounts.sort.createdAt') },
                  { value: 'plan', label: t('accounts.sort.plan') },
                  { value: 'quota', label: t('accounts.sort.quota') },
                ]}
                ariaLabel={t('common.shared.sortLabel')}
                icon={<ArrowDownWideNarrow size={14} />}
                onChange={setSortBy}
              />

              <button
                className="sort-direction-btn"
                onClick={() => setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
                title={
                  sortDirection === 'desc'
                    ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序')
                    : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序')
                }
                aria-label={t('common.shared.sort.toggleDirection', '切换排序方向')}
              >
                {sortDirection === 'desc' ? '⬇' : '⬆'}
              </button>
            </div>

            <div className="toolbar-right">
              <button
                className="btn btn-primary icon-only"
                onClick={() => openAddModal('oauth')}
                title={t('common.shared.addAccount')}
                aria-label={t('common.shared.addAccount')}
              >
                <Plus size={14} />
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={handleRefreshAll}
                disabled={refreshingAll || accounts.length === 0}
                title={t('common.shared.refreshAll', '刷新全部')}
                aria-label={t('common.shared.refreshAll', '刷新全部')}
              >
                <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={togglePrivacyMode}
                title={
                  privacyModeEnabled
                    ? t('privacy.showSensitive', '显示邮箱')
                    : t('privacy.hideSensitive', '隐藏邮箱')
                }
                aria-label={
                  privacyModeEnabled
                    ? t('privacy.showSensitive', '显示邮箱')
                    : t('privacy.hideSensitive', '隐藏邮箱')
                }
              >
                {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={() => openAddModal('import')}
                disabled={importing}
                title={t('common.shared.import.label', '导入')}
                aria-label={t('common.shared.import.label', '导入')}
              >
                <Download size={14} />
              </button>
              <button
                className="btn btn-secondary export-btn icon-only"
                onClick={() => void handleExport(filteredIds)}
                disabled={exporting || filteredIds.length === 0}
                title={
                  exportSelectionCount > 0
                    ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})`
                    : t('common.shared.export.title', '导出')
                }
                aria-label={
                  exportSelectionCount > 0
                    ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})`
                    : t('common.shared.export.title', '导出')
                }
              >
                <Upload size={14} />
              </button>
              {selected.size > 0 && (
                <button
                  className="btn btn-danger icon-only"
                  onClick={handleBatchDelete}
                  title={`${t('common.delete', '删除')} (${selected.size})`}
                  aria-label={`${t('common.delete', '删除')} (${selected.size})`}
                >
                  <Trash2 size={14} />
                </button>
              )}
              <QuickSettingsPopover type="trae" />
            </div>
          </div>

          {loading && accounts.length === 0 ? (
            <div className="loading-container">
              <RefreshCw size={24} className="loading-spinner" />
              <p>{t('common.loading', '加载中...')}</p>
            </div>
          ) : accounts.length === 0 ? (
            <div className="empty-state">
              <Globe size={48} />
              <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
              <p>
                {t(
                  'trae.empty.description',
                  '点击“添加账号”开始管理您的 Trae 账号，也可以从本机或 JSON 文件导入。',
                )}
              </p>
              <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
                <button className="btn btn-primary" onClick={() => openAddModal('oauth')}>
                  <Plus size={16} />
                  {t('common.shared.addAccount')}
                </button>
                <button
                  className="btn btn-secondary"
                  onClick={() =>
                    window.dispatchEvent(
                      new CustomEvent('app-request-navigate', { detail: 'manual' }),
                    )
                  }
                >
                  <BookOpen size={16} />
                  {t('manual.navTitle', '功能使用手册')}
                </button>
              </div>
            </div>
          ) : filteredAccounts.length === 0 ? (
            <div className="empty-state">
              <h3>{t('common.shared.noMatch.title', '没有匹配的账号')}</h3>
              <p>{t('common.shared.noMatch.desc', '请尝试调整搜索或筛选条件')}</p>
            </div>
          ) : viewMode === 'grid' ? (
        <div className="grid-view-container">
          {paginatedAccounts.length > 0 && (
            <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '13px', color: 'var(--text-color)' }}>
                <input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} />
                {t('common.selectAll', '全选')}
              </label>
            </div>
          )}
          {groupByTag ? (
              <div className="tag-group-list">
                {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                  <div key={groupKey} className="tag-group-section">
                    <div className="tag-group-header">
                      <span className="tag-group-title">{resolveGroupLabel(groupKey)}</span>
                      <span className="tag-group-count">{totalCount}</span>
                    </div>
                    <div className="tag-group-grid ghcp-accounts-grid">
                      {renderGridCards(items, groupKey)}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="ghcp-accounts-grid">{renderGridCards(paginatedAccounts)}</div>
            )}
        </div>
      ) : groupByTag ? (
            <div className="account-table-container grouped">
              <table className="account-table">
                <thead>
                  <tr>
                    <th style={{ width: 40 }}>
                      <input
                        type="checkbox"
                        checked={isAllPaginatedSelected}
                        onChange={() => toggleSelectAll(paginatedIds)}
                      />
                    </th>
                    <th style={{ width: 260 }}>{t('common.shared.columns.account')}</th>
                    <th>{t('instances.labels.quota', '配额')}</th>
                    <th style={{ width: 160 }}>{t('common.shared.columns.createdAt')}</th>
                    <th className="sticky-action-header table-action-header">
                      {t('common.shared.columns.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                    <Fragment key={groupKey}>
                      <tr className="tag-group-row">
                        <td colSpan={5}>
                          <div className="tag-group-header">
                            <span className="tag-group-title">{resolveGroupLabel(groupKey)}</span>
                            <span className="tag-group-count">{totalCount}</span>
                          </div>
                        </td>
                      </tr>
                      {renderTableRows(items, groupKey)}
                    </Fragment>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <div className="account-table-container">
              <table className="account-table">
                <thead>
                  <tr>
                    <th style={{ width: 40 }}>
                      <input
                        type="checkbox"
                        checked={isAllPaginatedSelected}
                        onChange={() => toggleSelectAll(paginatedIds)}
                      />
                    </th>
                    <th style={{ width: 260 }}>{t('common.shared.columns.account')}</th>
                    <th>{t('instances.labels.quota', '配额')}</th>
                    <th style={{ width: 160 }}>{t('common.shared.columns.createdAt')}</th>
                    <th className="sticky-action-header table-action-header">
                      {t('common.shared.columns.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>{renderTableRows(paginatedAccounts)}</tbody>
              </table>
            </div>
          )}

          <PaginationControls
            totalItems={pagination.totalItems}
            currentPage={pagination.currentPage}
            totalPages={pagination.totalPages}
            pageSize={pagination.pageSize}
            pageSizeOptions={pagination.pageSizeOptions}
            rangeStart={pagination.rangeStart}
            rangeEnd={pagination.rangeEnd}
            canGoPrevious={pagination.canGoPrevious}
            canGoNext={pagination.canGoNext}
            onPageSizeChange={pagination.setPageSize}
            onPreviousPage={pagination.goToPreviousPage}
            onNextPage={pagination.goToNextPage}
          />

          {showAddModal && (
            <div className="modal-overlay" onClick={closeAddModal}>
              <div className="modal-content ghcp-add-modal" onClick={(event) => event.stopPropagation()}>
                <div className="modal-header">
                  <button className="btn btn-secondary icon-only" onClick={closeAddModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
                  <h2>{t('trae.addModal.title')}</h2>
                  <button
                    className="modal-close"
                    onClick={closeAddModal}
                    aria-label={t('common.close', '关闭')}
                  >
                    <X />
                  </button>
                </div>

                <div className="modal-tabs">
                  <button
                    className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`}
                    onClick={() => openAddModal('oauth')}
                  >
                    <Globe size={14} />
                    {t('common.shared.addModal.oauth', 'OAuth 授权')}
                  </button>
                  <button
                    className={`modal-tab ${addTab === 'token' ? 'active' : ''}`}
                    onClick={() => openAddModal('token')}
                  >
                    <KeyRound size={14} />
                    {t('common.shared.addModal.token', 'Token / JSON')}
                  </button>
                  <button
                    className={`modal-tab ${addTab === 'import' ? 'active' : ''}`}
                    onClick={() => openAddModal('import')}
                  >
                    <Database size={14} />
                    {t('accounts.tabs.import', '导入')}
                  </button>
                </div>

                <div className="modal-body">
                  {addTab === 'oauth' ? (
                    <div className="add-section">
                      <p className="section-desc">
                        {t('trae.oauth.desc', '点击下方按钮，在浏览器中完成 Trae OAuth 授权登录。')}
                      </p>

                      {oauthPrepareError ? (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>{oauthPrepareError}</span>
                          <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>
                            {t('common.shared.oauth.retry', '重新生成授权信息')}
                          </button>
                        </div>
                      ) : oauthUrl ? (
                        <div className="oauth-url-section">
                          <div className="oauth-link">
                            <label>{t('accounts.oauth.linkLabel', '授权链接')}</label>
                            <div className="oauth-url-box">
                              <input type="text" value={oauthUrl} readOnly />
                              <button onClick={handleCopyOauthUrl}>
                                {oauthUrlCopied ? <Check size={16} /> : <Copy size={16} />}
                              </button>
                            </div>
                          </div>
                          {oauthMeta && (
                            <p className="oauth-hint">
                              {t('common.shared.oauth.meta', '授权有效期：{{expires}}s；轮询间隔：{{interval}}s', {
                                expires: oauthMeta.expiresIn,
                                interval: oauthMeta.intervalSeconds,
                              })}
                            </p>
                          )}
                          <button className="btn btn-primary btn-full" onClick={handleOpenOauthUrl}>
                            <Globe size={16} />
                            {t('common.shared.oauth.openBrowser', '在浏览器中打开')}
                          </button>
                          {oauthSupportsManualCallback && (
                            <div className="oauth-link">
                              <label>{t('common.shared.oauth.manualCallbackLabel', '手动输入回调地址')}</label>
                              <div className="oauth-url-box oauth-manual-input">
                                <input
                                  type="text"
                                  value={oauthManualCallbackInput}
                                  onChange={(e) => setOauthManualCallbackInput(e.target.value)}
                                  placeholder={t('common.shared.oauth.manualCallbackPlaceholder', '粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...')}
                                />
                                <button
                                  className="oauth-copy-button"
                                  onClick={() => void handleSubmitOauthCallbackUrl()}
                                  disabled={oauthManualCallbackSubmitting || !oauthManualCallbackInput.trim()}
                                >
                                  {oauthManualCallbackSubmitting ? <RefreshCw size={16} className="loading-spinner" /> : <Check size={16} />}
                                  {t('accounts.oauth.continue', '我已授权，继续')}
                                </button>
                              </div>
                            </div>
                          )}
                          {oauthManualCallbackError && (
                            <div className="add-status error">
                              <CircleAlert size={16} />
                              <span>{oauthManualCallbackError}</span>
                            </div>
                          )}
                          {oauthPolling && (
                            <div className="add-status loading">
                              <RefreshCw size={16} className="loading-spinner" />
                              <span>{t('common.shared.oauth.waiting', '等待授权完成...')}</span>
                            </div>
                          )}
                          {oauthCompleteError && (
                            <div className="add-status error">
                              <CircleAlert size={16} />
                              <span>{oauthCompleteError}</span>
                              <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>
                                {oauthTimedOut
                                  ? t('common.shared.oauth.timeoutRetry', '刷新授权链接')
                                  : t('common.shared.oauth.retry', '重新生成授权信息')}
                              </button>
                            </div>
                          )}
                          <p className="oauth-hint">
                            {t(
                              'common.shared.oauth.hint',
                              'Once authorized, this window will update automatically',
                            )}
                          </p>
                        </div>
                      ) : (
                        <div className="oauth-loading">
                          <RefreshCw size={24} className="loading-spinner" />
                          <span>{t('common.shared.oauth.preparing', '正在准备授权信息...')}</span>
                        </div>
                      )}
                    </div>
                  ) : addTab === 'token' ? (
                    <div className="add-section">
                      <p className="section-desc">
                        {t('accounts.importJsonHint', '导入由本工具导出的 Trae JSON 文件。')}
                      </p>
                      <textarea
                        className="token-input"
                        value={tokenInput}
                        onChange={(event) => setTokenInput(event.target.value)}
                        placeholder={t('common.shared.token.placeholder', '粘贴 Token 或 JSON...')}
                      />
                      <button
                        className="btn btn-primary btn-full"
                        onClick={() => void handleTokenImport()}
                        disabled={addStatus === 'loading' || !tokenInput.trim()}
                      >
                        {addStatus === 'loading' ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Download size={16} />
                        )}
                        {t('common.shared.token.import', '导入')}
                      </button>
                    </div>
                  ) : (
                    <div className="add-section">
                      <p className="section-desc">
                        {t('trae.import.localDesc')}
                      </p>
                      <button
                        className="btn btn-secondary btn-full"
                        onClick={() => handleImportFromLocal?.()}
                        disabled={importing}
                      >
                        {importing ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Database size={16} />
                        )}
                        {t('common.shared.addModal.import')}
                      </button>
                      <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>
                        {t('common.shared.import.orJson', '或从 JSON 文件导入')}
                      </div>
                      <input
                        ref={importFileInputRef}
                        type="file"
                        accept=".json,application/json"
                        style={{ display: 'none' }}
                        onChange={(event) => {
                          const file = event.target.files?.[0];
                          event.target.value = '';
                          if (!file) return;
                          void handleImportJsonFile(file);
                        }}
                      />
                      <button
                        className="btn btn-primary btn-full"
                        onClick={handlePickImportFile}
                        disabled={importing}
                      >
                        {importing ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Upload size={16} />
                        )}
                        {t('common.shared.import.pickFile', '选择 JSON 文件导入')}
                      </button>
                    </div>
                  )}

                  {addStatus !== 'idle' && addMessage && (
                    <div className={`add-status ${addStatus}`}>
                      {addStatus === 'success' ? (
                        <Check size={16} />
                      ) : addStatus === 'loading' ? (
                        <RefreshCw size={16} className="loading-spinner" />
                      ) : (
                        <CircleAlert size={16} />
                      )}
                      <span>{addMessage}</span>
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}

          <ExportJsonModal
            isOpen={showExportModal}
            title={`${t('common.shared.export.title', '导出')} JSON`}
            jsonContent={exportJsonContent}
            hidden={exportJsonHidden}
            copied={exportJsonCopied}
            saving={savingExportJson}
            savedPath={exportSavedPath}
            canOpenSavedDirectory={canOpenExportSavedDirectory}
            pathCopied={exportPathCopied}
            onClose={closeExportModal}
            onToggleHidden={toggleExportJsonHidden}
            onCopyJson={copyExportJson}
            onSaveJson={saveExportJson}
            onOpenSavedDirectory={openExportSavedDirectory}
            onCopySavedPath={copyExportSavedPath}
          />

          {deleteConfirm && (
            <div className="modal-overlay" onClick={() => !deleting && setDeleteConfirm(null)}>
              <div className="modal" onClick={(event) => event.stopPropagation()}>
                <div className="modal-header">
                  <h2>{t('common.confirm', '确认')}</h2>
                  <button
                    className="modal-close"
                    onClick={() => !deleting && setDeleteConfirm(null)}
                    aria-label={t('common.close', '关闭')}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
                  <p>{deleteConfirm.message}</p>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => setDeleteConfirm(null)}
                    disabled={deleting}
                  >
                    {t('common.cancel', '取消')}
                  </button>
                  <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>
                    {t('common.confirm', '确认')}
                  </button>
                </div>
              </div>
            </div>
          )}

          {tagDeleteConfirm && (
            <div className="modal-overlay" onClick={() => !deletingTag && setTagDeleteConfirm(null)}>
              <div className="modal" onClick={(event) => event.stopPropagation()}>
                <div className="modal-header">
                  <h2>{t('common.confirm', '确认')}</h2>
                  <button
                    className="modal-close"
                    onClick={() => !deletingTag && setTagDeleteConfirm(null)}
                    aria-label={t('common.close', '关闭')}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} />
                  <p>
                    {t(
                      'accounts.confirmDeleteTag',
                      'Delete tag "{{tag}}"? This tag will be removed from {{count}} accounts.',
                      {
                        tag: tagDeleteConfirm.tag,
                        count: tagDeleteConfirm.count,
                      },
                    )}
                  </p>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => setTagDeleteConfirm(null)}
                    disabled={deletingTag}
                  >
                    {t('common.cancel', '取消')}
                  </button>
                  <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>
                    {deletingTag ? t('common.processing', '处理中...') : t('common.confirm', '确认')}
                  </button>
                </div>
              </div>
            </div>
          )}

          <TagEditModal
            isOpen={!!showTagModal}
            initialTags={accounts.find((account) => account.id === showTagModal)?.tags || []}
            availableTags={availableTags}
            onClose={() => setShowTagModal(null)}
            onSave={handleSaveTags}
          />
        </>
      )}

      {activeTab === 'instances' && (
        <TraeInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}
    </div>
  );
}
