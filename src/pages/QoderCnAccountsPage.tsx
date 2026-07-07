import { useMemo, useState, useCallback, useEffect, Fragment } from 'react';
import {
  ArrowDownWideNarrow,
  ChevronDown,
  ChevronLeft,
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
  Check,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { MfaQuickCodeSelect } from '../components/MfaQuickCodeSelect';
import { PaginationControls } from '../components/PaginationControls';
import { AccountSelectionToolbar } from '../components/AccountSelectionToolbar';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import {
  PlatformOverviewTab,
  PlatformOverviewTabsHeader,
} from '../components/platform/PlatformOverviewTabsHeader';
import { useQoderCnAccountStore } from '../stores/useQoderCnAccountStore';
import * as qoderCnService from '../services/qoderCnService';
import {
  QoderCnAccount,
  getQoderCnAccountDisplayEmail,
  getQoderCnPlanBadge,
  getQoderCnSubscriptionInfo,
  getQoderCnUsage,
  hasQoderCnQuotaData,
} from '../types/qoderCn';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
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

const QODER_CN_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.qoder_cn.flow_notice_collapsed';
const QODER_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.qoder_cn.current_account_id';
const QODER_CN_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('qoder_cn');
const FILTER_TYPES_FIELD = 'filter_types';

function formatQuotaValue(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return '--';
  const hasDecimal = Math.abs(value - Math.trunc(value)) > 0.001;
  return new Intl.NumberFormat('en-US', {
    maximumFractionDigits: hasDecimal ? 2 : 0,
  }).format(value);
}

function formatDateTime(value: number): string {
  const date = new Date(value * 1000);
  return date.toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function formatDisplayDate(value: number): string {
  return new Date(value).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

function resolveQoderCnPlanBadgeClass(plan: string): string {
  const normalized = plan.trim().toLowerCase();
  if (!normalized || normalized === 'unknown') return 'unknown';
  if (normalized.includes('free')) return 'free';
  if (normalized.includes('trial')) return 'trial';
  if (normalized.includes('pro')) return 'pro';
  if (normalized.includes('team')) return 'team';
  if (normalized.includes('enterprise')) return 'enterprise';
  if (normalized.includes('business')) return 'business';
  if (normalized.includes('individual') || normalized.includes('personal')) return 'individual';
  if (normalized.includes('plus')) return 'plus';
  if (normalized.includes('ultra')) return 'ultra';
  return 'unknown';
}

function computeQuotaClass(percent: number | null): 'high' | 'medium' | 'critical' {
  if (percent == null) return 'high';
  if (percent >= 90) return 'critical';
  if (percent >= 70) return 'medium';
  return 'high';
}

type QoderCnQuotaDisplayItem = {
  key: string;
  label: string;
  normalizedPercent: number;
  quotaClass: 'high' | 'medium' | 'critical';
  percentageText: string | null;
  valueText: string;
  showProgress: boolean;
};

type QoderCnQuotaDisplay = {
  planTag: string;
  planClass: string;
  items: QoderCnQuotaDisplayItem[];
  resetText: string | null;
};

export function QoderCnAccountsPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<PlatformOverviewTab>('overview');
  const store = useQoderCnAccountStore();

  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    readAccountsOverviewFilterPersistenceEnabled(QODER_CN_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterStringArray(QODER_CN_FILTER_PERSISTENCE_SCOPE, FILTER_TYPES_FIELD)
      : [],
  );

  const page = useProviderAccountsPage<QoderCnAccount>({
    platformKey: 'Qoder CN',
    oauthLogPrefix: 'QoderCnOAuth',
    flowNoticeCollapsedKey: QODER_CN_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: QODER_CN_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'qoder_cn_accounts',
    oauthTabKeys: ['oauth'],
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
      startLogin: qoderCnService.qoderCnOauthLoginStart,
      completeLogin: qoderCnService.qoderCnOauthLoginComplete,
      cancelLogin: qoderCnService.qoderCnOauthLoginCancel,
    },
    dataService: {
      importFromJson: qoderCnService.importQoderCnFromJson,
      importFromLocal: qoderCnService.importQoderCnFromLocal,
      addWithToken: qoderCnService.addQoderCnAccountWithToken,
      exportAccounts: qoderCnService.exportQoderCnAccounts,
      injectToVSCode: qoderCnService.switchQoderCnAccount,
    },
    getDisplayEmail: (account) => getQoderCnAccountDisplayEmail(account),
  });

  const {
    privacyModeEnabled, togglePrivacyMode, maskAccountText,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    filterPersistenceEnabled, filterPersistenceScope,
    sortDirection, sortBy,
    selected, toggleSelect, toggleSelectAll,
    tagFilter, groupByTag, setGroupByTag, showTagFilter, setShowTagFilter,
    showTagModal, setShowTagModal, tagFilterRef, availableTags,
    toggleTagFilterValue, clearTagFilter, tagDeleteConfirm, tagDeleteConfirmError, tagDeleteConfirmErrorScrollKey, setTagDeleteConfirm,
    deletingTag, confirmDeleteTag, openTagModal, handleSaveTags,
    refreshing, refreshingAll, injecting,
    handleRefresh, handleRefreshAll, handleDelete, handleBatchDelete,
    deleteConfirm, deleteConfirmError, deleteConfirmErrorScrollKey, setDeleteConfirm, deleting, confirmDelete,
    message, setMessage,
    exporting, handleExport, handleExportByIds, getScopedSelectedCount,
    showExportModal, exportJsonContent, exportJsonHidden,
    toggleExportJsonHidden, exportJsonCopied, copyExportJson,
    savingExportJson, saveExportJson, exportSavedPath,
    canOpenExportSavedDirectory, openExportSavedDirectory, copyExportSavedPath, exportPathCopied,
    closeExportModal,
    showAddModal, addTab, addStatus, addMessage, tokenInput, setTokenInput,
    importing, openAddModal, closeAddModal,
    handleTokenImport, handleImportJsonFile, handleImportFromLocal, handlePickImportFile, importFileInputRef,
    oauthUrl, oauthUrlCopied, oauthUserCode, oauthUserCodeCopied, oauthMeta,
    oauthPolling, oauthTimedOut,
    oauthPrepareError, oauthCompleteError,
    handleCopyOauthUrl, handleCopyOauthUserCode, handleRetryOauth, handleOpenOauthUrl,
    handleInjectToVSCode,
    isFlowNoticeCollapsed, setIsFlowNoticeCollapsed,
    currentAccountId, formatDate, normalizeTag,
  } = page;

  // Persist filter types
  const toggleFilterTypeValue = useCallback((value: string) => {
    setFilterTypes((prev) =>
      prev.includes(value) ? prev.filter((item) => item !== value) : [...prev, value],
    );
  }, []);

  const clearFilterTypes = useCallback(() => setFilterTypes([]), []);

  // Filter persistence
  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_TYPES_FIELD);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_TYPES_FIELD, filterTypes);
  }, [filterPersistenceEnabled, filterPersistenceScope, filterTypes]);

  const accounts = store.accounts;
  const loading = store.loading;

  const isAbnormalAccount = useCallback((_account: QoderCnAccount) => false, []);

  const tierSummary = useMemo(() => {
    const counts = new Map<string, number>();
    counts.set('UNKNOWN', 0);
    for (const account of accounts) {
      const plan = getQoderCnPlanBadge(account) || 'UNKNOWN';
      counts.set(plan, (counts.get(plan) ?? 0) + 1);
    }
    const entries = Array.from(counts.entries())
      .filter(([, count]) => count > 0)
      .sort((a, b) => a[0].localeCompare(b[0]));
    return {
      all: accounts.length,
      valid: accounts.reduce((count, a) => (isAbnormalAccount(a) ? count : count + 1), 0),
      entries,
    };
  }, [accounts, isAbnormalAccount]);

  const allFilterLabel = useMemo(() => {
    const text = t('common.shared.filter.all', { count: tierSummary.all, defaultValue: 'All ({{count}})' });
    if (!text.includes('{{count}}')) return text;
    return text.replace('{{count}}', String(tierSummary.all));
  }, [t, tierSummary.all]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => [
      ...tierSummary.entries.map(([plan, count]) => ({ value: plan, label: `${plan} (${count})` })),
      buildValidAccountsFilterOption(t, tierSummary.valid),
    ],
    [t, tierSummary.entries, tierSummary.valid],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    const query = searchQuery.trim().toLowerCase();
    if (query) {
      result = result.filter((account) => {
        const text = `${getQoderCnAccountDisplayEmail(account)} ${account.user_id || ''} ${getQoderCnPlanBadge(account)}`.toLowerCase();
        return text.includes(query);
      });
    }
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) result = result.filter((a) => !isAbnormalAccount(a));
      if (selectedTypes.size > 0) result = result.filter((a) => selectedTypes.has(getQoderCnPlanBadge(a)));
    }
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((a) => (a.tags || []).some((tag) => selectedTags.has(normalizeTag(tag))));
    }
    result.sort((a, b) => {
      const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
      if (currentFirstDiff !== 0) return currentFirstDiff;
      if (sortBy === 'plan') {
        const cmp = getQoderCnPlanBadge(a).localeCompare(getQoderCnPlanBadge(b));
        return sortDirection === 'asc' ? cmp : -cmp;
      }
      if (sortBy === 'quota') {
        const left = getQoderCnUsage(a).usagePercent ?? -1;
        const right = getQoderCnUsage(b).usagePercent ?? -1;
        const cmp = left - right;
        return sortDirection === 'asc' ? cmp : -cmp;
      }
      const cmp = a.created_at - b.created_at;
      return sortDirection === 'asc' ? cmp : -cmp;
    });
    return result;
  }, [accounts, currentAccountId, searchQuery, filterTypes, isAbnormalAccount, tagFilter, normalizeTag, sortBy, sortDirection]);

  const filteredIds = useMemo(() => filteredAccounts.map((a) => a.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('Qoder CN'),
  });
  const paginatedAccounts = pagination.pageItems;
  const paginatedIds = useMemo(() => paginatedAccounts.map((a) => a.id), [paginatedAccounts]);
  const isAllPaginatedSelected = useMemo(() => isEveryIdSelected(selected, paginatedIds), [paginatedIds, selected]);

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, typeof filteredAccounts]>;
    const groups = new Map<string, typeof filteredAccounts>();
    const selectedTags = new Set(tagFilter.map(normalizeTag));
    for (const account of filteredAccounts) {
      const tags = (account.tags || []).map(normalizeTag).filter(Boolean);
      const matched = selectedTags.size > 0 ? tags.filter((tag) => selectedTags.has(tag)) : tags;
      if (matched.length === 0) {
        const list = groups.get('__untagged__') || [];
        list.push(account);
        groups.set('__untagged__', list);
        continue;
      }
      for (const tag of matched) {
        const list = groups.get(tag) || [];
        list.push(account);
        groups.set(tag, list);
      }
    }
    return Array.from(groups.entries()).sort(([a], [b]) => {
      if (a === '__untagged__') return -1;
      if (b === '__untagged__') return 1;
      return a.localeCompare(b);
    });
  }, [filteredAccounts, groupByTag, tagFilter, normalizeTag]);

  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts],
  );

  const resolveQuotaDisplay = useCallback(
    (account: QoderCnAccount): QoderCnQuotaDisplay => {
      const subscription = getQoderCnSubscriptionInfo(account);
      const buildItem = (
        key: string,
        label: string,
        used: number | null | undefined,
        total: number | null | undefined,
        percentage: number | null | undefined,
      ): QoderCnQuotaDisplayItem => {
        const nUsed = used ?? 0;
        const nTotal = total ?? 0;
        const resolvedPercent = percentage ?? (nTotal > 0 ? (nUsed / nTotal) * 100 : 0);
        const normalizedPercent = Math.max(0, Math.min(100, Math.round(resolvedPercent)));
        return {
          key,
          label,
          normalizedPercent,
          quotaClass: computeQuotaClass(resolvedPercent),
          percentageText: `${normalizedPercent}%`,
          valueText: `${formatQuotaValue(nUsed)} / ${formatQuotaValue(nTotal)}`,
          showProgress: true,
        };
      };
      return {
        planTag: subscription.planTag,
        planClass: resolveQoderCnPlanBadgeClass(subscription.planTag),
        items: [
          buildItem('user_quota', t('qoderCn.quota.userQuota', '用户配额'), subscription.userQuota.used, subscription.userQuota.total, subscription.userQuota.percentage),
          buildItem('add_on', t('qoderCn.quota.addOn', '加量包'), subscription.addOnQuota.used, subscription.addOnQuota.total, subscription.addOnQuota.percentage),
          buildItem('org_pkg', t('qoderCn.quota.orgPackage', '组织资源包'), subscription.orgResourcePackage.used, subscription.orgResourcePackage.total, subscription.orgResourcePackage.percentage),
        ],
        resetText: subscription.expiresAt != null
          ? t('qoderCn.quota.resetAt', '订阅重置：{{date}}', { date: formatDisplayDate(subscription.expiresAt) })
          : null,
      };
    },
    [t],
  );

  const renderQuotaSection = useCallback(
    (account: QoderCnAccount) => {
      if (!hasQoderCnQuotaData(account)) {
        return (
          <div className="ghcp-quota-section qoder-usage-section">
            <div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>
          </div>
        );
      }
      const quota = resolveQuotaDisplay(account);
      return (
        <div className="ghcp-quota-section qoder-usage-section">
          {quota.items.map((item) => (
            <div key={item.key} className={`quota-item windsurf-credit-item qoder-usage-item ${item.showProgress ? '' : 'is-stat'}`}>
              <div className="quota-header">
                <span className="qoder-usage-label-wrap">
                  <span className="quota-label qoder-usage-label">{item.label}</span>
                </span>
              </div>
              {item.showProgress && (
                <div className="quota-bar-track">
                  <div className={`quota-bar ${item.quotaClass}`} style={{ width: `${item.normalizedPercent}%` }} />
                </div>
              )}
              <div className={`windsurf-credit-meta-row ${item.showProgress ? '' : 'qoder-usage-meta-row-stat'}`}>
                {item.percentageText && <span className="windsurf-credit-left qoder-usage-meta-primary">{item.percentageText}</span>}
                <span className="windsurf-credit-used qoder-usage-meta-secondary">{item.valueText}</span>
              </div>
            </div>
          ))}
          {quota.resetText && <div className="quota-reset qoder-usage-reset-note">{quota.resetText}</div>}
        </div>
      );
    },
    [resolveQuotaDisplay, t],
  );

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const maskedEmail = maskAccountText(getQoderCnAccountDisplayEmail(account));
      const isCurrent = currentAccountId === account.id;
      const isSelected = selected.has(account.id);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const plan = getQoderCnPlanBadge(account);
      const planClass = resolveQoderCnPlanBadgeClass(plan);
      const isRefreshing = refreshing === account.id;
      const isInjecting = injecting === account.id;
      const quotaError = account.quota_query_last_error?.trim();

      return (
        <div key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`ghcp-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}>
          <div className="card-top">
            <div className="card-select">
              <input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} />
            </div>
            <span className="account-email" title={maskedEmail}>{maskedEmail}</span>
            {quotaError && (
              <span className="status-pill warning" title={quotaError}>
                <CircleAlert size={12} />
                {t('common.shared.quota.queryFailed', '配额查询失败')}
              </span>
            )}
            <span className={`tier-badge ${planClass} raw-value`}>{plan}</span>
            {isCurrent && <span className="current-tag">{t('accounts.status.current', '当前')}</span>}
          </div>
          <div className="account-sub-line qoder-account-subline">
            <span className="kiro-table-subline" title={formatDateTime(account.created_at)}>
              {formatDate(account.created_at)}
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
          {renderQuotaSection(account)}
          <div className="card-footer">
            <span className="card-date qoder-card-created-at" title={formatDateTime(account.created_at)}>
              {formatDate(account.created_at)}
            </span>
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)}
                title={t('dashboard.switch', '切换')} disabled={!!isInjecting || deleting}>
                {isInjecting ? <RotateCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="card-action-btn" onClick={() => openTagModal(account.id)}
                title={t('accounts.tagButton', '编辑标签')} disabled={!!isInjecting || deleting}>
                <Tag size={14} />
              </button>
              <button className="card-action-btn" onClick={() => handleRefresh(account.id)}
                title={t('common.refresh', '刷新')} disabled={isRefreshing || !!isInjecting || deleting}>
                <RefreshCw size={14} className={isRefreshing ? 'loading-spinner' : ''} />
              </button>
              <button className="card-action-btn export-btn" onClick={() => void handleExportByIds([account.id], getQoderCnAccountDisplayEmail(account))}
                title={t('accounts.actions.export', '导出')} disabled={exporting}>
                <Download size={14} />
              </button>
              <button className="card-action-btn danger" onClick={() => handleDelete(account.id)}
                title={t('accounts.actions.delete', '删除')} disabled={deleting}>
                <Trash2 size={14} />
              </button>
            </div>
          </div>
        </div>
      );
    });

  const renderTableRows = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const plan = getQoderCnPlanBadge(account);
      const planClass = resolveQoderCnPlanBadgeClass(plan);
      const quota = resolveQuotaDisplay(account);
      const isCurrent = currentAccountId === account.id;
      const isSelected = selected.has(account.id);
      const isRefreshing = refreshing === account.id;
      const isInjecting = injecting === account.id;
      const quotaError = account.quota_query_last_error?.trim();
      return (
        <tr key={groupKey ? `${groupKey}-${account.id}` : account.id} className={isCurrent ? 'current' : undefined}>
          <td><input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} /></td>
          <td title={maskAccountText(getQoderCnAccountDisplayEmail(account))}>
            <div className="account-cell">
              <div className="account-main-line">{maskAccountText(getQoderCnAccountDisplayEmail(account))}</div>
              {quotaError && (
                <div className="account-sub-line">
                  <span className="status-pill warning" title={quotaError}>
                    <CircleAlert size={12} />{t('common.shared.quota.queryFailed', '配额查询失败')}
                  </span>
                </div>
              )}
            </div>
          </td>
          <td>{maskAccountText(account.user_id || '--')}</td>
          <td><span className={`tier-badge raw-value ${planClass}`}>{plan}</span></td>
          <td>
            <div className="qoder-table-quota">
              {quota.items.map((item) => (
                <div key={item.key} className={`quota-item qoder-table-quota-item ${item.showProgress ? '' : 'is-stat'}`}>
                  <div className="qoder-usage-summary-row">
                    <span className="qoder-usage-label-wrap">
                      <span className="quota-name qoder-usage-label">{item.label}</span>
                    </span>
                    {item.percentageText && <span className={`quota-value qoder-table-quota-pct ${item.quotaClass}`}>{item.percentageText}</span>}
                    <span className="windsurf-credit-left qoder-table-quota-total">{item.valueText}</span>
                  </div>
                  {item.showProgress && (
                    <div className="quota-progress-track">
                      <div className={`quota-progress-bar ${item.quotaClass}`} style={{ width: `${item.normalizedPercent}%` }} />
                    </div>
                  )}
                </div>
              ))}
              {quota.resetText && <div className="quota-reset qoder-table-reset">{quota.resetText}</div>}
            </div>
          </td>
          <td>{formatDateTime(account.created_at)}</td>
          <td>
            <div className="action-buttons">
              <button className="action-btn" onClick={() => handleRefresh(account.id)} disabled={isRefreshing || !!isInjecting || deleting}>
                <RefreshCw size={14} className={isRefreshing ? 'loading-spinner' : ''} />
              </button>
              <button className="action-btn" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!isInjecting || deleting}>
                {isInjecting ? <RotateCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="action-btn" onClick={() => openTagModal(account.id)} disabled={!!isInjecting || deleting}><Tag size={14} /></button>
              <button className="action-btn" onClick={() => void handleExportByIds([account.id])} disabled={exporting}><Upload size={14} /></button>
              <button className="action-btn danger" onClick={() => handleDelete(account.id)} disabled={deleting}><Trash2 size={14} /></button>
            </div>
          </td>
        </tr>
      );
    });

  return (
    <div className="ghcp-accounts-page qoder-accounts-page qoder-cn-accounts-page">
      <PlatformOverviewTabsHeader platform="qoder_cn" active={activeTab} onTabChange={setActiveTab} />

      {activeTab === 'instances' ? null : (
        <>
          <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
            <button type="button" className="ghcp-flow-notice-toggle"
              onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)} aria-expanded={!isFlowNoticeCollapsed}>
              <div className="ghcp-flow-notice-title">
                <CircleAlert size={16} />
                <span>{t('qoderCn.flowNotice.title', 'Qoder CN 账号管理说明（点击展开/收起）')}</span>
              </div>
              <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
            </button>
            {!isFlowNoticeCollapsed && (
              <div className="ghcp-flow-notice-body">
                <div className="ghcp-flow-notice-desc">
                  {t('qoderCn.flowNotice.desc', '切换账号将备份当前会话、关闭 Qoder CN 应用、恢复目标账号会话并重启应用。数据仅在本地处理。')}
                </div>
                <ul className="ghcp-flow-notice-list">
                  <li>{t('qoderCn.flowNotice.permission', '权限范围：读取 Qoder CN 本地认证存储（auth.dat, auth-v2.dat），用于账号切换与会话备份恢复。')}</li>
                  <li>{t('qoderCn.flowNotice.network', '网络范围：OAuth 授权登录与 Token 刷新需联网请求 qoder.com.cn 与 openapi.qoder.com.cn；配额查询通过 OpenAPI 获取用量数据。不上传本地密钥或凭证。')}</li>
                </ul>
              </div>
            )}
          </div>

          {message && (
            <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
              {message.text}
              <button onClick={() => setMessage(null)} aria-label={t('common.close', '关闭')}><X size={14} /></button>
            </div>
          )}

          <div className="toolbar">
            <div className="toolbar-left">
              <div className="search-box">
                <Search size={16} className="search-icon" />
                <input type="text" value={searchQuery}
                  placeholder={t('qoderCn.search', '搜索 Qoder CN 账号...')}
                  onChange={(e) => setSearchQuery(e.target.value)} />
              </div>
              <div className="view-switcher">
                <button className={`view-btn ${viewMode === 'list' ? 'active' : ''}`} onClick={() => setViewMode('list')}
                  title={t('common.shared.view.list', '列表视图')}><List size={16} /></button>
                <button className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`} onClick={() => setViewMode('grid')}
                  title={t('common.shared.view.grid', '卡片视图')}><LayoutGrid size={16} /></button>
              </div>
              <MultiSelectFilterDropdown
                options={tierFilterOptions} selectedValues={filterTypes} allLabel={allFilterLabel}
                filterLabel={t('common.shared.filterLabel', '筛选')} clearLabel={t('accounts.clearFilter', '清空筛选')}
                emptyLabel={t('common.none', '暂无')} ariaLabel={t('common.shared.filterLabel', '筛选')}
                onToggleValue={toggleFilterTypeValue} onClear={clearFilterTypes} />
              <div className="tag-filter" ref={tagFilterRef}>
                <button type="button" className={`tag-filter-btn ${tagFilter.length > 0 ? 'active' : ''}`}
                  onClick={() => setShowTagFilter((prev) => !prev)}>
                  <Tag size={14} />
                  {tagFilter.length > 0
                    ? `${t('accounts.filterTags', '标签筛选')} (${tagFilter.length})`
                    : t('accounts.filterTags', '标签筛选')}
                </button>
                {showTagFilter && (
                  <div ref={page.tagFilterPanelRef}
                    className={`tag-filter-panel ${page.tagFilterPanelPlacement === 'top' ? 'open-top' : ''}`}>
                    {availableTags.length === 0 ? (
                      <div className="tag-filter-empty">{t('accounts.noTags', '暂无标签')}</div>
                    ) : (
                      <>
                        <div className="tag-filter-header">
                          <label className="group-toggle">
                            <input type="checkbox" checked={groupByTag} onChange={() => setGroupByTag(!groupByTag)} />
                            {t('accounts.groupByTag', '按标签分组')}
                          </label>
                          {tagFilter.length > 0 && <button className="tag-filter-clear" onClick={clearTagFilter}>{t('common.shared.clear', '清除')}</button>}
                        </div>
                        <div className="tag-filter-list" style={page.tagFilterScrollContainerStyle}>
                          {availableTags.map((tag) => (
                            <label key={tag} className="tag-filter-item">
                              <input type="checkbox" checked={tagFilter.includes(tag)} onChange={() => toggleTagFilterValue(tag)} />
                              <span>{tag}</span>
                            </label>
                          ))}
                        </div>
                      </>
                    )}
                  </div>
                )}
              </div>
              <SingleSelectFilterDropdown
                value={sortBy}
                options={[
                  { value: 'created_at', label: t('accounts.sort.createdAt') },
                  { value: 'plan', label: t('accounts.sort.plan') },
                  { value: 'quota', label: t('accounts.sort.quota') },
                ]}
                ariaLabel={t('common.shared.sortLabel', '排序')}
                icon={<ArrowDownWideNarrow size={14} />}
                onChange={(value) => page.setSortBy(value)}
              />
              <button className="sort-direction-btn"
                onClick={() => page.setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
                title={sortDirection === 'desc'
                  ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序')
                  : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序')}>
                {sortDirection === 'desc' ? '⬇' : '⬆'}
              </button>
            </div>
            <div className="toolbar-right">
              <button className="btn btn-primary icon-only" onClick={() => openAddModal('oauth')}
                title={t('common.shared.addAccount')}><Plus size={14} /></button>
              <button className="btn btn-secondary icon-only" onClick={handleRefreshAll}
                disabled={refreshingAll || accounts.length === 0} title={t('accounts.actions.refreshAll', '刷新全部')}>
                <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
              </button>
              <button className="btn btn-secondary icon-only" onClick={togglePrivacyMode}
                title={privacyModeEnabled ? t('accounts.privacy.disable', '关闭隐私模式') : t('accounts.privacy.enable', '开启隐私模式')}>
                {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
              <button className="btn btn-secondary icon-only" onClick={() => openAddModal('token')}
                title={t('common.shared.import.label', '导入')}><Download size={14} /></button>
              <button className="btn btn-secondary export-btn icon-only" onClick={() => void handleExport(filteredIds)}
                disabled={exporting || filteredIds.length === 0}
                title={exportSelectionCount > 0 ? `${t('accounts.actions.export', '导出')} (${exportSelectionCount})` : t('accounts.actions.export', '导出')}>
                <Upload size={14} />
              </button>
              <QuickSettingsPopover type="qoder_cn" />
            </div>
          </div>

          {filteredAccounts.length > 0 && (
            <AccountSelectionToolbar
              selectedCount={selected.size} allSelected={isAllPaginatedSelected}
              disabled={paginatedIds.length === 0}
              onToggleSelectAll={() => toggleSelectAll(paginatedIds)}
              onClearSelection={() => toggleSelectAll(Array.from(selected))}
              actions={(
                <button className="btn btn-danger icon-only" onClick={handleBatchDelete}
                  disabled={deleting} title={`${t('common.delete', '删除')} (${selected.size})`}>
                  <Trash2 size={14} />
                </button>
              )}
            />
          )}

          {loading && accounts.length === 0 ? (
            <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
          ) : accounts.length === 0 ? (
            <div className="empty-state">
              <h3>{t('accounts.empty.title', '暂无账号')}</h3>
              <p>{t('qoderCn.empty.desc', '点击"添加账号"，可使用授权登录、本机导入或 JSON 导入。')}</p>
              <button className="btn btn-primary" onClick={() => openAddModal('oauth')}>
                <Plus size={16} />{t('common.shared.addAccount')}
              </button>
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
                        <span className="tag-group-title">{groupKey === '__untagged__' ? t('accounts.defaultGroup', '默认分组') : groupKey}</span>
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
                    <th><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
                    <th>{t('common.shared.columns.account')}</th>
                    <th>{t('common.shared.columns.userId', '用户 ID')}</th>
                    <th>{t('common.shared.columns.plan', '套餐')}</th>
                    <th>{t('instances.labels.quota', '配额')}</th>
                    <th>{t('common.shared.columns.createdAt')}</th>
                    <th>{t('common.shared.columns.actions')}</th>
                  </tr>
                </thead>
                <tbody>
                  {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                    <Fragment key={groupKey}>
                      <tr className="tag-group-row"><td colSpan={7}><div className="tag-group-header"><span className="tag-group-title">{groupKey === '__untagged__' ? t('accounts.defaultGroup', '默认分组') : groupKey}</span><span className="tag-group-count">{totalCount}</span></div></td></tr>
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
                    <th><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
                    <th>{t('common.shared.columns.account')}</th>
                    <th>{t('common.shared.columns.userId', '用户 ID')}</th>
                    <th>{t('common.shared.columns.plan', '套餐')}</th>
                    <th>{t('instances.labels.quota', '配额')}</th>
                    <th>{t('common.shared.columns.createdAt')}</th>
                    <th>{t('common.shared.columns.actions')}</th>
                  </tr>
                </thead>
                <tbody>{renderTableRows(paginatedAccounts)}</tbody>
              </table>
            </div>
          )}

          <PaginationControls
            totalItems={pagination.totalItems} currentPage={pagination.currentPage}
            totalPages={pagination.totalPages} pageSize={pagination.pageSize}
            pageSizeOptions={pagination.pageSizeOptions}
            rangeStart={pagination.rangeStart} rangeEnd={pagination.rangeEnd}
            canGoPrevious={pagination.canGoPrevious} canGoNext={pagination.canGoNext}
            onPageSizeChange={pagination.setPageSize}
            onPreviousPage={pagination.goToPreviousPage} onNextPage={pagination.goToNextPage}
          />
        </>
      )}

      {showAddModal && (
        <div className="modal-overlay">
          <div className="modal-content codex-add-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={closeAddModal} title={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{t('qoderCn.addModal.title', '添加 Qoder CN 账号')}</h2>
              <button className="modal-close" onClick={closeAddModal}><X /></button>
            </div>
            <div className="modal-tabs">
              <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}>
                <Globe size={14} />{t('common.shared.addModal.oauth', '授权登录')}
              </button>
              <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}>
                <KeyRound size={14} />{t('common.shared.addModal.token', 'Token / JSON')}
              </button>
              <button className={`modal-tab ${addTab === 'import' ? 'active' : ''}`} onClick={() => openAddModal('import')}>
                <Database size={14} />{t('accounts.tabs.import', '导入')}
              </button>
            </div>
            <div className="modal-body">
              <MfaQuickCodeSelect />
              {addTab === 'oauth' ? (
                <div className="add-section">
                  <p className="section-desc">{t('qoderCn.oauthDesc', '点击下方按钮，在浏览器中完成 Qoder CN 账号 OAuth 授权。')}</p>
                  {oauthPrepareError ? (
                    <div className="add-status error">
                      <CircleAlert size={16} /><span>{oauthPrepareError}</span>
                      <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>{t('common.shared.oauth.retry', '重新生成授权信息')}</button>
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
                        <p className="oauth-hint">{t('common.shared.oauth.meta', '授权有效期：{{expires}}s；轮询间隔：{{interval}}s', { expires: oauthMeta.expiresIn, interval: oauthMeta.intervalSeconds })}</p>
                      )}
                      <button className="btn btn-primary btn-full" onClick={handleOpenOauthUrl}>
                        <Globe size={16} />{t('common.shared.oauth.openBrowser', '在浏览器中打开')}
                      </button>
                      {oauthPolling && (
                        <div className="add-status loading">
                          <RefreshCw size={16} className="loading-spinner" />
                          <span>{t('qoderCn.oauthWaiting', '等待授权完成...')}</span>
                        </div>
                      )}
                      {oauthCompleteError && (
                        <div className="add-status error">
                          <CircleAlert size={16} /><span>{oauthCompleteError}</span>
                          {oauthTimedOut && <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>{t('common.shared.oauth.timeoutRetry', '刷新授权链接')}</button>}
                        </div>
                      )}
                      <p className="oauth-hint">{t('common.shared.oauth.hint', '完成授权后，此窗口将自动更新')}</p>
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
                  <p className="section-desc">{t('qoderCn.tokenDesc', '粘贴 Qoder CN 的 access token：')}</p>
                  <textarea className="token-input" value={tokenInput} onChange={(e) => setTokenInput(e.target.value)}
                    placeholder={t('common.shared.token.placeholder', '粘贴 Token 或 JSON...')} />
                  <button className="btn btn-primary btn-full" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                    {t('common.shared.token.import', '导入')}
                  </button>
                </div>
              ) : (
                <div className="add-section">
                  <p className="section-desc">{t('qoderCn.import.localDesc', '支持从本机 Qoder CN 客户端或 JSON 文件导入账号数据。')}</p>
                  <button className="btn btn-secondary btn-full" onClick={() => handleImportFromLocal?.()} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}
                    {t('qoderCn.import.localClient', '从本机 Qoder CN 导入')}
                  </button>
                  <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>{t('common.shared.import.orJson', '或从 JSON 文件导入')}</div>
                  <input ref={importFileInputRef} type="file" accept=".json,application/json" style={{ display: 'none' }}
                    onChange={(e) => { const file = e.target.files?.[0]; e.target.value = ''; if (!file) return; void handleImportJsonFile(file); }} />
                  <button className="btn btn-primary btn-full" onClick={handlePickImportFile} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Upload size={16} />}
                    {t('common.shared.import.pickFile', '选择 JSON 文件导入')}
                  </button>
                </div>
              )}
              {addStatus !== 'idle' && addStatus !== 'loading' && addMessage && (
                <div className={`add-status ${addStatus}`}>
                  {addStatus === 'success' ? <Check size={16} /> : <CircleAlert size={16} />}
                  <span>{addMessage}</span>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {deleteConfirm && (
        <div className="modal-overlay">
          <div className="modal confirm-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirmDelete', '确认删除')}</h2>
              <button className="modal-close" onClick={() => !deleting && setDeleteConfirm(null)}><X /></button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
              <p>{deleteConfirm.message}</p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>{t('common.cancel', '取消')}</button>
              <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>{deleting ? t('common.processing', '处理中...') : t('common.confirm', '确认')}</button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div className="modal-overlay">
          <div className="modal confirm-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirmDeleteTag', '确认删除标签')}</h2>
              <button className="modal-close" onClick={() => !deletingTag && setTagDeleteConfirm(null)}><X /></button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} />
              <p>{t('accounts.confirmDeleteTag', '删除标签 "{{tag}}"？将从 {{count}} 个账号中移除。', { tag: tagDeleteConfirm.tag, count: tagDeleteConfirm.count })}</p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setTagDeleteConfirm(null)} disabled={deletingTag}>{t('common.cancel', '取消')}</button>
              <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>{deletingTag ? t('common.processing', '处理中...') : t('common.confirm', '确认')}</button>
            </div>
          </div>
        </div>
      )}

      <ExportJsonModal
        isOpen={showExportModal} title={`${t('accounts.exportModal.title', '导出 JSON')}`}
        jsonContent={exportJsonContent} hidden={exportJsonHidden} copied={exportJsonCopied}
        saving={savingExportJson} savedPath={exportSavedPath}
        canOpenSavedDirectory={canOpenExportSavedDirectory} pathCopied={exportPathCopied}
        onClose={closeExportModal} onToggleHidden={toggleExportJsonHidden}
        onCopyJson={copyExportJson} onSaveJson={saveExportJson}
        onOpenSavedDirectory={openExportSavedDirectory} onCopySavedPath={copyExportSavedPath}
      />

      <TagEditModal
        isOpen={!!showTagModal}
        initialTags={accounts.find((a) => a.id === showTagModal)?.tags || []}
        availableTags={availableTags}
        onClose={() => setShowTagModal(null)}
        onSave={handleSaveTags}
      />
    </div>
  );
}
