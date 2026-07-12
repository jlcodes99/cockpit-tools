import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  ArrowDownWideNarrow,
  ChevronDown,
  CircleAlert,
  Download,
  Eye,
  EyeOff,
  LayoutGrid,
  List,
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
import { PaginationControls } from '../components/PaginationControls';
import { AccountSelectionToolbar } from '../components/AccountSelectionToolbar';
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from '../components/MultiSelectFilterDropdown';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import { TagEditModal } from '../components/TagEditModal';
import { GrokOverviewTabsHeader } from '../components/GrokOverviewTabsHeader';
import { GrokIcon } from '../components/icons/GrokIcon';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import {
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from '../hooks/usePagination';
import * as grokService from '../services/grokService';
import { useGrokAccountStore } from '../stores/useGrokAccountStore';
import {
  formatGrokQuotaSummary,
  getGrokAccountDisplayEmail,
  getGrokPlanBadge,
  getGrokPlanBadgeClass,
  getGrokUsage,
  hasGrokQuotaData,
  type GrokAccount,
} from '../types/grok';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
} from '../utils/accountValidityFilter';
import {
  normalizeAccountsOverviewScope,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from '../utils/accountsOverviewFilterPersistence';
import './GrokAccountsPage.css';
import './ZedAccountsPage.css';

const GROK_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.grok.flow_notice_collapsed';
const GROK_CURRENT_ACCOUNT_ID_KEY = 'agtools.grok.current_account_id';
const GROK_FILTER_SCOPE = normalizeAccountsOverviewScope('Grok');
const FILTER_TYPES_FIELD = 'filter_types';

function formatDateTime(timestamp?: number | null, locale = 'zh-CN'): string {
  if (!timestamp) return '--';
  const date = new Date(timestamp * 1000);
  if (Number.isNaN(date.getTime())) return '--';
  return date.toLocaleString(locale);
}

function getQuotaTone(usedPercent: number): string {
  if (usedPercent >= 90) return 'critical';
  if (usedPercent >= 70) return 'low';
  if (usedPercent >= 40) return 'medium';
  return 'high';
}

export function GrokAccountsPage() {
  const { t, i18n } = useTranslation();
  const store = useGrokAccountStore();
  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    readAccountsOverviewFilterPersistenceEnabled(GROK_FILTER_SCOPE)
      ? readAccountsOverviewFilterStringArray(GROK_FILTER_SCOPE, FILTER_TYPES_FIELD)
      : [],
  );
  const [jsonPaste, setJsonPaste] = useState('');
  const [jsonImporting, setJsonImporting] = useState(false);

  const page = useProviderAccountsPage<GrokAccount>({
    platformKey: 'Grok',
    oauthLogPrefix: 'GrokOAuth',
    flowNoticeCollapsedKey: GROK_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: GROK_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'grok_accounts',
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
      switchAccount: store.switchAccount,
    },
    oauthService: {
      startLogin: grokService.startGrokOAuthLogin,
      completeLogin: grokService.completeGrokOAuthLogin,
      cancelLogin: grokService.cancelGrokOAuthLogin,
    },
    dataService: {
      importFromJson: grokService.importGrokFromJson,
      importFromLocal: grokService.importGrokFromLocal,
      exportAccounts: grokService.exportGrokAccounts,
      injectToVSCode: async (accountId: string) => {
        await grokService.switchGrokAccount(accountId);
      },
    },
    getDisplayEmail: getGrokAccountDisplayEmail,
    onInjectSuccess: async () => {
      await store.fetchCurrentAccountId();
      await store.fetchAccounts();
    },
    resolveOauthSuccessMessage: () =>
      t('grok.oauth.success', '授权成功，账号已导入并可切换到 Grok CLI'),
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
    selected,
    toggleSelect,
    toggleSelectAll,
    openTagModal,
    handleSaveTags,
    showTagModal,
    setShowTagModal,
    refreshing,
    refreshingAll,
    injecting,
    handleRefresh,
    handleRefreshAll,
    handleDelete,
    handleBatchDelete,
    deleteConfirm,
    setDeleteConfirm,
    confirmDelete,
    message,
    setMessage,
    exporting,
    handleExport,
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
    openAddModal,
    closeAddModal,
    addTab,
    setAddTab,
    addStatus,
    addMessage,
    importing,
    handleImportFromLocal,
    handleImportJsonFile,
    handlePickImportFile,
    importFileInputRef,
    oauthUrl,
    oauthUrlCopied,
    oauthUserCode,
    oauthPrepareError,
    oauthCompleteError,
    oauthPolling,
    handleCopyOauthUrl,
    handleCopyOauthUserCode,
    handleRetryOauth,
    handleOpenOauthUrl,
    handleInjectToVSCode,
    isFlowNoticeCollapsed,
    setIsFlowNoticeCollapsed,
    currentAccountId,
    formatDate,
    filterPersistenceEnabled,
    sortBy,
    setSortBy,
    sortDirection,
    setSortDirection,
  } = page;

  useEffect(() => {
    void store.fetchAccounts();
    void store.fetchCurrentAccountId();
  }, [store.fetchAccounts, store.fetchCurrentAccountId]);

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(GROK_FILTER_SCOPE, FILTER_TYPES_FIELD);
      return;
    }
    writeAccountsOverviewFilterField(GROK_FILTER_SCOPE, FILTER_TYPES_FIELD, filterTypes);
  }, [filterTypes, filterPersistenceEnabled]);

  const accounts = store.accounts;
  const uiLocale = i18n.language || locale || 'zh-CN';

  const toggleFilterTypeValue = useCallback((value: string) => {
    setFilterTypes((prev) =>
      prev.includes(value) ? prev.filter((item) => item !== value) : [...prev, value],
    );
  }, []);

  const clearFilterTypes = useCallback(() => setFilterTypes([]), []);

  const planOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const counts = new Map<string, number>();
    accounts.forEach((account) => {
      const key = getGrokPlanBadge(account);
      counts.set(key, (counts.get(key) ?? 0) + 1);
    });
    const validCount = accounts.filter((a) => a.status !== 'error' && !a.requires_reauth).length;
    return [
      ...Array.from(counts.entries())
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([plan, count]) => ({ value: plan, label: `${plan} (${count})` })),
      buildValidAccountsFilterOption(t, validCount),
    ];
  }, [accounts, t]);

  const filteredAccounts = useMemo(() => {
    let list = [...accounts];
    const q = searchQuery.trim().toLowerCase();
    if (q) {
      list = list.filter((account) => {
        const hay = [
          getGrokAccountDisplayEmail(account),
          getGrokPlanBadge(account),
          account.user_id,
          ...(account.tags || []),
        ]
          .filter(Boolean)
          .join(' ')
          .toLowerCase();
        return hay.includes(q);
      });
    }
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        list = list.filter((a) => a.status !== 'error' && !a.requires_reauth);
      }
      if (selectedTypes.size > 0) {
        list = list.filter((a) => selectedTypes.has(getGrokPlanBadge(a)));
      }
    }
    list.sort((left, right) =>
      compareCurrentAccountFirst(left.id, right.id, currentAccountId, () => {
        if (sortBy === 'created_at') {
          const diff = right.created_at - left.created_at;
          return sortDirection === 'desc' ? diff : -diff;
        }
        const leftUsed = left.quota?.usage_percent ?? -1;
        const rightUsed = right.quota?.usage_percent ?? -1;
        const diff = rightUsed - leftUsed;
        return sortDirection === 'desc' ? diff : -diff;
      }),
    );
    return list;
  }, [accounts, currentAccountId, filterTypes, searchQuery, sortBy, sortDirection]);

  const filteredIds = useMemo(() => filteredAccounts.map((a) => a.id), [filteredAccounts]);
  const pagination = usePagination(filteredAccounts.length, {
    storageKey: buildPaginationPageSizeStorageKey('grok-accounts'),
  });
  const paginatedAccounts = useMemo(() => {
    const start = (pagination.page - 1) * pagination.pageSize;
    return filteredAccounts.slice(start, start + pagination.pageSize);
  }, [filteredAccounts, pagination.page, pagination.pageSize]);
  const paginatedIds = useMemo(() => paginatedAccounts.map((a) => a.id), [paginatedAccounts]);
  const isAllPaginatedSelected = isEveryIdSelected(paginatedIds, Array.from(selected));

  const buildUsagePanel = useCallback(
    (account: GrokAccount) => {
      if (!hasGrokQuotaData(account)) {
        return {
          note: t('common.shared.quota.noData', '暂无配额数据'),
          items: [] as Array<{
            key: string;
            label: string;
            value: string;
            usedText: string;
            leftText: string;
            progressPercent: number;
            tone: string;
            title: string;
          }>,
          title: t('common.shared.quota.noData', '暂无配额数据'),
        };
      }
      const usage = getGrokUsage(account);
      const usedPct = usage.totalPercentUsed ?? 0;
      const q = account.quota!;
      const used = q.used ?? 0;
      const limit = q.monthly_limit ?? 0;
      const remaining = q.remaining ?? Math.max(0, limit - used);
      const tone = getQuotaTone(usedPct);
      const updatedText = account.usage_updated_at
        ? t('grok.page.usageUpdatedAt', {
            time: formatDateTime(account.usage_updated_at, uiLocale),
            defaultValue: '更新于 {{time}}',
          })
        : '';
      return {
        note: updatedText,
        items: [
          {
            key: 'monthly',
            label: t('grok.quota.monthly', '月额度'),
            value: `${Math.round(usedPct)}%`,
            usedText: `${Math.round(used)} / ${Math.round(limit)} used`,
            leftText: `${Math.round(remaining)} left`,
            progressPercent: Math.max(0, Math.min(100, usedPct)),
            tone,
            title: formatGrokQuotaSummary(account),
          },
        ],
        title: formatGrokQuotaSummary(account),
      };
    },
    [t, uiLocale],
  );

  const renderUsagePanel = (
    panel: ReturnType<typeof buildUsagePanel>,
    options?: { compact?: boolean },
  ) => (
    <div
      className={`windsurf-official-usage ${options?.compact ? 'compact' : ''}`}
      title={panel.title}
    >
      {panel.note ? <div className="windsurf-official-usage-note">{panel.note}</div> : null}
      <div className="windsurf-official-usage-list">
        {panel.items.length === 0 ? (
          <div className="windsurf-official-usage-item">
            <div className="windsurf-official-usage-main">
              <span className="windsurf-official-usage-label">{panel.note || '--'}</span>
            </div>
          </div>
        ) : (
          panel.items.map((item) => (
            <div
              key={item.key}
              className="windsurf-official-usage-item zed-usage-metric-item"
              title={item.title}
            >
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
            </div>
          ))
        )}
      </div>
      {accountPeriodEnd(panel) ? (
        <div className="windsurf-plan-cycle compact">
          <span className="windsurf-plan-cycle-summary">{accountPeriodEnd(panel)}</span>
        </div>
      ) : null}
    </div>
  );

  function accountPeriodEnd(_panel: ReturnType<typeof buildUsagePanel>) {
    return '';
  }

  const renderGridCards = (items: GrokAccount[]) =>
    items.map((account) => {
      const isCurrent = currentAccountId === account.id;
      const isSelected = selected.has(account.id);
      const emailText = maskAccountText(getGrokAccountDisplayEmail(account));
      const planBadge = getGrokPlanBadge(account);
      const planClass = getGrokPlanBadgeClass(account);
      const usagePanel = buildUsagePanel(account);
      const period =
        account.quota?.billing_period_end != null
          ? t('grok.quota.periodEnd', '账期结束') + ': ' + account.quota.billing_period_end
          : t('common.shared.credits.planEndsUnknown', '配额周期时间未知');

      return (
        <div
          key={account.id}
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
            <span className="account-email" title={getGrokAccountDisplayEmail(account)}>
              {emailText}
            </span>
            {isCurrent ? (
              <span className="current-tag">{pageT('accounts.status.current', '当前')}</span>
            ) : null}
            {account.requires_reauth || account.status === 'error' ? (
              <span className="status-pill warning" title={account.status_reason || ''}>
                {pageT('accounts.status.error', '异常')}
              </span>
            ) : null}
            <span className={`tier-badge ${planClass}`}>{planBadge}</span>
          </div>

          <div className="grok-status-row zed-status-row">
            <span className="grok-brand-chip">
              <GrokIcon size={14} />
              Grok CLI
            </span>
            {account.has_grok_code_access != null ? (
              <span
                className={`status-pill ${account.has_grok_code_access ? 'normal' : 'warning'}`}
              >
                {account.has_grok_code_access
                  ? t('grok.codeAccess.yes', 'Grok Code 可用')
                  : t('grok.codeAccess.no', '无 Code 权限')}
              </span>
            ) : null}
            {account.tier != null ? <span className="tag-pill">Tier {account.tier}</span> : null}
          </div>

          {renderUsagePanel(usagePanel, { compact: true })}
          <div className="windsurf-plan-cycle compact" title={period}>
            <span className="windsurf-plan-cycle-summary">{period}</span>
          </div>

          {(account.tags || []).length > 0 ? (
            <div className="card-tags">
              {(account.tags || []).slice(0, 4).map((tag) => (
                <span key={`${account.id}-${tag}`} className="tag-pill">
                  {tag}
                </span>
              ))}
            </div>
          ) : null}

          <div className="card-footer">
            <span className="card-date">
              {formatDate(account.last_used || account.created_at)}
            </span>
            <div className="card-actions">
              <button
                type="button"
                className="card-action-btn success"
                disabled={isCurrent || injecting === account.id}
                onClick={() => void handleInjectToVSCode?.(account.id)}
              >
                {pageT('accounts.actions.switch', '切换')}
              </button>
              <button
                type="button"
                className="card-action-btn"
                disabled={refreshing === account.id}
                onClick={() => void handleRefresh(account.id)}
                title={pageT('accounts.actions.refresh', '刷新')}
              >
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button
                type="button"
                className="card-action-btn"
                onClick={() => openTagModal(account.id)}
                title={pageT('accounts.actions.tags', '标签')}
              >
                <Tag size={14} />
              </button>
              <button
                type="button"
                className="card-action-btn danger"
                onClick={() => handleDelete(account.id)}
                title={pageT('common.delete', '删除')}
              >
                <Trash2 size={14} />
              </button>
            </div>
          </div>
        </div>
      );
    });

  return (
    <div className="ghcp-accounts-page zed-accounts-page grok-accounts-page">
      <GrokOverviewTabsHeader />

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
            <span>{t('grok.flowNotice.title', 'Grok CLI 账号接入说明（点击展开/收起）')}</span>
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
                'grok.flowNotice.desc',
                '支持 Device OAuth 登录、本机 ~/.grok/auth.json 导入、JSON 导入，以及一键切号写回官方 Grok CLI 登录态。额度通过 cli-chat-proxy 实时查询。',
              )}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>
                {t(
                  'grok.flowNotice.reason',
                  '切号会原子写入 auth.json（支持 GROK_HOME；兼容 Windows / macOS / Linux）。官方 CLI 会自动热加载新凭据。',
                )}
              </li>
              <li>
                {t(
                  'grok.flowNotice.storage',
                  'refresh_token 单次有效：刷新后会同时回写 Cockpit 存储与当前 auth.json，避免 CLI 失效。仅本地保存账号索引与标签。',
                )}
              </li>
            </ul>
          </div>
        )}
      </div>

      {message && (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
          {message.text}
          <button type="button" onClick={() => setMessage(null)}>
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
              placeholder={t('grok.page.searchPlaceholder', '搜索账号、套餐或标签')}
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
            />
          </div>

          <div className="view-switcher">
            <button
              type="button"
              className={`view-btn ${viewMode === 'list' ? 'active' : ''}`}
              onClick={() => setViewMode('list')}
              title={t('common.shared.view.list', '列表视图')}
            >
              <List size={16} />
            </button>
            <button
              type="button"
              className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`}
              onClick={() => setViewMode('grid')}
              title={t('common.shared.view.grid', '卡片视图')}
            >
              <LayoutGrid size={16} />
            </button>
          </div>

          <MultiSelectFilterDropdown
            options={planOptions}
            selectedValues={filterTypes}
            allLabel={`ALL (${accounts.length})`}
            filterLabel={t('common.shared.filterLabel', '筛选')}
            clearLabel={t('accounts.clearFilter', '清空筛选')}
            emptyLabel={t('common.none', '暂无')}
            ariaLabel={t('common.shared.filterLabel', '筛选')}
            onToggleValue={toggleFilterTypeValue}
            onClear={clearFilterTypes}
          />

          <SingleSelectFilterDropdown
            value={sortBy}
            options={[
              { value: 'created_at', label: t('common.shared.sort.createdAt', '按创建时间') },
              { value: 'usage', label: t('grok.sort.usage', '按额度使用率') },
            ]}
            ariaLabel={t('common.shared.sortLabel', '排序')}
            icon={<ArrowDownWideNarrow size={14} />}
            onChange={setSortBy}
          />

          <button
            type="button"
            className="sort-direction-btn"
            onClick={() => setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
            title={
              sortDirection === 'desc'
                ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序')
                : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序')
            }
          >
            {sortDirection === 'desc' ? '⬇' : '⬆'}
          </button>
        </div>

        <div className="toolbar-right">
          <button
            type="button"
            className="btn btn-primary icon-only"
            onClick={() => openAddModal('oauth')}
            title={t('grok.actions.addAccount', '登录 Grok')}
          >
            <Plus size={14} />
          </button>
          <button
            type="button"
            className="btn btn-secondary icon-only"
            onClick={() => void handleRefreshAll()}
            disabled={refreshingAll || accounts.length === 0}
            title={t('common.shared.refreshAll', '刷新全部')}
          >
            <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
          </button>
          <button
            type="button"
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
            type="button"
            className="btn btn-secondary icon-only"
            onClick={() => openAddModal('local')}
            disabled={importing}
            title={t('grok.actions.importLocal', '导入本机')}
          >
            <Download size={14} />
          </button>
          <button
            type="button"
            className="btn btn-secondary export-btn icon-only"
            onClick={() => void handleExport(filteredIds)}
            disabled={exporting || filteredIds.length === 0}
            title={t('common.shared.export.title', '导出')}
          >
            <Upload size={14} />
          </button>
          <QuickSettingsPopover type="grok" />
        </div>
      </div>

      {filteredAccounts.length > 0 && (
        <AccountSelectionToolbar
          selectedCount={selected.size}
          allSelected={isAllPaginatedSelected}
          disabled={paginatedIds.length === 0}
          onToggleSelectAll={() => toggleSelectAll(paginatedIds)}
          onClearSelection={() => toggleSelectAll(Array.from(selected))}
          actions={
            <button
              type="button"
              className="btn btn-danger icon-only"
              onClick={handleBatchDelete}
              title={`${t('common.delete', '删除')} (${selected.size})`}
            >
              <Trash2 size={14} />
            </button>
          }
        />
      )}

      {store.loading && accounts.length === 0 ? (
        <div className="empty-state">{t('common.loading', '加载中…')}</div>
      ) : filteredAccounts.length === 0 ? (
        <div className="empty-state">
          {t('grok.empty', '暂无 Grok 账号。可从本机 auth.json 导入，或 Device OAuth 登录。')}
        </div>
      ) : viewMode === 'grid' ? (
        <div className="ghcp-accounts-grid">{renderGridCards(paginatedAccounts)}</div>
      ) : (
        <div className="accounts-table-wrap">
          <table className="accounts-table">
            <thead>
              <tr>
                <th />
                <th>{t('accounts.table.email', '账号')}</th>
                <th>{t('accounts.table.plan', '套餐')}</th>
                <th>{t('accounts.table.quota', '额度')}</th>
                <th>{t('accounts.table.updated', '更新')}</th>
                <th>{t('accounts.table.actions', '操作')}</th>
              </tr>
            </thead>
            <tbody>
              {paginatedAccounts.map((account) => {
                const isCurrent = currentAccountId === account.id;
                return (
                  <tr key={account.id} className={isCurrent ? 'current' : ''}>
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
                          {maskAccountText(getGrokAccountDisplayEmail(account))}
                          {isCurrent ? (
                            <span className="current-tag">
                              {pageT('accounts.status.current', '当前')}
                            </span>
                          ) : null}
                        </div>
                        <div className="account-sub-line">
                          <span className="grok-brand-chip">
                            <GrokIcon size={12} />
                            Tier {account.tier ?? '-'}
                          </span>
                        </div>
                      </div>
                    </td>
                    <td>
                      <span className={`tier-badge ${getGrokPlanBadgeClass(account)}`}>
                        {getGrokPlanBadge(account)}
                      </span>
                    </td>
                    <td>{formatGrokQuotaSummary(account)}</td>
                    <td>
                      {formatDate(
                        account.usage_updated_at || account.token_updated_at || account.last_used,
                      )}
                    </td>
                    <td>
                      <div className="action-buttons">
                        <button
                          type="button"
                          className="btn btn-sm btn-primary"
                          disabled={isCurrent}
                          onClick={() => void handleInjectToVSCode?.(account.id)}
                        >
                          {pageT('accounts.actions.switch', '切换')}
                        </button>
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={() => void handleRefresh(account.id)}
                        >
                          {pageT('accounts.actions.refresh', '刷新')}
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <PaginationControls
        page={pagination.page}
        pageSize={pagination.pageSize}
        total={filteredAccounts.length}
        onPageChange={pagination.setPage}
        onPageSizeChange={pagination.setPageSize}
      />

      {showAddModal ? (
        <div className="modal-overlay" onClick={closeAddModal}>
          <div
            className="modal-content ghcp-add-modal zed-add-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h3>{t('grok.addModal.title', '添加 Grok 账号')}</h3>
              <button type="button" className="icon-btn" onClick={closeAddModal}>
                <X size={16} />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-tabs">
                <button
                  type="button"
                  className={addTab === 'oauth' ? 'active' : ''}
                  onClick={() => setAddTab('oauth')}
                >
                  {t('grok.addModal.oauthTab', 'OAuth 登录')}
                </button>
                <button
                  type="button"
                  className={addTab === 'local' ? 'active' : ''}
                  onClick={() => setAddTab('local')}
                >
                  {t('grok.addModal.local', '本机导入')}
                </button>
                <button
                  type="button"
                  className={addTab === 'import' || addTab === 'json' ? 'active' : ''}
                  onClick={() => setAddTab('import')}
                >
                  JSON
                </button>
              </div>

              {addTab === 'oauth' ? (
                <div className="oauth-panel">
                  <p>
                    {t(
                      'grok.oauth.hint',
                      '在线 Device OAuth：打开 xAI 授权页登录 SuperGrok，授权完成后自动导入账号、刷新额度并切到 Grok CLI。',
                    )}
                  </p>
                  {oauthUrl ? (
                    <>
                      {oauthUserCode ? (
                        <div className="oauth-user-code-box" style={{ margin: '12px 0' }}>
                          <div style={{ fontSize: 12, opacity: 0.75 }}>
                            {t('grok.oauth.userCodeLabel', '设备码（在授权页输入）')}
                          </div>
                          <div
                            style={{
                              fontSize: 28,
                              fontWeight: 800,
                              letterSpacing: '0.12em',
                              fontFamily: 'var(--font-mono)',
                            }}
                          >
                            {oauthUserCode}
                          </div>
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={handleCopyOauthUserCode}
                            style={{ marginTop: 8 }}
                          >
                            {t('common.copy', '复制设备码')}
                          </button>
                        </div>
                      ) : null}
                      <a href={oauthUrl} target="_blank" rel="noreferrer">
                        {oauthUrl}
                      </a>
                      <div className="modal-actions">
                        <button type="button" className="btn" onClick={handleCopyOauthUrl}>
                          {oauthUrlCopied
                            ? t('common.copied', '已复制')
                            : t('common.copyLink', '复制链接')}
                        </button>
                        <button
                          type="button"
                          className="btn btn-primary"
                          onClick={handleOpenOauthUrl}
                        >
                          {t('grok.oauth.open', '打开在线授权页')}
                        </button>
                        {oauthPolling ? (
                          <span>{t('grok.oauth.polling', '等待授权中…自动轮询完成')}</span>
                        ) : (
                          <button type="button" className="btn" onClick={handleRetryOauth}>
                            {t('common.retry', '重新发起授权')}
                          </button>
                        )}
                      </div>
                    </>
                  ) : (
                    <button type="button" className="btn btn-primary" onClick={handleRetryOauth}>
                      {t('grok.oauth.start', '开始在线授权')}
                    </button>
                  )}
                  {oauthPrepareError || oauthCompleteError ? (
                    <ModalErrorMessage message={oauthPrepareError || oauthCompleteError || ''} />
                  ) : null}
                </div>
              ) : null}

              {addTab === 'local' ? (
                <div>
                  <p>
                    {t(
                      'grok.addModal.localDesc',
                      '读取 ~/.grok/auth.json（或 GROK_HOME/auth.json）',
                    )}
                  </p>
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={importing}
                    onClick={() => void handleImportFromLocal()}
                  >
                    {t('grok.addModal.importLocal', '导入本机登录')}
                  </button>
                </div>
              ) : null}

              {addTab === 'import' || addTab === 'json' ? (
                <div>
                  <p>
                    {t(
                      'grok.addModal.jsonDesc',
                      '支持官方 ~/.grok/auth.json 整包、单条 entry，或 Cockpit 导出的账号 JSON。',
                    )}
                  </p>
                  <textarea
                    value={jsonPaste}
                    onChange={(e) => setJsonPaste(e.target.value)}
                    rows={10}
                    placeholder='{"https://auth.x.ai::client_id":{"key":"...","refresh_token":"...","email":"..."}}'
                    style={{ width: '100%', fontFamily: 'var(--font-mono)', fontSize: 12 }}
                  />
                  <div className="modal-actions" style={{ marginTop: 12 }}>
                    <input
                      ref={importFileInputRef}
                      type="file"
                      accept="application/json,.json"
                      hidden
                      onChange={(e) => {
                        const file = e.target.files?.[0];
                        if (file) void handleImportJsonFile(file);
                      }}
                    />
                    <button type="button" className="btn" onClick={handlePickImportFile}>
                      {t('grok.addModal.pickJson', '选择 JSON 文件')}
                    </button>
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={jsonImporting || !jsonPaste.trim()}
                      onClick={() => {
                        void (async () => {
                          setJsonImporting(true);
                          try {
                            const imported = await grokService.importGrokFromJson(jsonPaste);
                            await store.fetchAccounts();
                            setMessage({
                              tone: 'success',
                              text: t('grok.import.success', {
                                count: imported.length,
                                defaultValue: '已导入 {{count}} 个账号',
                              }),
                            });
                            setJsonPaste('');
                            closeAddModal();
                          } catch (error) {
                            setMessage({
                              tone: 'error',
                              text: error instanceof Error ? error.message : String(error),
                            });
                          } finally {
                            setJsonImporting(false);
                          }
                        })();
                      }}
                    >
                      {jsonImporting
                        ? t('common.importing', '导入中…')
                        : t('common.import', '导入 JSON')}
                    </button>
                  </div>
                </div>
              ) : null}

              {addStatus === 'error' && addMessage ? (
                <ModalErrorMessage message={addMessage} />
              ) : null}
              {addStatus === 'success' && addMessage ? (
                <div className="success-message">{addMessage}</div>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}

      {deleteConfirm ? (
        <div className="modal-overlay">
          <div className="modal-card">
            <div className="modal-header">
              <h3>{t('accounts.deleteConfirm.title', '确认删除')}</h3>
              <button type="button" className="icon-btn" onClick={() => setDeleteConfirm(null)}>
                <X size={16} />
              </button>
            </div>
            <div className="modal-body">
              <p>{t('accounts.deleteConfirm.body', '删除后不可恢复，确认删除所选账号？')}</p>
            </div>
            <div className="modal-actions">
              <button type="button" className="btn" onClick={() => setDeleteConfirm(null)}>
                {t('common.cancel', '取消')}
              </button>
              <button type="button" className="btn btn-danger" onClick={() => void confirmDelete()}>
                {t('common.delete', '删除')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {showTagModal ? (
        <TagEditModal
          isOpen={Boolean(showTagModal)}
          initialTags={
            accounts.find((account) => account.id === showTagModal)?.tags || []
          }
          resetKey={showTagModal}
          onClose={() => setShowTagModal(false)}
          onSave={(tags) => void handleSaveTags(tags)}
        />
      ) : null}

      {showExportModal ? (
        <ExportJsonModal
          isOpen={showExportModal}
          title={t('accounts.export.title', '导出账号 JSON')}
          jsonContent={exportJsonContent}
          hidden={exportJsonHidden}
          copied={exportJsonCopied}
          saving={savingExportJson || exporting}
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
      ) : null}
    </div>
  );
}
