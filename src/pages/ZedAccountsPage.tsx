import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import {
  ArrowDownWideNarrow,
  Check,
  ChevronDown,
  CircleAlert,
  Copy,
  Database,
  Download,
  Eye,
  EyeOff,
  Globe,
  KeyRound,
  LayoutGrid,
  List,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Search,
  Tag,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { TagEditModal } from '../components/TagEditModal';
import { ZedOverviewTabsHeader } from '../components/ZedOverviewTabsHeader';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import * as zedService from '../services/zedService';
import { useZedAccountStore } from '../stores/useZedAccountStore';
import {
  getZedAccountDisplayEmail,
  getZedEditPredictionsMetrics,
  getZedPlanBadge,
  type ZedAccount,
} from '../types/zed';
import './ZedAccountsPage.css';

type ZedSortKey = 'created_at' | 'token_spend' | 'billing_end';

const ZED_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.zed.flow_notice_collapsed';
const ZED_CURRENT_ACCOUNT_ID_KEY = 'agtools.zed.current_account_id';
const ZED_UNTAGGED_KEY = '__untagged__';

function parseFiniteNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value.trim());
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function getZedPlanTone(planRaw?: string | null): string {
  const normalized = (planRaw || '').trim().toLowerCase();
  if (!normalized) return 'unknown';
  if (normalized.includes('enterprise') || normalized.includes('team')) return 'enterprise';
  if (normalized.includes('trial')) return 'trial';
  if (
    normalized.includes('pro') ||
    normalized.includes('plus') ||
    normalized.includes('ultra') ||
    normalized.includes('ultimate')
  ) {
    return 'pro';
  }
  if (normalized.includes('free')) return 'free';
  return 'unknown';
}

function getZedStatusTone(status?: string | null): 'normal' | 'warning' | 'forbidden' | 'neutral' {
  const normalized = (status || '').trim().toLowerCase();
  if (!normalized) return 'neutral';
  if (
    normalized.includes('cancel') ||
    normalized.includes('expire') ||
    normalized.includes('past_due') ||
    normalized.includes('past due') ||
    normalized.includes('suspend')
  ) {
    return 'forbidden';
  }
  if (normalized.includes('trial') || normalized.includes('pending')) {
    return 'warning';
  }
  if (normalized.includes('active')) {
    return 'normal';
  }
  return 'neutral';
}

function formatDateTime(timestamp?: number | null, locale = 'zh-CN'): string {
  if (!timestamp) return '--';
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) return '--';
  return date.toLocaleString(locale);
}

function formatDateOnly(timestamp?: number | null, locale = 'zh-CN'): string {
  if (!timestamp) return '';
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleDateString(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  });
}

type ZedUsagePanelItem =
  | {
      key: string;
      variant: 'simple';
      label: string;
      value: string;
      detail: string;
      title: string;
      tone?: 'high' | 'medium' | 'low';
    }
  | {
      key: string;
      variant: 'metric';
      label: string;
      value: string;
      detail: string;
      title: string;
      usedText: string;
      leftText: string;
      progressPercent: number;
      tone: 'high' | 'medium' | 'low';
    };

type ZedUsagePanel = {
  headline: string;
  note: string;
  items: ZedUsagePanelItem[];
  title: string;
};

function formatQuotaCount(value: number | null | undefined): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '0';
  return new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 }).format(
    Math.max(0, Math.round(value)),
  );
}

function getQuotaTone(progressPercent: number): 'high' | 'medium' | 'low' {
  const remaining = Math.max(0, Math.min(100, 100 - progressPercent));
  if (remaining <= 10) return 'low';
  if (remaining <= 30) return 'medium';
  return 'high';
}

export function ZedAccountsPage() {
  const { t } = useTranslation();
  const isMacOS = usePlatformRuntimeSupport('macos-only');
  const store = useZedAccountStore();
  const [filterTypes, setFilterTypes] = useState<string[]>([]);

  const page = useProviderAccountsPage<ZedAccount>({
    platformKey: 'Zed',
    oauthLogPrefix: 'ZedOAuth',
    flowNoticeCollapsedKey: ZED_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: ZED_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'zed_accounts',
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
      startLogin: zedService.zedOauthLoginStart,
      completeLogin: zedService.zedOauthLoginComplete,
      cancelLogin: zedService.zedOauthLoginCancel,
      submitCallbackUrl: zedService.zedOauthSubmitCallbackUrl,
    },
    dataService: {
      importFromJson: zedService.importZedFromJson,
      exportAccounts: zedService.exportZedAccounts,
      injectToVSCode: zedService.injectZedAccount,
      importFromLocal: zedService.importZedFromLocal,
    },
    getDisplayEmail: getZedAccountDisplayEmail,
    resolveOauthSuccessMessage: () =>
      t(
        'zed.oauth.importOnlySuccess',
        '授权成功，账号已导入；如需让官方 Zed 使用该账号，请点击“应用并重启”。',
      ),
    onInjectSuccess: async () => {
      await store.fetchCurrentAccountId();
    },
  });

  const {
    t: pageT,
    locale,
    privacyModeEnabled,
    togglePrivacyMode,
    maskAccountText,
    viewMode,
    setViewMode,
    searchQuery,
    setSearchQuery,
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
    handleImportFromLocal,
    handleImportJsonFile,
    handlePickImportFile,
    importFileInputRef,
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
    handleRetryOauthComplete,
    handleOpenOauthUrl,
    handleSubmitOauthCallbackUrl,
    handleInjectToVSCode,
    isFlowNoticeCollapsed,
    setIsFlowNoticeCollapsed,
    currentAccountId,
    formatDate,
    normalizeTag,
  } = page;

  const accounts = store.accounts;
  const loading = store.loading;

  const syncCurrentAccount = useCallback(async () => {
    try {
      await store.fetchCurrentAccountId();
    } catch (error) {
      console.error('加载 Zed 运行时状态失败:', error);
    }
  }, [store.fetchCurrentAccountId]);

  useEffect(() => {
    void syncCurrentAccount();
  }, [syncCurrentAccount]);

  const currentAccount = useMemo(
    () => accounts.find((account) => account.id === currentAccountId) ?? null,
    [accounts, currentAccountId],
  );

  useEffect(() => {
    if (loading || !currentAccountId || currentAccount) return;
    void syncCurrentAccount();
  }, [currentAccount, currentAccountId, loading, syncCurrentAccount]);

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

  const resolvePlanKey = useCallback((account: ZedAccount) => getZedPlanBadge(account), []);

  const resolveSingleExportBaseName = useCallback((account: ZedAccount) => {
    const display = (getZedAccountDisplayEmail(account) || account.id).trim();
    const atIndex = display.indexOf('@');
    return atIndex > 0 ? display.slice(0, atIndex) : display;
  }, []);

  const tierCounts = useMemo(() => {
    const counts = new Map<string, number>();
    accounts.forEach((account) => {
      const key = resolvePlanKey(account);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    });
    return counts;
  }, [accounts, resolvePlanKey]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      Array.from(tierCounts.entries())
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([plan, count]) => ({
          value: plan,
          label: `${plan} (${count})`,
        })),
    [tierCounts],
  );

  const compareAccountsBySort = useCallback(
    (left: ZedAccount, right: ZedAccount) => {
      const key = sortBy as ZedSortKey;
      if (key === 'billing_end') {
        const leftValue = left.billing_period_end_at ?? null;
        const rightValue = right.billing_period_end_at ?? null;
        if (leftValue == null && rightValue == null) return 0;
        if (leftValue == null) return 1;
        if (rightValue == null) return -1;
        return sortDirection === 'desc' ? rightValue - leftValue : leftValue - rightValue;
      }

      if (key === 'token_spend') {
        const leftValue = parseFiniteNumber(left.token_spend_used_cents) ?? -1;
        const rightValue = parseFiniteNumber(right.token_spend_used_cents) ?? -1;
        const diff = rightValue - leftValue;
        return sortDirection === 'desc' ? diff : -diff;
      }

      const diff = right.created_at - left.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    },
    [sortBy, sortDirection],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];

    if (searchQuery.trim()) {
      const query = searchQuery.trim().toLowerCase();
      result = result.filter((account) => {
        const fields = [
          getZedAccountDisplayEmail(account),
          account.github_login,
          account.user_id,
          account.plan_raw,
          ...(account.tags || []),
        ]
          .filter(Boolean)
          .map((value) => String(value).toLowerCase());
        return fields.some((value) => value.includes(query));
      });
    }

    if (filterTypes.length > 0) {
      const selectedTypes = new Set(filterTypes);
      result = result.filter((account) => selectedTypes.has(resolvePlanKey(account)));
    }

    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((account) =>
        (account.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag)),
      );
    }

    result.sort(compareAccountsBySort);
    return result;
  }, [accounts, compareAccountsBySort, filterTypes, normalizeTag, resolvePlanKey, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, typeof filteredAccounts]>;
    const groups = new Map<string, typeof filteredAccounts>();

    filteredAccounts.forEach((account) => {
      const tags = (account.tags || []).map(normalizeTag).filter(Boolean);
      const matchedTags =
        tagFilter.length > 0
          ? tags.filter((tag) => tagFilter.map(normalizeTag).includes(tag))
          : tags;

      if (matchedTags.length === 0) {
        if (!groups.has(ZED_UNTAGGED_KEY)) groups.set(ZED_UNTAGGED_KEY, []);
        groups.get(ZED_UNTAGGED_KEY)?.push(account);
        return;
      }

      matchedTags.forEach((tag) => {
        if (!groups.has(tag)) groups.set(tag, []);
        groups.get(tag)?.push(account);
      });
    });

    return Array.from(groups.entries()).sort(([leftKey], [rightKey]) => {
      if (leftKey === ZED_UNTAGGED_KEY) return 1;
      if (rightKey === ZED_UNTAGGED_KEY) return -1;
      return leftKey.localeCompare(rightKey);
    });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter]);

  const resolveGroupLabel = useCallback(
    (groupKey: string) =>
      groupKey === ZED_UNTAGGED_KEY ? t('accounts.defaultGroup', '默认分组') : groupKey,
    [t],
  );

  const resolveCycleDisplay = useCallback(
    (account: ZedAccount) => {
      const startText = formatDateOnly(account.billing_period_start_at, locale);
      const endText = formatDateOnly(account.billing_period_end_at, locale);
      const periodRangeText =
        startText && endText
          ? t('common.shared.credits.periodRange', {
              start: startText,
              end: endText,
              defaultValue: '周期：{{start}} - {{end}}',
            })
          : endText
            ? t('common.shared.credits.periodEndOnly', {
                end: endText,
                defaultValue: '周期结束：{{end}}',
              })
            : t('common.shared.credits.planEndsUnknown', '配额周期时间未知');

      const summary = periodRangeText;
      const detail = '';

      return {
        summary,
        detail,
        title: detail ? `${summary} · ${detail}` : summary,
      };
    },
    [locale, t],
  );

  const formatUsedLine = useCallback(
    (used: number | null | undefined, total: number | null | undefined) =>
      t('common.shared.credits.usedLine', {
        used: formatQuotaCount(used),
        total: formatQuotaCount(total),
        defaultValue: '{{used}} / {{total}} used',
      }),
    [t],
  );

  const formatLeftLine = useCallback(
    (left: number | null | undefined) =>
      t('common.shared.credits.leftInline', {
        left: formatQuotaCount(left),
        defaultValue: '{{left}} left',
      }),
    [t],
  );

  const buildUsagePanel = useCallback(
    (account: ZedAccount | null): ZedUsagePanel => {
      if (!account) {
        return {
          headline: '',
          note: t('zed.runtime.noCurrentAccount', '当前没有生效的 Zed 账号'),
          items: [
            {
              key: 'edit',
              variant: 'simple',
              label: 'Edit Predictions',
              value: '--',
              detail: '',
              title: 'Edit Predictions',
            },
            {
              key: 'overdue',
              variant: 'simple',
              label: t('zed.page.overdueField', '是否欠费'),
              value: '--',
              detail: '',
              title: t('zed.page.overdueField', '是否欠费'),
            },
          ],
          title: t('zed.runtime.noCurrentAccount', '当前没有生效的 Zed 账号'),
        };
      }

      const updatedText = account.usage_updated_at
        ? t('zed.page.usageUpdatedAt', {
            time: formatDateTime(account.usage_updated_at, locale),
            defaultValue: 'Updated: {{time}}',
          })
        : '';

      const hasEditPredictions =
        account.edit_predictions_used != null || Boolean(account.edit_predictions_limit_raw?.trim());

      const editMetrics = hasEditPredictions ? getZedEditPredictionsMetrics(account) : null;
      const items: ZedUsagePanelItem[] = [];
      if (editMetrics) {
        const editValue = `${formatQuotaCount(editMetrics.used)} / ${formatQuotaCount(editMetrics.total)}`;
        const editProgressPercent = Math.max(0, Math.min(100, editMetrics.usedPercent));
        items.push({
          key: 'edit',
          variant: 'metric',
          label: 'Edit Predictions',
          value: `${Math.round(editProgressPercent)}%`,
          detail: '',
          title: `Edit Predictions: ${editValue}`,
          usedText: formatUsedLine(editMetrics.used, editMetrics.total),
          leftText: formatLeftLine(editMetrics.left),
          progressPercent: editProgressPercent,
          tone: getQuotaTone(editProgressPercent),
        });
      }
      if (account.has_overdue_invoices != null) {
        const overdueValue = account.has_overdue_invoices
          ? t('zed.page.overdueYes', '是')
          : t('zed.page.overdueNo', '否');
        items.push({
          key: 'overdue',
          variant: 'simple',
          label: t('zed.page.overdueField', '是否欠费'),
          value: overdueValue,
          detail: updatedText,
          title: `${t('zed.page.overdueField', '是否欠费')}: ${overdueValue}`,
          tone: account.has_overdue_invoices ? 'low' : 'high',
        });
      }

      return {
        headline: '',
        note: updatedText,
        items,
        title: items.map((item) => item.title).join(' | ') || updatedText,
      };
    },
    [formatLeftLine, formatUsedLine, locale, t],
  );

  const renderUsagePanel = (
    panel: ReturnType<typeof buildUsagePanel>,
    options?: { compact?: boolean },
  ) => (
    <div className={`windsurf-official-usage ${options?.compact ? 'compact' : ''}`} title={panel.title}>
      {panel.headline ? <div className="windsurf-official-usage-headline">{panel.headline}</div> : null}
      {panel.note ? <div className="windsurf-official-usage-note">{panel.note}</div> : null}
      <div className="windsurf-official-usage-list">
        {panel.items.map((item) =>
          item.variant === 'metric' ? (
            <div key={item.key} className="windsurf-official-usage-item zed-usage-metric-item" title={item.title}>
              <div className="zed-usage-metric-header">
                <span className="zed-usage-metric-label">{item.label}</span>
                <span className={`zed-usage-metric-value ${item.tone}`}>{item.value}</span>
              </div>
              <div className={`windsurf-credit-meta-row ${options?.compact ? 'table' : ''}`}>
                <span className="windsurf-credit-used">{item.usedText}</span>
                <span className="windsurf-credit-left">{item.leftText}</span>
              </div>
              <div className={`zed-usage-metric-track ${options?.compact ? 'compact' : ''}`}>
                <div
                  className={`zed-usage-metric-bar ${item.tone}`}
                  style={{ width: `${item.progressPercent}%` }}
                />
              </div>
              {item.detail ? <div className="windsurf-official-usage-detail">{item.detail}</div> : null}
            </div>
          ) : (
            <div key={item.key} className="windsurf-official-usage-item" title={item.title}>
              <div className="windsurf-official-usage-main">
                <span className="windsurf-official-usage-label">{item.label}</span>
                <span className={`windsurf-official-usage-value ${item.tone ?? ''}`}>{item.value}</span>
              </div>
              {item.detail ? <div className="windsurf-official-usage-detail">{item.detail}</div> : null}
            </div>
          ),
        )}
      </div>
    </div>
  );

  const renderPlanDetails = (
    cycleDisplay: ReturnType<typeof resolveCycleDisplay>,
    options?: { compact?: boolean },
  ) => (
    <div className={`windsurf-plan-cycle ${options?.compact ? 'compact' : ''}`} title={cycleDisplay.title}>
      <span className="windsurf-plan-cycle-summary">{cycleDisplay.summary}</span>
      {cycleDisplay.detail ? (
        <span className="windsurf-plan-cycle-detail">{cycleDisplay.detail}</span>
      ) : null}
    </div>
  );

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const emailText = getZedAccountDisplayEmail(account);
      const cycleDisplay = resolveCycleDisplay(account);
      const usagePanel = buildUsagePanel(account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      const statusTone = account.subscription_status
        ? getZedStatusTone(account.subscription_status)
        : null;

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
            <span className="account-email" title={maskAccountText(emailText)}>
              {maskAccountText(emailText)}
            </span>
            {isCurrent && <span className="current-tag">{t('accounts.status.current')}</span>}
            <span className={`tier-badge ${getZedPlanTone(account.plan_raw)}`}>
              {getZedPlanBadge(account)}
            </span>
          </div>

          <div className="zed-status-row">
            {account.subscription_status && statusTone ? (
              <span className={`status-pill ${statusTone}`}>{account.subscription_status}</span>
            ) : null}
            <span className="tag-pill zed-login-pill">@{account.github_login}</span>
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

          {renderUsagePanel(usagePanel)}
          {renderPlanDetails(cycleDisplay)}

          <div className="card-footer">
            <span className="card-date">{formatDate(account.last_used || account.created_at)}</span>
            <div className="card-actions">
              <button
                className="card-action-btn success"
                onClick={() => handleInjectToVSCode?.(account.id)}
                disabled={injecting === account.id}
                title={
                  isCurrent
                    ? t('zed.actions.reapply', '重新应用')
                    : t('zed.actions.applyAndRestart', '应用并重启')
                }
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
                title={t('common.shared.refreshQuota', '刷新配额')}
              >
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button
                className="card-action-btn export-btn"
                onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
                title={t('common.shared.export', '导出')}
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
    });

  const renderTableRows = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const emailText = getZedAccountDisplayEmail(account);
      const cycleDisplay = resolveCycleDisplay(account);
      const usagePanel = buildUsagePanel(account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 3);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isCurrent = currentAccountId === account.id;

      return (
        <tr
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={isCurrent ? 'current' : ''}
        >
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
                <span className="account-email-text" title={maskAccountText(emailText)}>
                  {maskAccountText(emailText)}
                </span>
                {isCurrent && <span className="mini-tag current">{t('accounts.status.current')}</span>}
              </div>
              <div className="account-sub-line">
                <span className="kiro-table-subline">
                  {account.subscription_status
                    ? `@${account.github_login} · ${account.subscription_status}`
                    : `@${account.github_login}`}
                </span>
              </div>
              {accountTags.length > 0 && (
                <div className="account-tags-inline">
                  {visibleTags.map((tag, index) => (
                    <span key={`${account.id}-inline-${tag}-${index}`} className="tag-pill">
                      {tag}
                    </span>
                  ))}
                  {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                </div>
              )}
            </div>
          </td>
          <td>
            <span className={`tier-badge ${getZedPlanTone(account.plan_raw)}`}>
              {getZedPlanBadge(account)}
            </span>
          </td>
          <td>{renderUsagePanel(usagePanel, { compact: true })}</td>
          <td>{renderPlanDetails(cycleDisplay, { compact: true })}</td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button
                className="action-btn success"
                onClick={() => handleInjectToVSCode?.(account.id)}
                disabled={injecting === account.id}
                title={
                  isCurrent
                    ? t('zed.actions.reapply', '重新应用')
                    : t('zed.actions.applyAndRestart', '应用并重启')
                }
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
                title={t('common.shared.refreshQuota', '刷新配额')}
              >
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button
                className="action-btn"
                onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
                title={t('common.shared.export', '导出')}
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
    });

  return (
    <div className="ghcp-accounts-page zed-accounts-page">
      <ZedOverviewTabsHeader />

      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
        <button
          type="button"
          className="ghcp-flow-notice-toggle"
          onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)}
          aria-expanded={!isFlowNoticeCollapsed}
        >
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('zed.flowNotice.title', 'Zed 账号接入说明（点击展开/收起）')}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t(
                'zed.flowNotice.desc',
                '支持官方 OAuth 登录、JSON 导入、本机当前登录状态导入，以及按 Zed 客户端真实落盘规则应用账号并重启官方客户端。页面仅展示桌面端可直接读取的状态字段。',
              )}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>
                {t(
                  'zed.flowNotice.reason',
                  '权限范围：浏览器打开 Zed 官方同源登录；读取本机当前 Zed 登录凭据用于导入；应用账号时按官方客户端相同位点写回系统凭据，并可按需启动或重启 Zed。',
                )}
              </li>
              <li>
                {t(
                  'zed.flowNotice.storage',
                  '数据范围：本地仅保存导入或授权得到的账号记录、标签和导出内容；不会上传本机 Keychain/凭据原文，不会扫描浏览器网页登录会话，也不会修改无关系统凭据。',
                )}
              </li>
            </ul>
          </div>
        )}
      </div>

      {message && (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
          {message.text}
          <button onClick={() => setMessage(null)}>
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
              placeholder={t('zed.page.searchPlaceholder', '搜索账号、套餐或标签')}
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
            allLabel={`ALL (${accounts.length})`}
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
              <div className="tag-filter-panel">
                {availableTags.length === 0 ? (
                  <div className="tag-filter-empty">{t('accounts.noAvailableTags', '暂无可用标签')}</div>
                ) : (
                  <div className="tag-filter-options">
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

          <div className="sort-select">
            <ArrowDownWideNarrow size={14} className="sort-icon" />
            <select
              value={sortBy}
              onChange={(event) => setSortBy(event.target.value)}
              aria-label={t('common.shared.sortLabel', '排序')}
            >
              <option value="created_at">{t('common.shared.sort.createdAt', '按创建时间')}</option>
              <option value="token_spend">{t('zed.sort.tokenSpend', '按 Token Spend')}</option>
              <option value="billing_end">{t('common.shared.sort.planEnd', '按配额周期结束时间')}</option>
            </select>
          </div>

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
            title={t('zed.actions.addAccount', '登录 Zed')}
          >
            <Plus size={14} />
          </button>
          <button
            className="btn btn-secondary icon-only"
            onClick={handleRefreshAll}
            disabled={refreshingAll || accounts.length === 0}
            title={t('common.shared.refreshAll', '刷新全部')}
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
          >
            {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
          <button
            className="btn btn-secondary icon-only"
            onClick={() => openAddModal('import')}
            disabled={importing}
            title={t('common.shared.import.label', '导入')}
          >
            <Download size={14} />
          </button>
          <button
            className="btn btn-secondary export-btn icon-only"
            onClick={() => void handleExport(filteredIds)}
            disabled={exporting || filteredIds.length === 0}
            title={
              exportSelectionCount > 0
                ? `${t('common.shared.export', '导出')} (${exportSelectionCount})`
                : t('common.shared.export', '导出')
            }
          >
            <Upload size={14} />
          </button>
          {selected.size > 0 && (
            <button
              className="btn btn-danger icon-only"
              onClick={handleBatchDelete}
              title={`${t('common.delete', '删除')} (${selected.size})`}
            >
              <Trash2 size={14} />
            </button>
          )}
          <QuickSettingsPopover type="zed" />
        </div>
      </div>

      {loading && accounts.length === 0 ? (
        <div className="loading-container">
          <RefreshCw size={24} className="loading-spinner" />
          <p>{t('common.loading', '加载中...')}</p>
        </div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t('zed.empty.description', '点击“添加账号”开始管理你的 Zed 账号。')}</p>
          <button className="btn btn-primary" onClick={() => openAddModal('oauth')}>
            <Plus size={16} />
            {t('common.shared.addAccount', '添加账号')}
          </button>
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
                  <input
                    type="checkbox"
                    checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0}
                    onChange={() => toggleSelectAll(filteredAccounts.map((account) => account.id))}
                  />
                </th>
                <th style={{ width: 240 }}>{pageT('common.shared.columns.email', '邮箱')}</th>
                <th style={{ width: 120 }}>{pageT('common.shared.columns.plan', '计划')}</th>
                <th>{t('zed.page.tokenSpend', 'Token Spend')}</th>
                <th>{pageT('common.detail', '详情')}</th>
                <th className="sticky-action-header table-action-header">
                  {pageT('common.shared.columns.actions', '操作')}
                </th>
              </tr>
            </thead>
            <tbody>
              {groupedAccounts.map(([groupKey, groupAccounts]) => (
                <Fragment key={groupKey}>
                  <tr className="tag-group-row">
                    <td colSpan={6}>
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
                  <input
                    type="checkbox"
                    checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0}
                    onChange={() => toggleSelectAll(filteredAccounts.map((account) => account.id))}
                  />
                </th>
                <th style={{ width: 240 }}>{pageT('common.shared.columns.email', '邮箱')}</th>
                <th style={{ width: 120 }}>{pageT('common.shared.columns.plan', '计划')}</th>
                <th>{t('zed.page.tokenSpend', 'Token Spend')}</th>
                <th>{pageT('common.detail', '详情')}</th>
                <th className="sticky-action-header table-action-header">
                  {pageT('common.shared.columns.actions', '操作')}
                </th>
              </tr>
            </thead>
            <tbody>{renderTableRows(filteredAccounts)}</tbody>
          </table>
        </div>
      )}

      {showAddModal && (
        <div className="modal-overlay" onClick={closeAddModal}>
          <div className="modal-content ghcp-add-modal zed-add-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('zed.addModal.title', '添加 Zed 账号')}</h2>
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
              {isMacOS && (
                <button
                  className={`modal-tab ${addTab === 'import' ? 'active' : ''}`}
                  onClick={() => openAddModal('import')}
                >
                  <Database size={14} />
                  {t('accounts.tabs.import', '导入')}
                </button>
              )}
            </div>

            <div className="modal-body">
              {addTab === 'oauth' && (
                <div className="add-section">
                  <p className="section-desc">
                    {t('zed.oauth.desc', '点击下方按钮，在浏览器中完成 Zed 官方登录授权。')}
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
                      {oauthMeta ? (
                        <p className="oauth-hint">
                          {t('common.shared.oauth.meta', '授权有效期：{{expires}}s；轮询间隔：{{interval}}s', {
                            expires: oauthMeta.expiresIn,
                            interval: oauthMeta.intervalSeconds,
                          })}
                        </p>
                      ) : null}
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
                              onChange={(event) => setOauthManualCallbackInput(event.target.value)}
                              placeholder={t(
                                'common.shared.oauth.manualCallbackPlaceholder',
                                '粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...',
                              )}
                            />
                            <button
                              className="oauth-copy-button"
                              onClick={() => void handleSubmitOauthCallbackUrl()}
                              disabled={oauthManualCallbackSubmitting || !oauthManualCallbackInput.trim()}
                            >
                              {oauthManualCallbackSubmitting ? (
                                <RefreshCw size={16} className="loading-spinner" />
                              ) : (
                                <Check size={16} />
                              )}
                              {t('accounts.oauth.continue', '我已授权，继续')}
                            </button>
                          </div>
                        </div>
                      )}
                      {oauthManualCallbackError ? (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>{oauthManualCallbackError}</span>
                        </div>
                      ) : null}
                      {oauthPolling ? (
                        <div className="add-status loading">
                          <RefreshCw size={16} className="loading-spinner" />
                          <span>{t('common.shared.oauth.waiting', '等待授权完成...')}</span>
                        </div>
                      ) : null}
                      {oauthCompleteError ? (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>{oauthCompleteError}</span>
                          {oauthTimedOut ? (
                            <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>
                              {t('common.shared.oauth.timeoutRetry', '刷新授权链接')}
                            </button>
                          ) : (
                            <button className="btn btn-sm btn-outline" onClick={handleRetryOauthComplete}>
                              {t('common.refresh', 'Refresh')}
                            </button>
                          )}
                        </div>
                      ) : null}
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
                  <p className="section-desc">
                    {t('zed.import.jsonDesc', '导入由本工具导出的 Zed JSON 文件。')}
                  </p>
                  <textarea
                    className="token-input"
                    value={tokenInput}
                    onChange={(event) => setTokenInput(event.target.value)}
                    placeholder={t(
                      'common.shared.token.placeholder',
                      '示例：ghu_xxx / sk-ws-xxx / {"access_token":"eyJ...","refresh_token":"rt_..."} / [{...}]',
                    )}
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
              )}

              {isMacOS && addTab === 'import' && (
                <div className="add-section">
                  <p className="section-desc">
                    {t('zed.import.localDesc', '支持从本机 Zed 当前登录状态或导出的 JSON 文件导入账号数据。')}
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
                    {t('common.shared.addModal.import', '本地导入')}
                  </button>
                  <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>
                    {t('common.shared.import.orJson', '或从 JSON 文件导入')}
                  </div>
                  <input
                    ref={importFileInputRef}
                    type="file"
                    accept="application/json"
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
                      <Database size={16} />
                    )}
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
        title={t('zed.actions.exportTitle', '导出 Zed 账号 JSON')}
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
              <h2>{t('common.confirm')}</h2>
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
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>
                {t('common.cancel')}
              </button>
              <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>
                {t('common.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div className="modal-overlay" onClick={() => !deletingTag && setTagDeleteConfirm(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
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
                {t('accounts.confirmDeleteTag', 'Delete tag "{{tag}}"? This tag will be removed from {{count}} accounts.', {
                  tag: tagDeleteConfirm.tag,
                  count: tagDeleteConfirm.count,
                })}
              </p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setTagDeleteConfirm(null)}
                disabled={deletingTag}
              >
                {t('common.cancel')}
              </button>
              <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>
                {deletingTag ? t('common.processing', '处理中...') : t('common.confirm')}
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
    </div>
  );
}
