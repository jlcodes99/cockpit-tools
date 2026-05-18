import { useState, useEffect, useMemo, useCallback, Fragment } from 'react';
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
  ChevronLeft,
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
import { useKiroAccountStore } from '../stores/useKiroAccountStore';
import * as kiroService from '../services/kiroService';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { PaginationControls } from '../components/PaginationControls';
import {
  getKiroCreditsSummary,
  hasKiroQuotaData,
} from '../types/kiro';
import { buildKiroAccountPresentation } from '../presentation/platformAccountPresentation';

import { KiroOverviewTabsHeader, KiroTab } from '../components/KiroOverviewTabsHeader';
import { KiroInstancesContent } from './KiroInstancesPage';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import type { KiroAccount } from '../types/kiro';
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

const KIRO_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.kiro.flow_notice_collapsed';
const KIRO_CURRENT_ACCOUNT_ID_KEY = 'agtools.kiro.current_account_id';
const KIRO_KNOWN_PLAN_FILTERS = ['FREE', 'INDIVIDUAL', 'PRO', 'BUSINESS', 'ENTERPRISE'] as const;
const KIRO_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('Kiro');
const FILTER_TYPES_FIELD = 'filter_types';
const KIRO_TOKEN_SINGLE_EXAMPLE = `{"access_token":"eyJ...","refresh_token":"rt_..."}`;
const KIRO_TOKEN_BATCH_EXAMPLE = `[
  {"access_token":"eyJ...","refresh_token":"rt_a..."},
  {"access_token":"eyJ...","refresh_token":"rt_b..."}
]`;

export function KiroAccountsPage() {
  const [activeTab, setActiveTab] = useState<KiroTab>('overview');
  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    readAccountsOverviewFilterPersistenceEnabled(KIRO_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterStringArray(KIRO_FILTER_PERSISTENCE_SCOPE, FILTER_TYPES_FIELD)
      : [],
  );
  const untaggedKey = '__untagged__';

  const store = useKiroAccountStore();

  const page = useProviderAccountsPage<KiroAccount>({
    platformKey: 'Kiro',
    oauthLogPrefix: 'KiroOAuth',
    flowNoticeCollapsedKey: KIRO_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: KIRO_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'kiro_accounts',
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
      startLogin: kiroService.startKiroOAuthLogin,
      completeLogin: kiroService.completeKiroOAuthLogin,
      cancelLogin: kiroService.cancelKiroOAuthLogin,
      submitCallbackUrl: kiroService.submitKiroOAuthCallbackUrl,
    },
    dataService: {
      importFromJson: kiroService.importKiroFromJson,
      importFromLocal: kiroService.importKiroFromLocal,
      addWithToken: kiroService.addKiroAccountWithToken,
      exportAccounts: kiroService.exportKiroAccounts,
      injectToVSCode: kiroService.injectKiroToVSCode,
    },
    getDisplayEmail: (account) =>
      account.email ?? account.id,
  });

  const {
    t, locale, privacyModeEnabled, togglePrivacyMode, maskAccountText,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    filterPersistenceEnabled, filterPersistenceScope,
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
    oauthUrl, oauthUrlCopied, oauthUserCode, oauthUserCodeCopied, oauthMeta,
    oauthPrepareError, oauthCompleteError, oauthPolling, oauthTimedOut,
    oauthManualCallbackInput, setOauthManualCallbackInput,
    oauthManualCallbackSubmitting, oauthManualCallbackError, oauthSupportsManualCallback,
    handleCopyOauthUrl, handleCopyOauthUserCode, handleRetryOauth, handleOpenOauthUrl,
    handleSubmitOauthCallbackUrl,
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

  // ─── Platform-specific: Presentation & Credits ──────────────────────

  const creditsSummaryById = useMemo(() => {
    const map = new Map<string, ReturnType<typeof getKiroCreditsSummary>>();
    accounts.forEach((account) => {
      map.set(account.id, getKiroCreditsSummary(account));
    });
    return map;
  }, [accounts]);

  const resolveCreditsSummary = useCallback(
    (account: KiroAccount) =>
      creditsSummaryById.get(account.id) ?? getKiroCreditsSummary(account),
    [creditsSummaryById],
  );

  const accountPresentations = useMemo(() => {
    const map = new Map<string, ReturnType<typeof buildKiroAccountPresentation>>();
    accounts.forEach((account) => {
      map.set(account.id, buildKiroAccountPresentation(account, t));
    });
    return map;
  }, [accounts, t]);

  const resolvePresentation = useCallback(
    (account: KiroAccount) =>
      accountPresentations.get(account.id) ?? buildKiroAccountPresentation(account, t),
    [accountPresentations, t],
  );

  const resolvePlanKey = useCallback(
    (account: KiroAccount) => {
      const presentation = resolvePresentation(account);
      if (presentation.planClass && presentation.planClass !== 'unknown') {
        return presentation.planClass.toUpperCase();
      }
      const label = presentation.planLabel?.trim();
      return label ? label.toUpperCase() : 'UNKNOWN';
    },
    [resolvePresentation],
  );

  const resolvePlanLabel = useCallback(
    (account: KiroAccount, planKey: string) => {
      const label = resolvePresentation(account).planLabel?.trim();
      return label || planKey;
    },
    [resolvePresentation],
  );

  const isAbnormalAccount = useCallback(
    (account: KiroAccount) => {
      const presentation = resolvePresentation(account);
      return presentation.isBanned || presentation.hasStatusError;
    },
    [resolvePresentation],
  );

  const resolvePlanBadgeClass = useCallback(
    (account: KiroAccount) => resolvePresentation(account).planClass,
    [resolvePresentation],
  );

  const formatCreditsNumber = useCallback(
    (value: number | null | undefined) => {
      const n = typeof value === 'number' && Number.isFinite(value) ? value : 0;
      const hasDecimal = Math.abs(n - Math.trunc(n)) > 0.0001;
      return new Intl.NumberFormat(locale, {
        minimumFractionDigits: hasDecimal ? 2 : 0,
        maximumFractionDigits: 2,
      }).format(n);
    },
    [locale],
  );

  const resolvePromptMetrics = useCallback(
    (account: KiroAccount) =>
      resolvePresentation(account).quotaItems.find((item) => item.key === 'prompt') ?? null,
    [resolvePresentation],
  );

  const resolveAddOnMetrics = useCallback(
    (account: KiroAccount) =>
      resolvePresentation(account).quotaItems.find((item) => item.key === 'addon') ?? null,
    [resolvePresentation],
  );

  const resolveDisplayEmail = useCallback((account: KiroAccount) => {
    return resolvePresentation(account).displayName.trim();
  }, [resolvePresentation]);

  const resolveSingleExportBaseName = useCallback(
    (account: KiroAccount) => {
      const display = resolveDisplayEmail(account) || account.id;
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolveDisplayEmail],
  );

  const resolveDisplayUserId = useCallback((account: KiroAccount) => {
    return resolvePresentation(account).userIdText.trim();
  }, [resolvePresentation]);

  const resolveSignedInWithText = useCallback(
    (account: KiroAccount) => resolvePresentation(account).signedInWithText,
    [resolvePresentation],
  );

  const formatCycleDate = useCallback(
    (timestamp: number | null | undefined) => {
      if (!timestamp) return '';
      const d = new Date(timestamp * 1000);
      if (Number.isNaN(d.getTime())) return '';
      return d.toLocaleDateString(locale, { year: 'numeric', month: '2-digit', day: '2-digit' });
    },
    [locale],
  );

  const resolveCycleDisplay = useCallback(
    (credits: ReturnType<typeof getKiroCreditsSummary>) => {
      const end = credits.planEndsAt ?? null;
      const start = credits.planStartsAt ?? null;

      if (!end) {
        const summary = t('common.shared.credits.planEndsUnknown', '配额周期时间未知');
        return { summary, detail: '', title: summary };
      }

      const now = Math.floor(Date.now() / 1000);
      const secondsLeft = end - now;
      const summary =
        secondsLeft > 0 && secondsLeft < 86400
          ? t('common.shared.credits.planEndsInHours', {
              hours: Math.max(1, Math.floor(secondsLeft / 3600)),
              defaultValue: '配额周期剩余 {{hours}} 小时',
            })
          : t('common.shared.credits.planEndsIn', {
              days: secondsLeft <= 0 ? 0 : Math.floor(secondsLeft / 86400),
              defaultValue: '配额周期剩余 {{days}} 天',
            });

      const startText = formatCycleDate(start);
      const endText = formatCycleDate(end);
      let detail = '';
      if (startText && endText) {
        detail = t('common.shared.credits.periodRange', {
          start: startText, end: endText, defaultValue: '周期：{{start}} - {{end}}',
        });
      } else if (endText) {
        detail = t('common.shared.credits.periodEndOnly', {
          end: endText, defaultValue: '周期结束：{{end}}',
        });
      }

      const title = detail ? `${summary} · ${detail}` : summary;
      return { summary, detail, title };
    },
    [formatCycleDate, t],
  );

  const formatUsedLine = useCallback(
    (used: number | null | undefined, total: number | null | undefined) =>
      t('common.shared.credits.usedLine', {
        used: formatCreditsNumber(used), total: formatCreditsNumber(total),
        defaultValue: '{{used}} / {{total}} used',
      }),
    [formatCreditsNumber, t],
  );

  const formatLeftLine = useCallback(
    (left: number | null | undefined) =>
      t('common.shared.credits.leftInline', {
        left: formatCreditsNumber(left), defaultValue: '{{left}} left',
      }),
    [formatCreditsNumber, t],
  );

  const resolveBonusExpiryValue = useCallback(
    (account: KiroAccount) => resolvePresentation(account).addOnExpiryText,
    [resolvePresentation],
  );

  const shouldShowAddOnCredits = useCallback(
    (account: KiroAccount) => resolveAddOnMetrics(account) != null,
    [resolveAddOnMetrics],
  );

  // ─── Platform-specific: Dynamic tier filter ─────────────────────────

  const tierSummary = useMemo(() => {
    const knownCounts = { FREE: 0, INDIVIDUAL: 0, PRO: 0, BUSINESS: 0, ENTERPRISE: 0 };
    const dynamicCounts = new Map<string, number>();
    const displayLabels = new Map<string, string>();

    accounts.forEach((account) => {
      const tier = resolvePlanKey(account);
      dynamicCounts.set(tier, (dynamicCounts.get(tier) ?? 0) + 1);
      if (tier in knownCounts) {
        knownCounts[tier as keyof typeof knownCounts] += 1;
      }
      if (!displayLabels.has(tier)) {
        displayLabels.set(tier, resolvePlanLabel(account, tier));
      }
    });
    const validCount = accounts.reduce(
      (count, account) => (isAbnormalAccount(account) ? count : count + 1),
      0,
    );

    const extraKeys = Array.from(dynamicCounts.keys())
      .filter((tier) => !(KIRO_KNOWN_PLAN_FILTERS as readonly string[]).includes(tier))
      .sort((a, b) => a.localeCompare(b));

    return { all: accounts.length, validCount, knownCounts, dynamicCounts, extraKeys, displayLabels };
  }, [accounts, isAbnormalAccount, resolvePlanKey, resolvePlanLabel]);

  useEffect(() => {
    setFilterTypes((prev) => {
      const next = prev.filter(
        (value) => value === VALID_ACCOUNTS_FILTER_VALUE || tierSummary.dynamicCounts.has(value),
      );
      return next.length === prev.length ? prev : next;
    });
  }, [tierSummary.dynamicCounts]);

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
      { value: 'INDIVIDUAL', label: resolveFilterLabel('INDIVIDUAL', tierSummary.knownCounts.INDIVIDUAL) },
      { value: 'PRO', label: resolveFilterLabel('PRO', tierSummary.knownCounts.PRO) },
      { value: 'BUSINESS', label: resolveFilterLabel('BUSINESS', tierSummary.knownCounts.BUSINESS) },
      { value: 'ENTERPRISE', label: resolveFilterLabel('ENTERPRISE', tierSummary.knownCounts.ENTERPRISE) },
    ];
    tierSummary.extraKeys.forEach((planKey) => {
      options.push({
        value: planKey,
        label: resolveFilterLabel(planKey, tierSummary.dynamicCounts.get(planKey) ?? 0),
      });
    });
    options.push(buildValidAccountsFilterOption(t, tierSummary.validCount));
    return options;
  }, [resolveFilterLabel, t, tierSummary.dynamicCounts, tierSummary.extraKeys, tierSummary.knownCounts.BUSINESS, tierSummary.knownCounts.ENTERPRISE, tierSummary.knownCounts.FREE, tierSummary.knownCounts.INDIVIDUAL, tierSummary.knownCounts.PRO, tierSummary.validCount]);

  // ─── Filtering & Sorting ────────────────────────────────────────────
  const compareAccountsBySort = useCallback((a: KiroAccount, b: KiroAccount) => {
    const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (sortBy === 'created_at') {
      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    }
    if (sortBy === 'plan_end') {
      const aReset = resolveCreditsSummary(a).planEndsAt ?? null;
      const bReset = resolveCreditsSummary(b).planEndsAt ?? null;
      if (aReset == null && bReset == null) return 0;
      if (aReset == null) return 1;
      if (bReset == null) return -1;
      const diff = bReset - aReset;
      return sortDirection === 'desc' ? diff : -diff;
    }
    const aValue = resolveCreditsSummary(a).creditsLeft ?? -1;
    const bValue = resolveCreditsSummary(b).creditsLeft ?? -1;
    const diff = bValue - aValue;
    return sortDirection === 'desc' ? diff : -diff;
  }, [currentAccountId, resolveCreditsSummary, sortBy, sortDirection]);

  const sortedAccountsForInstances = useMemo(
    () => [...accounts].sort(compareAccountsBySort),
    [accounts, compareAccountsBySort],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];

    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((account) => {
        const presentation = resolvePresentation(account);
        const haystacks = [
          presentation.displayName, presentation.userIdText,
          presentation.signedInWithText, account.id,
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
        result = result.filter((account) => selectedTypes.has(resolvePlanKey(account)));
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
  }, [accounts, compareAccountsBySort, filterTypes, isAbnormalAccount, normalizeTag, resolvePlanKey, resolvePresentation, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('Kiro'),
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

  // ─── Render helpers ──────────────────────────────────────────────────

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const presentation = resolvePresentation(account);
      const displayEmail = resolveDisplayEmail(account);
      const displayUserId = resolveDisplayUserId(account);
      const emailText = displayEmail || displayUserId || account.id;
      const signedInWithText = resolveSignedInWithText(account);
      const userIdText = displayUserId || account.id;
      const credits = resolveCreditsSummary(account);
      const cycleDisplay = resolveCycleDisplay(credits);
      const promptMetrics = resolvePromptMetrics(account);
      const addOnMetrics = resolveAddOnMetrics(account);
      const planKey = resolvePlanKey(account);
      const planLabel = resolvePlanLabel(account, planKey);
      const bonusExpiryValue = resolveBonusExpiryValue(account);
      const showAddOnCredits = shouldShowAddOnCredits(account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      const quotaError = account.quota_query_last_error?.trim();
      const hasQuotaData = hasKiroQuotaData(account);
      const statusReason = presentation.accountStatusReason;
      const isBanned = presentation.isBanned;
      const hasStatusError = presentation.hasStatusError;
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
            <span className="kiro-table-subline" title={`${signedInWithText} | ${t('kiro.account.userId', 'User ID')}: ${maskAccountText(userIdText)}`}>
              {signedInWithText} | {t('kiro.account.userId', 'User ID')}: {maskAccountText(userIdText)}
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
            {hasQuotaData ? (
              <>
                <div className="quota-item windsurf-credit-item">
                  <div className="quota-header">
                    <span className="quota-label">{promptMetrics?.label ?? t('common.shared.columns.promptCredits', 'User Prompt credits')}</span>
                    <span className={`quota-pct ${promptMetrics?.quotaClass ?? 'high'}`}>{promptMetrics?.valueText ?? '0%'}</span>
                  </div>
                  <div className="windsurf-credit-meta-row">
                    <span className="windsurf-credit-used">{formatUsedLine(promptMetrics?.used, promptMetrics?.total)}</span>
                    <span className="windsurf-credit-left">{formatLeftLine(promptMetrics?.left)}</span>
                  </div>
                  <div className="quota-bar-track">
                    <div className={`quota-bar ${promptMetrics?.quotaClass ?? 'high'}`} style={{ width: `${promptMetrics?.percentage ?? 0}%` }} />
                  </div>
                </div>

                {showAddOnCredits ? (
                  <div className="quota-item windsurf-credit-item">
                    <div className="quota-header">
                      <span className="quota-label">{addOnMetrics?.label ?? t('common.shared.columns.addOnPromptCredits', 'Add-on prompt credits')}</span>
                      <span className={`quota-pct ${addOnMetrics?.quotaClass ?? 'high'}`}>{addOnMetrics?.valueText ?? '0%'}</span>
                    </div>
                    <div className="windsurf-credit-meta-row">
                      <span className="windsurf-credit-used">{formatUsedLine(addOnMetrics?.used, addOnMetrics?.total)}</span>
                      <span className="windsurf-credit-left">{formatLeftLine(addOnMetrics?.left)}</span>
                    </div>
                    <div className="windsurf-credit-meta-row expiry">
                      <span className="windsurf-credit-expiry">{t('kiro.columns.expiry', 'Expiry')}: {bonusExpiryValue}</span>
                    </div>
                    <div className="quota-bar-track">
                      <div className={`quota-bar ${addOnMetrics?.quotaClass ?? 'high'}`} style={{ width: `${addOnMetrics?.percentage ?? 0}%` }} />
                    </div>
                  </div>
                ) : null}
              </>
            ) : (
              <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
            )}
          </div>

          <div className="windsurf-plan-cycle" title={cycleDisplay.title}>
            <span className="windsurf-plan-cycle-summary">{cycleDisplay.summary}</span>
            {cycleDisplay.detail ? (<span className="windsurf-plan-cycle-detail">{cycleDisplay.detail}</span>) : null}
          </div>

          <div className="card-footer">
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('kiro.injectToVSCode', '切换到 Kiro')}>
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
      const presentation = resolvePresentation(account);
      const displayEmail = resolveDisplayEmail(account);
      const displayUserId = resolveDisplayUserId(account);
      const emailText = displayEmail || displayUserId || account.id;
      const signedInWithText = resolveSignedInWithText(account);
      const userIdText = displayUserId || account.id;
      const credits = resolveCreditsSummary(account);
      const cycleDisplay = resolveCycleDisplay(credits);
      const promptMetrics = resolvePromptMetrics(account);
      const addOnMetrics = resolveAddOnMetrics(account);
      const planKey = resolvePlanKey(account);
      const planLabel = resolvePlanLabel(account, planKey);
      const bonusExpiryValue = resolveBonusExpiryValue(account);
      const showAddOnCredits = shouldShowAddOnCredits(account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 3);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isCurrent = currentAccountId === account.id;
      const quotaError = account.quota_query_last_error?.trim();
      const hasQuotaData = hasKiroQuotaData(account);
      const statusReason = presentation.accountStatusReason;
      const isBanned = presentation.isBanned;
      const hasStatusError = presentation.hasStatusError;
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
                <span className="kiro-table-subline">{signedInWithText} | {t('kiro.account.userId', 'User ID')}: {maskAccountText(userIdText)}</span>
              </div>
              <div className="account-sub-line windsurf-cycle-line" title={cycleDisplay.title}>
                <span className="windsurf-cycle-text">{cycleDisplay.summary}</span>
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
          <td>
            {hasQuotaData ? (
              <div className="quota-item windsurf-table-credit-item">
                <div className="quota-header">
                  <span className="quota-name">{promptMetrics?.label ?? t('common.shared.columns.promptCredits', 'User Prompt credits')}</span>
                  <span className={`quota-value ${promptMetrics?.quotaClass ?? 'high'}`}>{promptMetrics?.valueText ?? '0%'}</span>
                </div>
                <div className="windsurf-credit-meta-row table">
                  <span className="windsurf-credit-used">{formatUsedLine(promptMetrics?.used, promptMetrics?.total)}</span>
                  <span className="windsurf-credit-left">{formatLeftLine(promptMetrics?.left)}</span>
                </div>
                <div className="quota-progress-track">
                  <div className={`quota-progress-bar ${promptMetrics?.quotaClass ?? 'high'}`} style={{ width: `${promptMetrics?.percentage ?? 0}%` }} />
                </div>
              </div>
            ) : (
              <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
            )}
          </td>
          <td>
            {hasQuotaData && showAddOnCredits ? (
              <div className="quota-item windsurf-table-credit-item">
                <div className="quota-header">
                  <span className="quota-name">{addOnMetrics?.label ?? t('common.shared.columns.addOnPromptCredits', 'Add-on prompt credits')}</span>
                  <span className={`quota-value ${addOnMetrics?.quotaClass ?? 'high'}`}>{addOnMetrics?.valueText ?? '0%'}</span>
                </div>
                <div className="windsurf-credit-meta-row table">
                  <span className="windsurf-credit-used">{formatUsedLine(addOnMetrics?.used, addOnMetrics?.total)}</span>
                  <span className="windsurf-credit-left">{formatLeftLine(addOnMetrics?.left)}</span>
                </div>
                <div className="windsurf-credit-meta-row table expiry">
                  <span className="windsurf-credit-expiry">{t('kiro.columns.expiry', 'Expiry')}: {bonusExpiryValue}</span>
                </div>
                <div className="quota-progress-track">
                  <div className={`quota-progress-bar ${addOnMetrics?.quotaClass ?? 'high'}`} style={{ width: `${addOnMetrics?.percentage ?? 0}%` }} />
                </div>
              </div>
            ) : (
              <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
            )}
          </td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button className="action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('kiro.injectToVSCode', '切换到 Kiro')}>
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
    <div className="ghcp-accounts-page kiro-accounts-page">
      <KiroOverviewTabsHeader active={activeTab} onTabChange={setActiveTab} />
      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)} aria-expanded={!isFlowNoticeCollapsed}>
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('kiro.flowNotice.title', 'Kiro 账号管理说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t('kiro.flowNotice.desc', 'Switching accounts requires reading VS Code local auth storage and using the system credential service for decrypt/re-encrypt. Data is processed locally only.')}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('kiro.flowNotice.reason', 'Permission scope: read VS Code auth database (state.vscdb) and call system credential capability (Windows DPAPI / macOS Keychain / Linux Secret Service) for decrypt/write-back.')}</li>
              <li>{t('kiro.flowNotice.storage', 'Data scope: only Kiro auth-session related entries are read/updated; system secrets are not modified and no key/token is uploaded.')}</li>
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
          <button className="btn btn-secondary icon-only" onClick={() => openAddModal('token')} disabled={importing} title={t('common.shared.import.label', '导入')} aria-label={t('common.shared.import.label', '导入')}><Download size={14} /></button>
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
            <QuickSettingsPopover type="kiro" />
        </div>
      </div>

      {loading && accounts.length === 0 ? (
        <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <Globe size={48} />
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t('kiro.empty.description', '点击"添加账号"开始管理您的 Kiro 账号')}</p>
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
                <th>{t('common.shared.columns.promptCredits', 'User Prompt credits')}</th>
                <th>{t('common.shared.columns.addOnPromptCredits', 'Add-on prompt credits')}</th>
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
                <th>{t('common.shared.columns.promptCredits', 'User Prompt credits')}</th>
                <th>{t('common.shared.columns.addOnPromptCredits', 'Add-on prompt credits')}</th>
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
        <div className="modal-overlay" onClick={closeAddModal}>
          <div className="modal-content ghcp-add-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={closeAddModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{t('kiro.addModal.title', '添加 Kiro 账号')}</h2>
              <button className="modal-close" onClick={closeAddModal} aria-label={t('common.close', '关闭')}><X /></button>
            </div>

            <div className="modal-tabs">
              <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}><Globe size={14} />{t('common.shared.addModal.oauth', 'OAuth Authorization')}</button>
              <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}><KeyRound size={14} />Token / JSON</button>
              <button className={`modal-tab ${addTab === 'import' ? 'active' : ''}`} onClick={() => openAddModal('import')}><Database size={14} />{t('common.shared.addModal.import', '本地导入')}</button>
            </div>

            <div className="modal-body">
              {addTab === 'oauth' && (
                <div className="add-section">
                  <p className="section-desc">{t('kiro.oauth.desc', '点击下方按钮，在浏览器中完成 Kiro OAuth 授权。')}</p>

                  {oauthPrepareError ? (
                    <div className="add-status error">
                      <CircleAlert size={16} /><span>{oauthPrepareError}</span>
                      <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>{t('common.shared.oauth.retry', '重新生成授权信息')}</button>
                    </div>
                  ) : oauthUrl ? (
                    <div className="oauth-url-section">
                      <div className="oauth-link">
                        <label>{t('accounts.oauth.linkLabel', '授权链接')}</label>
                        <div className="oauth-url-box">
                          <input type="text" value={oauthUrl} readOnly />
                          <button onClick={handleCopyOauthUrl}>{oauthUrlCopied ? <Check size={16} /> : <Copy size={16} />}</button>
                        </div>
                      </div>
                      {!oauthUrl.includes('user_code=') && oauthUserCode && (
                        <div className="oauth-url-box">
                          <input type="text" value={oauthUserCode} readOnly />
                          <button onClick={handleCopyOauthUserCode}>{oauthUserCodeCopied ? <Check size={16} /> : <Copy size={16} />}</button>
                        </div>
                      )}
                      {oauthMeta && (
                        <p className="oauth-hint">{t('common.shared.oauth.meta', '授权有效期：{{expires}}s；轮询间隔：{{interval}}s', { expires: oauthMeta.expiresIn, interval: oauthMeta.intervalSeconds })}</p>
                      )}
                      <button className="btn btn-primary btn-full" onClick={handleOpenOauthUrl}><Globe size={16} />{t('common.shared.oauth.openBrowser', '在浏览器中打开')}</button>
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
                        <div className="add-status error"><CircleAlert size={16} /><span>{oauthManualCallbackError}</span></div>
                      )}
                      {oauthPolling && (
                        <div className="add-status loading"><RefreshCw size={16} className="loading-spinner" /><span>{t('common.shared.oauth.waiting', '等待授权完成...')}</span></div>
                      )}
                      {oauthCompleteError && (
                        <div className="add-status error">
                          <CircleAlert size={16} /><span>{oauthCompleteError}</span>
                          {oauthTimedOut && (<button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>{t('common.shared.oauth.timeoutRetry', '刷新授权链接')}</button>)}
                        </div>
                      )}
                      <p className="oauth-hint">{t('common.shared.oauth.hint', 'Once authorized, this window will update automatically')}</p>
                    </div>
                  ) : (
                    <div className="oauth-loading"><RefreshCw size={24} className="loading-spinner" /><span>{t('common.shared.oauth.preparing', '正在准备授权信息...')}</span></div>
                  )}
                </div>
              )}

              {addTab === 'token' && (
                <div className="add-section">
                  <p className="section-desc">{t('kiro.token.desc', '粘贴您的 Kiro Access Token 或导出的 JSON 数据。')}</p>
                  <details className="token-format-collapse">
                    <summary className="token-format-collapse-summary">必填字段与示例（点击展开）</summary>
                    <div className="token-format">
                      <p className="token-format-required">必填字段：JSON 建议使用 access_token + refresh_token（accessToken/refreshToken 仅兼容）</p>
                      <div className="token-format-group">
                        <div className="token-format-label">单条示例（JSON）</div>
                        <pre className="token-format-code">{KIRO_TOKEN_SINGLE_EXAMPLE}</pre>
                      </div>
                      <div className="token-format-group">
                        <div className="token-format-label">批量示例（JSON）</div>
                        <pre className="token-format-code">{KIRO_TOKEN_BATCH_EXAMPLE}</pre>
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
                  <p className="section-desc">{t('kiro.import.localDesc', '支持从本机 Kiro 客户端或 JSON 文件导入账号数据。')}</p>
                  <button className="btn btn-secondary btn-full" onClick={() => handleImportFromLocal?.()} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}
                    {t('kiro.import.localClient', '从本机 Kiro 导入')}
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
        <div className="modal-overlay" onClick={() => !deleting && setDeleteConfirm(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button className="modal-close" onClick={() => !deleting && setDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
              <p>{deleteConfirm.message}</p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>{t('common.cancel')}</button>
              <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>{t('common.confirm')}</button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div className="modal-overlay" onClick={() => !deletingTag && setTagDeleteConfirm(null)}>
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
        <KiroInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}
    </div>
  );
}
