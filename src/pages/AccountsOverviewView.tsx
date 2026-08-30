import { createPortal } from 'react-dom';
import { Plus, RefreshCw, Upload, Trash2, Rocket, X, Globe, KeyRound, Database, Plug, Copy, Check, LayoutGrid, List, Search, CircleAlert, Info, RotateCw, History, ArrowDownWideNarrow, ArrowUp, ArrowDown, Wrench, Rows3, GripVertical, Eye, EyeOff, BookOpen, FileUp, ExternalLink, FolderOpen, FolderPlus, ChevronRight, LogOut, FileText, ChevronDown } from 'lucide-react';
import * as accountService from '../services/accountService';
import { getAntigravityTierBadge, getQuotaClass, formatResetTimeDisplay } from '../utils/account';
import { openUrl } from '@tauri-apps/plugin-opener';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { PaginationControls } from '../components/PaginationControls';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import { AccountGroupModal, AddToGroupModal } from '../components/AccountGroupModal';
import { GroupAccountPickerModal } from '../components/GroupAccountPickerModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { MfaQuickCodeSelect } from '../components/MfaQuickCodeSelect';
import { ANTIGRAVITY_RESET_SORT_PREFIX } from '../utils/antigravityAccountSort';
import { OverviewTabsHeader } from '../components/OverviewTabsHeader';
import { FileCorruptedModal } from '../components/FileCorruptedModal';
import { AccountSelectionToolbar } from '../components/AccountSelectionToolbar';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { MultiSelectFilterDropdown } from '../components/MultiSelectFilterDropdown';
import { AccountTagFilterDropdown } from '../components/AccountTagFilterDropdown';
import { getMfaOtpToken, loadSavedMfaRecords } from '../utils/mfaVault';
import type { AccountsFilterType, useAccountsPageController } from "./AccountsPage";

export type AccountsOverviewViewProps = ReturnType<typeof useAccountsPageController>;

/** 渲染 AccountsPage 的界面；业务状态与动作统一由 Controller 提供。 */
export function AccountsOverviewView(props: AccountsOverviewViewProps) {
  const {
    accountGroups,
    accountNoteCopiedKey,
    accountNoteError,
    accountNoteErrorScrollKey,
    accountNoteFieldError,
    accountNoteMailPreview,
    accountNoteMailPreviewError,
    accountNoteMailPreviewLoading,
    accountNoteMfaPickerOpen,
    accountNoteOtpToken,
    accountNotePasswordVisible,
    accountNoteSecretVisible,
    accounts,
    activeAccountNoteEmail,
    activeAccountNoteForm,
    activeGroup,
    activeGroupId,
    addMessage,
    addStatus,
    addTab,
    addTargetGroup,
    allPaginatedSelected,
    ANTIGRAVITY_ACCOUNT_NOTE_MAX_LENGTH,
    ANTIGRAVITY_TOKEN_BATCH_EXAMPLE,
    ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE,
    antigravitySeamlessSwitchUnlocked,
    availableTags,
    clearFilterTypes,
    clearTagFilter,
    closeAccountNoteModal,
    closeAddModal,
    confirmClearSwitchHistory,
    confirmDelete,
    confirmDeleteGroup,
    confirmDeleteTag,
    copyAccountNoteValue,
    currentAccount,
    customSortAccounts,
    customSortDropTargetId,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    deleting,
    deletingGroup,
    deletingTag,
    displayGroups,
    draggedCustomSortAccountId,
    editingAccountNoteAccount,
    exportAccountIdsRef,
    exporting,
    exportModal,
    exportSelectionCount,
    exportSensitiveRefreshSeqRef,
    fetchAccountNoteMailPreviewForUrl,
    fileCorruptedError,
    filteredAccounts,
    filterTypes,
    formatAntigravityMailPreviewTime,
    formatDate,
    formatMfaRecordOption,
    formatSwitchHistoryAutoReason,
    formatSwitchHistoryOrigin,
    formatSwitchHistoryStage,
    formatSwitchHistoryTrigger,
    getQuotaDisplayItems,
    getVerificationBadge,
    groupAccountPickerGroup,
    groupAccountPickerGroupId,
    groupByTag,
    groupDeleteConfirm,
    groupDeleteError,
    groupDeleteErrorScrollKey,
    groupQuickAddGroup,
    groupQuickAddGroupId,
    handleAssignAccountsToGroup,
    handleBatchDelete,
    handleClearSwitchHistory,
    handleCopyOauthUrl,
    handleCustomSortDragMove,
    handleCustomSortDragStart,
    handleExport,
    handleImportFromExtension,
    handleImportFromFiles,
    handleImportFromLocal,
    handleImportFromTools,
    handleOAuthComplete,
    handleOAuthStart,
    handlePendingOAuthComplete,
    handlePendingOAuthStart,
    handleRefresh,
    handleRefreshAll,
    handleRemoveFromGroup,
    handleSaveAccountNote,
    handleSavePendingOAuthAccount,
    handleSaveTags,
    handleSortByChange,
    handleSubmitOauthCallbackUrl,
    handleTokenImport,
    handleViewModeChange,
    handleWakeupSelected,
    hasAntigravityAccountNoteFormDetails,
    hasVisibleAccountGroups,
    importing,
    includeExportSensitiveNotes,
    includeExportSensitiveNotesRef,
    isCustomSortActive,
    loading,
    locale,
    maskAccountText,
    message,
    mfaTimeRemaining,
    moveCustomSortAccount,
    oauthAccountNoteForm,
    oauthAccountNoteMode,
    oauthCallbackError,
    oauthCallbackInput,
    oauthCallbackSubmitting,
    oauthUrl,
    oauthUrlCopied,
    onNavigate,
    openAddModal,
    openOAuthAccountNoteModal,
    openSwitchHistoryModal,
    paginatedIds,
    pagination,
    pendingOAuthAccount,
    pendingOAuthEmailError,
    pendingOAuthEmailInput,
    privacyModeEnabled,
    refreshing,
    refreshingAll,
    reloadAccountGroups,
    renderCompactView,
    renderErrorMessage,
    renderGridView,
    renderListView,
    requestDeleteTag,
    resetAddModalState,
    resetCustomSortOrder,
    savedMfaRecords,
    savingAccountNote,
    savingPendingOAuthAccount,
    searchQuery,
    selected,
    setAccountNoteMfaPickerOpen,
    setAccountNotePasswordVisible,
    setAccountNoteSecretVisible,
    setActiveGroupId,
    setAddTab,
    setDeleteConfirm,
    setDeleteConfirmError,
    setFileCorruptedError,
    setGroupAccountPickerGroupId,
    setGroupByTag,
    setGroupDeleteConfirm,
    setGroupDeleteError,
    setGroupQuickAddGroupId,
    setIncludeExportSensitiveNotes,
    setMessage,
    setOauthCallbackInput,
    setPendingOAuthEmailError,
    setPendingOAuthEmailInput,
    setSavedMfaRecords,
    setSearchQuery,
    setSelected,
    setShowAccountGroupModal,
    setShowAddToGroupModal,
    setShowCustomSortModal,
    setShowErrorModal,
    setShowQuotaModal,
    setShowSwitchHistoryModal,
    setShowTagModal,
    setShowVerificationErrorModal,
    setSortDirection,
    setSwitchHistoryClearConfirmOpen,
    setTagDeleteConfirm,
    setTagDeleteConfirmError,
    setTokenInput,
    showAccountGroupModal,
    showAddModal,
    showAddToGroupModal,
    showCustomSortModal,
    showErrorModal,
    showQuotaModal,
    showSwitchHistoryModal,
    showTagModal,
    showVerificationErrorModal,
    sortBy,
    sortDirection,
    stopCustomSortDragging,
    switchHistory,
    switchHistoryClearConfirmOpen,
    switchHistoryClearing,
    switchHistoryLoading,
    t,
    tagDeleteConfirm,
    tagDeleteConfirmError,
    tagDeleteConfirmErrorScrollKey,
    tagFilter,
    tierCounts,
    tierFilterOptions,
    toggleFilterTypeValue,
    togglePrivacyMode,
    toggleSelectAll,
    toggleTagFilterValue,
    tokenInput,
    updateEditingAccountNoteForm,
    verificationDetailMap,
    verificationStatusMap,
    viewMode,
    wakeupRunning,
  } = props;
  return (
    <>
      <main className="main-content accounts-page">
        <OverviewTabsHeader
          active="overview"
          onNavigate={onNavigate}
          onOpenManual={() => onNavigate?.('manual')}
          subtitle={t('overview.subtitle')}
        />

        {/* 面包屑：进入分组后显示 */}
        {activeGroup && (
          <div className="folder-breadcrumb">
            <button
              className="breadcrumb-back"
              onClick={() => {
                setActiveGroupId(null)
                setSelected(new Set())
              }}
            >
              <FolderOpen size={14} />
              {t('accounts.groups.allGroups')}
            </button>
            <ChevronRight size={14} className="breadcrumb-sep" />
            <span className="breadcrumb-current">
              {activeGroup.name}
              <span className="breadcrumb-count">({filteredAccounts.length})</span>
            </span>
            {selected.size > 0 && (
              <>
                <button
                  className="btn btn-secondary breadcrumb-remove-btn"
                  onClick={() => setGroupQuickAddGroupId(activeGroup.id)}
                  title={t('accounts.groups.addAccounts')}
                >
                  <FolderPlus size={14} />
                  {t('accounts.groups.addAccounts')}
                </button>
                <button
                  className="btn btn-secondary breadcrumb-remove-btn"
                  onClick={() => setShowAddToGroupModal(true)}
                  title={t('accounts.groups.moveToGroup')}
                >
                  <FolderPlus size={14} />
                  {t('accounts.groups.moveToGroup')} ({selected.size})
                </button>
                <button
                  className="btn btn-secondary breadcrumb-remove-btn"
                  onClick={handleRemoveFromGroup}
                  title={t('accounts.groups.removeFromGroup')}
                >
                  <LogOut size={14} />
                  {t('accounts.groups.removeFromGroup')} ({selected.size})
                </button>
              </>
            )}
            {selected.size === 0 && (
              <button
                className="btn btn-secondary breadcrumb-remove-btn"
                onClick={() => setGroupQuickAddGroupId(activeGroup.id)}
                title={t('accounts.groups.addAccounts')}
              >
                <FolderPlus size={14} />
                {t('accounts.groups.addAccounts')}
              </button>
            )}
          </div>
        )}

        {/* 分组文件夹已嵌入到 accounts-grid 内，此处不再单独显示 */}

        {/* 工具栏 */}
        <div className="toolbar">
          <div className="toolbar-left">
            <div className="search-box">
              <Search size={16} className="search-icon" />
              <input
                type="text"
                placeholder={t('accounts.search')}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>

            <div className="view-switcher">
              <button
                className={`view-btn ${viewMode === 'compact' ? 'active' : ''}`}
                onClick={() => handleViewModeChange('compact')}
                title={t('accounts.view.compact')}
              >
                <Rows3 size={16} />
              </button>
              <button
                className={`view-btn ${viewMode === 'list' ? 'active' : ''}`}
                onClick={() => handleViewModeChange('list')}
                title={t('accounts.view.list')}
              >
                <List size={16} />
              </button>
              <button
                className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`}
                onClick={() => handleViewModeChange('grid')}
                title={t('accounts.view.grid')}
              >
                <LayoutGrid size={16} />
              </button>
            </div>

            <MultiSelectFilterDropdown
              options={tierFilterOptions}
              selectedValues={filterTypes}
              allLabel={t('accounts.filter.all', { count: tierCounts.all })}
              filterLabel={t('accounts.filterLabel', '筛选')}
              clearLabel={t('accounts.clearFilter', '清空筛选')}
              emptyLabel={t('common.none', '暂无')}
              ariaLabel={t('accounts.filterLabel', '筛选')}
              onToggleValue={(value) => toggleFilterTypeValue(value as AccountsFilterType)}
              onClear={clearFilterTypes}
            />

            <AccountTagFilterDropdown
              availableTags={availableTags}
              selectedTags={tagFilter}
              onToggleTag={toggleTagFilterValue}
              onClear={clearTagFilter}
              onDeleteTag={requestDeleteTag}
              groupByTag={groupByTag}
              onToggleGroupByTag={setGroupByTag}
            />
            {/* 排序下拉菜单 */}
            <SingleSelectFilterDropdown
              value={sortBy}
              options={[
                {
                  value: 'overall',
                  label: t('accounts.sort.overall', '按综合配额'),
                },
                {
                  value: 'created_at',
                  label: t('accounts.sort.createdAt', '按创建时间'),
                },
                ...displayGroups.map((group) => ({
                  value: group.id,
                  label: t('accounts.sort.byGroup', {
                    group: group.name,
                    defaultValue: `按 ${group.name} 配额`,
                  }),
                })),
                ...displayGroups.map((group) => ({
                  value: `${ANTIGRAVITY_RESET_SORT_PREFIX}${group.id}`,
                  label: t('accounts.sort.byGroupReset', {
                    group: group.name,
                    defaultValue: `按 ${group.name} 重置时间`,
                  }),
                })),
                {
                  value: 'custom',
                  label: t('accounts.sort.custom', '自定义顺序'),
                },
              ]}
              ariaLabel={t('accounts.sortLabel', '排序')}
              icon={<ArrowDownWideNarrow size={14} />}
              onChange={handleSortByChange}
            />

            {/* 排序方向切换按钮 / 自定义排序配置按钮 */}
            {!isCustomSortActive ? (
              <button
                className="sort-direction-btn"
                onClick={() =>
                  setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))
                }
                title={
                  sortDirection === 'desc'
                    ? t('accounts.sort.descTooltip', '当前：降序，点击切换为升序')
                    : t('accounts.sort.ascTooltip', '当前：升序，点击切换为降序')
                }
                aria-label={t('accounts.sort.toggleDirection', '切换排序方向')}
              >
                {sortDirection === 'desc' ? '⬇' : '⬆'}
              </button>
            ) : (
              <button
                className="sort-direction-btn"
                onClick={() => setShowCustomSortModal(true)}
                title={t('accounts.sort.customSettingsTooltip', '配置自定义顺序')}
                aria-label={t('accounts.sort.customSettingsTooltip', '配置自定义顺序')}
              >
                <Wrench size={14} />
              </button>
            )}
          </div>

          <div className="toolbar-right">
            <button
              className="btn btn-primary icon-only"
              onClick={() => openAddModal('oauth')}
              title={t('accounts.addAccount')}
              aria-label={t('accounts.addAccount')}
            >
              <Plus size={14} />
            </button>
            <button
              className="btn btn-secondary icon-only"
              onClick={handleRefreshAll}
              disabled={refreshingAll}
              title={t('accounts.refreshAll')}
              aria-label={t('accounts.refreshAll')}
            >
              <RefreshCw
                size={14}
                className={refreshingAll ? 'loading-spinner' : ''}
              />
            </button>
            {antigravitySeamlessSwitchUnlocked && (
              <button
                className="btn btn-secondary icon-only"
                onClick={openSwitchHistoryModal}
                title={t('accounts.switchHistory.title', '切换记录')}
                aria-label={t('accounts.switchHistory.title', '切换记录')}
              >
                <History size={14} />
              </button>
            )}
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
              className="btn btn-secondary export-btn icon-only"
              onClick={handleExport}
              disabled={exporting || filteredAccounts.length === 0}
              title={
                exportSelectionCount > 0
                  ? `${t('accounts.export')} (${exportSelectionCount})`
                  : t('accounts.export')
              }
              aria-label={
                exportSelectionCount > 0
                  ? `${t('accounts.export')} (${exportSelectionCount})`
                  : t('accounts.export')
              }
            >
              <Upload size={14} />
            </button>
            {!activeGroupId && (
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setShowAccountGroupModal(true)}
                title={t('accounts.groups.manageTitle')}
                aria-label={t('accounts.groups.manageTitle')}
              >
                <FolderOpen size={14} />
              </button>
            )}
            <QuickSettingsPopover type="antigravity" />
          </div>
        </div>

        {filteredAccounts.length > 0 && (
          <AccountSelectionToolbar
            selectedCount={selected.size}
            allSelected={allPaginatedSelected}
            disabled={paginatedIds.length === 0}
            onToggleSelectAll={toggleSelectAll}
            onClearSelection={() => setSelected(new Set())}
            actions={(
              <>
                <button
                  className="btn btn-secondary icon-only"
                  onClick={() => void handleWakeupSelected()}
                  disabled={wakeupRunning || selected.size === 0}
                  title={`${t('wakeup.runTest')} (${selected.size})`}
                  aria-label={`${t('wakeup.runTest')} (${selected.size})`}
                >
                  {wakeupRunning ? (
                    <RefreshCw size={14} className="loading-spinner" />
                  ) : (
                    <Rocket size={14} />
                  )}
                </button>
                <button
                  className="btn btn-secondary icon-only"
                  onClick={() => setShowAddToGroupModal(true)}
                  title={t('accounts.groups.addToGroup')}
                  aria-label={t('accounts.groups.addToGroup')}
                >
                  <FolderPlus size={14} />
                </button>
                <button
                  className="btn btn-danger icon-only"
                  onClick={handleBatchDelete}
                  title={`${t('common.delete')} (${selected.size})`}
                  aria-label={`${t('common.delete')} (${selected.size})`}
                >
                  <Trash2 size={14} />
                </button>
              </>
            )}
          />
        )}

        {message && (
          <div
            className={`action-message${message.tone ? ` ${message.tone}` : ''}`}
          >
            <span className="action-message-text">{message.text}</span>
            <button
              className="action-message-close"
              onClick={() => setMessage(null)}
              aria-label={t('common.close')}
            >
              <X size={14} />
            </button>
          </div>
        )}

        {/* 内容区域 */}
        {loading ? (
          <div className="empty-state">
            <div
              className="loading-spinner"
              style={{ width: 40, height: 40 }}
            />
          </div>
        ) : accounts.length === 0 ? (
          <div className="empty-state">
            <div className="icon">
              <Rocket size={40} />
            </div>
            <h3>{t('accounts.empty.title')}</h3>
            <p>{t('accounts.empty.desc')}</p>
            <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
              <button
                className="btn btn-primary"
                onClick={() => openAddModal('oauth')}
              >
                <Plus size={18} />
                {t('accounts.empty.btn')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => onNavigate?.('manual')}
              >
                <BookOpen size={18} />
                {t('manual.navTitle', '查阅接入手册')}
              </button>
            </div>
          </div>
        ) : filteredAccounts.length === 0 && !hasVisibleAccountGroups ? (
          <div className="empty-state">
            <h3>{t('accounts.noMatch.title')}</h3>
            <p>{t('accounts.noMatch.desc')}</p>
          </div>
        ) : viewMode === 'grid' ? (
          renderGridView()
        ) : viewMode === 'list' ? (
          renderListView()
        ) : (
          renderCompactView()
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
      </main>

      {/* Add Account Modal */}
      {showAddModal && (
        <div className="modal-overlay">
          <div
            className="modal modal-lg add-account-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t('modals.addAccount.title')}</h2>
              <button className="close-btn" onClick={closeAddModal}>
                <X size={20} />
              </button>
            </div>
            <div className="modal-body">
              <MfaQuickCodeSelect />
              {addTargetGroup && (
                <div className="accounts-add-target-group-hint">
                  <FolderPlus size={14} />
                  <span>
                    {t('accounts.addModal.targetGroup', {
                      defaultValue: '将添加到分组：{{group}}',
                      group: addTargetGroup.name,
                    })}
                  </span>
                </div>
              )}
              <div className="add-tabs">
                <button
                  className={`add-tab ${addTab === 'oauth' ? 'active' : ''}`}
                  onClick={() => {
                    setAddTab('oauth')
                    resetAddModalState()
                  }}
                >
                  <Globe size={14} /> {t('accounts.tabs.oauth')}
                </button>
                <button
                  className={`add-tab ${addTab === 'token' ? 'active' : ''}`}
                  onClick={() => {
                    setAddTab('token')
                    resetAddModalState()
                  }}
                >
                  <KeyRound size={14} /> {t('common.shared.addModal.token', 'Token / JSON')}
                </button>
                <button
                  className={`add-tab ${addTab === 'import' ? 'active' : ''}`}
                  onClick={() => {
                    setAddTab('import')
                    resetAddModalState()
                  }}
                >
                  <Database size={14} /> {t('accounts.tabs.import')}
                </button>
              </div>

              {addTab === 'oauth' && (
                <div className="add-panel">
                  <div className="codex-pending-oauth-draft antigravity-pending-oauth-draft">
                    <div className="oauth-link">
                      <label>{t('codex.pendingAuth.emailLabel', '待授权账号')}</label>
                      <div className="oauth-link-row oauth-manual-input">
                        <input
                          type="email"
                          value={pendingOAuthEmailInput}
                          onChange={(event) => {
                            setPendingOAuthEmailInput(event.target.value)
                            setPendingOAuthEmailError(null)
                          }}
                          placeholder={t('codex.pendingAuth.emailPlaceholder', '输入账号邮箱')}
                          readOnly={Boolean(pendingOAuthAccount)}
                          disabled={savingPendingOAuthAccount}
                        />
                        {pendingOAuthAccount ? (
                          <button type="button" className="btn btn-secondary icon-only" onClick={() => void copyAccountNoteValue('pendingEmail', pendingOAuthEmailInput)} aria-label={t('common.copy', '复制')}>
                            {accountNoteCopiedKey === 'pendingEmail' ? <Check size={14} /> : <Copy size={14} />}
                          </button>
                        ) : null}
                      </div>
                      {pendingOAuthEmailError ? <span className="codex-account-note-field-error">{pendingOAuthEmailError}</span> : null}
                    </div>
                    <button
                      type="button"
                      className={`codex-account-note-chip ${hasAntigravityAccountNoteFormDetails(oauthAccountNoteForm) ? 'has-note' : 'empty-note'}`}
                      onClick={openOAuthAccountNoteModal}
                      disabled={savingPendingOAuthAccount || addStatus === 'loading'}
                    >
                      <FileText size={12} />
                      <span>{hasAntigravityAccountNoteFormDetails(oauthAccountNoteForm) ? t('accounts.accountNote.short', '账号备注') : t('accounts.accountNote.addShort', '加备注')}</span>
                    </button>
                    {!pendingOAuthAccount ? (
                      <button type="button" className="btn btn-secondary btn-full" onClick={() => void handleSavePendingOAuthAccount()} disabled={savingPendingOAuthAccount || !pendingOAuthEmailInput.trim()}>
                        {savingPendingOAuthAccount ? <RefreshCw size={16} className="loading-spinner" /> : <FileText size={16} />}
                        {t('codex.pendingAuth.saveDraft', '保存待授权卡片')}
                      </button>
                    ) : null}
                  </div>
                  <div className="oauth-hint">
                    <Globe size={18} />
                    <span>{t('accounts.oauth.hint')}</span>
                  </div>
                  <div className="oauth-actions">
                    <button
                      className="btn btn-primary"
                      onClick={pendingOAuthAccount ? handlePendingOAuthStart : handleOAuthStart}
                      disabled={addStatus === 'loading'}
                    >
                      <Globe size={16} /> {t('accounts.oauth.start')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={pendingOAuthAccount ? handlePendingOAuthComplete : handleOAuthComplete}
                      disabled={!oauthUrl || addStatus === 'loading'}
                    >
                      <Check size={16} /> {t('accounts.oauth.continue')}
                    </button>
                  </div>
                  <div className="oauth-link">
                    <label>{t('accounts.oauth.linkLabel')}</label>
                    <div className="oauth-link-row">
                      <input
                        type="text"
                        value={oauthUrl || t('accounts.oauth.generatingLink')}
                        readOnly
                      />
                      <button
                        className="btn btn-secondary icon-only"
                        onClick={handleCopyOauthUrl}
                        disabled={!oauthUrl}
                        title={t('common.copy')}
                      >
                        {oauthUrlCopied ? (
                          <Check size={14} />
                        ) : (
                          <Copy size={14} />
                        )}
                      </button>
                    </div>
                  </div>
                  <div className="oauth-link">
                    <label>{t('common.shared.oauth.manualCallbackLabel', '手动输入回调地址')}</label>
                    <div className="oauth-link-row oauth-manual-input">
                      <input
                        type="text"
                        value={oauthCallbackInput}
                        onChange={(e) => setOauthCallbackInput(e.target.value)}
                        placeholder={t('common.shared.oauth.manualCallbackPlaceholder', '粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...')}
                      />
                      <button
                        className="btn btn-secondary"
                        onClick={handleSubmitOauthCallbackUrl}
                        disabled={!oauthCallbackInput.trim() || oauthCallbackSubmitting}
                      >
                        {oauthCallbackSubmitting ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Check size={16} />
                        )}{' '}
                        {t('accounts.oauth.continue')}
                      </button>
                    </div>
                  </div>
                  {oauthCallbackError && (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>{oauthCallbackError}</span>
                    </div>
                  )}
                </div>
              )}

              {addTab === 'token' && (
                <div className="add-panel">
                  <p className="add-panel-desc">{t('accounts.token.desc')}</p>
                  <details className="token-format-collapse">
                    <summary className="token-format-collapse-summary">
                      {t('messages.example', 'Example')}
                    </summary>
                    <div className="token-format">
                      <p className="token-format-required">{t('accounts.token.desc')}</p>
                      <div className="token-format-group">
                        <div className="token-format-label">{`${t('messages.example', 'Example')} 1`}</div>
                        <pre className="token-format-code">{ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE}</pre>
                      </div>
                      <div className="token-format-group">
                        <div className="token-format-label">{`${t('messages.example', 'Example')} 2`}</div>
                        <pre className="token-format-code">{ANTIGRAVITY_TOKEN_BATCH_EXAMPLE}</pre>
                      </div>
                    </div>
                  </details>
                  <textarea
                    className="token-input"
                    placeholder={t('accounts.token.placeholder')}
                    value={tokenInput}
                    onChange={(e) => setTokenInput(e.target.value)}
                    rows={6}
                  />
                  <div className="modal-actions">
                    <button
                      className="btn btn-primary"
                      onClick={handleTokenImport}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <KeyRound size={14} /> {t('accounts.token.importStart')}
                    </button>
                  </div>
                </div>
              )}

              {addTab === 'import' && (
                <div className="add-panel">
                  <div className="import-options">
                    <button
                      className="import-option"
                      onClick={handleImportFromExtension}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <Plug size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.fromExtension')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.syncBadge')}
                        </div>
                      </div>
                    </button>

                    <button
                      className="import-option"
                      onClick={handleImportFromLocal}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <Database size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.fromLocalDB')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.localDBDesc')}
                        </div>
                      </div>
                    </button>

                    <button
                      className="import-option"
                      onClick={handleImportFromTools}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <Rocket size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.tools')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.toolsDescMigrate')}
                        </div>
                      </div>
                    </button>

                    <button
                      className="import-option"
                      onClick={handleImportFromFiles}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <FileUp size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.fromFiles')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.fromFilesDesc')}
                        </div>
                      </div>
                    </button>
                  </div>
                </div>
              )}

              {addMessage && (
                <div className={`add-feedback ${addStatus}`}>{addMessage}</div>
              )}
            </div>
          </div>
        </div>
      )}

      <ExportJsonModal
        isOpen={exportModal.showModal}
        title={`${t('accounts.export')} JSON`}
        jsonContent={exportModal.jsonContent}
        hidden={exportModal.hidden}
        copied={exportModal.copied}
        saving={exportModal.saving}
        savedPath={exportModal.savedPath}
        canOpenSavedDirectory={exportModal.canOpenSavedDirectory}
        pathCopied={exportModal.pathCopied}
        toolbarContent={
          <>
            <label className="export-json-sensitive-toggle" title={t('accounts.accountNote.exportSensitiveToggleHint', '控制导出 JSON 是否包含 2FA 秘钥、密码、手机号和邮件地址。')}>
              <input type="checkbox" checked={includeExportSensitiveNotes} onChange={(event) => {
                includeExportSensitiveNotesRef.current = event.target.checked
                setIncludeExportSensitiveNotes(event.target.checked)
                const includeSensitive = event.target.checked
                const requestSeq = ++exportSensitiveRefreshSeqRef.current
                void accountService.exportAccounts(exportAccountIdsRef.current).then((raw) => {
                  if (exportSensitiveRefreshSeqRef.current !== requestSeq) return
                  if (includeSensitive) {
                    exportModal.replaceJsonContent(raw)
                    return
                  }
                  try {
                    const parsed = JSON.parse(raw) as Array<Record<string, unknown>>
                    parsed.forEach((item) => {
                      delete item.two_factor_secret
                      delete item.account_password
                      delete item.phone_number
                      delete item.mail_url
                    })
                    exportModal.replaceJsonContent(JSON.stringify(parsed, null, 2))
                  } catch {
                    exportModal.replaceJsonContent(raw)
                  }
                }).catch((error) => {
                  if (exportSensitiveRefreshSeqRef.current !== requestSeq) return
                  setMessage({
                    text: t('messages.exportFailed', { error: String(error) }),
                    tone: 'error',
                  })
                })
              }} />
              <span className="export-json-sensitive-switch" />
              <span>{includeExportSensitiveNotes ? t('accounts.accountNote.exportSensitiveIncluded', '包含敏感备注') : t('accounts.accountNote.exportSensitiveExcluded', '已排除敏感备注')}</span>
              <Info size={14} />
            </label>
            {includeExportSensitiveNotes ? (
              <div className="export-json-sensitive-notice">
                <Info size={14} />
                <span>{t('accounts.accountNote.exportSensitiveNotice', '导出内容包含 2FA 秘钥、密码、手机号或邮件地址，请只保存到可信位置。')}</span>
              </div>
            ) : null}
          </>
        }
        onClose={exportModal.closeModal}
        onToggleHidden={exportModal.toggleHidden}
        onCopyJson={exportModal.copyJson}
        onSaveJson={exportModal.saveJson}
        onOpenSavedDirectory={exportModal.openSavedDirectory}
        onCopySavedPath={exportModal.copySavedPath}
      />

      {showCustomSortModal && (
        <div className="modal-overlay">
          <div
            className="modal codex-custom-sort-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <div>
                <h2>
                  {t('accounts.sort.customModalTitle', '自定义账号排序')}
                </h2>
                <p className="codex-custom-sort-modal-desc">
                  {t(
                    'accounts.sort.customModalDesc',
                    '拖动账号或使用上下按钮调整展示顺序。'
                  )}
                </p>
              </div>
              <button
                className="modal-close"
                onClick={() => setShowCustomSortModal(false)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div
                className={`codex-custom-sort-list ${
                  draggedCustomSortAccountId ? 'is-sorting' : ''
                }`}
                onMouseUp={stopCustomSortDragging}
                onMouseLeave={stopCustomSortDragging}
              >
                {customSortAccounts.map((account, index) => {
                  const isCurrent = currentAccount?.id === account.id
                  const tierBadge = getAntigravityTierBadge(account.quota)
                  const quotaDisplayItems = getQuotaDisplayItems(account)
                  const rowClass = [
                    'codex-custom-sort-row',
                    draggedCustomSortAccountId === account.id
                      ? 'is-dragging'
                      : '',
                    draggedCustomSortAccountId &&
                    draggedCustomSortAccountId !== account.id
                      ? 'is-drop-candidate'
                      : '',
                    draggedCustomSortAccountId &&
                    draggedCustomSortAccountId !== account.id &&
                    customSortDropTargetId === account.id
                      ? 'is-drop-target'
                      : '',
                  ]
                    .join(' ')
                    .trim()

                  return (
                    <div
                      key={account.id}
                      className={rowClass}
                      onMouseEnter={() =>
                        handleCustomSortDragMove(account.id)
                      }
                    >
                      <div className="codex-custom-sort-row-main">
                        <button
                          type="button"
                          className="codex-custom-sort-drag-handle"
                          onMouseDown={(event) =>
                            handleCustomSortDragStart(event, account.id)
                          }
                          title={t(
                            'accounts.sort.customDragHandle',
                            '拖拽排序'
                          )}
                          aria-label={t(
                            'accounts.sort.customDragHandle',
                            '拖拽排序'
                          )}
                        >
                          <GripVertical size={16} />
                        </button>
                        <span className="codex-custom-sort-index">
                          {index + 1}
                        </span>
                        <div className="codex-custom-sort-account">
                          <div className="codex-custom-sort-account-title">
                            <span
                              title={maskAccountText(account.email)}
                            >
                              {maskAccountText(account.email)}
                            </span>
                            {isCurrent && (
                              <span className="mini-tag current">
                                {t('accounts.status.current', '当前')}
                              </span>
                            )}
                            <span
                              className={`tier-badge ${tierBadge.className}`}
                            >
                              {tierBadge.label}
                            </span>
                          </div>
                          <div className="codex-custom-sort-quota-line">
                            {quotaDisplayItems.length > 0 ? (
                              quotaDisplayItems.slice(0, 2).map((item) => (
                                <span
                                  key={`${account.id}-${item.key}`}
                                  className="codex-custom-sort-quota"
                                >
                                  <span>{item.key.includes('claude') ? 'Claude' : 'Gemini'} {item.key.includes('5h') ? '5h' : 'Weekly'}:</span>
                                  <strong className={getQuotaClass(item.percentage)}>
                                    {item.percentage}%
                                  </strong>
                                </span>
                              ))
                            ) : (
                              <span className="codex-custom-sort-quota-empty">
                                {t('common.shared.quota.noData', '暂无配额数据')}
                              </span>
                            )}
                          </div>
                        </div>
                      </div>
                      <div className="codex-custom-sort-row-actions">
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            moveCustomSortAccount(account.id, 'up')
                          }
                          disabled={index === 0}
                          title={t('accounts.sort.customMoveUp', '上移')}
                          aria-label={t('accounts.sort.customMoveUp', '上移')}
                        >
                          <ArrowUp size={14} />
                        </button>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            moveCustomSortAccount(account.id, 'down')
                          }
                          disabled={index === customSortAccounts.length - 1}
                          title={t('accounts.sort.customMoveDown', '下移')}
                          aria-label={t(
                            'accounts.sort.customMoveDown',
                            '下移'
                          )}
                        >
                          <ArrowDown size={14} />
                        </button>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={resetCustomSortOrder}
              >
                <RotateCw size={14} />
                {t('accounts.sort.customReset', '重置自定义顺序')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => setShowCustomSortModal(false)}
              >
                {t('common.confirm', '确认')}
              </button>
            </div>
          </div>
        </div>
      )}

      {antigravitySeamlessSwitchUnlocked && showSwitchHistoryModal && (
        <div
          className="modal-overlay"
        >
          <div className="modal modal-lg" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('accounts.switchHistory.title', '切换记录')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (switchHistoryClearing || switchHistoryClearConfirmOpen) return
                  setShowSwitchHistoryModal(false)
                  setSwitchHistoryClearConfirmOpen(false)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              {switchHistoryLoading ? (
                <div className="empty-state">
                  <div className="loading-spinner" style={{ width: 28, height: 28 }} />
                </div>
              ) : switchHistory.length === 0 ? (
                <div className="empty-state" style={{ minHeight: 180 }}>
                  <p>{t('accounts.switchHistory.empty', '暂无切换记录')}</p>
                </div>
              ) : (
                <div style={{ maxHeight: 420, overflowY: 'auto', display: 'grid', gap: 10 }}>
                  {switchHistory.map((item) => (
                    <div
                      key={item.id}
                      style={{
                        border: '1px solid var(--border)',
                        borderRadius: 10,
                        padding: '10px 12px',
                        display: 'grid',
                        gap: 6,
                      }}
                    >
                      <div
                        style={{
                          display: 'flex',
                          justifyContent: 'space-between',
                          alignItems: 'center',
                          gap: 12,
                        }}
                      >
                        <div style={{ fontSize: 13, color: 'var(--text-secondary)' }}>
                          {new Date(item.timestamp).toLocaleString(locale)}
                        </div>
                        <div
                          style={{
                            fontSize: 12,
                            color: item.success ? 'var(--success, #10b981)' : 'var(--danger, #ef4444)',
                          }}
                        >
                          {item.success
                            ? t('accounts.switchHistory.success', '成功')
                            : t('accounts.switchHistory.failed', '失败')}
                        </div>
                      </div>
                      <div style={{ fontWeight: 600, fontSize: 14 }}>
                        {t('accounts.switchHistory.target', {
                          email: maskAccountText(item.targetEmail) || item.targetEmail || '-',
                          defaultValue: '目标账号：{{email}}',
                        })}
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
                        {t('accounts.switchHistory.trigger', {
                          trigger: formatSwitchHistoryTrigger(item.triggerType),
                          defaultValue: '触发方式：{{trigger}}',
                        })}
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
                        {t('accounts.switchHistory.origin', {
                          origin: formatSwitchHistoryOrigin(item.triggerSource),
                          defaultValue: '触发端：{{origin}}',
                        })}
                      </div>
                      {item.triggerType === 'auto' && (
                        <div style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
                          {t('accounts.switchHistory.autoReasonLabel', {
                            reason: formatSwitchHistoryAutoReason(item.autoSwitchReason),
                            defaultValue: '自动原因：{{reason}}',
                          })}
                        </div>
                      )}
                      <div style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
                        {t('accounts.switchHistory.stageResult', {
                          local: item.localOk
                            ? t('accounts.switchHistory.success', '成功')
                            : t('accounts.switchHistory.failed', '失败'),
                          seamless: item.seamlessOk
                            ? t('accounts.switchHistory.success', '成功')
                            : t('accounts.switchHistory.failed', '失败'),
                          defaultValue: '本地：{{local}} / 无感：{{seamless}}',
                        })}
                      </div>
                      <div style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
                        {t('accounts.switchHistory.duration', {
                          total: item.totalDurationMs,
                          local: item.localDurationMs,
                          seamless: item.seamlessDurationMs ?? 0,
                          defaultValue: '耗时：总 {{total}}ms，本地 {{local}}ms，无感 {{seamless}}ms',
                        })}
                      </div>
                      {!item.success && (
                        <div style={{ fontSize: 12, color: 'var(--danger, #ef4444)' }}>
                          {t('accounts.switchHistory.error', {
                            stage: formatSwitchHistoryStage(item.errorStage),
                            code: item.errorCode || '-',
                            message: item.errorMessage || '-',
                            defaultValue: '失败阶段：{{stage}}（{{code}}）{{message}}',
                          })}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setShowSwitchHistoryModal(false)
                  setSwitchHistoryClearConfirmOpen(false)
                }}
                disabled={switchHistoryClearing}
              >
                {t('common.close', '关闭')}
              </button>
              <button
                className="btn btn-danger"
                onClick={handleClearSwitchHistory}
                disabled={switchHistoryClearing || switchHistoryLoading || switchHistory.length === 0}
              >
                {switchHistoryClearing
                  ? t('common.loading', '加载中...')
                  : t('accounts.switchHistory.clear', '清空记录')}
              </button>
            </div>
          </div>
        </div>
      )}

      {antigravitySeamlessSwitchUnlocked && showSwitchHistoryModal && switchHistoryClearConfirmOpen && (
        <div
          className="modal-overlay"
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (switchHistoryClearing) return
                  setSwitchHistoryClearConfirmOpen(false)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p>{t('accounts.switchHistory.clearConfirm', '确定清空全部切换记录吗？')}</p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setSwitchHistoryClearConfirmOpen(false)}
                disabled={switchHistoryClearing}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmClearSwitchHistory}
                disabled={switchHistoryClearing}
              >
                {switchHistoryClearing
                  ? t('common.loading', '加载中...')
                  : t('accounts.switchHistory.clear', '清空记录')}
              </button>
            </div>
          </div>
        </div>
      )}

      {deleteConfirm && (
        <div
          className="modal-overlay"
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (deleting) return
                  setDeleteConfirm(null)
                  setDeleteConfirmError(null)
                }}
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
                onClick={() => {
                  setDeleteConfirm(null)
                  setDeleteConfirmError(null)
                }}
                disabled={deleting}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmDelete}
                disabled={deleting}
              >
                {t('common.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}

      {groupDeleteConfirm && (
        <div
          className="modal-overlay"
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('accounts.groups.deleteTitle')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (deletingGroup) return
                  setGroupDeleteConfirm(null)
                  setGroupDeleteError(null)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={groupDeleteError} scrollKey={groupDeleteErrorScrollKey} />
              <p>
                {t('accounts.groups.deleteConfirm', {
                  name: groupDeleteConfirm.name,
                })}
              </p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setGroupDeleteConfirm(null)
                  setGroupDeleteError(null)
                }}
                disabled={deletingGroup}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmDeleteGroup}
                disabled={deletingGroup}
              >
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div
          className="modal-overlay"
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (deletingTag) return
                  setTagDeleteConfirm(null)
                  setTagDeleteConfirmError(null)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} />
              <p>
                {t('accounts.confirmDeleteTag', {
                  tag: tagDeleteConfirm.tag,
                  count: tagDeleteConfirm.count,
                  defaultValue: '确认删除标签 "{{tag}}" 吗？该标签将从 {{count}} 个账号中移除。',
                })}
              </p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setTagDeleteConfirm(null)
                  setTagDeleteConfirmError(null)
                }}
                disabled={deletingTag}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmDeleteTag}
                disabled={deletingTag}
              >
                {deletingTag ? '处理中...' : t('common.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Quota Details Modal */}
      {showQuotaModal &&
        (() => {
          const account = accounts.find((a) => a.id === showQuotaModal)
          if (!account) return null
          const tierBadge = getAntigravityTierBadge(account.quota)
          const tierClass =
            tierBadge.tier === 'PRO' || tierBadge.tier === 'ULTRA'
              ? 'pill-success'
              : 'pill-secondary'

          return (
            <div
              className="modal-overlay"
            >
              <div
                className="modal modal-lg"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t('modals.quota.title')}</h2>
                  <div className="badges">
                    <span className={`pill ${tierClass}`}>{tierBadge.label}</span>
                  </div>
                  <button
                    className="close-btn"
                    onClick={() => setShowQuotaModal(null)}
                  >
                    <X size={20} />
                  </button>
                </div>
                <div className="modal-body">
                  {(() => {
                    const quotaDisplayItems = getQuotaDisplayItems(account)
                    if (quotaDisplayItems.length === 0) {
                      return (
                        <div className="empty-state-small">
                          {t('overview.noQuotaData')}
                        </div>
                      )
                    }
                    return (
                      <div className="quota-list">
                        {quotaDisplayItems.map((item) => (
                          <div key={item.key} className="quota-card">
                            <h4>{item.label}</h4>
                            <div className="quota-value-row">
                              <span
                                className={`quota-value ${getQuotaClass(item.percentage)}`}
                              >
                                {item.percentage}%
                              </span>
                            </div>
                            <div className="quota-bar">
                              <div
                                className={`quota-fill ${getQuotaClass(item.percentage)}`}
                                style={{
                                  width: `${Math.min(100, item.percentage)}%`
                                }}
                              ></div>
                            </div>
                            <div className="quota-reset-info">
                              <p>
                                <strong>{t('modals.quota.resetTime')}:</strong>{' '}
                                {formatResetTimeDisplay(item.resetTime, t)}
                              </p>
                            </div>
                          </div>
                        ))}
                      </div>
                    )
                  })()}

                  <div className="modal-actions" style={{ marginTop: 20 }}>
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowQuotaModal(null)}
                    >
                      {t('common.close')}
                    </button>
                    <button
                      className="btn btn-primary"
                      onClick={() => {
                        handleRefresh(account.id)
                      }}
                    >
                      {refreshing.has(account.id) ? (
                        <div className="loading-spinner small" />
                      ) : (
                        <RefreshCw size={16} />
                      )}
                      {t('common.refresh')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )
        })()}

      {/* Error Details Modal */}
      {showErrorModal &&
        (() => {
          const account = accounts.find((a) => a.id === showErrorModal)
          if (!account) return null
          const errorInfo = account.quota_error

          return (
            <div
              className="modal-overlay"
            >
              <div
                className="modal modal-lg"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t('modals.errors.title')}</h2>
                  <button
                    className="close-btn"
                    onClick={() => setShowErrorModal(null)}
                  >
                    <X size={20} />
                  </button>
                </div>
                <div className="modal-body">
                  {!errorInfo?.message ? (
                    <div className="empty-state-small">
                      {t('modals.errors.empty')}
                    </div>
                  ) : (
                    <div className="error-detail">
                      <div className="error-detail-meta">
                        <span>
                          {t('modals.errors.account')}: {maskAccountText(account.email)}
                        </span>
                        {errorInfo.code && (
                          <span>
                            {t('modals.errors.code')}: {errorInfo.code}
                          </span>
                        )}
                        {errorInfo.timestamp && (
                          <span>
                            {t('modals.errors.time')}:{' '}
                            {formatDate(errorInfo.timestamp)}
                          </span>
                        )}
                      </div>
                      <div className="error-detail-message">
                        {renderErrorMessage(errorInfo.message)}
                      </div>
                    </div>
                  )}

                  <div className="modal-actions" style={{ marginTop: 20 }}>
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowErrorModal(null)}
                    >
                      {t('common.close')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )
        })()}

      {/* Verification Error Modal (verification_required / tos_violation) */}
      {showVerificationErrorModal &&
        (() => {
          const account = accounts.find((a) => a.id === showVerificationErrorModal)
          if (!account) return null
          const vReason = account.disabled_reason || verificationStatusMap[account.id]
          const vDetail = verificationDetailMap[account.id]
          const isTos = vReason === 'tos_violation'
          const title = isTos
            ? t('wakeup.errorUi.tosViolationTitle', 'TOS 违规')
            : t('wakeup.errorUi.verificationRequiredTitle', '需要验证')

          const openLink = async (url: string) => {
            try {
              await openUrl(url)
            } catch {
              window.open(url, '_blank', 'noopener,noreferrer')
            }
          }

          const copyLink = async (url: string) => {
            try {
              await navigator.clipboard.writeText(url)
            } catch (e) {
              console.error('复制失败', e)
            }
          }

          return (
            <div
              className="modal-overlay"
            >
              <div
                className="modal modal-lg"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{title}</h2>
                  <button
                    className="close-btn"
                    onClick={() => setShowVerificationErrorModal(null)}
                  >
                    <X size={20} />
                  </button>
                </div>
                <div className="modal-body">
                  <div className="error-detail">
                    <div className="error-detail-meta">
                      <span>{t('modals.errors.account')}: {maskAccountText(account.email)}</span>
                      {vDetail?.lastErrorCode && (
                        <span>{t('wakeup.errorUi.errorCode', { code: vDetail.lastErrorCode })}</span>
                      )}
                    </div>
                    {vDetail?.lastMessage && (
                      <div className="error-detail-message" style={{ marginTop: 12 }}>
                        {vDetail.lastMessage}
                      </div>
                    )}
                  </div>
                  {!vDetail && (
                    <div className="empty-state-small" style={{ marginTop: 12 }}>
                      {t('modals.errors.empty', '暂无验证详情')}
                    </div>
                  )}

                  {/* Action buttons based on error type */}
                  <div className="modal-actions" style={{ marginTop: 20, gap: 8, flexWrap: 'wrap' }}>
                    {!isTos && vDetail?.validationUrl && (
                      <>
                        <button
                          className="btn btn-primary"
                          onClick={() => openLink(vDetail.validationUrl!)}
                        >
                          <ExternalLink size={14} />
                          {t('wakeup.errorUi.completeVerification', '立即验证')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => copyLink(vDetail.validationUrl!)}
                        >
                          <Copy size={14} />
                          {t('wakeup.errorUi.copyValidationUrl', '复制验证地址')}
                        </button>
                      </>
                    )}
                    {isTos && vDetail?.appealUrl && (
                      <>
                        <button
                          className="btn btn-primary"
                          onClick={() => openLink(vDetail.appealUrl!)}
                        >
                          <ExternalLink size={14} />
                          {t('wakeup.errorUi.submitAppeal', '立即提交保证书')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => copyLink(vDetail.appealUrl!)}
                        >
                          <Copy size={14} />
                          {t('wakeup.errorUi.copyAppealUrl', '复制链接')}
                        </button>
                      </>
                    )}
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowVerificationErrorModal(null)}
                    >
                      {t('common.close')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )
        })()}

      {/* 标签编辑弹窗 */}
      <TagEditModal
        isOpen={!!showTagModal}
        initialTags={accounts.find((acc) => acc.id === showTagModal)?.tags || []}
        initialNotes={accounts.find((acc) => acc.id === showTagModal)?.notes ?? ''}
        availableTags={availableTags}
        onClose={() => setShowTagModal(null)}
        onSave={handleSaveTags}
      />

      {(editingAccountNoteAccount || oauthAccountNoteMode) && createPortal(
        <div className="modal-overlay">
          <div
            className="modal antigravity-account-note-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="antigravity-account-note-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2 id="antigravity-account-note-title">
                {t('accounts.accountNote.title', '账号备注')}
              </h2>
              <button
                type="button"
                className="modal-close"
                onClick={closeAccountNoteModal}
                disabled={savingAccountNote}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body antigravity-account-note-body">
              <ModalErrorMessage
                message={accountNoteError}
                scrollKey={accountNoteErrorScrollKey}
              />
              <p className="antigravity-account-note-desc">
                {t('accounts.accountNote.desc', {
                  account: maskAccountText(activeAccountNoteEmail),
                  defaultValue: '给 {{account}} 填写密码、2FA、邮件地址、手机号和其他备注。',
                })}
              </p>
              <div className="codex-account-note-field">
                <span>{t('common.shared.columns.email', '邮箱')}</span>
                <div className="codex-account-note-readonly-row">
                  {oauthAccountNoteMode && !pendingOAuthAccount ? (
                    <input
                      className="codex-account-note-input"
                      type="email"
                      value={pendingOAuthEmailInput}
                      onChange={(event) => {
                        setPendingOAuthEmailInput(event.target.value)
                        setPendingOAuthEmailError(null)
                      }}
                      placeholder={t('codex.pendingAuth.emailPlaceholder', '输入账号邮箱')}
                      disabled={savingAccountNote}
                    />
                  ) : (
                    <span className={`codex-account-note-readonly-value ${activeAccountNoteEmail ? '' : 'is-empty'}`} title={activeAccountNoteEmail}>
                      {activeAccountNoteEmail || '-'}
                    </span>
                  )}
                  <button type="button" className="codex-account-note-icon-btn" onClick={() => void copyAccountNoteValue('email', activeAccountNoteEmail)} disabled={savingAccountNote || !activeAccountNoteEmail} aria-label={t('common.copy', '复制')}>
                    {accountNoteCopiedKey === 'email' ? <Check size={14} /> : <Copy size={14} />}
                  </button>
                </div>
                {pendingOAuthEmailError ? <span className="codex-account-note-field-error">{pendingOAuthEmailError}</span> : null}
              </div>
              <label className="codex-account-note-field">
                <span>{t('accounts.accountNote.passwordLabel', '账号密码')}</span>
                <div className="codex-account-note-input-row">
                  <input
                    className="codex-account-note-input"
                    type={accountNotePasswordVisible ? 'text' : 'password'}
                    value={activeAccountNoteForm.accountPassword}
                    onChange={(event) => updateEditingAccountNoteForm({ accountPassword: event.target.value })}
                    placeholder={t('accounts.accountNote.passwordPlaceholder', '登录密码或临时密码')}
                    disabled={savingAccountNote}
                    autoFocus
                  />
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => setAccountNotePasswordVisible((value) => !value)}
                    disabled={savingAccountNote}
                    aria-label={accountNotePasswordVisible ? t('accounts.accountNote.hide', '隐藏') : t('accounts.accountNote.show', '显示')}
                  >
                    {accountNotePasswordVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => void copyAccountNoteValue('password', activeAccountNoteForm.accountPassword)}
                    disabled={savingAccountNote || !activeAccountNoteForm.accountPassword.trim()}
                    aria-label={t('common.copy', '复制')}
                  >
                    {accountNoteCopiedKey === 'password' ? <Check size={14} /> : <Copy size={14} />}
                  </button>
                </div>
              </label>
              <label className="codex-account-note-field">
                <span>{t('accounts.accountNote.twoFactorSecretLabel', '2FA 秘钥')}</span>
                <div className="codex-account-note-input-row">
                  <input
                    className={`codex-account-note-input ${accountNoteFieldError ? 'has-error' : ''}`}
                    type={accountNoteSecretVisible ? 'text' : 'password'}
                    value={activeAccountNoteForm.twoFactorSecret}
                    onChange={(event) => updateEditingAccountNoteForm({ twoFactorSecret: event.target.value })}
                    placeholder={t('accounts.accountNote.twoFactorSecretPlaceholder', 'Base32 secret 或 otpauth:// 链接')}
                    disabled={savingAccountNote}
                  />
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => {
                      setSavedMfaRecords(loadSavedMfaRecords())
                      setAccountNoteMfaPickerOpen((value) => !value)
                    }}
                    disabled={savingAccountNote || savedMfaRecords.length === 0}
                    aria-label={t('mfaQuick.selectLabel', '选择 2FA 秘钥')}
                  >
                    <ChevronDown size={14} />
                  </button>
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => setAccountNoteSecretVisible((value) => !value)}
                    disabled={savingAccountNote}
                    aria-label={accountNoteSecretVisible ? t('accounts.accountNote.hide', '隐藏') : t('accounts.accountNote.show', '显示')}
                  >
                    {accountNoteSecretVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => void copyAccountNoteValue('twoFactorSecret', activeAccountNoteForm.twoFactorSecret)}
                    disabled={savingAccountNote || !activeAccountNoteForm.twoFactorSecret.trim()}
                    aria-label={t('common.copy', '复制')}
                  >
                    {accountNoteCopiedKey === 'twoFactorSecret' ? <Check size={14} /> : <Copy size={14} />}
                  </button>
                </div>
                {accountNoteMfaPickerOpen && savedMfaRecords.length > 0 ? (
                  <div className="codex-account-note-mfa-picker" role="listbox">
                    {savedMfaRecords.map((record) => (
                      <button
                        key={record.id}
                        type="button"
                        className={`codex-account-note-mfa-option ${record.secret.trim() === activeAccountNoteForm.twoFactorSecret.trim() ? 'is-selected' : ''}`}
                        onClick={() => {
                          updateEditingAccountNoteForm({ twoFactorSecret: record.secret })
                          setAccountNoteMfaPickerOpen(false)
                        }}
                      >
                        <span className="codex-account-note-mfa-option__main">
                          <strong>{formatMfaRecordOption(record, t('mfaQuick.unnamedSecret', '未命名秘钥'))}</strong>
                          {record.remark?.trim() ? <em>{record.remark}</em> : null}
                        </span>
                        <span className="codex-account-note-mfa-option__side">
                          {getMfaOtpToken(record.secret) || '••••••'}
                        </span>
                      </button>
                    ))}
                  </div>
                ) : null}
                {accountNoteFieldError ? (
                  <span className="codex-account-note-field-error">{accountNoteFieldError}</span>
                ) : activeAccountNoteForm.twoFactorSecret.trim() && accountNoteOtpToken ? (
                  <div className="codex-account-note-otp-preview">
                    <span>{t('accounts.accountNote.currentOtp', '当前验证码')}</span>
                    <strong>{accountNoteOtpToken}</strong>
                    <button
                      type="button"
                      className="codex-account-note-icon-btn"
                      onClick={() => void copyAccountNoteValue('otp', accountNoteOtpToken)}
                      aria-label={t('common.copy', '复制')}
                    >
                      {accountNoteCopiedKey === 'otp' ? <Check size={14} /> : <Copy size={14} />}
                    </button>
                    <em>{t('accounts.accountNote.otpRemaining', { seconds: mfaTimeRemaining, defaultValue: '{{seconds}}秒' })}</em>
                  </div>
                ) : null}
              </label>
              <label className="codex-account-note-field">
                <span>{t('accounts.accountNote.mailUrlLabel', '邮件地址')}</span>
                <div className="codex-account-note-input-row">
                  <input
                    className="codex-account-note-input"
                    type="url"
                    value={activeAccountNoteForm.mailUrl}
                    onChange={(event) => updateEditingAccountNoteForm({ mailUrl: event.target.value })}
                    placeholder={t('accounts.accountNote.mailUrlPlaceholder', '填写可打开的邮件查询网页地址')}
                    disabled={savingAccountNote}
                  />
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => void fetchAccountNoteMailPreviewForUrl(activeAccountNoteForm.mailUrl)}
                    disabled={savingAccountNote || !activeAccountNoteForm.mailUrl.trim()}
                    aria-label={t('accounts.accountNote.mailPreviewRefresh', '刷新邮件')}
                  >
                    <RefreshCw size={14} className={accountNoteMailPreviewLoading ? 'loading-spinner' : ''} />
                  </button>
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => void openUrl(activeAccountNoteForm.mailUrl.trim())}
                    disabled={savingAccountNote || !activeAccountNoteForm.mailUrl.trim()}
                    aria-label={t('accounts.accountNote.mailPreviewOpen', '浏览器查看')}
                  >
                    <ExternalLink size={14} />
                  </button>
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => void copyAccountNoteValue('mailUrl', activeAccountNoteForm.mailUrl)}
                    disabled={savingAccountNote || !activeAccountNoteForm.mailUrl.trim()}
                    aria-label={t('common.copy', '复制')}
                  >
                    {accountNoteCopiedKey === 'mailUrl' ? <Check size={14} /> : <Copy size={14} />}
                  </button>
                </div>
                {accountNoteMailPreviewLoading ? (
                  <div className="codex-account-note-mail-preview is-loading">{t('accounts.accountNote.mailPreviewLoading', '读取邮件中...')}</div>
                ) : accountNoteMailPreviewError ? (
                  <span className="codex-account-note-field-error">{accountNoteMailPreviewError}</span>
                ) : accountNoteMailPreview ? (
                  <div className={`codex-account-note-mail-preview ${accountNoteMailPreview.status === 'changed' ? 'is-changed' : ''}`}>
                    <div className="codex-account-note-mail-preview__code">
                      <span>{t('accounts.accountNote.mailPreviewCode', '最近一条邮箱验证码')}</span>
                      <strong>{accountNoteMailPreview.code}</strong>
                      <button type="button" className="codex-account-note-icon-btn" onClick={() => void copyAccountNoteValue('mailCode', accountNoteMailPreview.code)} disabled={savingAccountNote} aria-label={t('common.copy', '复制')}>
                        {accountNoteCopiedKey === 'mailCode' ? <Check size={14} /> : <Copy size={14} />}
                      </button>
                    </div>
                    <p title={accountNoteMailPreview.snippet}>{accountNoteMailPreview.snippet}</p>
                    <em className={`codex-account-note-mail-preview__status status-${accountNoteMailPreview.status}`}>
                      {accountNoteMailPreview.status === 'changed'
                        ? t('accounts.accountNote.mailPreviewStatusChanged', { defaultValue: '新验证码 · {{time}}', time: formatAntigravityMailPreviewTime(accountNoteMailPreview.fetchedAt) })
                        : accountNoteMailPreview.status === 'unchanged'
                          ? t('accounts.accountNote.mailPreviewStatusUnchanged', { defaultValue: '未变化 · {{time}}', time: formatAntigravityMailPreviewTime(accountNoteMailPreview.fetchedAt) })
                          : t('accounts.accountNote.mailPreviewStatusInitial', { defaultValue: '获取于 {{time}}', time: formatAntigravityMailPreviewTime(accountNoteMailPreview.fetchedAt) })}
                    </em>
                    {accountNoteMailPreview.truncated ? <em>{t('accounts.accountNote.mailPreviewTruncated', '内容已截断')}</em> : null}
                  </div>
                ) : null}
              </label>
              <label className="codex-account-note-field">
                <span>{t('accounts.accountNote.phoneNumberLabel', '手机号')}</span>
                <div className="codex-account-note-input-row">
                  <input
                    className="codex-account-note-input"
                    type="tel"
                    value={activeAccountNoteForm.phoneNumber}
                    onChange={(event) => updateEditingAccountNoteForm({ phoneNumber: event.target.value })}
                    placeholder={t('accounts.accountNote.phoneNumberPlaceholder', '绑定手机号')}
                    disabled={savingAccountNote}
                  />
                  <button
                    type="button"
                    className="codex-account-note-icon-btn"
                    onClick={() => void copyAccountNoteValue('phoneNumber', activeAccountNoteForm.phoneNumber)}
                    disabled={savingAccountNote || !activeAccountNoteForm.phoneNumber.trim()}
                    aria-label={t('common.copy', '复制')}
                  >
                    {accountNoteCopiedKey === 'phoneNumber' ? <Check size={14} /> : <Copy size={14} />}
                  </button>
                </div>
              </label>
              <label className="antigravity-account-note-field">
                <span>{t('accounts.accountNote.otherNoteLabel', '其他备注')}</span>
                <textarea
                  value={activeAccountNoteForm.note}
                  onChange={(event) => updateEditingAccountNoteForm({ note: event.target.value })}
                  placeholder={t('accounts.accountNote.placeholder', '其他交付备注、辅助邮箱或账号说明')}
                  maxLength={ANTIGRAVITY_ACCOUNT_NOTE_MAX_LENGTH}
                  rows={4}
                  disabled={savingAccountNote}
                  autoFocus
                />
                <span className="antigravity-account-note-count">
                  {activeAccountNoteForm.note.length}/{ANTIGRAVITY_ACCOUNT_NOTE_MAX_LENGTH}
                </span>
              </label>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={closeAccountNoteModal}
                disabled={savingAccountNote}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void handleSaveAccountNote()}
                disabled={savingAccountNote}
              >
                {savingAccountNote
                  ? t('common.saving', '保存中...')
                  : t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>,
        document.body
      )}

      {/* 账号分组管理弹窗 */}
      <AccountGroupModal
        isOpen={showAccountGroupModal}
        onClose={() => setShowAccountGroupModal(false)}
        onGroupsChanged={reloadAccountGroups}
      />

      {/* 添加到分组弹窗 */}
      <AddToGroupModal
        isOpen={showAddToGroupModal}
        onClose={() => setShowAddToGroupModal(false)}
        accountIds={Array.from(selected)}
        sourceGroupId={activeGroupId || undefined}
        onAdded={async () => {
          await reloadAccountGroups()
          setSelected(new Set())
        }}
      />

      <GroupAccountPickerModal
        isOpen={!!groupAccountPickerGroupId}
        targetGroup={groupAccountPickerGroup}
        accounts={accounts}
        accountGroups={accountGroups}
        verificationStatusMap={verificationStatusMap}
        getVerificationBadge={getVerificationBadge}
        maskAccountText={maskAccountText}
        onClose={() => setGroupAccountPickerGroupId(null)}
        onConfirm={({ name, accountIds }) =>
          handleAssignAccountsToGroup(groupAccountPickerGroupId!, name, accountIds)
        }
      />
      <GroupAccountPickerModal
        isOpen={!!groupQuickAddGroupId}
        targetGroup={groupQuickAddGroup}
        accounts={accounts}
        accountGroups={accountGroups}
        verificationStatusMap={verificationStatusMap}
        getVerificationBadge={getVerificationBadge}
        maskAccountText={maskAccountText}
        onClose={() => setGroupQuickAddGroupId(null)}
        onConfirm={({ name, accountIds }) =>
          handleAssignAccountsToGroup(groupQuickAddGroupId!, name, accountIds)
        }
        mode="addAccounts"
      />

      {/* 文件损坏弹窗 */}
      {fileCorruptedError && (
        <FileCorruptedModal
          error={fileCorruptedError}
          onClose={() => setFileCorruptedError(null)}
        />
      )}
    </>
  );
}
