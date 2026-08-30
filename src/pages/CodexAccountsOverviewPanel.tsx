import { Fragment } from "react";
import { createPortal } from "react-dom";
import { Plus, RefreshCw, Download, Upload, Trash2, X, Globe, KeyRound, Power, Copy, Check, Play, Pause, RotateCw, CircleAlert, Info, Rows3, LayoutGrid, List, Search, ArrowDownWideNarrow, ArrowUp, ArrowDown, GripVertical, Clock, Tag, Star, Eye, EyeOff, BookOpen, FileText, ExternalLink, Pencil, FolderOpen, FolderPlus, ChevronRight, LogOut, Terminal, ChevronDown } from "lucide-react";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { TagEditModal } from "../components/TagEditModal";
import { ExportJsonModal } from "../components/ExportJsonModal";
import { ModalErrorMessage } from "../components/ModalErrorMessage";
import { PaginationControls } from "../components/PaginationControls";
import { CodexAccountGroupModal, CodexAddToGroupModal } from "../components/CodexAccountGroupModal";
import { CodexGroupAccountPickerModal } from "../components/CodexGroupAccountPickerModal";
import { CodexLocalAccessModal } from "../components/CodexLocalAccessModal";
import { CodexAccountPoolHealthModal } from "../components/CodexAccountPoolHealthModal";
import { isCodexApiKeyAccount, isCodexAgentIdentityAccount, isCodexWebSessionAccount, isCodexChatCompletionsApiKeyAccount, isCodexNewApiAccount } from "../types/codex";
import { isCodexOAuthBindingEligibleAccount } from "../utils/codexLocalAccessAccounts";
import { CodexModelContextWindowTable } from "../components/codex/CodexModelContextWindowTable";
import { QuickSettingsPopover } from "../components/QuickSettingsPopover";
import { MultiSelectFilterDropdown } from "../components/MultiSelectFilterDropdown";
import { AccountTagFilterDropdown } from "../components/AccountTagFilterDropdown";
import { SingleSelectFilterDropdown } from "../components/SingleSelectFilterDropdown";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { CODEX_API_PROVIDER_CUSTOM_ID, CODEX_API_PROVIDER_PRESETS, COCKPIT_API_PROVIDER_ID } from "../utils/codexProviderPresets";
import { formatCodexQuotaPoolBalance, formatCodexQuotaPoolPercent, formatCodexQuotaPoolWindowLabel } from "../utils/codexQuotaPool";
import { getCodexLocalAccessRiskNoticeConfirmLabel } from "../utils/codexLocalAccessRiskNotice";
import { getMfaOtpToken } from "../utils/mfaVault";
import type { CodexExportFormat } from "../utils/codexExportFormats";
import type { CodexAccountsViewProps } from "./CodexAccountsView";
import { CodexAddAccountDialog } from "./CodexAddAccountDialog";
import { parseCodexSwitchAuthFailure } from "../utils/codexSwitchAuthFailure";


/** 渲染 CodexAccountsView 的 activeTab === "overview" 业务面板。 */
export function CodexAccountsOverviewPanel(props: CodexAccountsViewProps) {
  const {
    accountNoteCopiedKey,
    accountNoteError,
    accountNoteErrorScrollKey,
    accountNoteFieldErrors,
    accountNoteMailPreview,
    accountNoteMailPreviewError,
    accountNoteMailPreviewLoading,
    accountNoteMfaPickerOpen,
    accountNotePasswordVisible,
    accountNoteSecretVisible,
    accounts,
    activeAccountNoteDisplayName,
    activeAccountNoteEmail,
    activeAccountNoteForm,
    activeAccountNoteMode,
    activeAccountNoteOtpToken,
    activeAccountNoteSaving,
    activeAccountUsesPersonalAccessToken,
    activeGroup,
    activeGroupId,
    authFailedExportAccountIds,
    availableTags,
    batchDeleteBusy,
    batchDeleteJob,
    batchDeleteModalError,
    batchImportBusy,
    batchImportOpen,
    batchImportPreview,
    batchImportProgress,
    batchImportResult,
    batchImportSessionId,
    canOpenFormattedExportSavedDirectory,
    canSelectAllFilteredAccounts,
    clearAllOverviewFilters,
    clearFilterTypes,
    clearGroupFilter,
    clearTagFilter,
    closeAccountNoteModal,
    closeApiKeyCredentialsModal,
    closeLocalAccessRiskNotice,
    closeOAuthBindingModal,
    closeOAuthBindingQuotaReserveEditor,
    closeQuickSwitchModal,
    closeResetCreditConfirmModal,
    codexAccountSortOptions,
    codexGroups,
    codexOverviewGroupFilterOptions,
    confirmCodexDelete,
    confirmDeleteGroup,
    confirmDeleteTag,
    confirmHideLocalAccessEntry,
    confirmOAuthBindingQuotaReserveEditor,
    copyAccountNoteValue,
    copyFormattedExportJson,
    copyFormattedExportSavedPath,
    customSortAccounts,
    customSortDropTargetId,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    deletingGroup,
    deletingTag,
    draggedCustomSortAccountId,
    editingApiBaseUrlCredentialsValue,
    editingApiKeyCredentialsId,
    editingApiKeyCredentialsValue,
    editingApiKeyCredentialsVisible,
    editingApiModelCatalogDraft,
    editingApiModelCatalogError,
    editingApiModelCatalogFetching,
    editingApiModelCatalogInput,
    editingApiModelCatalogSyncAvailable,
    editingApiModelContextWindowsInput,
    editingApiProviderPresetId,
    editingApiSyncModelCatalogToCodex,
    editingManagedProviderApiKeyId,
    editingManagedProviderId,
    editingNewManagedProviderNameInput,
    errorAccountIds,
    exportCanIncludeSensitiveNotes,
    exportFormat,
    exportFormatOptions,
    exportHasAgentIdentity,
    exporting,
    exportJsonHidden,
    exportModalError,
    exportModalErrorScrollKey,
    exportSelectionCount,
    filteredAccounts,
    filteredIds,
    filterTypes,
    formatCodexAccountNoteMailPreviewTime,
    formatCodexManagedApiKeyOptionLabel,
    formatMfaRecordOption,
    formatMfaSecretPreview,
    formatResetCreditAbsoluteTime,
    formatResetCreditTime,
    formattedExportJsonContent,
    formattedExportJsonCopied,
    formattedExportModalCustomContent,
    formattedExportPathCopied,
    formattedExportSavedPath,
    formattedSavingExportJson,
    getResetCreditStatusLabel,
    getResetCreditStatusTone,
    groupByTag,
    groupDeleteConfirm,
    groupDeleteError,
    groupDeleteErrorScrollKey,
    groupFilter,
    groupQuickAddGroup,
    groupQuickAddGroupId,
    handleChangeOverviewLayoutMode,
    handleClearBatchDelete,
    handleClearErrorAccounts,
    handleClearLocalAccessStats,
    handleClearOAuthBinding,
    handleClearOverviewSelection,
    handleCloseExportModal,
    handleCodexBatchDelete,
    handleConfirmConsumeResetCredit,
    handleCustomSortDragMove,
    handleCustomSortDragStart,
    handleDismissBatchImportTask,
    handleEditingApiBaseUrlCredentialsChange,
    handleEditingApiKeyCredentialsChange,
    handleExport,
    handleExportAuthFailedAccounts,
    handleFetchEditingApiModelCatalog,
    handleKillLocalAccessPort,
    handleLeaveGroup,
    handleLocalAccessAddressKindChange,
    handleOAuthBindingQuotaReserveToggle,
    handleOpenAccountNoteMailUrl,
    handleOpenProviderLink,
    handlePauseBatchDelete,
    handlePendingOAuthEmailInputChange,
    handleQuickAddAccountsToGroup,
    handleReauthorizeOAuthBinding,
    handleRecoverLocalAccessAccounts,
    handleRefreshAccountNoteMailPreview,
    handleRefreshAll,
    handleRemoveFromGroup,
    handleRestartLocalAccessSidecar,
    handleResumeBatchDelete,
    handleRetryFailedBatchDelete,
    handleRotateLocalAccessApiKey,
    handleSaveLocalAccessAccounts,
    handleSaveTags,
    handleSelectAllFilteredAccounts,
    handleSelectEditingApiProviderPreset,
    handleSelectEditingManagedProvider,
    handleSelectEditingManagedProviderApiKey,
    handleSelectQuickSwitchApiKey,
    handleSelectQuickSwitchProvider,
    handleSortByChange,
    handleSubmitAccountNote,
    handleSubmitApiKeyCredentials,
    handleSubmitOAuthBinding,
    handleSubmitQuickSwitch,
    handleToggleExportJsonHidden,
    handleToggleLocalAccessEnabled,
    handleToggleSelectAllPaginated,
    handleUpdateLocalAccessAccessScope,
    handleUpdateLocalAccessCustomRouting,
    handleUpdateLocalAccessPort,
    handleUpdateLocalAccessRoutingStrategy,
    handleUpdateLocalAccessUpstreamProxyConfig,
    hasActiveOverviewFilters,
    hasDetectableFullQuotaWakeupAccounts,
    hasGroupEntryCards,
    includeExportSensitiveNotes,
    inlineFolderCards,
    isAllFilteredSelectionActive,
    isAllPaginatedSelected,
    isCustomSortActive,
    isLocalAccessOAuthBinding,
    isResetCreditConfirmSubmitting,
    loading,
    localAccessAddressOptions,
    localAccessCollection,
    localAccessHealthActionBusy,
    localAccessHideSubmitting,
    localAccessModalMode,
    localAccessModalSelectedIds,
    localAccessPortKilling,
    localAccessQuotaPoolLabels,
    localAccessQuotaPoolSummary,
    localAccessRiskNoticeAction,
    localAccessRiskNoticeRemember,
    localAccessSaving,
    localAccessSidecarRestarting,
    localAccessStarting,
    localAccessState,
    managedProviders,
    managedProvidersLoading,
    maskAccountText,
    message,
    mfaTimeRemaining,
    moveCustomSortAccount,
    oauthAccounts,
    oauthBindingAccount,
    oauthBindingAvailableTags,
    oauthBindingEligibleAccounts,
    oauthBindingError,
    oauthBindingErrorScrollKey,
    oauthBindingFilteredAccounts,
    oauthBindingHasExistingBinding,
    oauthBindingHourlyReserveDraft,
    oauthBindingHourlyReserveInputRef,
    oauthBindingPagination,
    oauthBindingQuotaReserve,
    oauthBindingQuotaReserveEditorOpen,
    oauthBindingQuotaReserveFieldErrors,
    oauthBindingSaving,
    oauthBindingSelectedAccountId,
    oauthBindingTargetActive,
    oauthBindingTargetKind,
    oauthBindingTierCounts,
    oauthBindingTierFilterOptions,
    oauthBindingWeeklyReserveDraft,
    oauthBindingWeeklyReserveInputRef,
    OPENAI_OFFICIAL_PRESET_ID,
    openCodexAddModal,
    openCodexApiServicePage,
    openFormattedExportSavedDirectory,
    openFullQuotaWakeupTestModal,
    openOAuthBindingQuotaReserveEditor,
    overviewAccounts,
    overviewCurrentAccountId,
    overviewFilterChips,
    overviewLayoutMode,
    overviewTotalCount,
    overviewVisibleCount,
    page,
    paginatedAccounts,
    paginatedGroupedAccounts,
    pagination,
    pendingOAuthEmailInput,
    pendingOAuthFieldErrors,
    pendingWebSessionImport,
    performTokenImport,
    privacyModeEnabled,
    quickSwitchAccount,
    quickSwitchAccountId,
    quickSwitchApiKeyId,
    quickSwitchError,
    quickSwitchProviderId,
    quickSwitchSubmitting,
    refreshingAll,
    refreshSavedMfaRecords,
    reloadCodexGroups,
    reloadLocalAccessState,
    renderCompactRows,
    renderGridCards,
    renderGroupTableRows,
    renderTableRows,
    reportExportModalError,
    requestDeleteTag,
    resetCreditConfirmAccount,
    resetCreditConfirmActionLocked,
    resetCreditConfirmAvailableCount,
    resetCreditConfirmCredits,
    resetCreditConfirmError,
    resetCreditConfirmErrorScrollKey,
    resetCreditConfirmLoading,
    resetCreditConfirmNextExpiresAt,
    resetCustomSortOrder,
    resolveGroupLabel,
    resolvePresentation,
    resolveSubscriptionPresentation,
    savedMfaRecords,
    saveFormattedExportJson,
    savingApiKeyCredentials,
    searchQuery,
    selected,
    selectedEditingApiProviderPreset,
    selectedEditingManagedProvider,
    selectedLocalAccessAddressKind,
    selectedOAuthBindingAccount,
    selectedQuickSwitchApiKey,
    selectedQuickSwitchProvider,
    setAccountNoteMfaPickerOpen,
    setAccountNotePasswordVisible,
    setAccountNoteSecretVisible,
    setActiveTab,
    setBatchImportOpen,
    setDeleteConfirm,
    setEditingApiKeyCredentialsVisible,
    setEditingApiModelCatalogError,
    setEditingApiModelCatalogInput,
    setEditingApiModelContextWindowsInput,
    setEditingApiSyncModelCatalogToCodex,
    setEditingNewManagedProviderNameInput,
    setExportFormat,
    setGroupByTag,
    setGroupDeleteConfirm,
    setGroupDeleteError,
    setGroupQuickAddGroupId,
    setIncludeExportSensitiveNotes,
    setLocalAccessRiskNoticeRemember,
    setLocalAccessState,
    setMessage,
    setOauthBindingError,
    setOauthBindingHourlyReserveDraft,
    setOauthBindingQuotaReserveFieldErrors,
    setOauthBindingSelectedAccountId,
    setOauthBindingWeeklyReserveDraft,
    setPendingWebSessionImport,
    setSearchQuery,
    setShowAddToCodexGroupModal,
    setShowCodexGroupModal,
    setShowCustomSortModal,
    setShowLocalAccessHealthModal,
    setShowLocalAccessHideConfirm,
    setShowLocalAccessModal,
    setShowLocalAccessQuotaStatsModal,
    setShowTagFilter,
    setShowTagModal,
    setSortBy,
    setSortDirection,
    setTagDeleteConfirm,
    showAddToCodexGroupModal,
    showCodexGroupModal,
    showCustomSortModal,
    showExportModal,
    showLocalAccessHealthModal,
    showLocalAccessHideConfirm,
    showLocalAccessModal,
    showLocalAccessQuotaStatsModal,
    showOverviewFilterBanner,
    showOverviewSelectionBar,
    showTagFilter,
    showTagModal,
    sortBy,
    sortDirection,
    stopCustomSortDragging,
    store,
    t,
    tagDeleteConfirm,
    tagDeleteConfirmError,
    tagDeleteConfirmErrorScrollKey,
    tagFilter,
    tagFilterRef,
    tierCounts,
    tierFilterOptions,
    toggleFilterTypeValue,
    toggleGroupFilterValue,
    togglePrivacyMode,
    toggleTagFilterValue,
    updateActiveAccountNoteForm,
    validateOAuthBindingQuotaReserveField,
    viewMode,
  } = props;
  return (
        <>
          {message && (
            <div
              className={`message-bar ${message.tone === "error" ? "error" : "success"}`}
            >
              {message.text}
              <button onClick={() => setMessage(null)}>
                <X size={14} />
              </button>
            </div>
          )}

          {activeGroup && (
            <div className="folder-breadcrumb">
              <button className="breadcrumb-back" onClick={handleLeaveGroup}>
                <FolderOpen size={14} />
                {t("accounts.groups.allGroups")}
              </button>
              <ChevronRight size={14} className="breadcrumb-sep" />
              <span className="breadcrumb-current">
                {activeGroup.name}
                <span className="breadcrumb-count">
                  ({filteredAccounts.length})
                </span>
              </span>
              <button
                className="btn btn-secondary breadcrumb-remove-btn"
                onClick={() => setGroupQuickAddGroupId(activeGroup.id)}
                title={t("accounts.groups.addAccounts")}
              >
                <FolderPlus size={14} />
                {t("accounts.groups.addAccounts")}
              </button>
              {selected.size > 0 && (
                <button
                  className="btn btn-secondary breadcrumb-remove-btn"
                  onClick={() => void handleRemoveFromGroup()}
                  title={t("accounts.groups.removeFromGroup")}
                >
                  <LogOut size={14} />
                  {t("accounts.groups.removeFromGroup")} ({selected.size})
                </button>
              )}
            </div>
          )}

          <div className="toolbar">
            <div className="toolbar-left">
              <div className="search-box">
                <Search size={16} className="search-icon" />
                <input
                  type="text"
                  placeholder={t("common.shared.search", "搜索账号...")}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                />
              </div>
              <div className="view-switcher">
                <button
                  className={`view-btn ${overviewLayoutMode === "compact" ? "active" : ""}`}
                  onClick={() => handleChangeOverviewLayoutMode("compact")}
                  title={t("accounts.view.compact", "紧凑视图")}
                >
                  <Rows3 size={16} />
                </button>
                <button
                  className={`view-btn ${overviewLayoutMode === "list" ? "active" : ""}`}
                  onClick={() => handleChangeOverviewLayoutMode("list")}
                  title={t("common.shared.view.list", "列表视图")}
                >
                  <List size={16} />
                </button>
                <button
                  className={`view-btn ${overviewLayoutMode === "grid" ? "active" : ""}`}
                  onClick={() => handleChangeOverviewLayoutMode("grid")}
                  title={t("common.shared.view.grid", "卡片视图")}
                >
                  <LayoutGrid size={16} />
                </button>
              </div>
              <MultiSelectFilterDropdown
                options={tierFilterOptions}
                selectedValues={filterTypes}
                allLabel={t("codex.filters.allPlans", {
                  count: tierCounts.all,
                  defaultValue: "全部套餐 ({{count}})",
                })}
                filterLabel={t("common.shared.filterLabel", "筛选")}
                clearLabel={t("accounts.clearFilter", "清空筛选")}
                emptyLabel={t("common.none", "暂无")}
                ariaLabel={t("common.shared.filterLabel", "筛选")}
                onToggleValue={toggleFilterTypeValue}
                onClear={clearFilterTypes}
              />
              <div className="tag-filter" ref={tagFilterRef}>
                <button
                  type="button"
                  className={`tag-filter-btn ${tagFilter.length > 0 ? "active" : ""}`}
                  onClick={() => setShowTagFilter((prev) => !prev)}
                  aria-label={t("accounts.filterTags", "标签筛选")}
                >
                  <Tag size={14} />
                  {tagFilter.length > 0
                    ? `${t("accounts.filterTagsCount", "标签")}(${tagFilter.length})`
                    : t("accounts.filterTags", "标签筛选")}
                </button>
                {showTagFilter && (
                  <div
                    ref={page.tagFilterPanelRef}
                    className={`tag-filter-panel ${page.tagFilterPanelPlacement === "top" ? "open-top" : ""}`}
                  >
                    {availableTags.length === 0 ? (
                      <div className="tag-filter-empty">
                        {t("accounts.noAvailableTags", "暂无可用标签")}
                      </div>
                    ) : (
                      <div
                        className="tag-filter-options"
                        style={page.tagFilterScrollContainerStyle}
                      >
                        {availableTags.map((tag) => (
                          <label
                            key={tag}
                            className={`tag-filter-option ${tagFilter.includes(tag) ? "selected" : ""}`}
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
                              onClick={(e) => {
                                e.preventDefault();
                                e.stopPropagation();
                                requestDeleteTag(tag);
                              }}
                              aria-label={t("accounts.deleteTagAria", {
                                tag,
                                defaultValue: "删除标签 {{tag}}",
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
                        onChange={(e) => setGroupByTag(e.target.checked)}
                      />
                      <span>{t("accounts.groupByTag", "按标签分组展示")}</span>
                    </label>
                    {tagFilter.length > 0 && (
                      <button
                        type="button"
                        className="tag-filter-clear"
                        onClick={clearTagFilter}
                      >
                        {t("accounts.clearFilter", "清空筛选")}
                      </button>
                    )}
                  </div>
                )}
              </div>

              <SingleSelectFilterDropdown
                value={sortBy}
                options={codexAccountSortOptions}
                ariaLabel={t("common.shared.sortLabel", "排序")}
                icon={<ArrowDownWideNarrow size={14} />}
                onChange={handleSortByChange}
              />
              {!isCustomSortActive && (
                <button
                  className="sort-direction-btn"
                  onClick={() =>
                    setSortDirection((prev) =>
                      prev === "desc" ? "asc" : "desc",
                    )
                  }
                  title={
                    sortDirection === "desc"
                      ? t(
                          "common.shared.sort.descTooltip",
                          "当前：降序，点击切换为升序",
                        )
                      : t(
                          "common.shared.sort.ascTooltip",
                          "当前：升序，点击切换为降序",
                        )
                  }
                  aria-label={t(
                    "common.shared.sort.toggleDirection",
                    "切换排序方向",
                  )}
                >
                  {sortDirection === "desc" ? "⬇" : "⬆"}
                </button>
              )}
            </div>
            <div className="toolbar-right">
              <button
                className="btn btn-primary icon-only"
                onClick={() => openCodexAddModal("oauth")}
                title={t("common.shared.addAccount", "添加账号")}
              >
                <Plus size={14} />
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={handleRefreshAll}
                disabled={refreshingAll || accounts.length === 0}
                title={t("common.shared.refreshAll", "刷新全部")}
              >
                <RefreshCw
                  size={14}
                  className={refreshingAll ? "loading-spinner" : ""}
                />
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={togglePrivacyMode}
                title={
                  privacyModeEnabled
                    ? t("privacy.showSensitive", "显示邮箱")
                    : t("privacy.hideSensitive", "隐藏邮箱")
                }
              >
                {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
              <button
                className="btn btn-secondary export-btn icon-only"
                onClick={() => void handleExport(filteredIds)}
                disabled={exporting || filteredIds.length === 0}
                title={
                  exportSelectionCount > 0
                    ? `${t("common.shared.export.title", "导出")} (${exportSelectionCount})`
                    : t("common.shared.export.title", "导出")
                }
              >
                <Upload size={14} />
              </button>
              {!activeGroupId && (
                <button
                  className={`btn btn-secondary icon-only ${groupFilter.length > 0 ? "btn-filter-active" : ""}`}
                  onClick={() => setShowCodexGroupModal(true)}
                  title={
                    groupFilter.length > 0
                      ? `${t("accounts.groups.manageTitle", "分组管理")} (${groupFilter.length})`
                      : t("accounts.groups.manageTitle", "分组管理")
                  }
                >
                  <FolderOpen size={14} />
                </button>
              )}
              <QuickSettingsPopover type="codex" />
            </div>
          </div>

          {(showOverviewFilterBanner || hasActiveOverviewFilters) && (
            <div
              className={`codex-overview-filter-banner${
                showOverviewFilterBanner ? " is-active" : ""
              }`}
              role="status"
            >
              <div className="codex-overview-filter-banner-main">
                <span className="codex-overview-filter-banner-count">
                  {t("codex.filters.visibleOfTotal", {
                    visible: overviewVisibleCount,
                    total: overviewTotalCount,
                    defaultValue: "显示 {{visible}} / 共 {{total}}",
                  })}
                </span>
                {showOverviewFilterBanner && (
                  <span className="codex-overview-filter-banner-text">
                    {t("codex.filters.activeBanner", {
                      visible: overviewVisibleCount,
                      total: overviewTotalCount,
                      defaultValue:
                        "当前筛选仅显示 {{visible}}/{{total}} 个账号",
                    })}
                  </span>
                )}
                {overviewFilterChips.length > 0 && (
                  <span className="codex-overview-filter-banner-chips">
                    {overviewFilterChips.join(" · ")}
                  </span>
                )}
              </div>
              <button
                type="button"
                className="btn btn-secondary codex-overview-filter-clear-btn"
                onClick={clearAllOverviewFilters}
              >
                {t("codex.filters.clearAll", "清除筛选")}
              </button>
            </div>
          )}

          {loading && accounts.length === 0 ? (
            <div className="loading-container">
              <RefreshCw size={24} className="loading-spinner" />
              <p>{t("common.loading", "加载中...")}</p>
            </div>
          ) : accounts.length === 0 && !hasGroupEntryCards ? (
            <div className="empty-state">
              <Globe size={48} />
              <h3>{t("common.shared.empty.title", "暂无账号")}</h3>
              <p>
                {t(
                  "codex.empty.description",
                  '点击"添加账号"开始管理您的 Codex 账号',
                )}
              </p>
              <div
                style={{
                  display: "flex",
                  gap: "12px",
                  justifyContent: "center",
                  marginTop: "16px",
                }}
              >
                <button
                  className="btn btn-primary"
                  onClick={() => openCodexAddModal("oauth")}
                >
                  <Plus size={16} />
                  {t("common.shared.addAccount", "添加账号")}
                </button>
                <button
                  className="btn btn-secondary"
                  onClick={() =>
                    window.dispatchEvent(
                      new CustomEvent("app-request-navigate", {
                        detail: "manual",
                      }),
                    )
                  }
                >
                  <BookOpen size={16} />
                  {t("manual.navTitle", "功能使用手册")}
                </button>
              </div>
            </div>
          ) : filteredAccounts.length === 0 && !hasGroupEntryCards ? (
            <div className="empty-state">
              <h3>{t("common.shared.noMatch.title", "没有匹配的账号")}</h3>
              <p>
                {t("common.shared.noMatch.desc", "请尝试调整搜索或筛选条件")}
              </p>
              {hasActiveOverviewFilters && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={clearAllOverviewFilters}
                >
                  {t("codex.filters.clearAll", "清除筛选")}
                </button>
              )}
            </div>
          ) : (
            <>
              {showOverviewSelectionBar && (
                <div className="codex-overview-selection-bar">
                  <div className="codex-overview-selection-left">
                    <label className="codex-overview-select-all">
                      <input
                        type="checkbox"
                        checked={isAllPaginatedSelected}
                        onChange={handleToggleSelectAllPaginated}
                      />
                      <span>{t("common.selectAll", "全选")}</span>
                    </label>
                    {selected.size > 0 && !isAllFilteredSelectionActive && (
                      <span className="codex-overview-selected-count">
                        {t(
                          "codex.apiService.customRoutingSelected",
                          "已选 {{count}}",
                        ).replace("{{count}}", String(selected.size))}
                      </span>
                    )}
                    {canSelectAllFilteredAccounts && (
                      <button
                        type="button"
                        className="codex-overview-select-filtered-btn"
                        onClick={handleSelectAllFilteredAccounts}
                      >
                        {t("messages.selectAllFilteredAccounts", {
                          count: filteredIds.length,
                          defaultValue: "选择全部符合条件 {{count}} 条",
                        })}
                      </button>
                    )}
                    {isAllFilteredSelectionActive && (
                      <>
                        <span className="codex-overview-selected-count">
                          {t("messages.selectedAllFilteredAccounts", {
                            count: filteredIds.length,
                            defaultValue: "已选择全部符合条件 {{count}} 条",
                          })}
                        </span>
                        <button
                          type="button"
                          className="codex-overview-clear-selection-btn"
                          onClick={handleClearOverviewSelection}
                        >
                          {t("messages.clearSelection", "取消选择")}
                        </button>
                      </>
                    )}
                  </div>
                  {(selected.size > 0 ||
                    errorAccountIds.length > 0 ||
                    authFailedExportAccountIds.length > 0 ||
                    hasDetectableFullQuotaWakeupAccounts) && (
                    <div className="codex-overview-selection-actions">
                      <button
                        type="button"
                        className="btn btn-secondary codex-overview-full-quota-wakeup-btn"
                        onClick={openFullQuotaWakeupTestModal}
                        disabled={!hasDetectableFullQuotaWakeupAccounts}
                        title={t(
                          "codex.wakeup.fullQuotaActionTitle",
                          "打开账号唤醒测试，账号默认按 5h 额度从高到低排序。",
                        )}
                      >
                        <Power size={14} />
                        <span>
                          {t("codex.wakeup.fullQuotaAction", "唤醒账号")}
                        </span>
                      </button>
                      {authFailedExportAccountIds.length > 0 && (
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={handleExportAuthFailedAccounts}
                          disabled={exporting}
                          title={t(
                            "codex.exportAuthFailedTitle",
                            "导出全部授权失败账号",
                          )}
                        >
                          <Download size={14} />
                          <span>
                            {t("codex.exportAuthFailed", "导出失败账号")}
                            {` (${authFailedExportAccountIds.length})`}
                          </span>
                        </button>
                      )}
                      {errorAccountIds.length > 0 && (
                        <button
                          className="btn btn-danger icon-only codex-overview-clear-error-btn"
                          onClick={handleClearErrorAccounts}
                          title={`${t("messages.cleanErrorAccountsAction", "清理 ERROR 账号")} (${errorAccountIds.length})`}
                        >
                          <CircleAlert size={14} />
                        </button>
                      )}
                      {selected.size > 0 && (
                        <>
                          <button
                            className="btn btn-secondary icon-only"
                            onClick={() => setShowAddToCodexGroupModal(true)}
                            title={
                              activeGroupId
                                ? `${t("accounts.groups.moveToGroup")} (${selected.size})`
                                : `${t("codex.groups.addToGroup", "添加至分组")} (${selected.size})`
                            }
                          >
                            <FolderPlus size={14} />
                          </button>
                          <button
                            className="btn btn-danger icon-only"
                            onClick={handleCodexBatchDelete}
                            title={`${t("common.delete", "删除")} (${selected.size})`}
                          >
                            <Trash2 size={14} />
                          </button>
                        </>
                      )}
                    </div>
                  )}
                </div>
              )}
              {batchDeleteJob && (
                <div className="codex-batch-delete-job">
                  <div className="codex-batch-delete-job__head">
                    <div>
                      <strong>{t("codex.batchDelete.title")}</strong>
                      <span>
                        {t("codex.batchDelete.summary", {
                          completed: batchDeleteJob.completed,
                          total: batchDeleteJob.total,
                          failed: batchDeleteJob.failed,
                        })}
                      </span>
                    </div>
                    <span
                      className={`codex-batch-delete-job__status ${batchDeleteJob.status}`}
                    >
                      {t(`codex.batchDelete.${batchDeleteJob.status}`)}
                    </span>
                  </div>
                  <div className="codex-batch-delete-job__progress">
                    <span
                      style={{
                        width: `${Math.min(
                          100,
                          Math.round(
                            (batchDeleteJob.completed /
                              Math.max(1, batchDeleteJob.total)) *
                              100,
                          ),
                        )}%`,
                      }}
                    />
                  </div>
                  {batchDeleteJob.errors.length > 0 && (
                    <div className="codex-batch-delete-job__errors">
                      {batchDeleteJob.errors.slice(0, 5).map((item) => (
                        <span key={`${item.accountId}-${item.error}`}>
                          {item.accountId}: {item.error}
                        </span>
                      ))}
                    </div>
                  )}
                  <div className="codex-batch-delete-job__actions">
                    {batchDeleteJob.status === "running" && (
                      <button
                        className="btn btn-secondary"
                        onClick={handlePauseBatchDelete}
                        disabled={batchDeleteBusy}
                      >
                        <Pause size={14} />
                        <span>{t("codex.batchDelete.pause")}</span>
                      </button>
                    )}
                    {batchDeleteJob.status === "paused" && (
                      <button
                        className="btn btn-primary"
                        onClick={handleResumeBatchDelete}
                        disabled={batchDeleteBusy}
                      >
                        <Play size={14} />
                        <span>{t("codex.batchDelete.resume")}</span>
                      </button>
                    )}
                    {batchDeleteJob.status === "failed" &&
                      batchDeleteJob.failed > 0 && (
                        <button
                          className="btn btn-secondary"
                          onClick={handleRetryFailedBatchDelete}
                          disabled={batchDeleteBusy}
                        >
                          <RotateCw size={14} />
                          <span>{t("codex.batchDelete.retryFailed")}</span>
                        </button>
                      )}
                    {batchDeleteJob.status !== "running" && (
                      <button
                        className="btn btn-secondary"
                        onClick={handleClearBatchDelete}
                        disabled={batchDeleteBusy}
                      >
                        <X size={14} />
                        <span>{t("codex.batchDelete.clear")}</span>
                      </button>
                    )}
                  </div>
                </div>
              )}
              {batchImportSessionId &&
                !batchImportOpen &&
                !batchImportResult && (
                  <div className="codex-batch-import-task">
                    <div className="codex-batch-import-task__copy">
                      <strong>
                        {t(
                          "codex.batchImport.hiddenTask",
                          "Codex 批量导入进行中",
                        )}
                      </strong>
                      <span>
                        {batchImportBusy
                          ? t(
                              "codex.batchImport.taskRunning",
                              "进度 {{current}}/{{total}}",
                            )
                              .replace(
                                "{{current}}",
                                String(batchImportProgress?.current ?? 0),
                              )
                              .replace(
                                "{{total}}",
                                String(
                                  batchImportProgress?.total ??
                                    batchImportPreview?.total ??
                                    0,
                                ),
                              )
                          : batchImportPreview
                            ? t(
                                "codex.batchImport.taskPreview",
                                "已解析 {{total}} 个账号，可继续导入",
                              ).replace(
                                "{{total}}",
                                String(batchImportPreview.total),
                              )
                            : t(
                                "codex.batchImport.preparing",
                                "正在准备导入任务...",
                              )}
                      </span>
                    </div>
                    <div className="codex-batch-import-task__actions">
                      <button
                        className="btn btn-secondary"
                        onClick={() => setBatchImportOpen(true)}
                      >
                        <FileText size={14} />
                        <span>{t("codex.batchImport.reopen", "查看任务")}</span>
                      </button>
                      <button
                        className="btn btn-secondary"
                        onClick={handleDismissBatchImportTask}
                        title={
                          batchImportBusy
                            ? t(
                                "codex.batchImport.cancelAndDismiss",
                                "取消并丢弃任务",
                              )
                            : t("codex.batchImport.dismissTask", "丢弃任务")
                        }
                      >
                        <X size={14} />
                        <span>
                          {batchImportBusy
                            ? t("common.cancel", "取消")
                            : t("codex.batchImport.dismissTask", "丢弃")}
                        </span>
                      </button>
                    </div>
                  </div>
                )}
              {overviewLayoutMode === "compact" ? (
                <>
                  {inlineFolderCards && (
                    <div className="codex-group-entry-grid">
                      {inlineFolderCards}
                    </div>
                  )}
                  {groupByTag ? (
                    <div className="tag-group-list">
                      {paginatedGroupedAccounts.map(
                        ({ groupKey, items, totalCount }) => (
                          <div key={groupKey} className="tag-group-section">
                            <div className="tag-group-header">
                              <span className="tag-group-title">
                                {resolveGroupLabel(groupKey)}
                              </span>
                              <span className="tag-group-count">
                                {totalCount}
                              </span>
                            </div>
                            <div className="codex-compact-list">
                              {renderCompactRows(items, groupKey)}
                            </div>
                          </div>
                        ),
                      )}
                    </div>
                  ) : (
                    <div className="codex-compact-list">
                      {renderCompactRows(paginatedAccounts)}
                    </div>
                  )}
                </>
              ) : viewMode === "grid" ? (
                <div className="grid-view-container">
                  {!showOverviewSelectionBar &&
                    paginatedAccounts.length > 0 && (
                      <div
                        className="grid-view-header"
                        style={{ marginBottom: "12px", paddingLeft: "4px" }}
                      >
                        <label
                          style={{
                            display: "inline-flex",
                            alignItems: "center",
                            gap: "8px",
                            cursor: "pointer",
                            fontSize: "13px",
                            color: "var(--text-color)",
                          }}
                        >
                          <input
                            type="checkbox"
                            checked={isAllPaginatedSelected}
                            onChange={handleToggleSelectAllPaginated}
                          />
                          {t("common.selectAll", "全选")}
                        </label>
                      </div>
                    )}
                  {groupByTag ? (
                    <>
                      {inlineFolderCards && (
                        <div className="codex-group-entry-grid">
                          {inlineFolderCards}
                        </div>
                      )}
                      <div className="tag-group-list">
                        {paginatedGroupedAccounts.map(
                          ({ groupKey, items, totalCount }) => (
                            <div key={groupKey} className="tag-group-section">
                              <div className="tag-group-header">
                                <span className="tag-group-title">
                                  {resolveGroupLabel(groupKey)}
                                </span>
                                <span className="tag-group-count">
                                  {totalCount}
                                </span>
                              </div>
                              <div className="tag-group-grid codex-accounts-grid">
                                {renderGridCards(items, groupKey)}
                              </div>
                            </div>
                          ),
                        )}
                      </div>
                    </>
                  ) : (
                    <div className="codex-accounts-grid">
                      {inlineFolderCards}
                      {renderGridCards(paginatedAccounts)}
                    </div>
                  )}
                </div>
              ) : groupByTag ? (
                <>
                  {inlineFolderCards && (
                    <div className="codex-group-entry-grid">
                      {inlineFolderCards}
                    </div>
                  )}
                  <div className="account-table-container grouped">
                    <table className="account-table">
                      <thead>
                        <tr>
                          <th style={{ width: 40 }}>
                            {showOverviewSelectionBar ? null : (
                              <input
                                type="checkbox"
                                checked={isAllPaginatedSelected}
                                onChange={handleToggleSelectAllPaginated}
                              />
                            )}
                          </th>
                          <th style={{ width: 260 }}>
                            {t("common.shared.columns.email", "账号")}
                          </th>
                          <th style={{ width: 140 }}>
                            {t("common.shared.columns.plan", "订阅")}
                          </th>
                          <th style={{ width: 150 }}>
                            {t("codex.subscription.column", "订阅信息")}
                          </th>
                          <th>{t("accounts.columns.quota", "配额状态")}</th>
                          <th className="sticky-action-header table-action-header">
                            {t("common.shared.columns.actions", "操作")}
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {paginatedGroupedAccounts.map(
                          ({ groupKey, items, totalCount }) => (
                            <Fragment key={groupKey}>
                              <tr className="tag-group-row">
                                <td colSpan={6}>
                                  <div className="tag-group-header">
                                    <span className="tag-group-title">
                                      {resolveGroupLabel(groupKey)}
                                    </span>
                                    <span className="tag-group-count">
                                      {totalCount}
                                    </span>
                                  </div>
                                </td>
                              </tr>
                              {renderTableRows(items, groupKey)}
                            </Fragment>
                          ),
                        )}
                      </tbody>
                    </table>
                  </div>
                </>
              ) : (
                <>
                  {inlineFolderCards && (
                    <div className="codex-group-entry-grid">
                      {inlineFolderCards}
                    </div>
                  )}
                  <div className="account-table-container">
                    <table className="account-table">
                      <thead>
                        <tr>
                          <th style={{ width: 40 }}>
                            {showOverviewSelectionBar ? null : (
                              <input
                                type="checkbox"
                                checked={isAllPaginatedSelected}
                                onChange={handleToggleSelectAllPaginated}
                              />
                            )}
                          </th>
                          <th style={{ width: 260 }}>
                            {t("common.shared.columns.email", "账号")}
                          </th>
                          <th style={{ width: 140 }}>
                            {t("common.shared.columns.plan", "订阅")}
                          </th>
                          <th style={{ width: 150 }}>
                            {t("codex.subscription.column", "订阅信息")}
                          </th>
                          <th>{t("accounts.columns.quota", "配额状态")}</th>
                          <th className="sticky-action-header table-action-header">
                            {t("common.shared.columns.actions", "操作")}
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {renderGroupTableRows()}
                        {renderTableRows(paginatedAccounts)}
                      </tbody>
                    </table>
                  </div>
                </>
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

          {<CodexAddAccountDialog {...props} />}

          {quickSwitchAccountId && (
            <div className="modal-overlay">
              <div
                className="modal-content codex-add-modal codex-api-key-edit-modal"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t("codex.quickSwitch.title", "快速切换供应商")}</h2>
                  <button
                    className="modal-close"
                    onClick={closeQuickSwitchModal}
                    aria-label={t("common.close", "关闭")}
                    disabled={quickSwitchSubmitting}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <div className="add-section">
                    <p className="section-desc">
                      {t(
                        "codex.quickSwitch.desc",
                        "为当前 API Key 账号快速切换到已保存的供应商与 API Key。",
                      )}
                    </p>
                    {quickSwitchAccount && (
                      <div className="section-desc">
                        {t("codex.quickSwitch.currentAccount", {
                          defaultValue: "当前账号：{{name}}",
                          name: maskAccountText(
                            resolvePresentation(quickSwitchAccount).displayName,
                          ),
                        })}
                      </div>
                    )}
                    <div className="oauth-link">
                      <label>
                        {t(
                          "codex.modelProviders.selectSavedProvider",
                          "已保存供应商",
                        )}
                      </label>
                      {managedProvidersLoading ? (
                        <div className="section-desc">
                          {t("common.loading", "加载中...")}
                        </div>
                      ) : managedProviders.length === 0 ? (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>
                            {t(
                              "codex.quickSwitch.noProviders",
                              "暂无已保存供应商，请先在“模型供应商”中添加。",
                            )}
                          </span>
                        </div>
                      ) : (
                        <div className="api-provider-chip-list">
                          {managedProviders.map((provider) => (
                            <button
                              key={provider.id}
                              className={`api-provider-chip ${quickSwitchProviderId === provider.id ? "active" : ""}`}
                              onClick={() =>
                                handleSelectQuickSwitchProvider(provider.id)
                              }
                              type="button"
                              disabled={quickSwitchSubmitting}
                            >
                              <span>{provider.name}</span>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>

                    {selectedQuickSwitchProvider &&
                      selectedQuickSwitchProvider.apiKeys.length > 0 && (
                        <div className="oauth-link">
                          <label>
                            {t(
                              "codex.modelProviders.selectSavedApiKey",
                              "已保存 API Key",
                            )}
                          </label>
                          <SingleSelectDropdown
                            className="codex-managed-api-key-select"
                            value={quickSwitchApiKeyId}
                            options={selectedQuickSwitchProvider.apiKeys.map(
                              (item) => ({
                                value: item.id,
                                label: formatCodexManagedApiKeyOptionLabel(
                                  item,
                                  t(
                                    "codex.modelProviders.unnamedKey",
                                    "未命名 Key",
                                  ),
                                ),
                              }),
                            )}
                            onChange={handleSelectQuickSwitchApiKey}
                            disabled={quickSwitchSubmitting}
                            placeholder={t(
                              "codex.modelProviders.selectSavedApiKeyPlaceholder",
                              "选择 API Key",
                            )}
                            ariaLabel={t(
                              "codex.modelProviders.selectSavedApiKey",
                              "已保存 API Key",
                            )}
                          />
                          {selectedQuickSwitchProvider.apiKeys.length > 1 && (
                            <p className="api-provider-hint">
                              {t(
                                "codex.modelProviders.selectSavedApiKeyHint",
                                "该供应商有多个 API Key，可在此切换选择。",
                              )}
                            </p>
                          )}
                        </div>
                      )}

                    {selectedQuickSwitchProvider &&
                      selectedQuickSwitchProvider.apiKeys.length === 0 && (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>
                            {t(
                              "codex.quickSwitch.providerHasNoKeys",
                              "该供应商没有可用 API Key，请先在模型供应商中添加。",
                            )}
                          </span>
                        </div>
                      )}

                    {quickSwitchError && (
                      <div className="add-status error">
                        <CircleAlert size={16} />
                        <span>{quickSwitchError}</span>
                      </div>
                    )}

                    <div className="api-key-edit-actions">
                      <button
                        className="btn btn-secondary"
                        onClick={() => {
                          setActiveTab("providers");
                          closeQuickSwitchModal();
                        }}
                        disabled={quickSwitchSubmitting}
                      >
                        {t("codex.quickSwitch.gotoProviders", "管理供应商")}
                      </button>
                      <button
                        className="btn btn-primary"
                        onClick={() => void handleSubmitQuickSwitch()}
                        disabled={
                          quickSwitchSubmitting ||
                          managedProvidersLoading ||
                          !selectedQuickSwitchProvider ||
                          !selectedQuickSwitchApiKey
                        }
                      >
                        {quickSwitchSubmitting
                          ? t("common.saving", "保存中...")
                          : t("codex.quickSwitch.apply", "立即切换")}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {oauthBindingTargetActive && (
            <div className="modal-overlay">
              <div
                className="modal-content codex-add-modal codex-oauth-binding-modal"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    {t("codex.api.oauthBinding.title", "绑定 OAuth 账号")}
                  </h2>
                  <button
                    className="modal-close"
                    onClick={closeOAuthBindingModal}
                    aria-label={t("common.close", "关闭")}
                    disabled={oauthBindingSaving}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage
                    message={oauthBindingError}
                    scrollKey={oauthBindingErrorScrollKey}
                  />
                  {parseCodexSwitchAuthFailure(oauthBindingError) && (
                    <div className="codex-oauth-binding-reauthorize">
                      <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={handleReauthorizeOAuthBinding}
                        disabled={oauthBindingSaving}
                      >
                        {t("common.reauthorize", "重新授权")}
                      </button>
                    </div>
                  )}
                  <div className="add-section">
                    <div className="codex-oauth-binding-context">
                      <p className="section-desc codex-oauth-binding-desc">
                        {oauthBindingTargetKind === "local_access"
                          ? t(
                              "codex.localAccess.oauthBinding.desc",
                              "可选绑定。只要 OAuth 账号带 refresh_token 即可选择；未绑定时 API 服务按原 API Key 逻辑运行；绑定后登录态使用 OAuth 账号，Provider 使用当前 API 服务配置。",
                            )
                          : t(
                              "codex.api.oauthBinding.desc",
                              "可选绑定。只要 OAuth 账号带 refresh_token 即可选择；未绑定时该账号按原 API Key 逻辑切换；绑定后登录态使用 OAuth 账号，Provider 使用当前 API Key 账号配置。",
                            )}
                      </p>
                      <div className="section-desc codex-oauth-binding-current-target">
                        {oauthBindingTargetKind === "local_access"
                          ? t("codex.localAccess.oauthBinding.currentService", {
                              defaultValue: "API 服务：{{name}}",
                              name: t("codex.localAccess.title", "API 服务"),
                            })
                          : oauthBindingAccount
                            ? t("codex.api.oauthBinding.currentAccount", {
                                defaultValue: "API Key 账号：{{name}}",
                                name: maskAccountText(
                                  resolvePresentation(oauthBindingAccount)
                                    .displayName,
                                ),
                              })
                            : null}
                      </div>
                    </div>
                    <div className="codex-oauth-binding-picker">
                      <div className="codex-oauth-binding-picker-header">
                        <label>
                          {t(
                            "codex.api.oauthBinding.selectLabel",
                            "选择 OAuth 账号",
                          )}
                        </label>
                        <div className="codex-oauth-binding-picker-controls">
                          {isLocalAccessOAuthBinding && (
                            <div className="codex-oauth-binding-quota-control">
                              <label
                                className="codex-oauth-binding-gateway-toggle codex-oauth-binding-quota-toggle"
                                title={t(
                                  "codex.localAccess.oauthBinding.quotaReserveDesc",
                                  "API 服务仅在 5 小时和周剩余额度均高于保留值时使用该 OAuth 账号。",
                                )}
                              >
                                <input
                                  type="checkbox"
                                  checked={Boolean(oauthBindingQuotaReserve)}
                                  onChange={(event) =>
                                    handleOAuthBindingQuotaReserveToggle(
                                      event.target.checked,
                                    )
                                  }
                                  disabled={oauthBindingSaving}
                                />
                                <span
                                  className="codex-oauth-binding-checkbox-ui"
                                  aria-hidden="true"
                                />
                                <span>
                                  {t(
                                    "codex.localAccess.oauthBinding.quotaReserveToggle",
                                    "保留 OAuth 额度",
                                  )}
                                </span>
                              </label>
                              {oauthBindingQuotaReserve && (
                                <button
                                  type="button"
                                  className="btn btn-icon codex-oauth-binding-quota-edit"
                                  onClick={openOAuthBindingQuotaReserveEditor}
                                  disabled={oauthBindingSaving}
                                  title={`${t(
                                    "codex.localAccess.oauthBinding.quotaReserveHourlyLabel",
                                    "5 小时保留",
                                  )} ${oauthBindingQuotaReserve.hourlyPercent}% · ${t(
                                    "codex.localAccess.oauthBinding.quotaReserveWeeklyLabel",
                                    "周保留",
                                  )} ${oauthBindingQuotaReserve.weeklyPercent}%`}
                                  aria-label={`${t("instances.actions.edit", "编辑")} ${t(
                                    "codex.localAccess.oauthBinding.quotaReserveToggle",
                                    "保留 OAuth 额度",
                                  )}`}
                                >
                                  <Pencil size={12} />
                                </button>
                              )}
                            </div>
                          )}
                        </div>
                      </div>
                      {oauthAccounts.length === 0 ? (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>
                            {t(
                              "codex.api.oauthBinding.empty",
                              "暂无 OAuth 账号，请先添加 OAuth 授权账号。",
                            )}
                          </span>
                        </div>
                      ) : (
                        <>
                          {oauthBindingEligibleAccounts.length === 0 && (
                            <div className="add-status error">
                              <CircleAlert size={16} />
                              <span>
                                {t(
                                  "codex.api.oauthBinding.emptyEligible",
                                  "没有带 refresh_token 的 OAuth 账号，请重新 OAuth 授权或添加符合条件的 OAuth 账号。",
                                )}
                              </span>
                            </div>
                          )}
                          <div className="codex-oauth-binding-toolbar">
                            <div className="search-box codex-oauth-binding-search">
                              <Search size={16} className="search-icon" />
                              <input
                                type="text"
                                placeholder={t(
                                  "common.shared.search",
                                  "搜索账号...",
                                )}
                                value={searchQuery}
                                onChange={(event) =>
                                  setSearchQuery(event.target.value)
                                }
                                disabled={oauthBindingSaving}
                              />
                            </div>
                            <MultiSelectFilterDropdown
                              options={oauthBindingTierFilterOptions}
                              selectedValues={filterTypes}
                              allLabel={t("common.shared.filter.all", {
                                count: oauthBindingTierCounts.all,
                              })}
                              filterLabel={t(
                                "common.shared.filterLabel",
                                "筛选",
                              )}
                              clearLabel={t("accounts.clearFilter", "清空筛选")}
                              emptyLabel={t("common.none", "暂无")}
                              ariaLabel={t("common.shared.filterLabel", "筛选")}
                              onToggleValue={toggleFilterTypeValue}
                              onClear={clearFilterTypes}
                            />
                            <AccountTagFilterDropdown
                              availableTags={oauthBindingAvailableTags}
                              selectedTags={tagFilter}
                              onToggleTag={toggleTagFilterValue}
                              onClear={clearTagFilter}
                            />
                            <SingleSelectFilterDropdown
                              value={sortBy}
                              options={codexAccountSortOptions}
                              ariaLabel={t("common.shared.sortLabel", "排序")}
                              icon={<ArrowDownWideNarrow size={14} />}
                              disabled={oauthBindingSaving}
                              onChange={setSortBy}
                            />
                            {sortBy !== "custom" && (
                              <button
                                type="button"
                                className="sort-direction-btn"
                                onClick={() =>
                                  setSortDirection((prev) =>
                                    prev === "desc" ? "asc" : "desc",
                                  )
                                }
                                disabled={oauthBindingSaving}
                                title={
                                  sortDirection === "desc"
                                    ? t(
                                        "common.shared.sort.descTooltip",
                                        "当前：降序，点击切换为升序",
                                      )
                                    : t(
                                        "common.shared.sort.ascTooltip",
                                        "当前：升序，点击切换为降序",
                                      )
                                }
                                aria-label={t(
                                  "common.shared.sort.toggleDirection",
                                  "切换排序方向",
                                )}
                              >
                                {sortDirection === "desc" ? (
                                  <ArrowDown size={15} />
                                ) : (
                                  <ArrowUp size={15} />
                                )}
                              </button>
                            )}
                          </div>
                          {oauthBindingFilteredAccounts.length === 0 ? (
                            <div className="group-account-empty">
                              <span>
                                {t(
                                  "common.shared.noMatch.title",
                                  "没有匹配的账号",
                                )}
                              </span>
                            </div>
                          ) : (
                            <div className="codex-oauth-binding-list">
                              {oauthBindingPagination.pageItems.map(
                                (account) => {
                                  const presentation =
                                    resolvePresentation(account);
                                  const subscriptionInfo =
                                    resolveSubscriptionPresentation(account);
                                  const selected =
                                    oauthBindingSelectedAccountId ===
                                    account.id;
                                  const eligible =
                                    isCodexOAuthBindingEligibleAccount(account);
                                  const rowDisabled =
                                    oauthBindingSaving || !eligible;
                                  const emailText = maskAccountText(
                                    account.email ||
                                      account.account_name ||
                                      presentation.displayName ||
                                      account.id,
                                  );
                                  return (
                                    <label
                                      key={account.id}
                                      className={`codex-oauth-binding-row ${selected ? "is-selected" : ""}`}
                                      aria-label={emailText}
                                      aria-disabled={rowDisabled}
                                      title={
                                        eligible
                                          ? emailText
                                          : isCodexAgentIdentityAccount(account)
                                            ? t(
                                                "codex.agentIdentityRegistration.oauthBindingUnsupported",
                                                "Agent Identity 账号仅用于 API 服务，不能作为 OAuth 绑定账号。",
                                              )
                                            : isCodexWebSessionAccount(account)
                                              ? t(
                                                  "codex.webSessionImport.oauthBindingUnsupported",
                                                  "Web Session 账号仅支持查看额度，不能作为 OAuth 绑定账号。",
                                                )
                                              : t(
                                                  "codex.api.oauthBinding.validationSubscriptionRequired",
                                                  "只能绑定带 refresh_token 的 OAuth 账号",
                                                )
                                      }
                                      onClick={(event) => {
                                        if (rowDisabled) {
                                          event.preventDefault();
                                          return;
                                        }
                                        setOauthBindingSelectedAccountId(
                                          account.id,
                                        );
                                        setOauthBindingError(null);
                                      }}
                                    >
                                      <input
                                        type="radio"
                                        name="codex-oauth-binding-account"
                                        checked={selected}
                                        onChange={() => {
                                          setOauthBindingSelectedAccountId(
                                            account.id,
                                          );
                                          setOauthBindingError(null);
                                        }}
                                        disabled={rowDisabled}
                                      />
                                      <div className="codex-oauth-binding-row-main">
                                        <span
                                          className="codex-oauth-binding-row-name"
                                          title={emailText}
                                        >
                                          {emailText}
                                        </span>
                                        <span
                                          className={`tier-badge codex-oauth-binding-row-plan ${presentation.planClass || "unknown"}`}
                                          title={presentation.planLabel}
                                        >
                                          {presentation.planLabel}
                                        </span>
                                        <span
                                          className={`codex-oauth-binding-row-term ${subscriptionInfo.tone}`}
                                          title={subscriptionInfo.titleText}
                                        >
                                          <Clock size={12} />
                                          <span>
                                            {t(
                                              "codex.subscription.label",
                                              "有效期",
                                            )}
                                          </span>
                                          <strong>
                                            {subscriptionInfo.valueText}
                                          </strong>
                                          <span>
                                            {subscriptionInfo.detailText}
                                          </span>
                                        </span>
                                      </div>
                                    </label>
                                  );
                                },
                              )}
                            </div>
                          )}
                          <PaginationControls
                            totalItems={oauthBindingPagination.totalItems}
                            currentPage={oauthBindingPagination.currentPage}
                            totalPages={oauthBindingPagination.totalPages}
                            pageSize={oauthBindingPagination.pageSize}
                            pageSizeOptions={
                              oauthBindingPagination.pageSizeOptions
                            }
                            rangeStart={oauthBindingPagination.rangeStart}
                            rangeEnd={oauthBindingPagination.rangeEnd}
                            canGoPrevious={oauthBindingPagination.canGoPrevious}
                            canGoNext={oauthBindingPagination.canGoNext}
                            onPageSizeChange={
                              oauthBindingPagination.setPageSize
                            }
                            onPreviousPage={
                              oauthBindingPagination.goToPreviousPage
                            }
                            onNextPage={oauthBindingPagination.goToNextPage}
                          />
                        </>
                      )}
                    </div>
                    <div className="api-key-edit-actions">
                      {oauthAccounts.length === 0 && (
                        <button
                          className="btn btn-secondary"
                          onClick={() => {
                            closeOAuthBindingModal();
                            openCodexAddModal("oauth");
                          }}
                          disabled={oauthBindingSaving}
                        >
                          {t("codex.addModal.oauth", "OAuth 授权")}
                        </button>
                      )}
                      {oauthBindingHasExistingBinding && (
                        <button
                          className="btn btn-secondary codex-oauth-binding-clear"
                          onClick={() => void handleClearOAuthBinding()}
                          disabled={oauthBindingSaving}
                        >
                          {t("codex.api.oauthBinding.clearAction", "解除绑定")}
                        </button>
                      )}
                      <button
                        className="btn btn-secondary"
                        onClick={closeOAuthBindingModal}
                        disabled={oauthBindingSaving}
                      >
                        {t("common.cancel")}
                      </button>
                      <button
                        className="btn btn-primary"
                        onClick={() => void handleSubmitOAuthBinding()}
                        disabled={
                          oauthBindingSaving ||
                          !selectedOAuthBindingAccount ||
                          oauthBindingEligibleAccounts.length === 0
                        }
                      >
                        {oauthBindingSaving
                          ? t("common.saving", "保存中...")
                          : t("common.save")}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {oauthBindingQuotaReserveEditorOpen && isLocalAccessOAuthBinding && (
            <div className="modal-overlay codex-oauth-binding-quota-overlay">
              <div
                className="modal-content codex-add-modal codex-oauth-binding-quota-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    {t(
                      "codex.localAccess.oauthBinding.quotaReserveToggle",
                      "保留 OAuth 额度",
                    )}
                  </h2>
                  <button
                    type="button"
                    className="modal-close"
                    onClick={closeOAuthBindingQuotaReserveEditor}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <div className="add-section">
                    <p className="section-desc codex-oauth-binding-quota-desc">
                      {t(
                        "codex.localAccess.oauthBinding.quotaReserveDesc",
                        "API 服务仅在 5 小时和周剩余额度均高于保留值时使用该 OAuth 账号。",
                      )}
                    </p>
                    <div className="codex-oauth-binding-quota-fields">
                      <label className="codex-oauth-binding-quota-field">
                        <span>
                          {t(
                            "codex.localAccess.oauthBinding.quotaReserveHourlyLabel",
                            "5 小时保留",
                          )}
                        </span>
                        <div className="codex-oauth-binding-quota-input-wrap">
                          <input
                            ref={oauthBindingHourlyReserveInputRef}
                            className={
                              oauthBindingQuotaReserveFieldErrors.hourlyPercent
                                ? "codex-account-note-input has-error"
                                : "codex-account-note-input"
                            }
                            type="text"
                            inputMode="numeric"
                            pattern="[0-9]*"
                            maxLength={3}
                            value={oauthBindingHourlyReserveDraft}
                            onChange={(event) => {
                              if (!/^\d*$/.test(event.target.value)) return;
                              setOauthBindingHourlyReserveDraft(
                                event.target.value,
                              );
                              setOauthBindingQuotaReserveFieldErrors(
                                (prev) => ({
                                  ...prev,
                                  hourlyPercent: undefined,
                                }),
                              );
                            }}
                            onBlur={() =>
                              validateOAuthBindingQuotaReserveField(
                                "hourlyPercent",
                                oauthBindingHourlyReserveDraft,
                              )
                            }
                          />
                          <span aria-hidden="true">%</span>
                        </div>
                        {oauthBindingQuotaReserveFieldErrors.hourlyPercent && (
                          <span className="codex-account-note-field-error codex-oauth-binding-quota-error">
                            {oauthBindingQuotaReserveFieldErrors.hourlyPercent}
                          </span>
                        )}
                      </label>
                      <label className="codex-oauth-binding-quota-field">
                        <span>
                          {t(
                            "codex.localAccess.oauthBinding.quotaReserveWeeklyLabel",
                            "周保留",
                          )}
                        </span>
                        <div className="codex-oauth-binding-quota-input-wrap">
                          <input
                            ref={oauthBindingWeeklyReserveInputRef}
                            className={
                              oauthBindingQuotaReserveFieldErrors.weeklyPercent
                                ? "codex-account-note-input has-error"
                                : "codex-account-note-input"
                            }
                            type="text"
                            inputMode="numeric"
                            pattern="[0-9]*"
                            maxLength={3}
                            value={oauthBindingWeeklyReserveDraft}
                            onChange={(event) => {
                              if (!/^\d*$/.test(event.target.value)) return;
                              setOauthBindingWeeklyReserveDraft(
                                event.target.value,
                              );
                              setOauthBindingQuotaReserveFieldErrors(
                                (prev) => ({
                                  ...prev,
                                  weeklyPercent: undefined,
                                }),
                              );
                            }}
                            onBlur={() =>
                              validateOAuthBindingQuotaReserveField(
                                "weeklyPercent",
                                oauthBindingWeeklyReserveDraft,
                              )
                            }
                          />
                          <span aria-hidden="true">%</span>
                        </div>
                        {oauthBindingQuotaReserveFieldErrors.weeklyPercent && (
                          <span className="codex-account-note-field-error codex-oauth-binding-quota-error">
                            {oauthBindingQuotaReserveFieldErrors.weeklyPercent}
                          </span>
                        )}
                      </label>
                    </div>
                    <div className="api-key-edit-actions">
                      <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={closeOAuthBindingQuotaReserveEditor}
                      >
                        {t("common.cancel", "取消")}
                      </button>
                      <button
                        type="button"
                        className="btn btn-primary"
                        onClick={confirmOAuthBindingQuotaReserveEditor}
                      >
                        {t("common.confirm", "确认")}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {editingApiKeyCredentialsId && (
            <div className="modal-overlay">
              <div
                className="modal-content codex-add-modal codex-api-key-edit-modal"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{`${t("instances.actions.edit", "编辑")} ${t("codex.addModal.token", "API Key")}`}</h2>
                  <button
                    className="modal-close"
                    onClick={closeApiKeyCredentialsModal}
                    aria-label={t("common.close", "关闭")}
                    disabled={savingApiKeyCredentials}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <div className="add-section">
                    <div className="oauth-link">
                      <label>
                        {t(
                          "codex.modelProviders.selectSavedProvider",
                          "已保存供应商",
                        )}
                      </label>
                      {managedProvidersLoading ? (
                        <div className="section-desc">
                          {t("common.loading", "加载中...")}
                        </div>
                      ) : managedProviders.length === 0 ? (
                        <div className="section-desc">
                          {t(
                            "codex.modelProviders.noSavedProviders",
                            "暂无已保存供应商，可直接填写后自动保存。",
                          )}
                        </div>
                      ) : (
                        <div className="api-provider-chip-list">
                          {managedProviders.map((provider) => (
                            <button
                              key={provider.id}
                              className={`api-provider-chip ${editingManagedProviderId === provider.id ? "active" : ""}`}
                              onClick={() =>
                                handleSelectEditingManagedProvider(provider.id)
                              }
                              type="button"
                              disabled={savingApiKeyCredentials}
                            >
                              <span>{provider.name}</span>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                    {selectedEditingManagedProvider &&
                      selectedEditingManagedProvider.apiKeys.length > 0 && (
                        <div className="oauth-link">
                          <label>
                            {t(
                              "codex.modelProviders.selectSavedApiKey",
                              "已保存 API Key",
                            )}
                          </label>
                          <SingleSelectDropdown
                            className="codex-managed-api-key-select"
                            value={editingManagedProviderApiKeyId}
                            options={[
                              {
                                value: "",
                                label: t(
                                  "codex.modelProviders.manualApiKeyOption",
                                  "手动输入新 Key",
                                ),
                              },
                              ...selectedEditingManagedProvider.apiKeys.map(
                                (item) => ({
                                  value: item.id,
                                  label: formatCodexManagedApiKeyOptionLabel(
                                    item,
                                    t(
                                      "codex.modelProviders.unnamedKey",
                                      "未命名 Key",
                                    ),
                                  ),
                                }),
                              ),
                            ]}
                            onChange={handleSelectEditingManagedProviderApiKey}
                            disabled={savingApiKeyCredentials}
                            placeholder={t(
                              "codex.modelProviders.selectSavedApiKeyPlaceholder",
                              "选择 API Key",
                            )}
                            ariaLabel={t(
                              "codex.modelProviders.selectSavedApiKey",
                              "已保存 API Key",
                            )}
                          />
                          {selectedEditingManagedProvider.apiKeys.length >
                            1 && (
                            <p className="api-provider-hint">
                              {t(
                                "codex.modelProviders.selectSavedApiKeyHint",
                                "该供应商有多个 API Key，可在此切换选择。",
                              )}
                            </p>
                          )}
                        </div>
                      )}
                    <div className="oauth-link">
                      <label>{t("codex.api.provider.label", "供应商")}</label>
                      <div className="api-provider-chip-list">
                        <button
                          className={`api-provider-chip ${editingApiProviderPresetId === CODEX_API_PROVIDER_CUSTOM_ID ? "active" : ""}`}
                          onClick={() =>
                            handleSelectEditingApiProviderPreset(
                              CODEX_API_PROVIDER_CUSTOM_ID,
                            )
                          }
                          type="button"
                          disabled={savingApiKeyCredentials}
                        >
                          <span>
                            {t("codex.api.provider.custom", "自定义")}
                          </span>
                        </button>
                        {CODEX_API_PROVIDER_PRESETS.map((preset) => (
                          <button
                            key={preset.id}
                            className={`api-provider-chip ${editingApiProviderPresetId === preset.id ? "active" : ""}`}
                            onClick={() =>
                              handleSelectEditingApiProviderPreset(preset.id)
                            }
                            type="button"
                            disabled={savingApiKeyCredentials}
                          >
                            <span>
                              {t(
                                `codex.api.providers.${preset.id}.name`,
                                preset.name,
                              )}
                            </span>
                            {preset.isPartner && (
                              <Star
                                size={12}
                                className="api-provider-chip-badge"
                              />
                            )}
                          </button>
                        ))}
                      </div>
                    </div>
                    {selectedEditingApiProviderPreset &&
                      selectedEditingApiProviderPreset.baseUrls.length > 1 && (
                        <div className="oauth-link">
                          <label>
                            {t("codex.api.provider.endpoint", "供应商端点")}
                          </label>
                          <div className="api-provider-endpoint-list">
                            {selectedEditingApiProviderPreset.baseUrls.map(
                              (baseUrl) => (
                                <button
                                  key={baseUrl}
                                  className={`api-provider-endpoint-chip ${editingApiBaseUrlCredentialsValue === baseUrl ? "active" : ""}`}
                                  onClick={() =>
                                    handleEditingApiBaseUrlCredentialsChange(
                                      baseUrl,
                                    )
                                  }
                                  type="button"
                                  disabled={savingApiKeyCredentials}
                                >
                                  {baseUrl}
                                </button>
                              ),
                            )}
                          </div>
                        </div>
                      )}
                    {selectedEditingApiProviderPreset && (
                      <div className="api-provider-hint-block">
                        <p className="api-provider-hint">
                          {t(
                            "codex.api.provider.hint",
                            "已自动填写兼容 Base URL，可继续手动调整。",
                          )}
                        </p>
                        <div className="api-provider-links">
                          {selectedEditingApiProviderPreset.website && (
                            <button
                              className="btn btn-secondary"
                              onClick={() =>
                                void handleOpenProviderLink(
                                  selectedEditingApiProviderPreset.website ||
                                    "",
                                )
                              }
                              disabled={savingApiKeyCredentials}
                            >
                              <ExternalLink size={14} />
                              {t("codex.api.provider.website", "官网")}
                            </button>
                          )}
                          {selectedEditingApiProviderPreset.apiKeyUrl && (
                            <button
                              className="btn btn-secondary"
                              onClick={() =>
                                void handleOpenProviderLink(
                                  selectedEditingApiProviderPreset.apiKeyUrl ||
                                    "",
                                )
                              }
                              disabled={savingApiKeyCredentials}
                            >
                              <KeyRound size={14} />
                              {selectedEditingApiProviderPreset.id ===
                              COCKPIT_API_PROVIDER_ID
                                ? t("codex.api.provider.getApiKey", "获取秘钥")
                                : t(
                                    "codex.api.provider.apiKeyPage",
                                    "API Key 页面",
                                  )}
                            </button>
                          )}
                        </div>
                      </div>
                    )}
                    <div className="oauth-link">
                      <label>{t("codex.addModal.token", "API Key")}</label>
                      <div className="oauth-url-box oauth-manual-input codex-secret-input">
                        <input
                          type={
                            editingApiKeyCredentialsVisible
                              ? "text"
                              : "password"
                          }
                          value={editingApiKeyCredentialsValue}
                          onChange={(e) =>
                            handleEditingApiKeyCredentialsChange(e.target.value)
                          }
                          disabled={savingApiKeyCredentials}
                          autoComplete="off"
                          spellCheck={false}
                        />
                        <button
                          type="button"
                          className="codex-secret-toggle-btn"
                          onClick={() =>
                            setEditingApiKeyCredentialsVisible(
                              (visible) => !visible,
                            )
                          }
                          disabled={savingApiKeyCredentials}
                          title={
                            editingApiKeyCredentialsVisible
                              ? t("codex.api.hideApiKey", "隐藏 API Key")
                              : t("codex.api.showApiKey", "显示 API Key")
                          }
                          aria-label={
                            editingApiKeyCredentialsVisible
                              ? t("codex.api.hideApiKey", "隐藏 API Key")
                              : t("codex.api.showApiKey", "显示 API Key")
                          }
                        >
                          {editingApiKeyCredentialsVisible ? (
                            <EyeOff size={16} />
                          ) : (
                            <Eye size={16} />
                          )}
                        </button>
                      </div>
                    </div>
                    <div className="oauth-link">
                      <label>{t("codex.api.baseUrl", "Base URL")}</label>
                      <div className="oauth-url-box oauth-manual-input">
                        <input
                          type="text"
                          value={editingApiBaseUrlCredentialsValue}
                          onChange={(e) =>
                            handleEditingApiBaseUrlCredentialsChange(
                              e.target.value,
                            )
                          }
                          placeholder={t(
                            "codex.api.baseUrlPlaceholder",
                            "不填写则是官方默认",
                          )}
                          disabled={savingApiKeyCredentials}
                        />
                      </div>
                    </div>
                    {editingApiProviderPresetId !== COCKPIT_API_PROVIDER_ID && (
                      <div className="oauth-link">
                        <label>
                          {t(
                            "codex.modelProviders.newProviderName",
                            "供应商名称（自动保存时使用，可选）",
                          )}
                        </label>
                        <div className="oauth-url-box oauth-manual-input">
                          <input
                            type="text"
                            value={editingNewManagedProviderNameInput}
                            onChange={(e) =>
                              setEditingNewManagedProviderNameInput(
                                e.target.value,
                              )
                            }
                            placeholder={t(
                              "codex.modelProviders.newProviderNamePlaceholder",
                              "不填则按域名自动生成",
                            )}
                            disabled={savingApiKeyCredentials}
                          />
                        </div>
                      </div>
                    )}
                    {editingApiProviderPresetId !==
                      OPENAI_OFFICIAL_PRESET_ID && (
                      <>
                        <div className="api-model-catalog-panel">
                          <div className="api-model-catalog-header">
                            <label htmlFor="codex-api-model-catalog-edit">
                              {t("codex.api.modelCatalog.label", "模型列表")}
                            </label>
                            <span className="api-model-catalog-count">
                              {t("codex.api.modelCatalog.count", {
                                defaultValue: "{{count}} 个模型",
                                count: editingApiModelCatalogDraft.length,
                              })}
                            </span>
                          </div>
                          <textarea
                            id="codex-api-model-catalog-edit"
                            className="form-input api-model-catalog-input"
                            rows={6}
                            value={editingApiModelCatalogInput}
                            onChange={(event) => {
                              setEditingApiModelCatalogInput(
                                event.target.value,
                              );
                              setEditingApiModelCatalogError(null);
                            }}
                            placeholder={t(
                              "codex.api.modelCatalog.placeholder",
                              "每行填写一个模型 ID，也可以使用逗号分隔。",
                            )}
                            disabled={savingApiKeyCredentials}
                            aria-describedby="codex-api-model-catalog-edit-hint"
                          />
                          <CodexModelContextWindowTable
                            models={editingApiModelCatalogDraft}
                            drafts={editingApiModelContextWindowsInput}
                            onChange={(model, value) => {
                              setEditingApiModelContextWindowsInput(
                                (current) => ({
                                  ...current,
                                  [model]: value,
                                }),
                              );
                              setEditingApiModelCatalogError(null);
                            }}
                            disabled={savingApiKeyCredentials}
                          />
                          <div className="api-model-catalog-toolbar">
                            <p
                              id="codex-api-model-catalog-edit-hint"
                              className="api-model-catalog-hint"
                            >
                              {t(
                                "codex.api.modelCatalog.editHint",
                                "上游结果仅填入当前草稿，可在保存前删除、补充或调整模型。",
                              )}
                            </p>
                            <button
                              type="button"
                              className="btn btn-secondary api-model-catalog-fetch"
                              onClick={() =>
                                void handleFetchEditingApiModelCatalog()
                              }
                              disabled={
                                editingApiModelCatalogFetching ||
                                savingApiKeyCredentials ||
                                !editingApiKeyCredentialsValue.trim()
                              }
                            >
                              <RefreshCw
                                size={14}
                                className={
                                  editingApiModelCatalogFetching
                                    ? "loading-spinner"
                                    : undefined
                                }
                              />
                              {editingApiModelCatalogFetching
                                ? t(
                                    "codex.api.modelCatalog.fetching",
                                    "获取中...",
                                  )
                                : t(
                                    "codex.api.modelCatalog.fetch",
                                    "从上游获取",
                                  )}
                            </button>
                          </div>
                          {editingApiModelCatalogError && (
                            <div className="add-status error api-model-catalog-error">
                              <CircleAlert size={16} />
                              <span>{editingApiModelCatalogError}</span>
                            </div>
                          )}
                        </div>
                        {editingApiModelCatalogSyncAvailable && (
                          <label className="codex-import-api-service-toggle api-model-catalog-sync-toggle">
                            <span className="codex-import-api-service-toggle-copy">
                              <strong>
                                {t(
                                  "codex.api.modelCatalog.syncToggle",
                                  "同步供应商模型到 Codex",
                                )}
                              </strong>
                              <small>
                                {t(
                                  "codex.api.modelCatalog.syncDescription",
                                  "保存后使用当前模型列表生成 Cockpit 受管的 Codex 模型目录，不覆盖用户自定义目录。",
                                )}
                              </small>
                            </span>
                            <input
                              type="checkbox"
                              checked={editingApiSyncModelCatalogToCodex}
                              disabled={savingApiKeyCredentials}
                              onChange={(event) => {
                                setEditingApiSyncModelCatalogToCodex(
                                  event.target.checked,
                                );
                                setEditingApiModelCatalogError(null);
                              }}
                            />
                            <span className="codex-import-api-service-switch" />
                          </label>
                        )}
                      </>
                    )}
                    <div className="api-key-edit-actions">
                      <button
                        className="btn btn-secondary"
                        onClick={closeApiKeyCredentialsModal}
                        disabled={savingApiKeyCredentials}
                      >
                        {t("common.cancel")}
                      </button>
                      <button
                        className="btn btn-primary"
                        onClick={() => void handleSubmitApiKeyCredentials()}
                        disabled={
                          savingApiKeyCredentials ||
                          editingApiModelCatalogFetching ||
                          !editingApiKeyCredentialsValue.trim()
                        }
                      >
                        {savingApiKeyCredentials
                          ? t("common.saving", "保存中...")
                          : t("common.save")}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {showCustomSortModal && (
            <div className="modal-overlay">
              <div
                className="modal codex-custom-sort-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <div>
                    <h2>
                      {t("codex.sort.customModalTitle", "自定义账号排序")}
                    </h2>
                    <p className="codex-custom-sort-modal-desc">
                      {t(
                        "codex.sort.customModalDesc",
                        "拖动账号或使用上下按钮调整展示顺序。",
                      )}
                    </p>
                  </div>
                  <button
                    className="modal-close"
                    onClick={() => setShowCustomSortModal(false)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <div
                    className={`codex-custom-sort-list ${
                      draggedCustomSortAccountId ? "is-sorting" : ""
                    }`}
                    onMouseUp={stopCustomSortDragging}
                    onMouseLeave={stopCustomSortDragging}
                  >
                    {customSortAccounts.map((account, index) => {
                      const presentation = resolvePresentation(account);
                      const isCurrent = overviewCurrentAccountId === account.id;
                      const isChatCompletionsApiKey =
                        isCodexChatCompletionsApiKeyAccount(account);
                      const quotaItems =
                        isChatCompletionsApiKey ||
                        (isCodexApiKeyAccount(account) &&
                          !isCodexNewApiAccount(account))
                          ? []
                          : presentation.quotaItems
                              .filter((item) => item.key !== "code_review")
                              .slice(0, 2);
                      const rowClass = [
                        "codex-custom-sort-row",
                        draggedCustomSortAccountId === account.id
                          ? "is-dragging"
                          : "",
                        draggedCustomSortAccountId &&
                        draggedCustomSortAccountId !== account.id
                          ? "is-drop-candidate"
                          : "",
                        draggedCustomSortAccountId &&
                        draggedCustomSortAccountId !== account.id &&
                        customSortDropTargetId === account.id
                          ? "is-drop-target"
                          : "",
                      ]
                        .join(" ")
                        .trim();

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
                                "codex.sort.customDragHandle",
                                "拖拽排序",
                              )}
                              aria-label={t(
                                "codex.sort.customDragHandle",
                                "拖拽排序",
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
                                  title={maskAccountText(
                                    presentation.displayName,
                                  )}
                                >
                                  {maskAccountText(presentation.displayName)}
                                </span>
                                {isCurrent && (
                                  <span className="mini-tag current">
                                    {t("codex.current", "当前")}
                                  </span>
                                )}
                                <span
                                  className={`tier-badge ${presentation.planClass || "unknown"}`}
                                >
                                  {presentation.planLabel}
                                </span>
                              </div>
                              <div className="codex-custom-sort-quota-line">
                                {quotaItems.length > 0 ? (
                                  quotaItems.map((item) => (
                                    <span
                                      key={`${account.id}-${item.key}`}
                                      className="codex-custom-sort-quota"
                                      title={item.hintText}
                                    >
                                      <span>{item.label}</span>
                                      <strong className={item.quotaClass}>
                                        {item.valueText}
                                      </strong>
                                    </span>
                                  ))
                                ) : isChatCompletionsApiKey ? null : (
                                  <span className="codex-custom-sort-quota-empty">
                                    {t(
                                      "common.shared.quota.noData",
                                      "暂无配额数据",
                                    )}
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
                                moveCustomSortAccount(account.id, "up")
                              }
                              disabled={index === 0}
                              title={t("codex.sort.customMoveUp", "上移")}
                              aria-label={t("codex.sort.customMoveUp", "上移")}
                            >
                              <ArrowUp size={14} />
                            </button>
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() =>
                                moveCustomSortAccount(account.id, "down")
                              }
                              disabled={index === customSortAccounts.length - 1}
                              title={t("codex.sort.customMoveDown", "下移")}
                              aria-label={t(
                                "codex.sort.customMoveDown",
                                "下移",
                              )}
                            >
                              <ArrowDown size={14} />
                            </button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={resetCustomSortOrder}
                  >
                    <RotateCw size={14} />
                    {t("codex.sort.customReset", "重置自定义顺序")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => setShowCustomSortModal(false)}
                  >
                    {t("common.confirm", "确认")}
                  </button>
                </div>
              </div>
            </div>
          )}

          <ExportJsonModal
            isOpen={showExportModal}
            title={`${t("common.shared.export.title", "导出")} JSON`}
            jsonContent={formattedExportJsonContent}
            customContent={formattedExportModalCustomContent}
            errorMessage={exportModalError}
            errorScrollKey={exportModalErrorScrollKey}
            hidden={exportJsonHidden}
            copied={formattedExportJsonCopied}
            saving={formattedSavingExportJson}
            savedPath={formattedExportSavedPath}
            canOpenSavedDirectory={canOpenFormattedExportSavedDirectory}
            pathCopied={formattedExportPathCopied}
            toolbarContent={
              <>
                <span className="export-json-toolbar-label">
                  {t("codex.exportFormat.label", "导出格式")}
                </span>
                <div className="export-json-toolbar-dropdown">
                  <SingleSelectFilterDropdown
                    value={exportFormat}
                    options={exportFormatOptions}
                    ariaLabel={t("codex.exportFormat.label", "导出格式")}
                    onChange={(value) => {
                      if (value === "cpa" && exportHasAgentIdentity) {
                        reportExportModalError(
                          t(
                            "codex.exportFormat.agentIdentityCpaUnsupported",
                            "Agent Identity 账号不支持 cpa 格式，请使用 Cockpit Tools 或 sub2api 格式导出。",
                          ),
                        );
                        return;
                      }
                      setExportFormat(value as CodexExportFormat);
                    }}
                  />
                </div>
                {exportHasAgentIdentity ? (
                  <div className="export-json-sensitive-notice">
                    <Info size={14} />
                    <span>
                      {t(
                        "codex.exportFormat.agentIdentityPrivateKeyNotice",
                        "导出内容包含 Agent Identity 私钥，可用于在其他设备恢复账号。请像密码一样妥善保管。",
                      )}
                    </span>
                  </div>
                ) : null}
                {exportCanIncludeSensitiveNotes ? (
                  <label
                    className="export-json-sensitive-toggle"
                    title={t(
                      "codex.accountNote.exportSensitiveToggleHint",
                      "控制导出 JSON 是否包含 2FA 秘钥、密码和手机号。",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={includeExportSensitiveNotes}
                      onChange={(event) =>
                        setIncludeExportSensitiveNotes(event.target.checked)
                      }
                    />
                    <span className="export-json-sensitive-switch" />
                    <span>
                      {includeExportSensitiveNotes
                        ? t(
                            "codex.accountNote.exportSensitiveIncluded",
                            "包含敏感备注",
                          )
                        : t(
                            "codex.accountNote.exportSensitiveExcluded",
                            "已排除敏感备注",
                          )}
                    </span>
                    <Info size={14} />
                  </label>
                ) : null}
                {exportCanIncludeSensitiveNotes &&
                includeExportSensitiveNotes ? (
                  <div className="export-json-sensitive-notice">
                    <Info size={14} />
                    <span>
                      {t(
                        "codex.accountNote.exportSensitiveNotice",
                        "导出内容包含 2FA 秘钥、密码或手机号，请只保存到可信位置。",
                      )}
                    </span>
                  </div>
                ) : null}
              </>
            }
            onClose={handleCloseExportModal}
            onToggleHidden={handleToggleExportJsonHidden}
            onCopyJson={copyFormattedExportJson}
            onSaveJson={saveFormattedExportJson}
            onOpenSavedDirectory={openFormattedExportSavedDirectory}
            onCopySavedPath={copyFormattedExportSavedPath}
          />

          {showLocalAccessQuotaStatsModal && (
            <div className="modal-overlay codex-local-access-stats-overlay">
              <div
                className="modal codex-local-access-stats-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    {t(
                      "codex.localAccess.quotaPool.modalTitle",
                      "API 服务额度池",
                    )}
                  </h2>
                  <button
                    className="modal-close"
                    onClick={() => setShowLocalAccessQuotaStatsModal(false)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  {localAccessQuotaPoolSummary.visiblePlans.length === 0 ? (
                    <div className="codex-local-access-stats-empty">
                      {t("codex.localAccess.quotaPool.empty", "暂无额度统计")}
                    </div>
                  ) : (
                    <div className="codex-local-access-stats-list">
                      {localAccessQuotaPoolSummary.visiblePlans.map((item) => (
                        <div
                          key={item.key}
                          className="codex-local-access-stats-row"
                        >
                          <div className="codex-local-access-stats-plan">
                            <strong>
                              {item.key} ({item.count})
                            </strong>
                          </div>
                          <div className="codex-local-access-stats-values">
                            {item.windows.map((window) => (
                              <span key={window.key}>
                                <b>
                                  {formatCodexQuotaPoolWindowLabel(
                                    window.label,
                                    localAccessQuotaPoolLabels.weekly,
                                  )}
                                </b>
                                <strong>
                                  {formatCodexQuotaPoolPercent(
                                    window.percentage,
                                  )}
                                </strong>
                              </span>
                            ))}
                            {item.balance != null && (
                              <span>
                                <b>
                                  {t(
                                    "codex.localAccess.quotaPool.balance",
                                    "余额",
                                  )}
                                </b>
                                <strong>
                                  {formatCodexQuotaPoolBalance(item.balance)}
                                </strong>
                              </span>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-primary"
                    onClick={() => setShowLocalAccessQuotaStatsModal(false)}
                  >
                    {t("common.confirm", "确认")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {showLocalAccessHideConfirm && (
            <div className="modal-overlay codex-local-access-hide-confirm-overlay">
              <div
                className="modal codex-local-access-hide-confirm-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    {t(
                      "codex.localAccess.hideEntryAction",
                      "关闭 API 服务入口",
                    )}
                  </h2>
                  <button
                    className="modal-close"
                    onClick={() => {
                      if (localAccessHideSubmitting) return;
                      setShowLocalAccessHideConfirm(false);
                    }}
                    aria-label={t("common.close", "关闭")}
                    disabled={localAccessHideSubmitting}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <p className="codex-local-access-hide-confirm-desc">
                    {t(
                      "codex.localAccess.hideEntryConfirm",
                      "关闭后会同时隐藏总览中的 API 服务入口，并停用当前 API 服务。你仍可在 Codex 设置或快捷设置中重新打开。",
                    )}
                  </p>
                  <div className="codex-local-access-hide-confirm-points">
                    <div className="codex-local-access-hide-confirm-point">
                      <span className="codex-local-access-hide-confirm-dot" />
                      <span>
                        {t(
                          "codex.localAccess.hideEntryEffectHide",
                          "隐藏总览中的 API 服务入口",
                        )}
                      </span>
                    </div>
                    <div className="codex-local-access-hide-confirm-point">
                      <span className="codex-local-access-hide-confirm-dot" />
                      <span>
                        {t(
                          "codex.localAccess.hideEntryEffectDisable",
                          "停用当前 API 服务",
                        )}
                      </span>
                    </div>
                    <div className="codex-local-access-hide-confirm-point">
                      <span className="codex-local-access-hide-confirm-dot" />
                      <span>
                        {t(
                          "codex.localAccess.hideEntryEffectRestore",
                          "可在 Codex 设置或快捷设置中重新开启",
                        )}
                      </span>
                    </div>
                  </div>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => setShowLocalAccessHideConfirm(false)}
                    disabled={localAccessHideSubmitting}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    className="btn btn-danger"
                    onClick={() => void confirmHideLocalAccessEntry()}
                    disabled={localAccessHideSubmitting}
                  >
                    {localAccessHideSubmitting
                      ? t("common.processing", "处理中...")
                      : t("common.confirm", "确认")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {pendingWebSessionImport && (
            <div className="modal-overlay codex-local-access-hide-confirm-overlay codex-local-access-risk-notice-overlay">
              <div
                className="modal codex-local-access-hide-confirm-modal codex-local-access-risk-notice-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    <CircleAlert size={18} />
                    {t(
                      "codex.webSessionImport.noticeTitle",
                      "Web Session 导入须知",
                    )}
                  </h2>
                  <button
                    className="modal-close"
                    onClick={() => setPendingWebSessionImport(null)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <p className="codex-local-access-hide-confirm-desc">
                    {t(
                      "codex.webSessionImport.noticeMessage",
                      "检测到 {{count}} 个 Web Session 账号。此格式不支持实际使用，仅支持查看额度。",
                      { count: pendingWebSessionImport.accountLabels.length },
                    )}
                  </p>
                  <div className="codex-local-access-hide-confirm-points codex-local-access-risk-notice-points">
                    {pendingWebSessionImport.accountLabels.map(
                      (accountLabel, index) => (
                        <div
                          className="codex-local-access-hide-confirm-point"
                          key={`${accountLabel}-${index}`}
                        >
                          <span className="codex-local-access-hide-confirm-dot" />
                          <span>{maskAccountText(accountLabel)}</span>
                        </div>
                      ),
                    )}
                  </div>
                  <div className="codex-local-access-hide-confirm-points codex-local-access-risk-notice-points">
                    <div className="codex-local-access-hide-confirm-point">
                      <span className="codex-local-access-hide-confirm-dot" />
                      <span>
                        {t(
                          "codex.webSessionImport.noticeQuotaOnly",
                          "仅支持查看额度，不能启动官方客户端或 CLI，也不能切号。",
                        )}
                      </span>
                    </div>
                    <div className="codex-local-access-hide-confirm-point">
                      <span className="codex-local-access-hide-confirm-dot" />
                      <span>
                        {t(
                          "codex.webSessionImport.noticeNoApi",
                          "不能加入 Codex API 服务账号池，也不能作为 OAuth 绑定账号。",
                        )}
                      </span>
                    </div>
                  </div>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => setPendingWebSessionImport(null)}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => {
                      const pending = pendingWebSessionImport;
                      setPendingWebSessionImport(null);
                      void performTokenImport(pending.content);
                    }}
                  >
                    {t(
                      "codex.webSessionImport.noticeConfirm",
                      "已知晓，继续导入",
                    )}
                  </button>
                </div>
              </div>
            </div>
          )}

          {localAccessRiskNoticeAction && (
            <div className="modal-overlay codex-local-access-hide-confirm-overlay codex-local-access-risk-notice-overlay">
              <div
                className="modal codex-local-access-hide-confirm-modal codex-local-access-risk-notice-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    {t("codex.localAccess.riskNotice.title", "使用风险提示")}
                  </h2>
                  <button
                    className="modal-close"
                    onClick={() => closeLocalAccessRiskNotice(false)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <p className="codex-local-access-hide-confirm-desc">
                    {t(
                      "codex.localAccess.riskNotice.message",
                      "当前 Codex API 服务相关功能，本质上属于代理转发使用方式。就目前情况看，官方暂未对此类行为进行明确管控，但后续政策、规则或可用性是否发生变化，仍存在不确定性。继续使用该功能，即表示您已知悉相关情况，并愿意自行承担可能产生的风险。",
                    )}
                  </p>
                  <div className="codex-local-access-hide-confirm-points codex-local-access-risk-notice-points">
                    <label className="codex-local-access-risk-notice-remember">
                      <input
                        type="checkbox"
                        checked={localAccessRiskNoticeRemember}
                        onChange={(event) => {
                          setLocalAccessRiskNoticeRemember(
                            event.target.checked,
                          );
                        }}
                      />
                      <span>
                        {t(
                          "codex.localAccess.riskNotice.remember",
                          "我已知晓，不再提示",
                        )}
                      </span>
                    </label>
                  </div>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => closeLocalAccessRiskNotice(false)}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => closeLocalAccessRiskNotice(true)}
                  >
                    {getCodexLocalAccessRiskNoticeConfirmLabel(
                      localAccessRiskNoticeAction,
                      t,
                    )}
                  </button>
                </div>
              </div>
            </div>
          )}

          {resetCreditConfirmAccount && (
            <div className="modal-overlay codex-reset-credit-confirm-overlay">
              <div
                className="modal codex-reset-credit-confirm-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="codex-reset-credit-confirm-visual">
                  <button
                    type="button"
                    className="modal-close codex-reset-credit-confirm-close"
                    onClick={closeResetCreditConfirmModal}
                    aria-label={t("common.close", "关闭")}
                    disabled={isResetCreditConfirmSubmitting}
                  >
                    <X />
                  </button>
                  <div className="codex-reset-credit-confirm-icon">
                    <Terminal size={30} />
                    <RotateCw
                      size={18}
                      className="codex-reset-credit-confirm-icon-badge"
                    />
                  </div>
                </div>
                <div className="modal-body codex-reset-credit-confirm-body">
                  <h2>
                    {t(
                      "codex.quota.resetCreditDialogTitle",
                      "要重置你的使用量吗？",
                    )}
                  </h2>
                  <p>
                    {t("codex.quota.resetCreditDialogDesc", {
                      count: resetCreditConfirmAvailableCount ?? 0,
                      defaultValue:
                        "重置速率限制后，继续不间断地工作。你还有 {{count}} 次重置可用。",
                    })}
                  </p>
                  <div className="codex-reset-credit-confirm-account">
                    <span>{t("common.shared.columns.email", "账号")}</span>
                    <strong>
                      {maskAccountText(
                        resolvePresentation(resetCreditConfirmAccount)
                          .displayName,
                      )}
                    </strong>
                  </div>
                  {resetCreditConfirmNextExpiresAt && (
                    <div className="codex-reset-credit-confirm-expiry">
                      <Clock size={14} />
                      <span>
                        {t("codex.quota.resetCreditNextExpiry", {
                          time: formatResetCreditTime(
                            resetCreditConfirmNextExpiresAt,
                          ),
                          defaultValue: "最近到期：{{time}}",
                        })}
                      </span>
                    </div>
                  )}
                  <div className="codex-reset-credit-confirm-details">
                    <div className="codex-reset-credit-confirm-details-title">
                      {t("codex.quota.resetCreditDetailsTitle", "重置次数明细")}
                    </div>
                    {resetCreditConfirmLoading ? (
                      <div className="codex-reset-credit-confirm-empty">
                        <RefreshCw size={14} className="loading-spinner" />
                        <span>{t("common.loading", "加载中...")}</span>
                      </div>
                    ) : resetCreditConfirmCredits.length > 0 ? (
                      resetCreditConfirmCredits.map((credit, index) => (
                        <div
                          className="codex-reset-credit-confirm-detail"
                          key={
                            credit.id || `${credit.status || "credit"}-${index}`
                          }
                        >
                          <span
                            className={`codex-reset-credit-confirm-detail-status ${getResetCreditStatusTone(credit)}`}
                          >
                            {getResetCreditStatusLabel(credit)}
                          </span>
                          <span>
                            {t("codex.quota.resetCreditGrantedAt", "发放")}：
                            <strong>
                              {formatResetCreditAbsoluteTime(credit.granted_at)}
                            </strong>
                          </span>
                          <span>
                            {t("codex.quota.resetCreditExpiresAt", "到期")}：
                            <strong>
                              {formatResetCreditTime(credit.expires_at)}
                            </strong>
                          </span>
                        </div>
                      ))
                    ) : (
                      <div className="codex-reset-credit-confirm-empty">
                        {t("codex.quota.resetCreditNoRecords", "暂无重置记录")}
                      </div>
                    )}
                  </div>
                  <ModalErrorMessage
                    message={resetCreditConfirmError}
                    scrollKey={resetCreditConfirmErrorScrollKey}
                    position="bottom"
                  />
                </div>
                <div className="modal-footer codex-reset-credit-confirm-footer">
                  <button
                    type="button"
                    className="btn btn-primary codex-reset-credit-confirm-action"
                    onClick={() => void handleConfirmConsumeResetCredit()}
                    disabled={
                      isResetCreditConfirmSubmitting ||
                      resetCreditConfirmLoading ||
                      resetCreditConfirmActionLocked ||
                      resetCreditConfirmAvailableCount == null ||
                      resetCreditConfirmAvailableCount <= 0
                    }
                  >
                    {isResetCreditConfirmSubmitting ? (
                      <>
                        <RefreshCw size={14} className="loading-spinner" />
                        {t("common.processing", "处理中...")}
                      </>
                    ) : (
                      t("codex.quota.resetCreditDialogAction", "重置使用次数")
                    )}
                  </button>
                </div>
              </div>
            </div>
          )}

          {deleteConfirm && (
            <div className="modal-overlay">
              <div className="modal" onClick={(e) => e.stopPropagation()}>
                <div className="modal-header">
                  <h2>{t("common.confirm")}</h2>
                  <button
                    className="modal-close"
                    onClick={() => !batchDeleteBusy && setDeleteConfirm(null)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage
                    message={batchDeleteModalError || deleteConfirmError}
                    scrollKey={deleteConfirmErrorScrollKey}
                  />
                  <p>{deleteConfirm.message}</p>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => setDeleteConfirm(null)}
                    disabled={batchDeleteBusy}
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    className="btn btn-danger"
                    onClick={confirmCodexDelete}
                    disabled={batchDeleteBusy}
                  >
                    {batchDeleteBusy
                      ? t("common.processing", "处理中...")
                      : t("common.confirm")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {tagDeleteConfirm && (
            <div className="modal-overlay">
              <div className="modal" onClick={(e) => e.stopPropagation()}>
                <div className="modal-header">
                  <h2>{t("common.confirm")}</h2>
                  <button
                    className="modal-close"
                    onClick={() => !deletingTag && setTagDeleteConfirm(null)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage
                    message={tagDeleteConfirmError}
                    scrollKey={tagDeleteConfirmErrorScrollKey}
                  />
                  <p>
                    {t(
                      "accounts.confirmDeleteTag",
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
                    {t("common.cancel")}
                  </button>
                  <button
                    className="btn btn-danger"
                    onClick={confirmDeleteTag}
                    disabled={deletingTag}
                  >
                    {deletingTag
                      ? t("common.processing", "处理中...")
                      : t("common.confirm")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {groupDeleteConfirm && (
            <div className="modal-overlay">
              <div
                className="modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t("accounts.groups.deleteTitle")}</h2>
                  <button
                    className="modal-close"
                    onClick={() => {
                      if (deletingGroup) return;
                      setGroupDeleteConfirm(null);
                      setGroupDeleteError(null);
                    }}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage
                    message={groupDeleteError}
                    scrollKey={groupDeleteErrorScrollKey}
                  />
                  <p>
                    {t("accounts.groups.deleteConfirm", {
                      name: groupDeleteConfirm.name,
                    })}
                  </p>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => {
                      setGroupDeleteConfirm(null);
                      setGroupDeleteError(null);
                    }}
                    disabled={deletingGroup}
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    className="btn btn-danger"
                    onClick={() => void confirmDeleteGroup()}
                    disabled={deletingGroup}
                  >
                    {t("common.delete")}
                  </button>
                </div>
              </div>
            </div>
          )}

          <TagEditModal
            isOpen={!!showTagModal}
            resetKey={showTagModal}
            initialTags={
              accounts.find((a) => a.id === showTagModal)?.tags || []
            }
            availableTags={availableTags}
            onClose={() => setShowTagModal(null)}
            onSave={handleSaveTags}
          />

          {activeAccountNoteMode &&
            createPortal(
              <div className="modal-overlay">
                <div
                  className="modal codex-account-note-modal"
                  onClick={(event) => event.stopPropagation()}
                >
                  <div className="modal-header">
                    <h2>{t("codex.accountNote.title", "账号备注")}</h2>
                    <button
                      className="modal-close"
                      onClick={closeAccountNoteModal}
                      aria-label={t("common.close", "关闭")}
                      disabled={activeAccountNoteSaving}
                    >
                      <X />
                    </button>
                  </div>
                  <div className="modal-body">
                    <ModalErrorMessage
                      message={accountNoteError}
                      scrollKey={accountNoteErrorScrollKey}
                    />
                    <p className="codex-account-note-desc">
                      {t("codex.accountNote.desc", {
                        account: maskAccountText(activeAccountNoteDisplayName),
                        defaultValue:
                          "给 {{account}} 填写密码、2FA、邮件地址、手机号和其他备注。",
                      })}
                    </p>
                    <div className="codex-account-note-field">
                      <span>{t("common.shared.columns.email", "邮箱")}</span>
                      {activeAccountNoteMode === "pendingOAuth" ? (
                        <>
                          <div className="codex-account-note-input-row">
                            <input
                              className={`codex-account-note-input ${
                                pendingOAuthFieldErrors.email ? "has-error" : ""
                              }`}
                              type="email"
                              value={pendingOAuthEmailInput}
                              onChange={(event) => {
                                handlePendingOAuthEmailInputChange(
                                  event.target.value,
                                );
                              }}
                              placeholder={t(
                                "codex.pendingAuth.emailPlaceholder",
                                "输入 OpenAI 账号邮箱",
                              )}
                              disabled={activeAccountNoteSaving}
                              autoFocus
                            />
                            <button
                              type="button"
                              className="codex-account-note-icon-btn"
                              onClick={() =>
                                void copyAccountNoteValue(
                                  "modal:email",
                                  activeAccountNoteEmail,
                                )
                              }
                              disabled={
                                activeAccountNoteSaving ||
                                !activeAccountNoteEmail
                              }
                              aria-label={t("common.copy", "复制")}
                              title={t("common.copy", "复制")}
                            >
                              {accountNoteCopiedKey === "modal:email" ? (
                                <Check size={14} />
                              ) : (
                                <Copy size={14} />
                              )}
                            </button>
                          </div>
                          {pendingOAuthFieldErrors.email ? (
                            <span className="codex-account-note-field-error">
                              {pendingOAuthFieldErrors.email}
                            </span>
                          ) : null}
                        </>
                      ) : (
                        <div className="codex-account-note-readonly-row">
                          <span
                            className={`codex-account-note-readonly-value ${
                              activeAccountNoteEmail ? "" : "is-empty"
                            }`}
                            title={activeAccountNoteEmail}
                          >
                            {activeAccountNoteEmail || "-"}
                          </span>
                          <button
                            type="button"
                            className="codex-account-note-icon-btn"
                            onClick={() =>
                              void copyAccountNoteValue(
                                "modal:email",
                                activeAccountNoteEmail,
                              )
                            }
                            disabled={
                              activeAccountNoteSaving || !activeAccountNoteEmail
                            }
                            aria-label={t("common.copy", "复制")}
                            title={t("common.copy", "复制")}
                          >
                            {accountNoteCopiedKey === "modal:email" ? (
                              <Check size={14} />
                            ) : (
                              <Copy size={14} />
                            )}
                          </button>
                        </div>
                      )}
                    </div>
                    {activeAccountUsesPersonalAccessToken ? (
                      <label className="codex-account-note-field">
                        <span>
                          {t(
                            "codex.accountNote.workspaceIdLabel",
                            "ChatGPT Workspace ID",
                          )}
                        </span>
                        <div className="codex-account-note-input-row">
                          <input
                            className="codex-account-note-input"
                            type="text"
                            value={activeAccountNoteForm.chatgptAccountId}
                            onChange={(event) => {
                              updateActiveAccountNoteForm({
                                chatgptAccountId: event.target.value,
                              });
                            }}
                            placeholder={t(
                              "codex.accountNote.workspaceIdPlaceholder",
                              "输入 Team / Workspace UUID",
                            )}
                            autoComplete="off"
                            spellCheck={false}
                            disabled={activeAccountNoteSaving}
                          />
                          <button
                            type="button"
                            className="codex-account-note-icon-btn"
                            onClick={() =>
                              void copyAccountNoteValue(
                                "modal:chatgptAccountId",
                                activeAccountNoteForm.chatgptAccountId,
                              )
                            }
                            disabled={
                              activeAccountNoteSaving ||
                              !activeAccountNoteForm.chatgptAccountId.trim()
                            }
                            aria-label={t("common.copy", "复制")}
                            title={t("common.copy", "复制")}
                          >
                            {accountNoteCopiedKey ===
                            "modal:chatgptAccountId" ? (
                              <Check size={14} />
                            ) : (
                              <Copy size={14} />
                            )}
                          </button>
                        </div>
                        <small className="codex-account-note-field-hint">
                          {t(
                            "codex.accountNote.workspaceIdHint",
                            "仅用于 at-* 个人访问令牌；API 服务会将其作为 ChatGPT-Account-Id 发送。",
                          )}
                        </small>
                      </label>
                    ) : null}
                    <label className="codex-account-note-field">
                      <span>
                        {t("codex.accountNote.passwordLabel", "账号密码")}
                      </span>
                      <div className="codex-account-note-input-row">
                        <input
                          className="codex-account-note-input"
                          type={
                            accountNotePasswordVisible ? "text" : "password"
                          }
                          value={activeAccountNoteForm.accountPassword}
                          onChange={(event) => {
                            updateActiveAccountNoteForm({
                              accountPassword: event.target.value,
                            });
                          }}
                          placeholder={t(
                            "codex.accountNote.passwordPlaceholder",
                            "登录密码或临时密码",
                          )}
                          disabled={activeAccountNoteSaving}
                          autoFocus={activeAccountNoteMode !== "pendingOAuth"}
                        />
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() =>
                            setAccountNotePasswordVisible((prev) => !prev)
                          }
                          disabled={activeAccountNoteSaving}
                          aria-label={
                            accountNotePasswordVisible
                              ? t("codex.accountNote.hide", "隐藏")
                              : t("codex.accountNote.show", "显示")
                          }
                          title={
                            accountNotePasswordVisible
                              ? t("codex.accountNote.hide", "隐藏")
                              : t("codex.accountNote.show", "显示")
                          }
                        >
                          {accountNotePasswordVisible ? (
                            <EyeOff size={14} />
                          ) : (
                            <Eye size={14} />
                          )}
                        </button>
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() =>
                            void copyAccountNoteValue(
                              "modal:password",
                              activeAccountNoteForm.accountPassword,
                            )
                          }
                          disabled={
                            activeAccountNoteSaving ||
                            !activeAccountNoteForm.accountPassword.trim()
                          }
                          aria-label={t("common.copy", "复制")}
                          title={t("common.copy", "复制")}
                        >
                          {accountNoteCopiedKey === "modal:password" ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </div>
                    </label>
                    <label className="codex-account-note-field">
                      <span>
                        {t(
                          "codex.accountNote.twoFactorSecretLabel",
                          "2FA 秘钥",
                        )}
                      </span>
                      <div className="codex-account-note-input-row">
                        <input
                          className={`codex-account-note-input ${
                            accountNoteFieldErrors.twoFactorSecret
                              ? "has-error"
                              : ""
                          }`}
                          type={accountNoteSecretVisible ? "text" : "password"}
                          value={activeAccountNoteForm.twoFactorSecret}
                          onChange={(event) => {
                            updateActiveAccountNoteForm({
                              twoFactorSecret: event.target.value,
                            });
                          }}
                          placeholder={t(
                            "codex.accountNote.twoFactorSecretPlaceholder",
                            "Base32 secret 或 otpauth:// 链接",
                          )}
                          disabled={activeAccountNoteSaving}
                        />
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() => {
                            refreshSavedMfaRecords();
                            setAccountNoteMfaPickerOpen((prev) => !prev);
                          }}
                          disabled={
                            activeAccountNoteSaving ||
                            savedMfaRecords.length === 0
                          }
                          aria-label={t(
                            "mfaQuick.selectLabel",
                            "选择 2FA 秘钥",
                          )}
                          title={t("mfaQuick.selectLabel", "选择 2FA 秘钥")}
                        >
                          <ChevronDown size={14} />
                        </button>
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() =>
                            setAccountNoteSecretVisible((prev) => !prev)
                          }
                          disabled={activeAccountNoteSaving}
                          aria-label={
                            accountNoteSecretVisible
                              ? t("codex.accountNote.hide", "隐藏")
                              : t("codex.accountNote.show", "显示")
                          }
                          title={
                            accountNoteSecretVisible
                              ? t("codex.accountNote.hide", "隐藏")
                              : t("codex.accountNote.show", "显示")
                          }
                        >
                          {accountNoteSecretVisible ? (
                            <EyeOff size={14} />
                          ) : (
                            <Eye size={14} />
                          )}
                        </button>
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() =>
                            void copyAccountNoteValue(
                              "modal:twoFactorSecret",
                              activeAccountNoteForm.twoFactorSecret,
                            )
                          }
                          disabled={
                            activeAccountNoteSaving ||
                            !activeAccountNoteForm.twoFactorSecret.trim()
                          }
                          aria-label={t("common.copy", "复制")}
                          title={t("common.copy", "复制")}
                        >
                          {accountNoteCopiedKey === "modal:twoFactorSecret" ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </div>
                      {accountNoteMfaPickerOpen &&
                      savedMfaRecords.length > 0 ? (
                        <div
                          className="codex-account-note-mfa-picker"
                          role="listbox"
                          aria-label={t(
                            "mfaQuick.selectLabel",
                            "选择 2FA 秘钥",
                          )}
                        >
                          {savedMfaRecords.map((record) => {
                            const title = formatMfaRecordOption(
                              record,
                              t("mfaQuick.unnamedSecret", "未命名秘钥"),
                            );
                            const remark = record.remark?.trim();
                            const isSelected =
                              record.secret.trim() ===
                              activeAccountNoteForm.twoFactorSecret.trim();
                            const token = getMfaOtpToken(record.secret);
                            return (
                              <button
                                key={record.id}
                                type="button"
                                className={`codex-account-note-mfa-option ${isSelected ? "is-selected" : ""}`}
                                onClick={() => {
                                  updateActiveAccountNoteForm({
                                    twoFactorSecret: record.secret,
                                  });
                                  setAccountNoteMfaPickerOpen(false);
                                }}
                              >
                                <span className="codex-account-note-mfa-option__main">
                                  <strong title={title}>{title}</strong>
                                  {remark ? (
                                    <em title={remark}>{remark}</em>
                                  ) : null}
                                </span>
                                <span className="codex-account-note-mfa-option__side">
                                  {isSelected ? <Check size={14} /> : null}
                                  {token ||
                                    formatMfaSecretPreview(record.secret)}
                                </span>
                              </button>
                            );
                          })}
                        </div>
                      ) : null}
                      {accountNoteFieldErrors.twoFactorSecret ? (
                        <span className="codex-account-note-field-error">
                          {accountNoteFieldErrors.twoFactorSecret}
                        </span>
                      ) : activeAccountNoteForm.twoFactorSecret.trim() &&
                        activeAccountNoteOtpToken ? (
                        <div className="codex-account-note-otp-preview">
                          <span>
                            {t("codex.accountNote.currentOtp", "当前验证码")}
                          </span>
                          <strong>{activeAccountNoteOtpToken}</strong>
                          <button
                            type="button"
                            className="codex-account-note-icon-btn"
                            onClick={() =>
                              void copyAccountNoteValue(
                                "modal:otp",
                                activeAccountNoteOtpToken,
                              )
                            }
                            disabled={activeAccountNoteSaving}
                            aria-label={t("common.copy", "复制")}
                            title={t("common.copy", "复制")}
                          >
                            {accountNoteCopiedKey === "modal:otp" ? (
                              <Check size={14} />
                            ) : (
                              <Copy size={14} />
                            )}
                          </button>
                          <em>
                            {t("codex.accountNote.otpRemaining", {
                              defaultValue: "{{seconds}}秒",
                              seconds: mfaTimeRemaining,
                            })}
                          </em>
                        </div>
                      ) : activeAccountNoteForm.twoFactorSecret.trim() ? (
                        <span className="codex-account-note-field-error">
                          {t(
                            "codex.accountNote.twoFactorSecretInvalid",
                            "2FA 秘钥格式无效，请输入 Base32 secret 或 otpauth:// 链接",
                          )}
                        </span>
                      ) : null}
                    </label>
                    <label className="codex-account-note-field">
                      <span>
                        {t("codex.accountNote.mailUrlLabel", "邮件地址")}
                      </span>
                      <div className="codex-account-note-input-row">
                        <input
                          className="codex-account-note-input"
                          type="url"
                          value={activeAccountNoteForm.mailUrl}
                          onChange={(event) => {
                            updateActiveAccountNoteForm({
                              mailUrl: event.target.value,
                            });
                          }}
                          placeholder={t(
                            "codex.accountNote.mailUrlPlaceholder",
                            "填写可打开的邮件查询网页地址",
                          )}
                          disabled={activeAccountNoteSaving}
                        />
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={handleRefreshAccountNoteMailPreview}
                          disabled={
                            activeAccountNoteSaving ||
                            accountNoteMailPreviewLoading ||
                            !activeAccountNoteForm.mailUrl.trim()
                          }
                          aria-label={t(
                            "codex.accountNote.mailPreviewRefresh",
                            "刷新邮件",
                          )}
                          title={t(
                            "codex.accountNote.mailPreviewRefresh",
                            "刷新邮件",
                          )}
                        >
                          <RefreshCw
                            size={14}
                            className={
                              accountNoteMailPreviewLoading
                                ? "loading-spinner"
                                : ""
                            }
                          />
                        </button>
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() => void handleOpenAccountNoteMailUrl()}
                          disabled={
                            activeAccountNoteSaving ||
                            !activeAccountNoteForm.mailUrl.trim()
                          }
                          aria-label={t(
                            "codex.accountNote.mailPreviewOpen",
                            "浏览器查看",
                          )}
                          title={t(
                            "codex.accountNote.mailPreviewOpen",
                            "浏览器查看",
                          )}
                        >
                          <ExternalLink size={14} />
                        </button>
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() =>
                            void copyAccountNoteValue(
                              "modal:mailUrl",
                              activeAccountNoteForm.mailUrl,
                            )
                          }
                          disabled={
                            activeAccountNoteSaving ||
                            !activeAccountNoteForm.mailUrl.trim()
                          }
                          aria-label={t("common.copy", "复制")}
                          title={t("common.copy", "复制")}
                        >
                          {accountNoteCopiedKey === "modal:mailUrl" ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </div>
                      {accountNoteMailPreviewLoading ? (
                        <div className="codex-account-note-mail-preview is-loading">
                          {t(
                            "codex.accountNote.mailPreviewLoading",
                            "读取邮件中...",
                          )}
                        </div>
                      ) : accountNoteMailPreviewError ? (
                        <span className="codex-account-note-field-error">
                          {accountNoteMailPreviewError}
                        </span>
                      ) : accountNoteMailPreview ? (
                        <div
                          key={`${accountNoteMailPreview.code}-${accountNoteMailPreview.fetchedAt}`}
                          className={`codex-account-note-mail-preview ${
                            accountNoteMailPreview.status === "changed"
                              ? "is-changed"
                              : ""
                          }`}
                        >
                          <div className="codex-account-note-mail-preview__code">
                            <span>
                              {t(
                                "codex.accountNote.mailPreviewCode",
                                "最近一条邮箱验证码",
                              )}
                            </span>
                            <strong>{accountNoteMailPreview.code}</strong>
                            <button
                              type="button"
                              className="codex-account-note-icon-btn"
                              onClick={() =>
                                void copyAccountNoteValue(
                                  "modal:mailCode",
                                  accountNoteMailPreview.code,
                                )
                              }
                              disabled={activeAccountNoteSaving}
                              aria-label={t("common.copy", "复制")}
                              title={t("common.copy", "复制")}
                            >
                              {accountNoteCopiedKey === "modal:mailCode" ? (
                                <Check size={14} />
                              ) : (
                                <Copy size={14} />
                              )}
                            </button>
                          </div>
                          <p title={accountNoteMailPreview.snippet}>
                            {accountNoteMailPreview.snippet}
                          </p>
                          <em
                            className={`codex-account-note-mail-preview__status status-${accountNoteMailPreview.status}`}
                          >
                            {accountNoteMailPreview.status === "changed"
                              ? t(
                                  "codex.accountNote.mailPreviewStatusChanged",
                                  {
                                    defaultValue: "新验证码 · {{time}}",
                                    time: formatCodexAccountNoteMailPreviewTime(
                                      accountNoteMailPreview.fetchedAt,
                                    ),
                                  },
                                )
                              : accountNoteMailPreview.status === "unchanged"
                                ? t(
                                    "codex.accountNote.mailPreviewStatusUnchanged",
                                    {
                                      defaultValue: "未变化 · {{time}}",
                                      time: formatCodexAccountNoteMailPreviewTime(
                                        accountNoteMailPreview.fetchedAt,
                                      ),
                                    },
                                  )
                                : t(
                                    "codex.accountNote.mailPreviewStatusInitial",
                                    {
                                      defaultValue: "获取于 {{time}}",
                                      time: formatCodexAccountNoteMailPreviewTime(
                                        accountNoteMailPreview.fetchedAt,
                                      ),
                                    },
                                  )}
                          </em>
                          {accountNoteMailPreview.truncated ? (
                            <em>
                              {t(
                                "codex.accountNote.mailPreviewTruncated",
                                "内容已截断",
                              )}
                            </em>
                          ) : null}
                        </div>
                      ) : null}
                    </label>
                    <label className="codex-account-note-field">
                      <span>
                        {t("codex.accountNote.phoneNumberLabel", "手机号")}
                      </span>
                      <div className="codex-account-note-input-row">
                        <input
                          className="codex-account-note-input"
                          type="tel"
                          value={activeAccountNoteForm.phoneNumber}
                          onChange={(event) => {
                            updateActiveAccountNoteForm({
                              phoneNumber: event.target.value,
                            });
                          }}
                          placeholder={t(
                            "codex.accountNote.phoneNumberPlaceholder",
                            "绑定手机号",
                          )}
                          disabled={activeAccountNoteSaving}
                        />
                        <button
                          type="button"
                          className="codex-account-note-icon-btn"
                          onClick={() =>
                            void copyAccountNoteValue(
                              "modal:phoneNumber",
                              activeAccountNoteForm.phoneNumber,
                            )
                          }
                          disabled={
                            activeAccountNoteSaving ||
                            !activeAccountNoteForm.phoneNumber.trim()
                          }
                          aria-label={t("common.copy", "复制")}
                          title={t("common.copy", "复制")}
                        >
                          {accountNoteCopiedKey === "modal:phoneNumber" ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </div>
                    </label>
                    <label className="codex-account-note-field">
                      <span>
                        {t("codex.accountNote.otherNoteLabel", "其他备注")}
                      </span>
                      <textarea
                        className="codex-account-note-textarea"
                        value={activeAccountNoteForm.note}
                        onChange={(event) => {
                          updateActiveAccountNoteForm({
                            note: event.target.value,
                          });
                        }}
                        placeholder={t(
                          "codex.accountNote.placeholder",
                          "其他交付备注、辅助邮箱或账号说明",
                        )}
                        disabled={activeAccountNoteSaving}
                        rows={4}
                      />
                    </label>
                  </div>
                  <div className="modal-footer">
                    <button
                      className="btn btn-secondary"
                      onClick={closeAccountNoteModal}
                      disabled={activeAccountNoteSaving}
                    >
                      {t("common.cancel", "取消")}
                    </button>
                    <button
                      className="btn btn-primary"
                      onClick={() => void handleSubmitAccountNote()}
                      disabled={activeAccountNoteSaving}
                    >
                      {activeAccountNoteSaving
                        ? t("common.saving", "保存中...")
                        : t("common.save", "保存")}
                    </button>
                  </div>
                </div>
              </div>,
              document.body,
            )}

          <CodexGroupAccountPickerModal
            isOpen={!!groupQuickAddGroupId}
            targetGroup={groupQuickAddGroup}
            accounts={overviewAccounts}
            accountGroups={codexGroups}
            maskAccountText={maskAccountText}
            onClose={() => setGroupQuickAddGroupId(null)}
            onConfirm={({ accountIds }) =>
              handleQuickAddAccountsToGroup(groupQuickAddGroupId!, accountIds)
            }
          />

          <CodexAccountPoolHealthModal
            isOpen={showLocalAccessHealthModal}
            accountIds={localAccessCollection?.accountIds ?? []}
            accounts={accounts}
            accountHealth={localAccessState?.accountHealth ?? []}
            accountPoolHealth={localAccessState?.accountPoolHealth ?? []}
            actionBusy={localAccessHealthActionBusy}
            maskAccountText={maskAccountText}
            onClose={() => setShowLocalAccessHealthModal(false)}
            onRecover={(accountId) =>
              handleRecoverLocalAccessAccounts([accountId])
            }
            onRecoverAll={handleRecoverLocalAccessAccounts}
          />

          <CodexLocalAccessModal
            isOpen={showLocalAccessModal}
            mode={localAccessModalMode}
            state={localAccessState}
            addressKind={selectedLocalAccessAddressKind}
            addressOptions={localAccessAddressOptions}
            onAddressKindChange={handleLocalAccessAddressKindChange}
            accounts={accounts}
            accountsLoaded={store.accountsLoaded}
            accountGroups={codexGroups}
            memberView={
              localAccessModalMode === "members"
                ? {
                    accounts: filteredAccounts,
                    searchQuery,
                    filterTypes,
                    tagFilter,
                    groupFilter,
                    tierFilterOptions,
                    tierFilterAllLabel: t("common.shared.filter.all", {
                      count: tierCounts.all,
                    }),
                    availableTags,
                    groupFilterOptions: codexOverviewGroupFilterOptions,
                    onSearchQueryChange: setSearchQuery,
                    onToggleFilterType: toggleFilterTypeValue,
                    onClearFilterTypes: clearFilterTypes,
                    onToggleTagFilter: toggleTagFilterValue,
                    onClearTagFilter: clearTagFilter,
                    onToggleGroupFilter: toggleGroupFilterValue,
                    onClearGroupFilter: clearGroupFilter,
                  }
                : undefined
            }
            initialSelectedIds={localAccessModalSelectedIds}
            maskAccountText={maskAccountText}
            onClose={() => setShowLocalAccessModal(false)}
            onOpenFullPage={openCodexApiServicePage}
            onSaveAccounts={({
              accountIds,
              restrictFreeAccounts,
              backupAccountIds,
              preferredAccountIds,
              sessionAffinity,
              sessionAffinityTtlMs,
              imageGenerationAccountPolicies,
            }) =>
              handleSaveLocalAccessAccounts(accountIds, {
                restrictFreeAccounts,
                backupAccountIds,
                preferredAccountIds,
                sessionAffinity,
                sessionAffinityTtlMs,
                imageGenerationAccountPolicies,
              })
            }
            onClearStats={handleClearLocalAccessStats}
            onRefreshStats={reloadLocalAccessState}
            onUpdatePort={handleUpdateLocalAccessPort}
            onUpdateRoutingStrategy={handleUpdateLocalAccessRoutingStrategy}
            onUpdateCustomRouting={handleUpdateLocalAccessCustomRouting}
            onUpdateAccessScope={handleUpdateLocalAccessAccessScope}
            onUpdateDebugLogs={(debugLogs) =>
              codexLocalAccessService
                .updateCodexLocalAccessDebugLogs(debugLogs)
                .then(setLocalAccessState)
            }
            onUpdateUpstreamProxyConfig={
              handleUpdateLocalAccessUpstreamProxyConfig
            }
            onRotateApiKey={handleRotateLocalAccessApiKey}
            onRestartSidecar={handleRestartLocalAccessSidecar}
            onKillPort={handleKillLocalAccessPort}
            onToggleEnabled={handleToggleLocalAccessEnabled}
            onRecoverAccounts={handleRecoverLocalAccessAccounts}
            healthActionBusy={localAccessHealthActionBusy}
            onStreamTestMessage={({ sessionId, modelId, messages }) =>
              codexLocalAccessService.streamCodexLocalAccessChatTest(
                sessionId,
                modelId,
                messages,
              )
            }
            saving={localAccessSaving}
            testing={false}
            starting={localAccessStarting}
            portCleanupBusy={localAccessPortKilling}
            sidecarRestarting={localAccessSidecarRestarting}
          />

          {/* Codex 分组管理弹窗 */}
          <CodexAccountGroupModal
            isOpen={showCodexGroupModal}
            onClose={() => setShowCodexGroupModal(false)}
            onGroupsChanged={reloadCodexGroups}
            groupFilter={groupFilter}
            onToggleGroupFilter={toggleGroupFilterValue}
            onClearGroupFilter={clearGroupFilter}
          />

          {/* Codex 添加到分组弹窗 */}
          <CodexAddToGroupModal
            isOpen={showAddToCodexGroupModal}
            onClose={() => setShowAddToCodexGroupModal(false)}
            accountIds={Array.from(selected)}
            sourceGroupId={activeGroupId ?? undefined}
            onAdded={reloadCodexGroups}
          />
        </>
      );
}
