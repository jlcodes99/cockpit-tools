import { useAccountsOverviewFilterState } from '../hooks/useAccountsOverviewFilterState';
import { useEffect, useMemo, useCallback, useRef, useState, Fragment } from 'react';
import {
  Plus,
  RefreshCw,
  Download,
  Upload,
  Trash2,
  X,
  Globe,
  KeyRound,
  Database,
  Copy,
  Check,
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
} from 'lucide-react';
import { AccountLastUsed } from '../components/AccountLastUsed';
import { useCursorAccountStore } from '../stores/useCursorAccountStore';
import * as cursorService from '../services/cursorService';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage, useModalErrorState } from '../components/ModalErrorMessage';
import { MfaQuickCodeSelect } from '../components/MfaQuickCodeSelect';
import { PaginationControls } from '../components/PaginationControls';
import { AccountSelectionToolbar } from '../components/AccountSelectionToolbar';
import { usePlatformAccountGroups } from '../hooks/usePlatformAccountGroups';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import {
  getCursorPlanBadge,
  getCursorPlanDisplayName,
  getCursorPlanBadgeClass,
  getCursorAccountDisplayEmail,
  getCursorUsage,
  hasCursorQuotaQueryError,
  isCursorAccountBanned,
} from '../types/cursor';
import {
  buildCursorAccountPresentation,
  type UnifiedQuotaMetric,
} from '../presentation/platformAccountPresentation';
import type { CursorAccount } from '../types/cursor';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { ACCOUNTS_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { subscribeTauriEvent } from '../utils/tauriEventListener';
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
} from '../utils/accountsOverviewFilterPersistence';

import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { CursorOverviewTabsHeader, CursorTab } from '../components/CursorOverviewTabsHeader';
import { CursorInstancesContent } from './CursorInstancesPage';

const CURSOR_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.cursor.flow_notice_collapsed';
const CURSOR_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('Cursor');
const FILTER_TYPES_FIELD = 'filter_types';
const CURSOR_KNOWN_PLAN_FILTERS = [
  'FREE',
  'PRO',
  'PRO_PLUS',
  'ENTERPRISE',
  'FREE_TRIAL',
  'ULTRA',
] as const;
const CURSOR_TOKEN_SINGLE_EXAMPLE = `eyJhbGciOiJIUzI1NiIs...`;
const CURSOR_TOKEN_BATCH_EXAMPLE = `[
  {"access_token":"eyJhbGciOiJIUzI1NiIs...","email":"a@example.com"},
  {"access_token":"eyJhbGciOiJIUzI1NiIs...","email":"b@example.com"}
]`;
const CURSOR_QUOTA_FAILED_FILTER_VALUE = 'QUOTA_FAILED';

export function CursorAccountsPage() {
  const [activeTab, setActiveTab] = useState<CursorTab>('overview');
  const [quotaFailureConfirm, setQuotaFailureConfirm] = useState(false);
  const [retryingQuotaFailures, setRetryingQuotaFailures] = useState(false);
  const {
    message: quotaRetryError,
    scrollKey: quotaRetryErrorScrollKey,
    set: setQuotaRetryError,
  } = useModalErrorState();
  const [filterTypes, setFilterTypes] = useAccountsOverviewFilterState<string[]>(CURSOR_FILTER_PERSISTENCE_SCOPE, FILTER_TYPES_FIELD, () =>
    readAccountsOverviewFilterPersistenceEnabled(CURSOR_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterStringArray(CURSOR_FILTER_PERSISTENCE_SCOPE, FILTER_TYPES_FIELD)
      : [],
  );
  const untaggedKey = '__untagged__';

  const store = useCursorAccountStore();
  const fetchAccounts = store.fetchAccounts;
  useEffect(() => subscribeTauriEvent<AccountSyncEventPayload>(ACCOUNTS_CHANGED_EVENT, ({ payload }) => {
    if (payload.platformId === 'cursor' && payload.reason === 'oauth-refresh') {
      void fetchAccounts();
    }
  }), [fetchAccounts]);

  const page = useProviderAccountsPage<CursorAccount>({
    platformKey: 'Cursor',
    oauthLogPrefix: 'CursorOAuth',
    flowNoticeCollapsedKey: CURSOR_FLOW_NOTICE_COLLAPSED_KEY,
    exportFilePrefix: 'cursor_accounts',
    disableEnterConfirmDelete: quotaFailureConfirm,
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
      startLogin: async () => {
        const resp = await cursorService.startCursorOAuthLogin();
        return {
          loginId: resp.loginId,
          verificationUri: resp.verificationUri,
          expiresIn: resp.expiresIn,
          intervalSeconds: resp.intervalSeconds,
        };
      },
      completeLogin: (loginId: string) => cursorService.completeCursorOAuthLogin(loginId),
      cancelLogin: (loginId?: string) => cursorService.cancelCursorOAuthLogin(loginId),
    },
    dataService: {
      importFromJson: cursorService.importCursorFromJson,
      importFromLocal: cursorService.importCursorFromLocal,
      addWithToken: cursorService.addCursorAccountWithToken,
      exportAccounts: cursorService.exportCursorAccounts,
      injectToVSCode: cursorService.injectCursorAccount,
    },
    getDisplayEmail: (account) => getCursorAccountDisplayEmail(account),
  });

  const {
    t, privacyModeEnabled, togglePrivacyMode, maskAccountText,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    sortBy, setSortBy, sortDirection, setSortDirection,
    selected, toggleSelect, toggleSelectAll,
    tagFilter, groupByTag, setGroupByTag, showTagFilter, setShowTagFilter,
    showTagModal, setShowTagModal, tagFilterRef, availableTags,
    toggleTagFilterValue, clearTagFilter, tagDeleteConfirm, tagDeleteConfirmError, tagDeleteConfirmErrorScrollKey, setTagDeleteConfirm,
    deletingTag, requestDeleteTag, confirmDeleteTag, openTagModal, handleSaveTags,
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
    handleTokenImport, handleImportJsonFile, handleImportFromLocal, handlePickImportFile, importFileInputRef,
    handleInjectToVSCode,
    oauthUrl, oauthUrlCopied, oauthUserCode, oauthUserCodeCopied, oauthPolling, oauthTimedOut, oauthPrepareError, oauthCompleteError,
    oauthMeta,
    handleCopyOauthUrl, handleCopyOauthUserCode, handleRetryOauth, handleOpenOauthUrl,
    isFlowNoticeCollapsed, setIsFlowNoticeCollapsed,
    currentAccountId,
    formatDate, normalizeTag,
  } = page;

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

  const grouping = usePlatformAccountGroups('cursor', () => toggleSelectAll(Array.from(selected)));

  // ─── Platform-specific: Plan resolution ────────────────────────────

  const resolvePlanKey = useCallback(
    (account: CursorAccount) => getCursorPlanBadge(account),
    [],
  );

  const resolvePlanLabel = useCallback(
    (account: CursorAccount) => getCursorPlanDisplayName(account),
    [],
  );

  const isAbnormalAccount = useCallback(
    (account: CursorAccount) =>
      isCursorAccountBanned(account) || (account.status || '').toLowerCase() === 'error',
    [],
  );

  const resolvePlanBadgeClass = useCallback(
    (account: CursorAccount) =>
      getCursorPlanBadgeClass(account.membership_type?.trim() || account.plan_type, account),
    [],
  );

  const resolveDisplayEmail = useCallback(
    (account: CursorAccount) => getCursorAccountDisplayEmail(account),
    [],
  );

  const resolveSingleExportBaseName = useCallback(
    (account: CursorAccount) => {
      const display = resolveDisplayEmail(account);
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolveDisplayEmail],
  );

  // ─── Platform-specific: Quota ──────────────────────────────────────

  const renderQuotaMetric = (item: UnifiedQuotaMetric, table = false) => {
    const quotaClass = item.quotaClass === 'low' ? 'critical' : item.quotaClass;
    return (
      <div key={item.key} className={table ? 'quota-item windsurf-table-credit-item' : 'quota-item windsurf-credit-item'}>
        <div className="quota-header">
          <span className={table ? 'quota-name' : 'quota-label'}>{item.label}</span>
          <span className={`${table ? 'quota-value' : 'quota-pct'} ${item.showProgress === false ? '' : quotaClass}`}>
            {item.hintText && <span title={item.hintText} aria-label={item.hintText}><CircleAlert size={12} /></span>}
            {item.valueText}
          </span>
        </div>
        {item.resetText && (
          <div className={table ? 'windsurf-credit-meta-row table' : 'windsurf-credit-meta-row'}>
            <span className="windsurf-credit-used">{item.resetText}</span>
          </div>
        )}
        {item.showProgress !== false && (
          <div className={table ? 'quota-progress-track' : 'quota-bar-track'}>
            <div className={`${table ? 'quota-progress-bar' : 'quota-bar'} ${quotaClass}`} style={{ width: `${item.percentage}%` }} />
          </div>
        )}
      </div>
    );
  };

  // ─── Platform-specific: Dynamic tier filter ────────────────────────

  const tierSummary = useMemo(() => {
    const knownCounts = {
      FREE: 0,
      PRO: 0,
      PRO_PLUS: 0,
      ENTERPRISE: 0,
      FREE_TRIAL: 0,
      ULTRA: 0,
    };
    const dynamicCounts = new Map<string, number>();
    const displayLabels = new Map<string, string>();

    accounts.forEach((account) => {
      const tier = resolvePlanKey(account);
      dynamicCounts.set(tier, (dynamicCounts.get(tier) ?? 0) + 1);
      if (tier in knownCounts) {
        knownCounts[tier as keyof typeof knownCounts] += 1;
      }
      if (!displayLabels.has(tier)) {
        displayLabels.set(tier, resolvePlanLabel(account));
      }
    });
    const validCount = accounts.reduce(
      (count, account) => (isAbnormalAccount(account) ? count : count + 1),
      0,
    );
    const quotaFailedCount = accounts.reduce(
      (count, account) => (hasCursorQuotaQueryError(account) ? count + 1 : count),
      0,
    );

    const extraKeys = Array.from(dynamicCounts.keys())
      .filter((tier) => !(CURSOR_KNOWN_PLAN_FILTERS as readonly string[]).includes(tier))
      .sort((a, b) => a.localeCompare(b));

    return { all: accounts.length, validCount, quotaFailedCount, knownCounts, dynamicCounts, extraKeys, displayLabels };
  }, [accounts, isAbnormalAccount, resolvePlanKey, resolvePlanLabel]);

  useEffect(() => {
    setFilterTypes((prev) => {
      const next = prev.filter(
        (value) =>
          value === VALID_ACCOUNTS_FILTER_VALUE ||
          (value === CURSOR_QUOTA_FAILED_FILTER_VALUE && tierSummary.quotaFailedCount > 0) ||
          tierSummary.dynamicCounts.has(value),
      );
      return next.length === prev.length ? prev : next;
    });
  }, [tierSummary.dynamicCounts, tierSummary.quotaFailedCount]);

  const resolveFilterLabel = useCallback(
    (planKey: string, count: number) => {
      const label = tierSummary.displayLabels.get(planKey) ?? planKey;
      return `${label} (${count})`;
    },
    [tierSummary.displayLabels],
  );

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const options: MultiSelectFilterOption[] = [
      { value: 'FREE', label: resolveFilterLabel('FREE', tierSummary.knownCounts.FREE) },
      { value: 'PRO', label: resolveFilterLabel('PRO', tierSummary.knownCounts.PRO) },
      { value: 'PRO_PLUS', label: resolveFilterLabel('PRO_PLUS', tierSummary.knownCounts.PRO_PLUS) },
      { value: 'ENTERPRISE', label: resolveFilterLabel('ENTERPRISE', tierSummary.knownCounts.ENTERPRISE) },
      { value: 'FREE_TRIAL', label: resolveFilterLabel('FREE_TRIAL', tierSummary.knownCounts.FREE_TRIAL) },
      { value: 'ULTRA', label: resolveFilterLabel('ULTRA', tierSummary.knownCounts.ULTRA) },
    ];
    tierSummary.extraKeys.forEach((planKey) => {
      options.push({
        value: planKey,
        label: resolveFilterLabel(planKey, tierSummary.dynamicCounts.get(planKey) ?? 0),
      });
    });
    if (tierSummary.quotaFailedCount > 0) {
      options.push({
        value: CURSOR_QUOTA_FAILED_FILTER_VALUE,
        label: t('cursor.filters.quotaFailed', {
          count: tierSummary.quotaFailedCount,
          defaultValue: '配额查询失败 ({{count}})',
        }),
      });
    }
    options.push(buildValidAccountsFilterOption(t, tierSummary.validCount));
    return options;
  }, [resolveFilterLabel, t, tierSummary.dynamicCounts, tierSummary.extraKeys, tierSummary.knownCounts.ENTERPRISE, tierSummary.knownCounts.FREE, tierSummary.knownCounts.FREE_TRIAL, tierSummary.knownCounts.PRO, tierSummary.knownCounts.PRO_PLUS, tierSummary.knownCounts.ULTRA, tierSummary.quotaFailedCount, tierSummary.validCount]);

  // ─── Filtering & Sorting ──────────────────────────────────────────

  const compareAccountsBySort = useCallback((a: CursorAccount, b: CursorAccount) => {
    const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (sortBy === 'created_at') {
      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    }
    if (sortBy === 'plan_end') {
      const aReset = getCursorUsage(a).allowanceResetAt ?? null;
      const bReset = getCursorUsage(b).allowanceResetAt ?? null;
      if (aReset == null && bReset == null) return 0;
      if (aReset == null) return 1;
      if (bReset == null) return -1;
      const diff = bReset - aReset;
      return sortDirection === 'desc' ? diff : -diff;
    }
    const aUsage = getCursorUsage(a);
    const bUsage = getCursorUsage(b);
    const aUsed = aUsage.inlineSuggestionsUsedPercent;
    const bUsed = bUsage.inlineSuggestionsUsedPercent;
    if (aUsed == null) return bUsed == null ? 0 : 1;
    if (bUsed == null) return -1;
    const diff = aUsed - bUsed;
    return sortDirection === 'desc' ? diff : -diff;
  }, [currentAccountId, sortBy, sortDirection]);

  const sortedAccountsForInstances = useMemo(
    () => [...accounts].sort(compareAccountsBySort),
    [accounts, compareAccountsBySort],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    if (grouping.activeGroupId) {
      result = grouping.filterAccountsByGroup(result);
    }

    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((account) => {
        const haystacks = [
          getCursorAccountDisplayEmail(account),
          account.id,
          account.auth_id ?? '',
          account.membership_type ?? '',
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
        result = result.filter((account) => {
          const matchesQuotaFailed =
            selectedTypes.has(CURSOR_QUOTA_FAILED_FILTER_VALUE) &&
            hasCursorQuotaQueryError(account);
          const matchesPlan = Array.from(selectedTypes).some(
            (key) =>
              key !== CURSOR_QUOTA_FAILED_FILTER_VALUE &&
              resolvePlanKey(account) === key,
          );
          return matchesQuotaFailed || matchesPlan;
        });
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
  }, [accounts, grouping.activeGroupId, grouping.filterAccountsByGroup, compareAccountsBySort, filterTypes, isAbnormalAccount, normalizeTag, resolvePlanKey, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const quotaFailedAccountIds = useMemo(
    () => accounts.filter(hasCursorQuotaQueryError).map((account) => account.id),
    [accounts],
  );
  const activeDeleteConfirm = useRef(deleteConfirm);
  activeDeleteConfirm.current = deleteConfirm;
  useEffect(() => {
    if (deleteConfirm) return;
    setQuotaFailureConfirm(false);
    setQuotaRetryError(null);
  }, [deleteConfirm, setQuotaRetryError]);
  const setQuotaFailureIds = (ids: string[]) => {
    setDeleteConfirm(ids.length > 0 ? {
      ids,
      message: t('cursor.filters.cleanQuotaFailedConfirm', { count: ids.length }),
    } : null);
  };
  const handleOpenQuotaFailures = () => {
    setQuotaFailureConfirm(true);
    setQuotaFailureIds(quotaFailedAccountIds);
  };
  const handleRetryQuotaFailures = async () => {
    if (!deleteConfirm || retryingQuotaFailures || deleting) return;
    setRetryingQuotaFailures(true);
    setQuotaRetryError(null);
    try {
      const remainingIds = await cursorService.retryCursorFailedQuotas(deleteConfirm.ids);
      await fetchAccounts();
      if (activeDeleteConfirm.current !== deleteConfirm) return;
      setQuotaFailureIds(remainingIds);
      if (remainingIds.length > 0) {
        setQuotaRetryError(t('cursor.filters.quotaFailed', { count: remainingIds.length }));
      } else {
        setMessage({ text: t('common.shared.refreshSuccess'), tone: 'success' });
      }
    } catch (error) {
      if (activeDeleteConfirm.current === deleteConfirm) {
        setQuotaRetryError(t('common.shared.refreshFailed', { error: String(error) }));
      }
    } finally {
      setRetryingQuotaFailures(false);
    }
  };
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('Cursor'),
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

  // ─── Render helpers ────────────────────────────────────────────────

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const displayEmail = resolveDisplayEmail(account);
      const emailText = displayEmail || account.id;
      const authIdText = (account.auth_id || '').trim();
      const maskedAuthIdText = authIdText ? maskAccountText(authIdText) : '--';
      const planLabel = resolvePlanLabel(account);
      const { quotaItems } = buildCursorAccountPresentation(account, t);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      const quotaError = account.quota_query_last_error?.trim();
      const isBanned = isCursorAccountBanned(account);
      const hasStatusError = (account.status || '').toLowerCase() === 'error';
      const statusReason = account.status_reason ?? null;
      const bannedTitle = statusReason || t('accounts.status.forbidden_tooltip');
      const errorTitle = statusReason || t('accounts.status.refreshFailed');

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
            {quotaError && (
              <span className="status-pill warning" title={quotaError}>
                <CircleAlert size={12} />
                {t('common.shared.quota.queryFailed', '配额查询失败')}
              </span>
            )}
            {isBanned && (
              <span className="status-pill forbidden" title={bannedTitle}>
                <Lock size={12} />
                {t('accounts.status.forbidden')}
              </span>
            )}
            <span className={`tier-badge ${resolvePlanBadgeClass(account)}`}>{planLabel}</span>
          </div>

          <div className="account-sub-line">
            <span className="kiro-table-subline" title={`Auth ID: ${maskedAuthIdText}`}>
              Auth ID: {maskedAuthIdText}
            </span>
          </div>

          {accountTags.length > 0 && (
            <div className="card-tags">
              {visibleTags.map((tag, idx) => (
                <span key={`${account.id}-${tag}-${idx}`} className="tag-pill">{tag}</span>
              ))}
              {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
            </div>
          )}

          <div className="ghcp-quota-section">
            {quotaItems.map((item) => renderQuotaMetric(item))}
          </div>

          <div className="card-footer">
            <AccountLastUsed accountId={account.id} lastSwitchedAt={account.last_switched_at} createdAt={account.created_at} formatDate={formatDate} />
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('cursor.injectToCursor', '切换到 Cursor')}>
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
      const authIdText = (account.auth_id || '').trim();
      const maskedAuthIdText = authIdText ? maskAccountText(authIdText) : '--';
      const planLabel = resolvePlanLabel(account);
      const { quotaItems: [primaryQuota, ...otherQuotas] } = buildCursorAccountPresentation(account, t);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 3);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isCurrent = currentAccountId === account.id;
      const isBanned = isCursorAccountBanned(account);
      const quotaError = account.quota_query_last_error?.trim();
      const hasStatusError = (account.status || '').toLowerCase() === 'error';
      const statusReason = account.status_reason ?? null;
      const bannedTitle = statusReason || t('accounts.status.forbidden_tooltip');
      const errorTitle = statusReason || t('accounts.status.refreshFailed');

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
              {quotaError && (
                <div className="account-sub-line">
                  <span className="status-pill warning" title={quotaError}>
                    <CircleAlert size={12} />
                    {t('common.shared.quota.queryFailed', '配额查询失败')}
                  </span>
                </div>
              )}
              <div className="account-sub-line">
                <span className="kiro-table-subline">Auth ID: {maskedAuthIdText}</span>
              </div>
              {accountTags.length > 0 && (
                <div className="account-tags-inline">
                  {visibleTags.map((tag, idx) => (<span key={`${account.id}-inline-${tag}-${idx}`} className="tag-pill">{tag}</span>))}
                  {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                </div>
              )}
            </div>
          </td>
          <td><span className={`tier-badge ${resolvePlanBadgeClass(account)}`}>{planLabel}</span></td>
          <td>{primaryQuota && renderQuotaMetric(primaryQuota, true)}</td>
          <td>{otherQuotas.map((item) => renderQuotaMetric(item, true))}</td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button className="action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('cursor.injectToCursor', '切换到 Cursor')}>
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
    <div className="ghcp-accounts-page cursor-accounts-page">
      <CursorOverviewTabsHeader active={activeTab} onTabChange={setActiveTab} />
      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)} aria-expanded={!isFlowNoticeCollapsed}>
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('cursor.flowNotice.title', 'Cursor 账号管理说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t('cursor.flowNotice.desc', 'Manage Cursor accounts by importing from local Cursor installation or pasting JWT tokens. Data is processed locally only.')}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('cursor.flowNotice.reason', 'Permission scope: read Cursor local auth storage for account import and token injection.')}</li>
              <li>{t('cursor.flowNotice.storage', 'Data scope: only Cursor auth-session related entries are read/updated; no key/token is uploaded.')}</li>
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
            allLabel={t('common.shared.filter.all', '全部 ({{count}})', { count: tierSummary.all })}
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
                        <button type="button" className="tag-filter-delete" onClick={(e) => { e.preventDefault(); e.stopPropagation(); requestDeleteTag(tag); }}
                          aria-label={t('accounts.deleteTagAria', { tag, defaultValue: '删除标签 {{tag}}' })}>
                          <X size={12} />
                        </button>
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
              { value: 'credits', label: t('common.shared.sort.credits', '按剩余 Credits') },
              { value: 'plan_end', label: t('common.shared.sort.planEnd', '按配额周期结束时间') },
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
          <button className="btn btn-primary icon-only" onClick={() => openAddModal('oauth')} title={t('common.shared.addAccount', '添加账号')} aria-label={t('common.shared.addAccount', '添加账号')}><Plus size={14} /></button>
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
          {quotaFailedAccountIds.length > 0 && (
            <button
              type="button"
              className="btn btn-secondary icon-only"
              onClick={handleOpenQuotaFailures}
              disabled={retryingQuotaFailures}
              title={t('cursor.filters.quotaFailed', { count: quotaFailedAccountIds.length })}
              aria-label={t('cursor.filters.quotaFailed', { count: quotaFailedAccountIds.length })}
            >
              <CircleAlert size={14} />
            </button>
          )}
          <QuickSettingsPopover type="cursor" />
        </div>
      </div>

      {(accounts.length > 0 || grouping.groups.length > 0) && (
        <AccountSelectionToolbar
          selectedCount={selected.size}
          allSelected={isAllPaginatedSelected}
          disabled={paginatedIds.length === 0}
          onToggleSelectAll={() => toggleSelectAll(paginatedIds)}
          onClearSelection={() => toggleSelectAll(Array.from(selected))}
          grouping={grouping}
          accounts={accounts}
          selectedIds={Array.from(selected)}
          actions={(
            <button
              className="btn btn-danger icon-only"
              onClick={handleBatchDelete}
              title={`${t('common.delete', '删除')} (${selected.size})`}
              aria-label={`${t('common.delete', '删除')} (${selected.size})`}
            >
              <Trash2 size={14} />
            </button>
          )}
        />
      )}

      {loading && accounts.length === 0 ? (
        <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <Globe size={48} />
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t('cursor.empty.description', '点击"添加账号"开始管理您的 Cursor 账号')}</p>
          <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
            <button className="btn btn-primary" onClick={() => openAddModal('oauth')}>
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
                <th>{t('common.shared.quota.usage')}</th>
                <th>{t('cursor.quota.details')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
              </tr>
            </thead>
            <tbody>
              {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                <Fragment key={groupKey}>
                  <tr className="tag-group-row">
                    <td colSpan={6}>
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
                <th>{t('common.shared.quota.usage')}</th>
                <th>{t('cursor.quota.details')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
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
        <div className="modal-overlay">
          <div className="modal-content ghcp-add-modal platform-account-add-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('cursor.addModal.title', '添加 Cursor 账号')}</h2>
              <button className="modal-close" onClick={closeAddModal} aria-label={t('common.close', '关闭')}><X /></button>
            </div>

            <div className="modal-tabs">
              <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}><Globe size={14} />{t('common.shared.addModal.oauth', '授权登录')}</button>
              <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}><KeyRound size={14} />{t('common.shared.addModal.token', 'Token / JSON')}</button>
              <button className={`modal-tab ${addTab === 'import' ? 'active' : ''}`} onClick={() => openAddModal('import')}><Database size={14} />{t('common.shared.addModal.import', '本地导入')}</button>
            </div>

            <div className="modal-body">
              <MfaQuickCodeSelect />
              {addTab === 'oauth' && (
                <div className="add-section">
                  <p className="section-desc">{t('cursor.oauth.desc', '点击下方按钮，在浏览器中完成 Cursor 授权登录。')}</p>

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
                      <div className="oauth-url-box">
                        <input type="text" value={oauthUrl} readOnly />
                        <button onClick={handleCopyOauthUrl}>
                          {oauthUrlCopied ? <Check size={16} /> : <Copy size={16} />}
                        </button>
                      </div>
                      {!oauthUrl.includes('user_code=') && oauthUserCode && (
                        <div className="oauth-url-box">
                          <input type="text" value={oauthUserCode} readOnly />
                          <button onClick={handleCopyOauthUserCode}>
                            {oauthUserCodeCopied ? <Check size={16} /> : <Copy size={16} />}
                          </button>
                        </div>
                      )}
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
                          {oauthTimedOut && (
                            <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>
                              {t('common.shared.oauth.timeoutRetry', '刷新授权链接')}
                            </button>
                          )}
                        </div>
                      )}
                      <p className="oauth-hint">
                        {t('common.shared.oauth.hint', 'Once authorized, this window will update automatically')}
                      </p>
                    </div>
                  ) : (
                    <div className="oauth-loading">
                      <RefreshCw size={24} className="loading-spinner" />
                      <span>{t('common.shared.oauth.preparing', '正在准备授权信息...')}</span>
                    </div>
                  )}
                </div>
              )}

              {addTab === 'token' && (
                <div className="add-section">
                  <p className="section-desc">{t('cursor.token.desc')}</p>
                  <details className="token-format-collapse">
                    <summary className="token-format-collapse-summary">{t('cursor.token.formatHint', '必填字段与示例（点击展开）')}</summary>
                    <div className="token-format">
                      <p className="token-format-required">{t('cursor.token.formatRequired')}</p>
                      <div className="token-format-group">
                        <div className="token-format-label">{t('cursor.token.singleExample', '单条示例（JWT）')}</div>
                        <pre className="token-format-code">{CURSOR_TOKEN_SINGLE_EXAMPLE}</pre>
                      </div>
                      <div className="token-format-group">
                        <div className="token-format-label">{t('cursor.token.batchExample', '批量示例（JSON）')}</div>
                        <pre className="token-format-code">{CURSOR_TOKEN_BATCH_EXAMPLE}</pre>
                      </div>
                    </div>
                  </details>
                  <textarea className="token-input" value={tokenInput} onChange={(e) => setTokenInput(e.target.value)} placeholder={t('common.shared.token.placeholder', '粘贴 Token 或 JSON...')} />
                  <button className="btn btn-primary btn-full" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                    {t('common.shared.token.import', 'Import')}
                  </button>
                </div>
              )}

              {addTab === 'import' && (
                <div className="add-section">
                  <p className="section-desc">{t('cursor.import.localDesc', '支持从本机 Cursor 客户端或 JSON 文件导入账号数据。')}</p>
                  <button className="btn btn-secondary btn-full" onClick={() => handleImportFromLocal?.()} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}
                    {t('cursor.import.localClient', '从本机 Cursor 导入')}
                  </button>
                  <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>{t('common.shared.import.orJson', '或从 JSON 文件导入')}</div>
                  <input ref={importFileInputRef} type="file" accept="application/json" style={{ display: 'none' }}
                    onChange={(e) => { const file = e.target.files?.[0]; e.target.value = ''; if (!file) return; void handleImportJsonFile(file); }} />
                  <button className="btn btn-primary btn-full" onClick={handlePickImportFile} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}
                    {t('common.shared.import.pickFile', '选择 JSON 文件导入')}
                  </button>
                </div>
              )}

              {addStatus !== 'idle' && addStatus !== 'loading' && (
                <div className={`add-status ${addStatus}`}>
                  {addStatus === 'success' ? <Check size={16} /> : <CircleAlert size={16} />}
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
        <div className="modal-overlay">
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{quotaFailureConfirm ? t('common.shared.quota.queryFailed') : t('common.confirm')}</h2>
              <button className="modal-close" onClick={() => setDeleteConfirm(null)} disabled={deleting} aria-label={t('common.close', '关闭')}><X /></button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
              {quotaFailureConfirm && <ModalErrorMessage message={quotaRetryError} scrollKey={quotaRetryErrorScrollKey} />}
              <p>{deleteConfirm.message}</p>
            </div>
            <div className="modal-footer" style={quotaFailureConfirm ? { flexWrap: 'wrap' } : undefined}>
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>{t('common.cancel')}</button>
              {quotaFailureConfirm && (
                <button type="button" className="btn btn-primary" onClick={() => void handleRetryQuotaFailures()} disabled={deleting || retryingQuotaFailures} aria-busy={retryingQuotaFailures}>
                  <RefreshCw size={14} className={retryingQuotaFailures ? 'loading-spinner' : ''} />
                  {t('common.shared.refreshQuota')}
                </button>
              )}
              <button className="btn btn-danger" onClick={() => {
                setQuotaRetryError(null);
                void confirmDelete();
              }} disabled={deleting || (quotaFailureConfirm && retryingQuotaFailures)}>
                {quotaFailureConfirm ? t('common.delete') : t('common.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div className="modal-overlay">
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button className="modal-close" onClick={() => !deletingTag && setTagDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} />
              <p>{t('accounts.confirmDeleteTag', 'Delete tag "{{tag}}"? This tag will be removed from {{count}} accounts.', { tag: tagDeleteConfirm.tag, count: tagDeleteConfirm.count })}</p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setTagDeleteConfirm(null)} disabled={deletingTag}>{t('common.cancel')}</button>
              <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>{deletingTag ? t('common.processing', '处理中...') : t('common.confirm')}</button>
            </div>
          </div>
        </div>
      )}

      <TagEditModal
        isOpen={!!showTagModal}
        initialTags={accounts.find((a) => a.id === showTagModal)?.tags || []}
        availableTags={availableTags}
        onClose={() => setShowTagModal(null)}
        onSave={handleSaveTags}
      />
        </>
      )}

      {activeTab === 'instances' && (
        <CursorInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}
    </div>
  );
}
