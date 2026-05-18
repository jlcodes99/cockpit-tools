import { useMemo, useCallback, Fragment, useState, useEffect, type ComponentType } from 'react';
import {
  Plus, RefreshCw, Download, Upload, Trash2, X, Globe, KeyRound, Database,
  Copy, Check, RotateCw, LayoutGrid, List, Search,
  Tag, Play, Eye, EyeOff, CircleAlert, ChevronDown, ChevronLeft, ArrowRightLeft, CalendarCheck,
} from 'lucide-react';
import { TagEditModal } from '../TagEditModal';
import { ExportJsonModal } from '../ExportJsonModal';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { QuickSettingsPopover } from '../QuickSettingsPopover';
import { PaginationControls } from '../PaginationControls';
import { useEscClose } from '../../hooks/useEscClose';
import { useCodebuddySuitePage, formatQuotaNumber } from '../../hooks/useCodebuddySuitePage';
import type { UseProviderAccountsPageReturn } from '../../hooks/useProviderAccountsPage';
import {
  buildPaginatedGroups,
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from '../../hooks/usePagination';
import { KNOWN_PLAN_FILTERS } from './CodebuddySuiteConfig';
import { DosageNotifyUsageStatus } from '../platform/DosageNotifyUsageStatus';
import { CodeBuddyQuotaCategoryList } from '../codebuddy/CodeBuddyQuotaCategoryList';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../MultiSelectFilterDropdown';
import type { CodebuddySuiteAccountBase, QuotaCategoryGroup, CodebuddyUsage } from '../../types/codebuddy-suite';
import { buildValidAccountsFilterOption } from '../../utils/accountValidityFilter';
import {
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from '../../utils/accountsOverviewFilterPersistence';

const FILTER_TYPES_FIELD = 'filter_types';

interface CheckinModalProps<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  onClose: () => void;
  onCheckinComplete?: () => void;
}

export interface CodebuddySuiteAccountsPlatformConfig<TAccount extends CodebuddySuiteAccountBase> {
  pageClassName: string;
  quickSettingsType: 'codebuddy_cn' | 'workbuddy';
  searchPlaceholderKey: string;
  searchPlaceholderDefault: string;
  flowNotice: {
    titleKey: string;
    titleDefault: string;
    descKey: string;
    descDefault: string;
    permissionKey: string;
    permissionDefault: string;
    networkKey: string;
    networkDefault: string;
  };
  noAccountsKey: string;
  noAccountsDefault: string;
  addAccountTitleKey: string;
  addAccountTitleDefault: string;
  oauthDescKey: string;
  oauthDescDefault: string;
  oauthFeatureCardClassName: string;
  oauthFeatureTitleKey: string;
  oauthFeatureTitleDefault: string;
  oauthFeatureItem1Key: string;
  oauthFeatureItem1Default: string;
  oauthFeatureItem2Key: string;
  oauthFeatureItem2Default: string;
  oauthFeatureItem3Key: string;
  oauthFeatureItem3Default: string;
  oauthUrlInputPlaceholderKey: string;
  oauthUrlInputPlaceholderDefault: string;
  oauthWaitingKey: string;
  oauthWaitingDefault: string;
  tokenDescKey: string;
  tokenDescDefault: string;
  importLocalDescKey: string;
  importLocalDescDefault: string;
  importLocalClientKey: string;
  importLocalClientDefault: string;
  syncButtonTitle: (t: UseProviderAccountsPageReturn['t']) => string;
  syncSuccessMessage: (t: UseProviderAccountsPageReturn['t'], count: number) => string;
  syncFailedMessage: (t: UseProviderAccountsPageReturn['t'], error: string) => string;
  runSync: () => Promise<number>;
  getDisplayEmail: (account: TAccount) => string;
  getPlanBadge: (account: TAccount) => string;
  getUsage: (account: TAccount) => CodebuddyUsage;
  getQuotaGroups: (account: TAccount, t: (key: string, defaultValue?: string) => string) => QuotaCategoryGroup[];
  hasQuotaData: (account: TAccount, groups: QuotaCategoryGroup[]) => boolean;
  usagePrefix: 'codebuddy' | 'workbuddy';
  quotaPrefix: 'codebuddy' | 'workbuddy';
  tableUsageClassName: string;
  CheckinModal?: ComponentType<CheckinModalProps<TAccount>>;
}

interface CodebuddySuiteAccountsSharedViewProps<TAccount extends CodebuddySuiteAccountBase> {
  accounts: TAccount[];
  loading: boolean;
  page: UseProviderAccountsPageReturn;
  platformConfig: CodebuddySuiteAccountsPlatformConfig<TAccount>;
  onRefreshAccounts: () => void;
}

export function CodebuddySuiteAccountsSharedView<TAccount extends CodebuddySuiteAccountBase>({
  accounts,
  loading,
  page,
  platformConfig,
  onRefreshAccounts,
}: CodebuddySuiteAccountsSharedViewProps<TAccount>) {
  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    page.filterPersistenceEnabled
      ? readAccountsOverviewFilterStringArray(page.filterPersistenceScope, FILTER_TYPES_FIELD)
      : [],
  );
  const [syncing, setSyncing] = useState(false);
  const [syncMessage, setSyncMessage] = useState<string | null>(null);
  const [showCheckinModal, setShowCheckinModal] = useState(false);

  const {
    t, locale, privacyModeEnabled, togglePrivacyMode, maskAccountText,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    filterPersistenceEnabled, filterPersistenceScope,
    sortDirection,
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

  useEscClose(showAddModal, closeAddModal);
  useEscClose(!!deleteConfirm, () => setDeleteConfirm(null));
  useEscClose(!!tagDeleteConfirm, () => setTagDeleteConfirm(null));
  useEscClose(showCheckinModal, () => setShowCheckinModal(false));

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

  const handleSync = useCallback(async () => {
    setSyncing(true);
    setSyncMessage(null);
    try {
      const count = await platformConfig.runSync();
      setSyncMessage(platformConfig.syncSuccessMessage(t, count));
      setTimeout(() => setSyncMessage(null), 3000);
    } catch (err) {
      setSyncMessage(platformConfig.syncFailedMessage(t, String(err)));
    } finally {
      setSyncing(false);
    }
  }, [platformConfig, t]);

  const isAbnormalAccount = useCallback(
    (account: TAccount) => !platformConfig.getUsage(account).isNormal,
    [platformConfig],
  );

  const suitePage = useCodebuddySuitePage({
    accounts,
    currentAccountId,
    searchQuery,
    filterTypes,
    tagFilter,
    sortDirection,
    getPlanBadge: platformConfig.getPlanBadge,
    isAbnormalAccount,
    normalizeTag,
    groupByTag,
  });

  const {
    tierSummary,
    filteredAccounts,
    filteredIds,
    groupedAccounts,
    resolvePlanKey,
    resolveTierBadgeClass,
    formatQuotaDateTime,
  } = suitePage;

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const options: MultiSelectFilterOption[] = [];
    KNOWN_PLAN_FILTERS.forEach((plan) => {
      const count = tierSummary.dynamicCounts.get(plan) ?? 0;
      if (count === 0) return;
      options.push({ value: plan, label: `${plan} (${count})` });
    });
    tierSummary.extraKeys.forEach((key) => {
      options.push({
        value: key,
        label: `${key} (${tierSummary.dynamicCounts.get(key) ?? 0})`,
      });
    });
    options.push(buildValidAccountsFilterOption(t, tierSummary.validCount));
    return options;
  }, [t, tierSummary.dynamicCounts, tierSummary.extraKeys, tierSummary.validCount]);

  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey(platformConfig.quickSettingsType),
  });
  const paginatedAccounts = pagination.pageItems;
  const paginatedIds = useMemo(
    () => paginatedAccounts.map((account) => account.id),
    [paginatedAccounts],
  );
  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts],
  );
  const isAllPaginatedSelected = useMemo(
    () => isEveryIdSelected(selected, paginatedIds),
    [paginatedIds, selected],
  );

  const resolveGroupLabel = (groupKey: string) =>
    groupKey === '__untagged__' ? t('accounts.defaultGroup', '默认分组') : groupKey;

  const renderResourceQuotaItems = useCallback((account: TAccount, _variant: 'card' | 'table') => {
    const groups = platformConfig.getQuotaGroups(account, t as (key: string, defaultValue?: string) => string);
    return (
      <CodeBuddyQuotaCategoryList
        groups={groups}
        formatNumber={formatQuotaNumber}
        formatDateTime={formatQuotaDateTime}
      />
    );
  }, [formatQuotaDateTime, platformConfig, t]);

  const renderUsageInfo = useCallback((account: TAccount) => {
    const usage = platformConfig.getUsage(account);
    return (
      <DosageNotifyUsageStatus
        usage={usage}
        locale={locale}
        accountLabel={maskAccountText(platformConfig.getDisplayEmail(account))}
        normalText={t(`${platformConfig.usagePrefix}.usageNormal`, '正常')}
        abnormalText={t(`${platformConfig.usagePrefix}.usageAbnormal`, '异常')}
        viewDetailText={t(`${platformConfig.usagePrefix}.usageViewDetail`, '查看详情')}
        detailTitle={t(`${platformConfig.usagePrefix}.usageDetailTitle`, '用量状态详情')}
        accountText={t('common.shared.columns.account', '账号')}
        confirmText={t('common.confirm', '确认')}
        closeText={t('common.close', '关闭')}
        classPrefix={platformConfig.usagePrefix}
      />
    );
  }, [locale, maskAccountText, platformConfig, t]);

  const renderQuotaQuerySection = useCallback((account: TAccount, variant: 'card' | 'table') => {
    const groups = platformConfig.getQuotaGroups(account, t as (key: string, defaultValue?: string) => string);
    const hasQuotaData = platformConfig.hasQuotaData(account, groups);
    const refreshFailed = !!account.quota_query_last_error?.trim();
    const shouldShowQuota = hasQuotaData && !refreshFailed;
    const statusText = refreshFailed
      ? t(`${platformConfig.quotaPrefix}.quotaQuery.failedRefreshCompact`, '配额查询失败')
      : t(`${platformConfig.quotaPrefix}.quotaQuery.empty`, '暂无可用配额数据');
    return (
      <>
        <div className="quota-item">
          <div className="quota-header">
            <span className="quota-name">{t(`${platformConfig.usagePrefix}.usage`, '用量状态')}</span>
            {renderUsageInfo(account)}
          </div>
        </div>
        <div className={`quota-item ${platformConfig.usagePrefix}-quota-item`}>
          <div className={`quota-header ${platformConfig.usagePrefix}-quota-header`}>
            <span className="quota-name">{t(`${platformConfig.quotaPrefix}.quotaQuery.sectionTitle`, '配额查询')}</span>
          </div>
          {shouldShowQuota ? (
            renderResourceQuotaItems(account, variant)
          ) : (
            <div className="quota-empty">{statusText}</div>
          )}
        </div>
      </>
    );
  }, [platformConfig, renderResourceQuotaItems, renderUsageInfo, t]);

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const displayEmail = platformConfig.getDisplayEmail(account);
      const planBadge = resolvePlanKey(account);
      const tierBadgeClass = resolveTierBadgeClass(planBadge);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      return (
        <div key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`ghcp-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}>
          <div className="card-top">
            <div className="card-select">
              <input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} />
            </div>
            <span className="account-email" title={maskAccountText(displayEmail)}>{maskAccountText(displayEmail)}</span>
            {isCurrent && <span className="current-tag">{t('accounts.status.current', '当前')}</span>}
            <span className={`tier-badge ${tierBadgeClass}`}>{planBadge}</span>
          </div>
          {accountTags.length > 0 && (
            <div className="card-tags">
              {visibleTags.map((tag, idx) => <span key={`${account.id}-${tag}-${idx}`} className="tag-pill">{tag}</span>)}
              {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
            </div>
          )}
          <div className="ghcp-quota-section">
            {renderQuotaQuerySection(account, 'card')}
          </div>
          <div className="card-footer">
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting}
                title={t('common.shared.switchAccount', '切换账号')}>
                {injecting === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="card-action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}><Tag size={14} /></button>
              <button className="card-action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新')}>
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button className="card-action-btn export-btn" onClick={() => handleExportByIds([account.id])} title={t('common.shared.export.title', '导出')}><Upload size={14} /></button>
              <button className="card-action-btn danger" onClick={() => handleDelete(account.id)} title={t('common.delete', '删除')}><Trash2 size={14} /></button>
            </div>
          </div>
        </div>
      );
    });

  const renderTableRows = (items: typeof filteredAccounts, _groupKey?: string) =>
    items.map((account) => {
      const displayEmail = platformConfig.getDisplayEmail(account);
      const planBadge = resolvePlanKey(account);
      const tierBadgeClass = resolveTierBadgeClass(planBadge);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      return (
        <tr key={account.id} className={`${isCurrent ? 'current-row' : ''} ${isSelected ? 'selected-row' : ''}`}>
          <td><input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} /></td>
          <td>
            <span className="table-email" title={maskAccountText(displayEmail)}>{maskAccountText(displayEmail)}</span>
            {isCurrent && <span className="current-tag">{t('accounts.status.current', '当前')}</span>}
          </td>
          <td><span className={`tier-badge ${tierBadgeClass}`}>{planBadge}</span></td>
          <td>
            <div className={platformConfig.tableUsageClassName}>
              {renderQuotaQuerySection(account, 'table')}
            </div>
          </td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button className="action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting}><Play size={14} /></button>
              <button className="action-btn" onClick={() => openTagModal(account.id)}><Tag size={14} /></button>
              <button className="action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id}><RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} /></button>
              <button className="action-btn" onClick={() => handleExportByIds([account.id])}><Upload size={14} /></button>
              <button className="action-btn danger" onClick={() => handleDelete(account.id)}><Trash2 size={14} /></button>
            </div>
          </td>
        </tr>
      );
    });

  const CheckinModal = platformConfig.CheckinModal;

  return (
    <>
      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)}>
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t(platformConfig.flowNotice.titleKey, platformConfig.flowNotice.titleDefault)}</span>
          </div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">
              {t(platformConfig.flowNotice.descKey, platformConfig.flowNotice.descDefault)}
            </div>
            <ul className="ghcp-flow-notice-list">
              <li>{t(platformConfig.flowNotice.permissionKey, platformConfig.flowNotice.permissionDefault)}</li>
              <li>{t(platformConfig.flowNotice.networkKey, platformConfig.flowNotice.networkDefault)}</li>
            </ul>
          </div>
        )}
      </div>

      {message && (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
          {message.text}
          <button onClick={() => setMessage(null)}><X size={14} /></button>
        </div>
      )}

      {syncMessage && (
        <div className="message-bar success">
          {syncMessage}
          <button onClick={() => setSyncMessage(null)}><X size={14} /></button>
        </div>
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search size={16} className="search-icon" />
            <input type="text" placeholder={t(platformConfig.searchPlaceholderKey, platformConfig.searchPlaceholderDefault)} value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} />
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
            <button type="button" className={`tag-filter-btn ${tagFilter.length > 0 ? 'active' : ''}`} onClick={() => setShowTagFilter((prev) => !prev)}>
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
                  <>
                    <div className="tag-filter-header">
                      <label className="group-toggle"><input type="checkbox" checked={groupByTag} onChange={() => setGroupByTag(!groupByTag)} /> {t('accounts.groupByTag', '按标签分组')}</label>
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
        </div>
        <div className="toolbar-right">
          <button className="btn btn-primary icon-only" onClick={() => openAddModal('oauth')} title={t('common.shared.addAccount', '添加账号')}><Plus size={14} /></button>
          <button className="btn btn-secondary icon-only" onClick={handleSync} disabled={syncing || accounts.length === 0} title={platformConfig.syncButtonTitle(t)}>
            {syncing ? <RefreshCw size={14} className="loading-spinner" /> : <ArrowRightLeft size={14} />}
          </button>
          {CheckinModal && (
            <button className="btn btn-secondary icon-only" onClick={() => setShowCheckinModal(true)} disabled={accounts.length === 0} title={t('workbuddy.checkin.modalTitle', '每日签到')}>
              <CalendarCheck size={14} />
            </button>
          )}
          <button className="btn btn-secondary icon-only" onClick={handleRefreshAll} disabled={refreshingAll || accounts.length === 0} title={t('common.shared.refreshAll', '刷新全部')}>
            <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
          </button>
          <button className="btn btn-secondary icon-only" onClick={togglePrivacyMode}
            title={privacyModeEnabled ? t('privacy.showSensitive', '显示邮箱') : t('privacy.hideSensitive', '隐藏邮箱')}>
            {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
          <button className="btn btn-secondary icon-only" onClick={() => openAddModal('token')} disabled={importing} title={t('common.shared.import.label', '导入')}><Download size={14} /></button>
          <button className="btn btn-secondary export-btn icon-only" onClick={() => void handleExport(filteredIds)} disabled={exporting || filteredIds.length === 0}
            title={exportSelectionCount > 0 ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})` : t('common.shared.export.title', '导出')}>
            <Upload size={14} />
          </button>
          {selected.size > 0 && (
            <button className="btn btn-danger icon-only" onClick={handleBatchDelete} title={`${t('common.delete', '删除')} (${selected.size})`}><Trash2 size={14} /></button>
          )}
          <QuickSettingsPopover type={platformConfig.quickSettingsType} />
        </div>
      </div>

      {loading && accounts.length === 0 ? (
        <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <Globe size={48} />
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t(platformConfig.noAccountsKey, platformConfig.noAccountsDefault)}</p>
          <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
            <button className="btn btn-primary" onClick={() => openAddModal('oauth')}>
              <Plus size={16} /> {t('common.shared.addAccount', '添加账号')}
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
                <th style={{ width: 40 }}><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
                <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th>
                <th style={{ width: 120 }}>{t('common.shared.columns.plan', '套餐')}</th>
                <th>{t('instances.labels.quota', '配额')}</th>
                <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
              </tr>
            </thead>
            <tbody>
              {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                <Fragment key={groupKey}>
                  <tr className="tag-group-row"><td colSpan={5}><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{totalCount}</span></div></td></tr>
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
                <th style={{ width: 40 }}><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
                <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th>
                <th style={{ width: 120 }}>{t('common.shared.columns.plan', '套餐')}</th>
                <th>{t('instances.labels.quota', '配额')}</th>
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
              <h2>{t(platformConfig.addAccountTitleKey, platformConfig.addAccountTitleDefault)}</h2>
              <button className="modal-close" onClick={closeAddModal}><X size={18} /></button>
            </div>
            <div className="modal-tabs">
              <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}><Globe size={14} /> {t('common.shared.addModal.oauth', '授权登录')}</button>
              <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}><KeyRound size={14} />{t('common.shared.addModal.token', 'Token / JSON')}</button>
              <button className={`modal-tab ${addTab === 'json' ? 'active' : ''}`} onClick={() => openAddModal('json')}><Database size={14} />{t('common.shared.addModal.import', '本地导入')}</button>
            </div>
            <div className="modal-body">
              {addTab === 'oauth' && (
                <div className="add-section oauth-section">
                  <p className="section-desc">
                    {t(platformConfig.oauthDescKey, platformConfig.oauthDescDefault)}
                  </p>
                  <div className={`${platformConfig.oauthFeatureCardClassName} oauth`}>
                    <p className="feature-title">{t(platformConfig.oauthFeatureTitleKey, platformConfig.oauthFeatureTitleDefault)}</p>
                    <ul className="feature-list">
                      <li>{t(platformConfig.oauthFeatureItem1Key, platformConfig.oauthFeatureItem1Default)}</li>
                      <li>{t(platformConfig.oauthFeatureItem2Key, platformConfig.oauthFeatureItem2Default)}</li>
                      <li>{t(platformConfig.oauthFeatureItem3Key, platformConfig.oauthFeatureItem3Default)}</li>
                    </ul>
                  </div>
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
                        <input
                          type="text"
                          value={oauthUrl}
                          readOnly
                          placeholder={t(platformConfig.oauthUrlInputPlaceholderKey, platformConfig.oauthUrlInputPlaceholderDefault)}
                        />
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
                          <span>{t(platformConfig.oauthWaitingKey, platformConfig.oauthWaitingDefault)}</span>
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
                      <p className="oauth-hint">{t('common.shared.oauth.hint', 'Once authorized, this window will update automatically')}</p>
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
                <div className="add-section token-section">
                  <p className="section-desc">{t(platformConfig.tokenDescKey, platformConfig.tokenDescDefault)}</p>
                  <textarea className="token-input" value={tokenInput} onChange={(e) => setTokenInput(e.target.value)} placeholder={t('common.shared.token.placeholder', '粘贴 Token 或 JSON...')} />
                  <button className="btn btn-primary btn-full" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                    {t('common.shared.token.import', 'Import')}
                  </button>
                </div>
              )}
              {addTab === 'json' && (
                <div className="add-section json-section">
                  <p className="section-desc">{t(platformConfig.importLocalDescKey, platformConfig.importLocalDescDefault)}</p>
                  <button className="btn btn-secondary btn-full" onClick={() => handleImportFromLocal?.()} disabled={importing}>
                    {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}
                    {t(platformConfig.importLocalClientKey, platformConfig.importLocalClientDefault)}
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
                  <span>{addMessage || t('common.shared.loginSuccess', '登录成功')}</span>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {deleteConfirm && (
        <div className="modal-overlay" onClick={() => !deleting && setDeleteConfirm(null)}>
          <div className="modal confirm-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirmDelete', '确认删除')}</h2>
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
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>{t('common.cancel', '取消')}</button>
              <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>{deleting ? t('common.processing', '处理中...') : t('common.confirm', '确认')}</button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div className="modal-overlay" onClick={() => !deletingTag && setTagDeleteConfirm(null)}>
          <div className="modal confirm-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirmDeleteTag', '确认删除标签')}</h2>
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
              <p>{t('common.deleteTagWarning', { tag: tagDeleteConfirm, defaultValue: '确定要从所有账号中移除标签 "{{tag}}" 吗？' })}</p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setTagDeleteConfirm(null)} disabled={deletingTag}>{t('common.cancel', '取消')}</button>
              <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>{deletingTag ? t('common.processing', '处理中...') : t('common.confirm', '确认')}</button>
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

      <TagEditModal
        isOpen={!!showTagModal}
        initialTags={accounts.find((a) => a.id === showTagModal)?.tags || []}
        availableTags={availableTags}
        onClose={() => setShowTagModal(null)}
        onSave={handleSaveTags}
      />

      {showCheckinModal && CheckinModal && (
        <CheckinModal
          accounts={filteredAccounts}
          onClose={() => setShowCheckinModal(false)}
          onCheckinComplete={onRefreshAccounts}
        />
      )}
    </>
  );
}
