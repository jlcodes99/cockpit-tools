import { useEffect, useMemo, useCallback, useState, Fragment } from 'react';
import {
  Plus,
  RefreshCw,
  Download,
  Upload,
  Trash2,
  X,
  Globe,
  RotateCw,
  CircleAlert,
  LayoutGrid,
  List,
  Search,
  ArrowDownWideNarrow,
  Tag,
  ChevronDown,
  Play,
  Eye,
  EyeOff,
  Lock,
  BookOpen,
  DollarSign,
} from 'lucide-react';
import { useOpencodeAccountStore } from '../stores/useOpencodeAccountStore';
import * as opencodeService from '../services/opencodeService';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { PaginationControls } from '../components/PaginationControls';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import {
  OpenCodeAccount,
  getOpenCodePlanBadge,
  getOpenCodePlanDisplayName,
  getOpenCodePlanBadgeClass,
  getOpenCodeAccountDisplayEmail,
  getOpenCodeGoUsage,
  getOpenCodeZenUsage,
  formatOpenCodeUsageDollars,
  isOpenCodeGo,
  isOpenCodeZen,
  isOpenCodeFree,
} from '../types/opencode';
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

import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { OpencodeOverviewTabsHeader, OpencodeTab } from '../components/OpencodeOverviewTabsHeader';

const OPENCODE_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.opencode.flow_notice_collapsed';
const OPENCODE_CURRENT_ACCOUNT_ID_KEY = 'agtools.opencode.current_account_id';
const OPENCODE_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('OpenCode');
const FILTER_TYPES_FIELD = 'filter_types';
const OPENCODE_KNOWN_TIER_FILTERS = ['GO', 'ZEN', 'FREE'] as const;

const ADD_ACCOUNT_TOKEN_EXAMPLE = 'oc_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx';

function getQuotaClass(percentage: number): string {
  if (percentage >= 90) return 'critical';
  if (percentage >= 70) return 'warning';
  return 'high';
}

export function OpencodeAccountsPage() {
  const [activeTab, setActiveTab] = useState<OpencodeTab>('overview');
  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    readAccountsOverviewFilterPersistenceEnabled(OPENCODE_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterStringArray(OPENCODE_FILTER_PERSISTENCE_SCOPE, FILTER_TYPES_FIELD)
      : [],
  );
  const untaggedKey = '__untagged__';

  const store = useOpencodeAccountStore();

  const [selectedAddTier, setSelectedAddTier] = useState<'go' | 'zen'>('go');

  const handleAddOpenCodeToken = useCallback(async (token: string) => {
    return await opencodeService.addOpenCodeAccount(token, selectedAddTier);
  }, [selectedAddTier]);

  const page = useProviderAccountsPage<OpenCodeAccount>({
    platformKey: 'OpenCode',
    oauthLogPrefix: 'OpenCode',
    flowNoticeCollapsedKey: OPENCODE_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: OPENCODE_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'opencode_accounts',
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
    dataService: {
      importFromJson: opencodeService.importOpenCodeFromJson,
      addWithToken: handleAddOpenCodeToken,
      exportAccounts: opencodeService.exportOpenCodeAccounts,
      injectToVSCode: opencodeService.injectOpenCodeAccount,
    },
    getDisplayEmail: (account) => getOpenCodeAccountDisplayEmail(account),
  });

  const {
    t, privacyModeEnabled, togglePrivacyMode, maskAccountText,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    filterPersistenceEnabled, filterPersistenceScope,
    sortBy, setSortBy, sortDirection, setSortDirection,
    selected, toggleSelect, toggleSelectAll,
    tagFilter, groupByTag, setGroupByTag, showTagFilter, setShowTagFilter,
    showTagModal, setShowTagModal, tagFilterRef, availableTags,
    toggleTagFilterValue, clearTagFilter,
    openTagModal, handleSaveTags,
    refreshing, refreshingAll, injecting,
    handleRefresh, handleRefreshAll, handleDelete, handleBatchDelete,
    deleteConfirm, deleteConfirmError, deleteConfirmErrorScrollKey, setDeleteConfirm, deleting, confirmDelete,
    message, setMessage,
    exporting, handleExport, handleExportByIds, getScopedSelectedCount,
    showExportModal, closeExportModal, exportJsonContent, exportJsonHidden,
    toggleExportJsonHidden, exportJsonCopied, copyExportJson,
    savingExportJson, saveExportJson, exportSavedPath,
    canOpenExportSavedDirectory, openExportSavedDirectory, copyExportSavedPath, exportPathCopied,
    showAddModal, addTab, addStatus, addMessage, tokenInput, setTokenInput,
    importing, openAddModal, closeAddModal,
    handleTokenImport, handleImportJsonFile, handlePickImportFile, importFileInputRef,
    handleInjectToVSCode,
    isFlowNoticeCollapsed, setIsFlowNoticeCollapsed,
    currentAccountId,
    formatDate, normalizeTag,
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

  const resolveTierKey = useCallback(
    (account: OpenCodeAccount) => getOpenCodePlanBadge(account),
    [],
  );

  const resolveTierLabel = useCallback(
    (account: OpenCodeAccount) => getOpenCodePlanDisplayName(account),
    [],
  );

  const isAbnormalAccount = useCallback(
    (account: OpenCodeAccount) =>
      (account.status || '').toLowerCase() === 'error',
    [],
  );

  const resolveTierBadgeClass = useCallback(
    (account: OpenCodeAccount) =>
      getOpenCodePlanBadgeClass(account),
    [],
  );

  const resolveDisplayEmail = useCallback(
    (account: OpenCodeAccount) => getOpenCodeAccountDisplayEmail(account),
    [],
  );

  const resolveSingleExportBaseName = useCallback(
    (account: OpenCodeAccount) => {
      const display = resolveDisplayEmail(account);
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolveDisplayEmail],
  );

  // Tier usage resolvers
  const resolveGoUsage = useCallback((account: OpenCodeAccount) => {
    const go = getOpenCodeGoUsage(account);
    return {
      usage5hDollars: go.usage5hDollars,
      usageWeeklyDollars: go.usageWeeklyDollars,
      usageMonthlyDollars: go.usageMonthlyDollars,
      limit5h: go.limit5h,
      limitWeekly: go.limitWeekly,
      limitMonthly: go.limitMonthly,
      percent5h: go.limit5h > 0 ? (go.usage5hDollars / go.limit5h) * 100 : 0,
      percentWeekly: go.limitWeekly > 0 ? (go.usageWeeklyDollars / go.limitWeekly) * 100 : 0,
      percentMonthly: go.limitMonthly > 0 ? (go.usageMonthlyDollars / go.limitMonthly) * 100 : 0,
    };
  }, []);

  const resolveZenUsage = useCallback((account: OpenCodeAccount) => {
    return getOpenCodeZenUsage(account);
  }, []);

  const tierSummary = useMemo(() => {
    const knownCounts = { GO: 0, ZEN: 0, FREE: 0 };
    const dynamicCounts = new Map<string, number>();
    const displayLabels = new Map<string, string>();

    accounts.forEach((account) => {
      const tier = resolveTierKey(account);
      dynamicCounts.set(tier, (dynamicCounts.get(tier) ?? 0) + 1);
      if (tier in knownCounts) {
        knownCounts[tier as keyof typeof knownCounts] += 1;
      }
      if (!displayLabels.has(tier)) {
        displayLabels.set(tier, resolveTierLabel(account));
      }
    });
    const validCount = accounts.reduce(
      (count, account) => (isAbnormalAccount(account) ? count : count + 1),
      0,
    );

    const extraKeys = Array.from(dynamicCounts.keys())
      .filter((tier) => !(OPENCODE_KNOWN_TIER_FILTERS as readonly string[]).includes(tier))
      .sort((a, b) => a.localeCompare(b));

    return { all: accounts.length, validCount, knownCounts, dynamicCounts, extraKeys, displayLabels };
  }, [accounts, isAbnormalAccount, resolveTierKey, resolveTierLabel]);

  useEffect(() => {
    setFilterTypes((prev) => {
      const next = prev.filter(
        (value) => value === VALID_ACCOUNTS_FILTER_VALUE || tierSummary.dynamicCounts.has(value),
      );
      return next.length === prev.length ? prev : next;
    });
  }, [tierSummary.dynamicCounts]);

  const resolveFilterLabel = useCallback(
    (tierKey: string, count: number) => {
      const label = tierSummary.displayLabels.get(tierKey) ?? tierKey;
      return `${label} (${count})`;
    },
    [tierSummary.displayLabels],
  );

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const options: MultiSelectFilterOption[] = [
      { value: 'GO', label: resolveFilterLabel('GO', tierSummary.knownCounts.GO) },
      { value: 'ZEN', label: resolveFilterLabel('ZEN', tierSummary.knownCounts.ZEN) },
      { value: 'FREE', label: resolveFilterLabel('FREE', tierSummary.knownCounts.FREE) },
    ];
    tierSummary.extraKeys.forEach((tierKey) => {
      options.push({
        value: tierKey,
        label: resolveFilterLabel(tierKey, tierSummary.dynamicCounts.get(tierKey) ?? 0),
      });
    });
    options.push(buildValidAccountsFilterOption(t, tierSummary.validCount));
    return options;
  }, [resolveFilterLabel, t, tierSummary]);

  const compareAccountsBySort = useCallback((a: OpenCodeAccount, b: OpenCodeAccount) => {
    const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (sortBy === 'created_at') {
      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    }

    return 0;
  }, [currentAccountId, sortBy, sortDirection]);

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];

    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((account) => {
        const haystacks = [
          getOpenCodeAccountDisplayEmail(account),
          account.id,
          account.tier,
          account.plan_name ?? '',
          account.subscription_status ?? '',
        ];
        return haystacks.some((item) => item.toLowerCase().includes(query));
      });
    }

    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        result = result.filter((account) => !isAbnormalAccount(account));
      }
      if (selectedTypes.size > 0) {
        result = result.filter((account) => selectedTypes.has(resolveTierKey(account)));
      }
    }

    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((acc) => {
        const tags = (acc.tags || []).map(normalizeTag);
        return tags.some((tag) => selectedTags.has(tag));
      });
    }

    result.sort(compareAccountsBySort);

    return result;
  }, [accounts, compareAccountsBySort, filterTypes, isAbnormalAccount, normalizeTag, resolveTierKey, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('OpenCode'),
  });
  const paginatedAccounts = pagination.pageItems;
  const paginatedIds = useMemo(() => paginatedAccounts.map((account) => account.id), [paginatedAccounts]);
  const isAllPaginatedSelected = useMemo(
    () => isEveryIdSelected(selected, paginatedIds),
    [paginatedIds, selected],
  );

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, typeof filteredAccounts]>;
    const groups = new Map<string, typeof filteredAccounts>();
    const selectedTags = new Set(tagFilter.map(normalizeTag));

    filteredAccounts.forEach((account) => {
      const tags = (account.tags || []).map(normalizeTag).filter(Boolean);
      const matchedTags = selectedTags.size > 0
        ? tags.filter((tag) => selectedTags.has(tag))
        : tags;
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

    return Array.from(groups.entries()).sort(([aKey], [bKey]) => {
      if (aKey === untaggedKey) return -1;
      if (bKey === untaggedKey) return 1;
      return aKey.localeCompare(bKey);
    });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);

  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts],
  );

  const resolveGroupLabel = (groupKey: string) =>
    groupKey === untaggedKey ? t('accounts.defaultGroup', '默认分组') : groupKey;

  // Render tier-specific quota info
  const renderGoQuota = (account: OpenCodeAccount) => {
    const usage = resolveGoUsage(account);
    return (
      <div className="opencode-quota-section">
        <div className="quota-item windsurf-credit-item">
          <div className="quota-header">
            <span className="quota-label">5h Usage</span>
            <span className={`quota-pct ${getQuotaClass(usage.percent5h)}`}>
              {formatOpenCodeUsageDollars(usage.usage5hDollars)} / {formatOpenCodeUsageDollars(usage.limit5h)}
            </span>
          </div>
          <div className="quota-bar-track">
            <div className={`quota-bar ${getQuotaClass(usage.percent5h)}`} style={{ width: `${Math.min(usage.percent5h, 100)}%` }} />
          </div>
        </div>
        <div className="quota-item windsurf-credit-item">
          <div className="quota-header">
            <span className="quota-label">Weekly Usage</span>
            <span className={`quota-pct ${getQuotaClass(usage.percentWeekly)}`}>
              {formatOpenCodeUsageDollars(usage.usageWeeklyDollars)} / {formatOpenCodeUsageDollars(usage.limitWeekly)}
            </span>
          </div>
          <div className="quota-bar-track">
            <div className={`quota-bar ${getQuotaClass(usage.percentWeekly)}`} style={{ width: `${Math.min(usage.percentWeekly, 100)}%` }} />
          </div>
        </div>
        <div className="quota-item windsurf-credit-item">
          <div className="quota-header">
            <span className="quota-label">Monthly Usage</span>
            <span className={`quota-pct ${getQuotaClass(usage.percentMonthly)}`}>
              {formatOpenCodeUsageDollars(usage.usageMonthlyDollars)} / {formatOpenCodeUsageDollars(usage.limitMonthly)}
            </span>
          </div>
          <div className="quota-bar-track">
            <div className={`quota-bar ${getQuotaClass(usage.percentMonthly)}`} style={{ width: `${Math.min(usage.percentMonthly, 100)}%` }} />
          </div>
        </div>
      </div>
    );
  };

  const renderZenQuota = (account: OpenCodeAccount) => {
    const zen = resolveZenUsage(account);
    return (
      <div className="opencode-quota-section">
        <div className="quota-item windsurf-credit-item">
          <div className="quota-header">
            <span className="quota-label">Balance</span>
            <span className="quota-pct">{formatOpenCodeUsageDollars(zen.balanceDollars)}</span>
          </div>
          {zen.monthlySpendLimit != null && (
            <div className="windsurf-credit-meta-row">
              <span className="windsurf-credit-used">
                Monthly limit: {formatOpenCodeUsageDollars(zen.monthlySpendLimit)}
              </span>
            </div>
          )}
          <div className="windsurf-credit-meta-row">
            <span className="windsurf-credit-used">
              Auto-reload: {zen.autoReloadEnabled ? t('common.enabled', 'Enabled') : t('common.disabled', 'Disabled')}
            </span>
          </div>
        </div>
      </div>
    );
  };

  // ─── Render helpers ────────────────────────────────────────────────

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const displayEmail = resolveDisplayEmail(account);
      const emailText = displayEmail || account.id;
      const tierLabel = resolveTierLabel(account);
      const tierBadgeClass = resolveTierBadgeClass(account);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      const isGo = isOpenCodeGo(account);
      const isZen = isOpenCodeZen(account);
      const isFree = isOpenCodeFree(account);
      const isBanned = (account.status || '').toLowerCase() === 'banned';
      const hasStatusError = (account.status || '').toLowerCase() === 'error';
      const statusReason = account.status_reason ?? null;
      const bannedTitle = statusReason || t('accounts.status.forbidden_tooltip');
      const errorTitle = statusReason || t('accounts.status.refreshFailed');
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);

      return (
        <div
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`ghcp-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''} ${isBanned ? 'disabled' : ''}`}
        >
          <div className="card-top">
            <div className="card-select">
              <input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} />
            </div>
            <span className="account-email" title={maskAccountText(emailText)}>
              {maskAccountText(emailText)}
            </span>
            {isCurrent && (<span className="current-tag">{t('accounts.status.current')}</span>)}
            {hasStatusError && (
              <span className="status-pill warning" title={errorTitle}>
                <CircleAlert size={12} />
                {t('accounts.status.refreshFailed')}
              </span>
            )}
            {isBanned && (
              <span className="status-pill forbidden" title={bannedTitle}>
                <Lock size={12} />
                {t('accounts.status.forbidden')}
              </span>
            )}
            <span className={`tier-badge ${tierBadgeClass}`}>{tierLabel}</span>
          </div>

          {accountTags.length > 0 && (
            <div className="card-tags">
              {visibleTags.map((tag, idx) => (
                <span key={`${account.id}-${tag}-${idx}`} className="tag-pill">{tag}</span>
              ))}
              {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
            </div>
          )}

          {isFree ? (
            <div className="quota-empty">
              {t('opencode.freeUsageNote', 'Free tier — no usage tracking')}
            </div>
          ) : isGo ? (
            renderGoQuota(account)
          ) : isZen ? (
            renderZenQuota(account)
          ) : null}

          <div className="card-footer">
            <div className="tier-models-info">
              <DollarSign size={12} />
              {isGo && <span className="tier-model-badge">Go — 12 models</span>}
              {isZen && <span className="tier-model-badge">Zen — 40+ models</span>}
              {isFree && <span className="tier-model-badge">Free — 5 models</span>}
            </div>
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('opencode.injectToVSCode', 'Inject into VS Code')}>
                {injecting === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="card-action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}>
                <Tag size={14} />
              </button>
              <button className="card-action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新配额')}>
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button
                className="card-action-btn export-btn"
                onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
                title={t('common.shared.export.title', '导出')}
              >
                <Upload size={14} />
              </button>
              <button className="card-action-btn danger" onClick={() => handleDelete(account.id)} title={t('common.delete', '删除')}>
                <Trash2 size={14} />
              </button>
            </div>
          </div>
        </div>
      );
    });

  const renderTableRows = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const displayEmail = resolveDisplayEmail(account);
      const emailText = displayEmail || account.id;
      const tierLabel = resolveTierLabel(account);
      const tierBadgeClass = resolveTierBadgeClass(account);
      const isCurrent = currentAccountId === account.id;
      const isBanned = (account.status || '').toLowerCase() === 'banned';
      const isGo = isOpenCodeGo(account);
      const isFree = isOpenCodeFree(account);
      const isZen = isOpenCodeZen(account);
      const hasStatusError = (account.status || '').toLowerCase() === 'error';
      const statusReason = account.status_reason ?? null;
      const bannedTitle = statusReason || t('accounts.status.forbidden_tooltip');
      const errorTitle = statusReason || t('accounts.status.refreshFailed');
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 3);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);

      return (
        <tr key={groupKey ? `${groupKey}-${account.id}` : account.id} className={`${isCurrent ? 'current' : ''} ${isBanned ? 'disabled' : ''}`}>
          <td><input type="checkbox" checked={selected.has(account.id)} onChange={() => toggleSelect(account.id)} /></td>
          <td>
            <div className="account-cell">
              <div className="account-main-line">
                <span className="account-email-text" title={maskAccountText(emailText)}>{maskAccountText(emailText)}</span>
                {isCurrent && <span className="mini-tag current">{t('accounts.status.current')}</span>}
              </div>
              {(hasStatusError || isBanned) && (
                <div className="account-sub-line">
                  {hasStatusError && (<span className="status-pill warning" title={errorTitle}><CircleAlert size={12} />{t('accounts.status.refreshFailed')}</span>)}
                  {isBanned && (<span className="status-pill forbidden" title={bannedTitle}><Lock size={12} />{t('accounts.status.forbidden')}</span>)}
                </div>
              )}
              {accountTags.length > 0 && (
                <div className="account-tags-inline">
                  {visibleTags.map((tag, idx) => (<span key={`${account.id}-inline-${tag}-${idx}`} className="tag-pill">{tag}</span>))}
                  {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                </div>
              )}
            </div>
          </td>
          <td><span className={`tier-badge ${tierBadgeClass}`}>{tierLabel}</span></td>
          <td>
            {isFree ? (
              <div className="quota-empty">{t('opencode.freeUsageNote', 'Free tier — no usage tracking')}</div>
            ) : isGo ? (
              renderGoQuota(account)
            ) : isZen ? (
              renderZenQuota(account)
            ) : (
              <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
            )}
          </td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button className="action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('opencode.injectToVSCode', 'Inject into VS Code')}>
                {injecting === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}>
                <Tag size={14} />
              </button>
              <button className="action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新配额')}>
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button
                className="action-btn"
                onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
                title={t('common.shared.export.title', '导出')}
              >
                <Upload size={14} />
              </button>
              <button className="action-btn danger" onClick={() => handleDelete(account.id)} title={t('common.delete', '删除')}>
                <Trash2 size={14} />
              </button>
            </div>
          </td>
        </tr>
      );
    });

  return (
    <div className="ghcp-accounts-page opencode-accounts-page">
      <OpencodeOverviewTabsHeader active={activeTab} onTabChange={setActiveTab} />
      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)} aria-expanded={!isFlowNoticeCollapsed}>
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('opencode.flowNotice.title', 'OpenCode 账号管理说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t('opencode.flowNotice.desc', 'Manage OpenCode accounts by pasting API keys. Go = monthly subscription, Zen = pay-as-you-go, Free = no key needed.')}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('opencode.flowNotice.go', 'Go tier: $5-10/mo subscription with 12 open-source models. Dollar-value limits: $12/5hr, $30/weekly, $60/monthly.')}</li>
              <li>{t('opencode.flowNotice.zen', 'Zen tier: $20 minimum top-up, 40+ models including GPT/Claude/Gemini, per-token pricing.')}</li>
              <li>{t('opencode.flowNotice.free', 'Free tier: Limited free models, no API key required.')}</li>
            </ul>
          </div>
        )}
      </div>

      {activeTab === 'overview' && (
        <>
      {message && (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
          {message.text}
          <button onClick={() => setMessage(null)}><X size={14} /></button>
        </div>
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search size={16} className="search-icon" />
            <input type="text" placeholder={t('common.shared.search', '搜索账号...')} value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} />
          </div>

          <div className="view-switcher">
            <button className={`view-btn ${viewMode === 'list' ? 'active' : ''}`} onClick={() => setViewMode('list')} title={t('common.shared.view.list', '列表视图')}><List size={16} /></button>
            <button className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`} onClick={() => setViewMode('grid')} title={t('common.shared.view.grid', '卡片视图')}><LayoutGrid size={16} /></button>
          </div>

          <MultiSelectFilterDropdown
            options={tierFilterOptions}
            selectedValues={filterTypes}
            allLabel={`ALL (${tierSummary.all})`}
            filterLabel={t('common.shared.filterLabel', '筛选')}
            clearLabel={t('accounts.clearFilter', '清空筛选')}
            emptyLabel={t('common.none', '暂无')}
            ariaLabel={t('common.shared.filterLabel', '筛选')}
            onToggleValue={toggleFilterTypeValue}
            onClear={clearFilterTypes}
          />

          <div className="tag-filter" ref={tagFilterRef}>
            <button type="button" className={`tag-filter-btn ${tagFilter.length > 0 ? 'active' : ''}`} onClick={() => setShowTagFilter((prev) => !prev)} aria-label={t('accounts.filterTags', '标签筛选')}>
              <Tag size={14} />
              {tagFilter.length > 0 ? `${t('accounts.filterTagsCount', '标签')}(${tagFilter.length})` : t('accounts.filterTags', '标签筛选')}
            </button>
            {showTagFilter && (
              <div
                ref={page.tagFilterPanelRef}
                className={`tag-filter-panel ${page.tagFilterPanelPlacement === 'top' ? 'open-top' : ''}`}
              >
                {availableTags.length === 0 ? (
                  <div className="tag-filter-empty">{t('accounts.noAvailableTags', '暂无可用标签')}</div>
                ) : (
                  <div className="tag-filter-options" style={page.tagFilterScrollContainerStyle}>
                    {availableTags.map((tag) => (
                      <label key={tag} className={`tag-filter-option ${tagFilter.includes(tag) ? 'selected' : ''}`}>
                        <input type="checkbox" checked={tagFilter.includes(tag)} onChange={() => toggleTagFilterValue(tag)} />
                        <span className="tag-filter-name">{tag}</span>
                      </label>
                    ))}
                  </div>
                )}
                <div className="tag-filter-divider" />
                <label className="tag-filter-group-toggle">
                  <input type="checkbox" checked={groupByTag} onChange={(e) => setGroupByTag(e.target.checked)} />
                  <span>{t('accounts.groupByTag', '按标签分组展示')}</span>
                </label>
                {tagFilter.length > 0 && (
                  <button type="button" className="tag-filter-clear" onClick={clearTagFilter}>{t('accounts.clearFilter', '清空筛选')}</button>
                )}
              </div>
            )}
          </div>

          <SingleSelectFilterDropdown
            value={sortBy}
            options={[
              { value: 'created_at', label: t('common.shared.sort.createdAt', '按创建时间') },
            ]}
            ariaLabel={t('common.shared.sortLabel', '排序')}
            icon={<ArrowDownWideNarrow size={14} />}
            onChange={setSortBy}
          />

          <button className="sort-direction-btn" onClick={() => setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
            title={sortDirection === 'desc' ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序') : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序')}
            aria-label={t('common.shared.sort.toggleDirection', '切换排序方向')}>
            {sortDirection === 'desc' ? '⬇' : '⬆'}
          </button>
        </div>
        <div className="toolbar-right">
          <button className="btn btn-primary icon-only" onClick={() => openAddModal('token')} title={t('common.shared.addAccount', '添加账号')} aria-label={t('common.shared.addAccount', '添加账号')}><Plus size={14} /></button>
          <button className="btn btn-secondary icon-only" onClick={handleRefreshAll} disabled={refreshingAll || accounts.length === 0} title={t('common.shared.refreshAll', '刷新全部')} aria-label={t('common.shared.refreshAll', '刷新全部')}>
            <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
          </button>
          <button className="btn btn-secondary icon-only" onClick={togglePrivacyMode}
            title={privacyModeEnabled ? t('privacy.showSensitive', '显示邮箱') : t('privacy.hideSensitive', '隐藏邮箱')}
            aria-label={privacyModeEnabled ? t('privacy.showSensitive', '显示邮箱') : t('privacy.hideSensitive', '隐藏邮箱')}>
            {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
          <button className="btn btn-secondary icon-only" onClick={() => openAddModal('import')} disabled={importing} title={t('common.shared.import.label', '导入')} aria-label={t('common.shared.import.label', '导入')}><Download size={14} /></button>
          <button className="btn btn-secondary export-btn icon-only" onClick={() => void handleExport(filteredIds)} disabled={exporting || filteredIds.length === 0}
            title={exportSelectionCount > 0 ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})` : t('common.shared.export.title', '导出')}
            aria-label={exportSelectionCount > 0 ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})` : t('common.shared.export.title', '导出')}>
            <Upload size={14} />
          </button>
          {selected.size > 0 && (
            <button className="btn btn-danger icon-only" onClick={handleBatchDelete} title={`${t('common.delete', '删除')} (${selected.size})`} aria-label={`${t('common.delete', '删除')} (${selected.size})`}>
              <Trash2 size={14} />
            </button>
          )}
          <QuickSettingsPopover type="opencode" />
        </div>
      </div>

      {loading && accounts.length === 0 ? (
        <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <Globe size={48} />
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t('opencode.empty.description', '点击"添加账号"开始管理您的 OpenCode 账号')}</p>
          <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
            <button className="btn btn-primary" onClick={() => openAddModal('token')}>
              <Plus size={16} />
              {t('common.shared.addAccount', '添加账号')}
            </button>
            <button className="btn btn-secondary" onClick={() => window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: 'manual' }))}>
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
                <div className="tag-group-grid ghcp-accounts-grid">{renderGridCards(items, groupKey)}</div>
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
                  <input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} />
                </th>
                <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th>
                <th style={{ width: 120 }}>{t('common.shared.columns.plan', '计划')}</th>
                <th>{t('common.shared.columns.usage', '用量')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
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
                  <input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} />
                </th>
                <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th>
                <th style={{ width: 120 }}>{t('common.shared.columns.plan', '计划')}</th>
                <th>{t('common.shared.columns.usage', '用量')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
              </tr>
            </thead>
            <tbody>
              {renderTableRows(paginatedAccounts)}
            </tbody>
          </table>
        </div>
      )}

      <TagEditModal
        isOpen={!!showTagModal}
        initialTags={accounts.find((a) => a.id === showTagModal)?.tags || []}
        availableTags={availableTags}
        onClose={() => setShowTagModal(null)}
        onSave={handleSaveTags}
      />

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
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button className="modal-close" onClick={() => !deleting && setDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button>
            </div>
            <p className="modal-message">{t('accounts.confirmDeleteMsg', '确定要删除选中的账号吗？此操作不可撤销。')}</p>
            {deleteConfirmError && (
              <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
            )}
            <div className="modal-actions">
              <button className="btn btn-secondary" disabled={deleting} onClick={() => setDeleteConfirm(null)}>
                {t('common.cancel', '取消')}
              </button>
              <button className="btn btn-danger" disabled={deleting} onClick={confirmDelete}>
                {deleting ? t('common.deleting', '删除中...') : t('common.delete', '删除')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showAddModal && (
        <div className="modal-overlay">
          <div className="modal add-account-modal">
            <div className="modal-header">
              <h3>{t('opencode.addAccount.title', '添加 OpenCode 账号')}</h3>
              <p className="add-account-desc">
                {t('opencode.addAccount.desc', '选择 tier 并粘贴你的 OpenCode API Key。')}
              </p>
            </div>

            {addTab === 'token' && (
            <div className="add-token-section">
              <div className="single-input-section">
                <label>{t('opencode.addAccount.tier', 'Tier')}</label>
                <div className="tier-selector" style={{ display: 'flex', gap: '8px', marginBottom: '12px' }}>
                  <button
                    className={`btn ${selectedAddTier === 'go' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setSelectedAddTier('go')}
                  >
                    Go
                  </button>
                  <button
                    className={`btn ${selectedAddTier === 'zen' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setSelectedAddTier('zen')}
                  >
                    Zen
                  </button>
                </div>
              </div>
              <div className="single-input-section">
                <label>{t('opencode.addAccount.apiKey', 'API Key')}</label>
                <input
                  type="text"
                  placeholder={ADD_ACCOUNT_TOKEN_EXAMPLE}
                  value={tokenInput}
                  onChange={(e) => setTokenInput(e.target.value)}
                />
              </div>
            </div>
            )}

            {addTab === 'import' && (
              <div className="batch-import-section">
                <label>{t('common.shared.import.jsonLabel', '粘贴 JSON')}</label>
                <textarea
                  rows={6}
                  placeholder='[{&quot;access_token&quot;:&quot;...&quot;,&quot;tier&quot;:&quot;go&quot;,&quot;email&quot;:&quot;...&quot;}]'
                  value={tokenInput}
                  onChange={(e) => setTokenInput(e.target.value)}
                />
                <button className="btn btn-secondary" onClick={handlePickImportFile}>
                  <Download size={14} />
                  {t('common.shared.import.fromFile', '从文件导入')}
                </button>
                <input ref={importFileInputRef} type="file" accept=".json" style={{ display: 'none' }} onChange={(event) => { if (event.target.files?.[0]) { handleImportJsonFile(event.target.files[0]); } }} />
              </div>
            )}

            {addMessage && (
              <div className={`add-message ${addStatus === 'error' ? 'error' : 'success'}`}>
                {addMessage}
              </div>
            )}

            <div className="modal-actions">
              <button className="btn btn-secondary" onClick={closeAddModal} disabled={importing}>
                {t('common.cancel', '取消')}
              </button>
              {addTab === 'token' && (
                <button className="btn btn-primary" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                  {importing ? t('common.importing', '导入中...') : t('common.shared.addAccount', '添加账号')}
                </button>
              )}
              {addTab === 'import' && (
                <button className="btn btn-primary" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                  {importing ? t('common.importing', '导入中...') : t('common.shared.import.title', '导入')}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
      </>
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
    </div>
  );
}
