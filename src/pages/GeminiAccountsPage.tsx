import { useEffect, useMemo, useCallback, useState, Fragment } from 'react';
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
import { useGeminiAccountStore } from '../stores/useGeminiAccountStore';
import * as geminiService from '../services/geminiService';
import * as geminiInstanceService from '../services/geminiInstanceService';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import {
  getGeminiPlanBadge,
  getGeminiPlanBadgeClass,
  getGeminiPlanDisplayName,
  getGeminiAccountDisplayEmail,
  getGeminiTierQuotaSummary,
  isGeminiAccountBanned,
} from '../types/gemini';
import type { GeminiAccount } from '../types/gemini';

import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { GeminiOverviewTabsHeader, GeminiTab } from '../components/GeminiOverviewTabsHeader';
import { GeminiInstancesContent } from './GeminiInstancesPage';

const CURSOR_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.gemini.flow_notice_collapsed';
const CURSOR_CURRENT_ACCOUNT_ID_KEY = 'agtools.gemini.current_account_id';
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

interface GeminiLaunchModalState {
  instanceId: string;
  instanceName: string;
  accountEmail: string;
  launchCommand: string;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
}

function getGeminiQuotaClass(percentage: number): string {
  if (percentage >= 90) return 'critical';
  if (percentage >= 70) return 'warning';
  return 'high';
}

export function GeminiAccountsPage() {
  const [activeTab, setActiveTab] = useState<GeminiTab>('overview');
  const [launchModal, setLaunchModal] = useState<GeminiLaunchModalState | null>(null);
  const [filterTypes, setFilterTypes] = useState<string[]>([]);
  const untaggedKey = '__untagged__';

  const store = useGeminiAccountStore();

  const page = useProviderAccountsPage<GeminiAccount>({
    platformKey: 'Gemini',
    oauthLogPrefix: 'GeminiOAuth',
    flowNoticeCollapsedKey: CURSOR_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: CURSOR_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'gemini_accounts',
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
        const resp = await geminiService.startGeminiOAuthLogin();
        return {
          loginId: resp.loginId,
          verificationUri: resp.verificationUri,
          expiresIn: resp.expiresIn,
          intervalSeconds: resp.intervalSeconds,
          callbackUrl: resp.callbackUrl ?? null,
        };
      },
      completeLogin: (loginId: string) => geminiService.completeGeminiOAuthLogin(loginId),
      cancelLogin: (loginId?: string) => geminiService.cancelGeminiOAuthLogin(loginId),
      submitCallbackUrl: (loginId: string, callbackUrl: string) =>
        geminiService.submitGeminiOAuthCallbackUrl(loginId, callbackUrl),
    },
    dataService: {
      importFromJson: geminiService.importGeminiFromJson,
      importFromLocal: geminiService.importGeminiFromLocal,
      addWithToken: geminiService.addGeminiAccountWithToken,
      exportAccounts: geminiService.exportGeminiAccounts,
      injectToVSCode: geminiService.injectGeminiAccount,
    },
    getDisplayEmail: (account) => getGeminiAccountDisplayEmail(account),
    onInjectSuccess: async ({ accountId, account, displayEmail }) => {
      try {
        const launchInfo = await geminiInstanceService.getGeminiInstanceLaunchCommand('__default__');
        setLaunchModal({
          instanceId: launchInfo.instanceId || '__default__',
          instanceName: '__default__',
          accountEmail: account ? getGeminiAccountDisplayEmail(account) : displayEmail || accountId,
          launchCommand: launchInfo.launchCommand,
          copied: false,
          executing: false,
          executeMessage: null,
          executeError: null,
        });
      } catch (error) {
        console.error('[GeminiAccountsPage] load default launch command failed:', error);
      }
    },
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
    oauthManualCallbackInput, setOauthManualCallbackInput,
    oauthManualCallbackSubmitting, oauthManualCallbackError, oauthSupportsManualCallback,
    handleCopyOauthUrl, handleCopyOauthUserCode, handleRetryOauth, handleOpenOauthUrl,
    handleSubmitOauthCallbackUrl,
    isFlowNoticeCollapsed, setIsFlowNoticeCollapsed,
    currentAccountId,
    normalizeTag,
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

  const handleCopyLaunchCommand = useCallback(async () => {
    if (!launchModal) return;
    try {
      await navigator.clipboard.writeText(launchModal.launchCommand);
      setLaunchModal((prev) => (prev ? { ...prev, copied: true, executeError: null } : prev));
      window.setTimeout(() => {
        setLaunchModal((prev) => (prev ? { ...prev, copied: false } : prev));
      }, 1200);
    } catch {
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executeError: t('common.shared.export.copyFailed', '复制失败，请手动复制'),
            }
          : prev,
      );
    }
  }, [launchModal, t]);

  const handleExecuteInTerminal = useCallback(async () => {
    if (!launchModal || launchModal.executing) return;
    setLaunchModal((prev) =>
      prev ? { ...prev, executing: true, executeMessage: null, executeError: null } : prev,
    );
    try {
      const result = await geminiInstanceService.executeGeminiInstanceLaunchCommand(launchModal.instanceId);
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeMessage: result,
            }
          : prev,
      );
    } catch (error) {
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeError: String(error),
            }
          : prev,
      );
    }
  }, [launchModal]);

  // ─── Platform-specific: Plan resolution ────────────────────────────

  const resolvePlanKey = useCallback(
    (account: GeminiAccount) => getGeminiPlanBadge(account),
    [],
  );

  const resolvePlanLabel = useCallback(
    (account: GeminiAccount) => getGeminiPlanDisplayName(account),
    [],
  );

  const resolveDisplayEmail = useCallback(
    (account: GeminiAccount) => getGeminiAccountDisplayEmail(account),
    [],
  );

  const resolveSingleExportBaseName = useCallback(
    (account: GeminiAccount) => {
      const display = resolveDisplayEmail(account);
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolveDisplayEmail],
  );

  // ─── Platform-specific: Quota ──────────────────────────────────────

  const formatRelativeDuration = useCallback(
    (seconds: number) => {
      const safe = Math.max(0, Math.floor(seconds));
      const totalMinutes = Math.floor(safe / 60);
      if (totalMinutes < 1) {
        return t('common.shared.time.lessThanMinute', '<1分钟');
      }
      const days = Math.floor(totalMinutes / (60 * 24));
      const hours = Math.floor((totalMinutes % (60 * 24)) / 60);
      const minutes = totalMinutes % 60;
      if (days > 0 && hours > 0) {
        return t('common.shared.time.relativeDaysHours', '{{days}}天{{hours}}小时', { days, hours });
      }
      if (days > 0) {
        return t('common.shared.time.relativeDays', '{{days}}天', { days });
      }
      if (hours > 0 && minutes > 0) {
        return t('common.shared.time.relativeHoursMinutes', '{{hours}}小时{{minutes}}分钟', { hours, minutes });
      }
      if (hours > 0) {
        return t('common.shared.time.relativeHours', '{{hours}}小时', { hours });
      }
      return t('common.shared.time.relativeMinutes', '{{minutes}}分钟', { minutes });
    },
    [t],
  );

  const resolveUpdatedText = useCallback(
    (account: GeminiAccount) => {
      const updatedAt = account.last_used || account.created_at || 0;
      const secondsAgo = Math.floor(Date.now() / 1000) - updatedAt;
      return t('gemini.updated.label', 'Updated {{relative}} ago', {
        relative: formatRelativeDuration(secondsAgo),
      });
    },
    [formatRelativeDuration, t],
  );

  const resolveTierQuota = useCallback(
    (account: GeminiAccount, tier: 'pro' | 'flash') => {
      const tierSummary = getGeminiTierQuotaSummary(account);
      const target = tier === 'pro' ? tierSummary.pro : tierSummary.flash;
      const hasRemaining =
        typeof target.remainingPercent === 'number' && Number.isFinite(target.remainingPercent);
      const remaining = hasRemaining
        ? Math.max(0, Math.min(100, Math.round(target.remainingPercent!)))
        : 0;
      const usedPercent = 100 - remaining;
      const resetText =
        typeof target.resetAt === 'number' && Number.isFinite(target.resetAt)
          ? (() => {
              const secondsLeft = Math.floor(target.resetAt - Date.now() / 1000);
              if (secondsLeft <= 0) {
                return t('gemini.quota.resetsSoon', 'Resets soon');
              }
              return t('gemini.quota.resetsIn', 'Resets in {{relative}}', {
                relative: formatRelativeDuration(secondsLeft),
              });
            })()
          : t('gemini.quota.resetsUnknown', 'Reset time unknown');

      return {
        label: t(`gemini.quota.${tier}`, tier === 'pro' ? 'Pro' : 'Flash'),
        percentage: remaining,
        quotaClass: getGeminiQuotaClass(usedPercent),
        leftText: hasRemaining
          ? t('gemini.quota.left', '{{value}}% left', { value: remaining })
          : '--',
        resetText,
        hasRemaining,
      };
    },
    [formatRelativeDuration, t],
  );

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

    const extraKeys = Array.from(dynamicCounts.keys())
      .filter((tier) => !(CURSOR_KNOWN_PLAN_FILTERS as readonly string[]).includes(tier))
      .sort((a, b) => a.localeCompare(b));

    return { all: accounts.length, knownCounts, dynamicCounts, extraKeys, displayLabels };
  }, [accounts, resolvePlanKey, resolvePlanLabel]);

  useEffect(() => {
    setFilterTypes((prev) => {
      const next = prev.filter((value) => tierSummary.dynamicCounts.has(value));
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
    return options;
  }, [resolveFilterLabel, tierSummary.dynamicCounts, tierSummary.extraKeys, tierSummary.knownCounts.ENTERPRISE, tierSummary.knownCounts.FREE, tierSummary.knownCounts.FREE_TRIAL, tierSummary.knownCounts.PRO, tierSummary.knownCounts.PRO_PLUS, tierSummary.knownCounts.ULTRA]);

  // ─── Filtering & Sorting ──────────────────────────────────────────

  const compareAccountsBySort = useCallback((a: GeminiAccount, b: GeminiAccount) => {
    const getTierMetrics = (account: GeminiAccount) => {
      const summary = getGeminiTierQuotaSummary(account);
      const remainingCandidates = [summary.pro.remainingPercent, summary.flash.remainingPercent]
        .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
      const remaining = remainingCandidates.length > 0 ? Math.min(...remainingCandidates) : null;
      const resetCandidates = [summary.pro.resetAt, summary.flash.resetAt]
        .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
      const nearestReset = resetCandidates.length > 0 ? Math.min(...resetCandidates) : null;
      return { remaining, nearestReset };
    };

    if (sortBy === 'created_at') {
      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    }
    if (sortBy === 'plan_end') {
      const aReset = getTierMetrics(a).nearestReset;
      const bReset = getTierMetrics(b).nearestReset;
      if (aReset == null && bReset == null) return 0;
      if (aReset == null) return 1;
      if (bReset == null) return -1;
      const diff = bReset - aReset;
      return sortDirection === 'desc' ? diff : -diff;
    }
    const aValue = getTierMetrics(a).remaining ?? -1;
    const bValue = getTierMetrics(b).remaining ?? -1;
    const diff = bValue - aValue;
    return sortDirection === 'desc' ? diff : -diff;
  }, [sortBy, sortDirection]);

  const sortedAccountsForInstances = useMemo(
    () => [...accounts].sort(compareAccountsBySort),
    [accounts, compareAccountsBySort],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];

    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((account) => {
        const haystacks = [
          getGeminiAccountDisplayEmail(account),
          account.id,
          account.auth_id ?? '',
          account.membership_type ?? '',
          account.subscription_status ?? '',
        ];
        return haystacks.some((item) => item.toLowerCase().includes(query));
      });
    }

    if (filterTypes.length > 0) {
      const selectedTypes = new Set(filterTypes);
      result = result.filter((account) => selectedTypes.has(resolvePlanKey(account)));
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
  }, [accounts, compareAccountsBySort, filterTypes, normalizeTag, resolvePlanKey, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);

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
      if (aKey === untaggedKey) return 1;
      if (bKey === untaggedKey) return -1;
      return aKey.localeCompare(bKey);
    });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);

  const resolveGroupLabel = (groupKey: string) =>
    groupKey === untaggedKey ? t('accounts.defaultGroup', '默认分组') : groupKey;

  // ─── Render helpers ────────────────────────────────────────────────

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const displayEmail = resolveDisplayEmail(account);
      const emailText = displayEmail || account.id;
      const updatedText = resolveUpdatedText(account);
      const proQuota = resolveTierQuota(account, 'pro');
      const flashQuota = resolveTierQuota(account, 'flash');
      const planLabel = resolvePlanLabel(account);
      const planClass = getGeminiPlanBadgeClass(undefined, account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      const isBanned = isGeminiAccountBanned(account);
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
            {planLabel && planLabel !== 'UNKNOWN' && (
              <span className={`tier-badge ${planClass}`}>{planLabel}</span>
            )}
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
          </div>

          <div className="account-sub-line">
            <span className="kiro-table-subline">{updatedText}</span>
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
            {[proQuota, flashQuota].map((quota) => (
              <div className="quota-item windsurf-credit-item" key={`${account.id}-${quota.label}`}>
                <div className="quota-header">
                  <span className="quota-label">{quota.label}</span>
                </div>
                <div className="quota-bar-track">
                  <div className={`quota-bar ${quota.quotaClass}`} style={{ width: `${Math.min(quota.percentage, 100)}%` }} />
                </div>
                <div className="windsurf-credit-meta-row">
                  <span className="windsurf-credit-left">{quota.leftText}</span>
                  <span className="windsurf-credit-used">{quota.resetText}</span>
                </div>
              </div>
            ))}
          </div>

          <div className="card-footer">
            <span className="card-date">{updatedText}</span>
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('gemini.injectToGemini', '切换到 Gemini')}>
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
                title={t('common.shared.export', '导出')}
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
      const updatedText = resolveUpdatedText(account);
      const proQuota = resolveTierQuota(account, 'pro');
      const flashQuota = resolveTierQuota(account, 'flash');
      const planLabel = resolvePlanLabel(account);
      const planClass = getGeminiPlanBadgeClass(undefined, account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 3);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isCurrent = currentAccountId === account.id;
      const isBanned = isGeminiAccountBanned(account);
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
                {planLabel && planLabel !== 'UNKNOWN' && (
                  <span className={`tier-badge ${planClass}`}>{planLabel}</span>
                )}
                {isCurrent && <span className="mini-tag current">{t('accounts.status.current')}</span>}
              </div>
              {(hasStatusError || isBanned) && (
                <div className="account-sub-line">
                  {hasStatusError && (<span className="status-pill warning" title={errorTitle}><CircleAlert size={12} />{t('accounts.status.refreshFailed')}</span>)}
                  {isBanned && (<span className="status-pill forbidden" title={bannedTitle}><Lock size={12} />{t('accounts.status.forbidden')}</span>)}
                </div>
              )}
              <div className="account-sub-line">
                <span className="kiro-table-subline">{updatedText}</span>
              </div>
              {accountTags.length > 0 && (
                <div className="account-tags-inline">
                  {visibleTags.map((tag, idx) => (<span key={`${account.id}-inline-${tag}-${idx}`} className="tag-pill">{tag}</span>))}
                  {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                </div>
              )}
            </div>
          </td>
          <td>
            <div className="quota-item windsurf-table-credit-item">
              <div className="quota-header">
                <span className="quota-name">{proQuota.label}</span>
                <span className={`quota-value ${proQuota.quotaClass}`}>{proQuota.leftText}</span>
              </div>
              <div className="windsurf-credit-meta-row table">
                <span className="windsurf-credit-used">{proQuota.resetText}</span>
              </div>
              <div className="quota-progress-track">
                <div className={`quota-progress-bar ${proQuota.quotaClass}`} style={{ width: `${Math.min(proQuota.percentage, 100)}%` }} />
              </div>
            </div>
          </td>
          <td>
            <div className="quota-item windsurf-table-credit-item">
              <div className="quota-header">
                <span className="quota-name">{flashQuota.label}</span>
                <span className={`quota-value ${flashQuota.quotaClass}`}>{flashQuota.leftText}</span>
              </div>
              <div className="windsurf-credit-meta-row table">
                <span className="windsurf-credit-used">{flashQuota.resetText}</span>
              </div>
              <div className="quota-progress-track">
                <div className={`quota-progress-bar ${flashQuota.quotaClass}`} style={{ width: `${Math.min(flashQuota.percentage, 100)}%` }} />
              </div>
            </div>
          </td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button className="action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting || isBanned}
                title={isBanned ? t('accounts.status.forbidden_msg') : t('gemini.injectToGemini', '切换到 Gemini')}>
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
                title={t('common.shared.export', '导出')}
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
    <div className="ghcp-accounts-page gemini-accounts-page">
      <GeminiOverviewTabsHeader active={activeTab} onTabChange={setActiveTab} />
      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)} aria-expanded={!isFlowNoticeCollapsed}>
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('gemini.flowNotice.title', 'Gemini 账号管理说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t('gemini.flowNotice.desc', 'Manage Gemini accounts by importing from local Gemini installation or pasting JWT tokens. Data is processed locally only.')}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('gemini.flowNotice.reason', 'Permission scope: read Gemini local auth storage for account import and token injection.')}</li>
              <li>{t('gemini.flowNotice.storage', 'Data scope: only Gemini auth-session related entries are read/updated; no key/token is uploaded.')}</li>
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
              <div className="tag-filter-panel">
                {availableTags.length === 0 ? (
                  <div className="tag-filter-empty">{t('accounts.noAvailableTags', '暂无可用标签')}</div>
                ) : (
                  <div className="tag-filter-options">
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

          <div className="sort-select">
            <ArrowDownWideNarrow size={14} className="sort-icon" />
            <select value={sortBy} onChange={(e) => setSortBy(e.target.value)} aria-label={t('common.shared.sortLabel', '排序')}>
              <option value="created_at">{t('common.shared.sort.createdAt', '按创建时间')}</option>
              <option value="credits">{t('common.shared.sort.credits', '按剩余 Credits')}</option>
              <option value="plan_end">{t('common.shared.sort.planEnd', '按配额周期结束时间')}</option>
            </select>
          </div>

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
            title={exportSelectionCount > 0 ? `${t('common.shared.export', '导出')} (${exportSelectionCount})` : t('common.shared.export', '导出')}
            aria-label={exportSelectionCount > 0 ? `${t('common.shared.export', '导出')} (${exportSelectionCount})` : t('common.shared.export', '导出')}>
            <Upload size={14} />
          </button>
          {selected.size > 0 && (
            <button className="btn btn-danger icon-only" onClick={handleBatchDelete} title={`${t('common.delete', '删除')} (${selected.size})`} aria-label={`${t('common.delete', '删除')} (${selected.size})`}>
              <Trash2 size={14} />
            </button>
          )}
          <QuickSettingsPopover type="gemini" />
        </div>
      </div>

      {loading && accounts.length === 0 ? (
        <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <Globe size={48} />
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t('gemini.empty.description', '点击"添加账号"开始管理您的 Gemini 账号')}</p>
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
          {filteredAccounts.length > 0 && (
            <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '13px', color: 'var(--text-color)' }}>
                <input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} />
                {t('common.selectAll', '全选')}
              </label>
            </div>
          )}
          {groupByTag ? (
          <div className="tag-group-list">
            {groupedAccounts.map(([groupKey, groupAccounts]) => (
              <div key={groupKey} className="tag-group-section">
                <div className="tag-group-header">
                  <span className="tag-group-title">{resolveGroupLabel(groupKey)}</span>
                  <span className="tag-group-count">{groupAccounts.length}</span>
                </div>
                <div className="tag-group-grid ghcp-accounts-grid">{renderGridCards(groupAccounts, groupKey)}</div>
              </div>
            ))}
          </div>
        ) : (
          <div className="ghcp-accounts-grid">{renderGridCards(filteredAccounts)}</div>
        )}
        </div>
      ) : groupByTag ? (
        <div className="account-table-container grouped">
          <table className="account-table">
            <thead>
              <tr>
                <th style={{ width: 40 }}>
                  <input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} />
                </th>
                <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th>
                <th>{t('gemini.quota.pro', 'Pro')}</th>
                <th>{t('gemini.quota.flash', 'Flash')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
              </tr>
            </thead>
            <tbody>
              {groupedAccounts.map(([groupKey, groupAccounts]) => (
                <Fragment key={groupKey}>
                  <tr className="tag-group-row">
                    <td colSpan={5}>
                      <div className="tag-group-header">
                        <span className="tag-group-title">{resolveGroupLabel(groupKey)}</span>
                        <span className="tag-group-count">{groupAccounts.length}</span>
                      </div>
                    </td>
                  </tr>
                  {renderTableRows(groupAccounts, groupKey)}
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
                  <input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} />
                </th>
                <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th>
                <th>{t('gemini.quota.pro', 'Pro')}</th>
                <th>{t('gemini.quota.flash', 'Flash')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
              </tr>
            </thead>
            <tbody>{renderTableRows(filteredAccounts)}</tbody>
          </table>
        </div>
      )}

      {showAddModal && (
        <div className="modal-overlay" onClick={closeAddModal}>
          <div className="modal-content ghcp-add-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('gemini.addModal.title', '添加 Gemini 账号')}</h2>
              <button className="modal-close" onClick={closeAddModal} aria-label={t('common.close', '关闭')}><X /></button>
            </div>

            <div className="modal-tabs">
              <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}><Globe size={14} />{t('common.shared.addModal.oauth', '授权登录')}</button>
              <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}><KeyRound size={14} />Token / JSON</button>
              <button className={`modal-tab ${addTab === 'import' ? 'active' : ''}`} onClick={() => openAddModal('import')}><Database size={14} />{t('common.shared.addModal.import', '本地导入')}</button>
            </div>

            <div className="modal-body">
              {addTab === 'oauth' && (
                <div className="add-section">
                  <p className="section-desc">{t('gemini.oauth.desc', '点击下方按钮，在浏览器中完成 Gemini 授权登录。')}</p>

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
                  <p className="section-desc">{t('gemini.token.desc', '粘贴您的 Gemini Access Token（JWT）或导出的 JSON 数据。')}</p>
                  <details className="token-format-collapse">
                    <summary className="token-format-collapse-summary">{t('gemini.token.formatHint', '必填字段与示例（点击展开）')}</summary>
                    <div className="token-format">
                      <p className="token-format-required">{t('gemini.token.formatRequired', '单条 Token 直接粘贴 JWT；批量导入使用 JSON 数组格式')}</p>
                      <div className="token-format-group">
                        <div className="token-format-label">{t('gemini.token.singleExample', '单条示例（JWT）')}</div>
                        <pre className="token-format-code">{CURSOR_TOKEN_SINGLE_EXAMPLE}</pre>
                      </div>
                      <div className="token-format-group">
                        <div className="token-format-label">{t('gemini.token.batchExample', '批量示例（JSON）')}</div>
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
                  <p className="section-desc">{t('gemini.import.localDesc', '支持从本机 Gemini 客户端或 JSON 文件导入账号数据。')}</p>
                  <button className="btn btn-secondary btn-full" onClick={() => handleImportFromLocal?.()} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}
                    {t('gemini.import.localClient', '从本机 Gemini 导入')}
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
        title={`${t('common.shared.export', '导出')} JSON`}
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

      {launchModal && (
        <div className="modal-overlay" onClick={() => setLaunchModal(null)}>
          <div className="modal modal-lg" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('gemini.instances.launchDialogTitle', '启动实例')}</h2>
              <button className="modal-close" onClick={() => setLaunchModal(null)} aria-label={t('common.close', '关闭')}>
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-status success">
                <Check size={16} />
                <span>{t('accounts.switched', '已切换至 {{email}}', { email: launchModal.accountEmail })}</span>
              </div>
              <div className="form-group">
                <label>{t('instances.columns.instance', '实例')}</label>
                <input
                  className="form-input"
                  value={
                    launchModal.instanceName === '__default__'
                      ? t('instances.defaultName', '默认实例')
                      : launchModal.instanceName
                  }
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t('instances.form.extraArgs', '自定义启动参数')}</label>
                <textarea className="form-input instance-args-input" value={launchModal.launchCommand} readOnly />
                <p className="form-hint">
                  {t('gemini.instances.launchHint', '可复制命令手动执行，或点击下方按钮直接在终端执行。')}
                </p>
              </div>
              {launchModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{launchModal.executeMessage}</span>
                </div>
              )}
              {launchModal.executeError && <div className="form-error">{launchModal.executeError}</div>}
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={handleCopyLaunchCommand}>
                <Copy size={16} />
                {launchModal.copied ? t('common.success', '成功') : t('common.copy', '复制')}
              </button>
              <button className="btn btn-primary" onClick={handleExecuteInTerminal} disabled={launchModal.executing}>
                <Play size={16} />
                {launchModal.executing
                  ? t('common.loading', '加载中...')
                  : t('gemini.instances.runInTerminal', '终端执行')}
              </button>
            </div>
          </div>
        </div>
      )}
        </>
      )}

      {activeTab === 'instances' && (
        <GeminiInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}
    </div>
  );
}
