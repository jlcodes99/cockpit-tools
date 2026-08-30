import { ArrowDownWideNarrow, ArrowDown, ArrowUp, Check, CircleAlert, ChevronDown, Copy, Clock, Database, ExternalLink, GripVertical, HelpCircle, KeyRound, Link2, LayoutGrid, Pencil, Plus, Rows3, Star, Trash2, X, Search, Settings, Activity, RefreshCw, RotateCw, Play } from "lucide-react";
import { MultiSelectFilterDropdown } from "../MultiSelectFilterDropdown";
import { SingleSelectFilterDropdown } from "../SingleSelectFilterDropdown";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { AccountTagFilterDropdown } from "../AccountTagFilterDropdown";
import { PaginationControls } from "../PaginationControls";
import { CodexModelContextWindowTable } from "./CodexModelContextWindowTable";
import { resolveNewApiQuotaSnapshot } from "../../services/modelProviderUsageService";
import { CODEX_API_PROVIDER_CUSTOM_ID, CODEX_API_PROVIDER_PRESETS, DEEPSEEK_API_PROVIDER_ID, resolveCodexApiProviderPresetId } from "../../utils/codexProviderPresets";
import { normalizeApiKeyFunOfficialUrl } from "../../utils/apikeyFunLinks";
import { getCodexSubscriptionPresentation } from "../../types/codex";
import { resolveCodexProviderCapabilityProfile } from "../../utils/codexProviderGateway";
import { CodexQuickConfigCard } from "./CodexQuickConfigCard";
import {
  CodexServicePanelModal,
  type CodexServicePanelActionItem,
  type CodexServicePanelMetricItem,
} from "./CodexServicePanelModal";
import type {
  OAuthBindingSortBy,
  useCodexModelProviderManagerController,
} from "./CodexModelProviderManager";

export type CodexModelProviderManagerViewProps = ReturnType<typeof useCodexModelProviderManagerController>;

/** 渲染 CodexModelProviderManager 的界面；业务状态与动作统一由 Controller 提供。 */
export function CodexModelProviderManagerView(props: CodexModelProviderManagerViewProps) {
  const {
    apiKeyPickerProviderId,
    batchTestCancelling,
    batchTestDeleting,
    batchTestError,
    batchTestFilter,
    batchTestModalOpen,
    batchTestModelCustom,
    batchTestModelId,
    batchTestModelOptions,
    batchTestResultSelectedProviderIds,
    batchTestSearchQuery,
    batchTestSelectedCount,
    batchTestSelectedProviderIds,
    batchTestSession,
    batchTestStep,
    closeBatchTestModal,
    closeModal,
    currentEditingProvider,
    deepSeekStart,
    displayInstances,
    draggedProviderCustomSortId,
    editingApiKey,
    enablingProviderId,
    error,
    existingApiKeySearchQuery,
    filteredProviderBatchTestRecords,
    filteredProviderIds,
    filteredProviders,
    form,
    formatDateTime,
    formatDurationMs,
    formatProviderBatchTestErrorMessage,
    formatUsageDetailLabel,
    formatUsageDetailValue,
    formatUsageMoney,
    formatUsageQuotaValue,
    formError,
    getInstanceName,
    getProviderInstanceId,
    getSelectedProviderApiKey,
    handleBatchDeleteProviders,
    handleDeleteApiKey,
    handleDeleteBatchTestResults,
    handleDeleteProvider,
    handleEnableProvider,
    handleProviderCustomSortDragMove,
    handleProviderCustomSortDragStart,
    handleProviderInstanceChange,
    handleProviderOauthBindingChange,
    handleProviderSortByChange,
    handleRenameApiKey,
    handleSaveApiKeyEdit,
    handleSaveProvider,
    handleSelectPresetEndpoint,
    handleSelectProviderPreset,
    handleSelectSponsorTemplate,
    handleStartBatchProviderTest,
    handleTestProvider,
    instancePickerProviderId,
    isAllBatchTestProvidersSelected,
    isAllProvidersSelected,
    isCurrentProviderActive,
    isInstanceReady,
    isProviderCustomSortActive,
    isSponsorProvider,
    loading,
    maskAccountText,
    maskApiKey,
    moveProviderCustomSortProvider,
    mutateForm,
    notice,
    openBatchTestModal,
    openCreateModal,
    openEditModal,
    parseModelCatalogText,
    pickerSearchQuery,
    previewPaths,
    providerBatchTestCounts,
    providerBatchTestFilterOptions,
    providerBatchTestSelectableIds,
    providerBatchTestVisibleProviders,
    providerCustomSortDropTargetId,
    providerCustomSortProviders,
    providerDetailId,
    providerFilterOptions,
    providerNameFilter,
    providerOauthAccounts,
    providerOauthAvailableTags,
    providerOauthEligibleAccounts,
    providerOauthFilteredAccounts,
    providerOauthFilterTypes,
    providerOauthHasExistingBinding,
    providerOauthPagination,
    providerOauthSaving,
    providerOauthSearchQuery,
    providerOauthSelectedAccountId,
    providerOauthSortBy,
    providerOauthSortDirection,
    providerOauthTagFilter,
    providerOauthTarget,
    providerOauthTierCounts,
    providerOauthTierFilterOptions,
    providerReferenceMap,
    providers,
    providerSortBy,
    providerSortDirection,
    providerUsageMap,
    providerUsageRefreshingAll,
    providerViewMode,
    refreshAllProviderUsage,
    refreshProviderUsage,
    requestBatchTestCancellation,
    resetProviderCustomSortOrder,
    resolveBoundOAuthAccount,
    resolveEnableModePreferenceForWireApi,
    resolveInstanceById,
    resolvePresentation,
    resolveProviderApiKeyLabel,
    resolveProviderWireApi,
    saving,
    searchQuery,
    selectedPreset,
    selectedPresetId,
    selectedProviderApiKeyMap,
    selectedProviderIds,
    selectedProviderOauthAccount,
    selectedSponsorTemplate,
    selectedSponsorTemplateId,
    selectFailedBatchTestResults,
    setApiKeyPickerProviderId,
    setBatchTestFilter,
    setBatchTestModelCustom,
    setBatchTestModelId,
    setBatchTestSearchQuery,
    setBatchTestStep,
    setEditingApiKey,
    setExistingApiKeySearchQuery,
    setInstancePickerProviderId,
    setNotice,
    setPickerSearchQuery,
    setProviderDetailId,
    setProviderNameFilter,
    setProviderOauthFilterTypes,
    setProviderOauthPickerId,
    setProviderOauthSearchQuery,
    setProviderOauthSelectedAccountId,
    setProviderOauthSortBy,
    setProviderOauthSortDirection,
    setProviderOauthTagFilter,
    setProviderSortDirection,
    setProviderViewMode,
    setSearchQuery,
    setSelectedProviderApiKeyMap,
    setShowProviderCustomSortModal,
    setShowQuickConfigModal,
    showModal,
    showProviderCustomSortModal,
    showQuickConfigModal,
    sponsorProviderTemplates,
    stopProviderCustomSortDragging,
    t,
    testingProviderId,
    toggleAllVisibleBatchTestProviders,
    toggleAllVisibleBatchTestResults,
    toggleBatchTestProvider,
    toggleBatchTestResultProvider,
    toggleProviderOAuthFilterTypeValue,
    toggleProviderOAuthTagFilterValue,
    toggleProviderSelected,
    toggleSelectAllProviders,
  } = props;
  return (
    <div className="codex-provider-manager-page">
      {notice && (
        <div
          className={`message-bar ${notice.tone === "error" ? "error" : "success"}`}
        >
          {notice.text}
          <button
            onClick={() => setNotice(null)}
            aria-label={t("common.close", "关闭")}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {showQuickConfigModal && (
        <CodexQuickConfigCard onClose={() => setShowQuickConfigModal(false)} />
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search className="search-icon" size={16} />
            <input
              type="text"
              placeholder={t("common.search", "搜索...")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
          <div className="view-switcher">
            <button
              className={`view-btn ${providerViewMode === "compact" ? "active" : ""}`}
              onClick={() => setProviderViewMode("compact")}
              title={t("accounts.view.compact", "紧凑视图")}
            >
              <Rows3 size={16} />
            </button>
            <button
              className={`view-btn ${providerViewMode === "grid" ? "active" : ""}`}
              onClick={() => setProviderViewMode("grid")}
              title={t("common.shared.view.grid", "卡片视图")}
            >
              <LayoutGrid size={16} />
            </button>
          </div>
          <MultiSelectFilterDropdown
            options={providerFilterOptions}
            selectedValues={providerNameFilter}
            allLabel={t("common.shared.filter.all", {
              count: providers.length,
            })}
            filterLabel={t("common.shared.filterLabel", "筛选")}
            clearLabel={t("accounts.clearFilter", "清空筛选")}
            emptyLabel={t("common.none", "暂无")}
            ariaLabel={t("common.shared.filterLabel", "筛选")}
            onToggleValue={(value) =>
              setProviderNameFilter((previous) =>
                previous.includes(value)
                  ? previous.filter((item) => item !== value)
                  : [...previous, value],
              )
            }
            onClear={() => setProviderNameFilter([])}
          />
          <SingleSelectFilterDropdown
            value={providerSortBy}
            options={[
              {
                value: "name",
                label: t("common.shared.sort.name", "按名称"),
              },
              {
                value: "created_at",
                label: t("common.shared.sort.createdAt", "按创建时间"),
              },
              {
                value: "custom",
                label: t("codex.modelProviders.sort.custom", "自定义顺序"),
              },
            ]}
            ariaLabel={t("common.shared.sortLabel", "排序")}
            icon={<ArrowDownWideNarrow size={14} />}
            onChange={handleProviderSortByChange}
          />
          {!isProviderCustomSortActive && (
            <button
              className="sort-direction-btn"
              onClick={() =>
                setProviderSortDirection((previous) =>
                  previous === "asc" ? "desc" : "asc",
                )
              }
              title={t("common.shared.sort.toggleDirection", "切换排序方向")}
            >
              {providerSortDirection === "desc" ? "⬇" : "⬆"}
            </button>
          )}
        </div>
        <div className="toolbar-right">
          <button
            className="btn btn-secondary icon-only"
            onClick={() => void refreshAllProviderUsage()}
            disabled={
              providerUsageRefreshingAll ||
              providers.every((provider) => !getSelectedProviderApiKey(provider))
            }
            title={t("common.shared.refreshQuota", "刷新配额")}
          >
            <RefreshCw
              size={14}
              className={providerUsageRefreshingAll ? "loading-spinner" : undefined}
            />
          </button>
          <button
            className="btn btn-primary icon-only"
            onClick={openCreateModal}
            title={t("codex.modelProviders.add", "新增供应商")}
          >
            <Plus size={14} />
          </button>
          <button
            className="btn btn-secondary icon-only"
            onClick={() => setShowQuickConfigModal(true)}
            title={t("codex.modelProviders.quickConfig.title", "当前 Codex 配置")}
          >
            <Settings size={14} />
          </button>
          {selectedProviderIds.size > 0 && (
            <button
              className="btn btn-danger icon-only"
              onClick={() => void handleBatchDeleteProviders()}
              title={`${t("common.delete", "删除")} (${selectedProviderIds.size})`}
            >
              <Trash2 size={14} />
            </button>
          )}
        </div>
      </div>

      {error && (
        <div className="add-status error">
          <CircleAlert size={16} />
          <span>{error}</span>
        </div>
      )}

      {filteredProviderIds.length > 0 && (
        <div className="codex-overview-selection-bar">
          <div className="codex-overview-selection-left">
            <label className="codex-overview-select-all">
              <input
                type="checkbox"
                checked={isAllProvidersSelected}
                onChange={() => toggleSelectAllProviders(filteredProviderIds)}
              />
              <span>{t("common.selectAll", "全选")}</span>
            </label>
            {selectedProviderIds.size > 0 && (
              <span className="codex-overview-selected-count">
                {t("codex.modelProviders.batchTest.selectedCount", {
                  defaultValue: "已选 {{count}} 个",
                  count: selectedProviderIds.size,
                })}
              </span>
            )}
          </div>
          <div className="codex-overview-selection-actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={openBatchTestModal}
              disabled={filteredProviders.every((provider) => !getSelectedProviderApiKey(provider))}
              title={t(
                "codex.modelProviders.batchTest.entryHint",
                "批量测试供应商真实对话能力",
              )}
            >
              <Activity size={14} />
              {t("codex.modelProviders.batchTest.entry", "一键测试")}
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="section-desc">{t("common.loading", "加载中...")}</div>
      ) : providers.length === 0 ? (
        <div className="empty-state">
          <h3>{t("codex.modelProviders.emptyTitle", "暂无模型供应商")}</h3>
          <p>
            {t(
              "codex.modelProviders.emptyDesc",
              "点击右上角“新增供应商”开始维护。",
            )}
          </p>
        </div>
      ) : filteredProviders.length === 0 ? (
        <div className="empty-state">
          <h3>{t("codex.modelProviders.noMatchTitle", "没有匹配的供应商")}</h3>
          <p>{t("common.shared.noMatch.desc", "请尝试调整搜索或筛选条件")}</p>
        </div>
      ) : (
        <div className={`codex-provider-grid ${providerViewMode === "compact" ? "compact" : ""}`}>
          {filteredProviders.map((provider) => {
            const presetId = resolveCodexApiProviderPresetId(provider.baseUrl);
            const capabilityProfile = resolveCodexProviderCapabilityProfile({
              presetId,
              baseUrl: provider.baseUrl,
              wireApi: provider.wireApi,
            });
            const primaryApiKey = getSelectedProviderApiKey(provider);
            const enabling = enablingProviderId === provider.id;
            const testing = testingProviderId === provider.id;
            const usageState = providerUsageMap[provider.id];
            const usageSummary = usageState?.summary;
            const usagePrimaryText = usageSummary
              ? formatUsageQuotaValue(
                  usageSummary,
                  usageSummary.quotaRemaining ??
                    usageSummary.remaining ??
                    usageSummary.balance,
                )
              : "-";
            const usageRequestText =
              usageSummary?.todayRequests != null
                ? String(usageSummary.todayRequests)
                : "-";
            const targetInstanceId = getProviderInstanceId(provider);
            const targetInstance = resolveInstanceById(targetInstanceId);
            const targetInstanceName = getInstanceName(targetInstance);
            const targetInstanceReady = isInstanceReady(targetInstance);
            const active = isCurrentProviderActive(provider, targetInstance);
            const sponsorProvider = isSponsorProvider(
              provider,
              sponsorProviderTemplates,
            );
            const selectedApiKeyLine = primaryApiKey
              ? `${t("codex.addModal.token", "API Key")}：${maskApiKey(
                  primaryApiKey.apiKey,
                )}`
              : `${t("codex.addModal.token", "API Key")}：${t(
                  "common.none",
                  "暂无",
                )}`;
            const oauthBindingLine = `${t(
              "codex.api.oauthBinding.label",
              "OAuth 绑定",
            )}：${
              resolveBoundOAuthAccount(provider)?.account_name ||
              resolveBoundOAuthAccount(provider)?.email ||
              resolveBoundOAuthAccount(provider)?.id ||
              t("codex.api.oauthBinding.unbound", "未绑定")
            }`;
            const providerLine = `${t("codex.api.provider.label", "供应商")}：${
              provider.name
            }`;
            const apiBaseUrlLine = `${t("codex.api.baseUrl", "Base URL")}：${
              provider.baseUrl
            }`;
            const usageMode =
              usageSummary?.mode === "new_api" ||
              usageSummary?.mode === "sub2api" ||
              usageSummary?.mode === "cockpit_tools" ||
              usageSummary?.mode === "deepseek" ||
              usageSummary?.mode === "token_plan"
                ? usageSummary.mode
                : provider.integrationType ?? null;
            const deepSeekDetailValue = (key: string) => {
              const item = usageSummary?.details?.find((detail) => detail.key === key);
              return item ? formatUsageDetailValue(item, usageSummary?.unit) : "-";
            };
            const cockpitToolsDetailValue = (key: string, percent = false) => {
              const value = deepSeekDetailValue(key);
              return value === "-" || !percent ? value : `${value}%`;
            };
            const {
              granted: totalGranted,
              available: totalAvailable,
              expiresAt,
            } = resolveNewApiQuotaSnapshot(usageSummary);
            const tokenPlanResetDetail = usageSummary?.details?.find((detail) =>
              ["intervalExpiresAt", "weeklyExpiresAt", "expiresAt"].includes(
                detail.key,
              ),
            );
            const tokenPlanResetText = tokenPlanResetDetail
              ? formatUsageDetailValue(
                  tokenPlanResetDetail,
                  usageSummary?.unit,
                )
              : "-";
            const progressPercent =
              usageMode === "new_api" &&
              totalGranted != null &&
              totalAvailable != null &&
              totalGranted > 0
                ? Math.max(
                    0,
                    Math.min(
                      100,
                      Math.round(
                        ((totalGranted - totalAvailable) / totalGranted) * 100,
                      ),
                    ),
                  )
                : usageSummary?.quotaUnlimited
                  ? 100
                  : 0;
            return (
              <div
                className={`codex-account-card codex-provider-card ${active ? "current" : ""} ${sponsorProvider ? "sponsor-api-account" : ""}`}
                key={provider.id}
              >
                <div className="card-top">
                  <div className="card-select">
                    <input
                      type="checkbox"
                      checked={selectedProviderIds.has(provider.id)}
                      onChange={() => toggleProviderSelected(provider.id)}
                    />
                  </div>
                  <span className="account-email" title={provider.name}>
                    {provider.name}
                  </span>
                </div>
                <div className="account-sub-line">
                  {provider.apiKeys.length > 0 && primaryApiKey ? (
                    <div className="codex-provider-inline-line codex-provider-api-key-line">
                      <div
                        className="codex-api-key-reveal-line codex-provider-api-key-trigger"
                        title={selectedApiKeyLine}
                      >
                        <span className="codex-login-subline">
                          {selectedApiKeyLine}
                        </span>
                        <button
                          type="button"
                          className="codex-provider-inline-icon-btn"
                          onClick={() => {
                            void navigator.clipboard.writeText(primaryApiKey.apiKey);
                            setNotice({
                              tone: "success",
                              text: t(
                                "codex.modelProviders.apiKeyCopied",
                                "API Key 已复制",
                              ),
                            });
                          }}
                          title={t("common.copy", "复制")}
                        >
                          <Copy size={12} />
                        </button>
                        {provider.apiKeys.length > 1 && (
                          <button
                            type="button"
                            className="codex-provider-inline-dropdown-btn"
                            onClick={() => {
                              setPickerSearchQuery("");
                              setApiKeyPickerProviderId(provider.id);
                            }}
                            title={t("codex.modelProviders.existingApiKeys", "已有 API Key")}
                          >
                            <ChevronDown size={12} />
                          </button>
                        )}
                      </div>
                    </div>
                  ) : (
                    <span
                      className="codex-login-subline codex-provider-inline-text"
                      title={selectedApiKeyLine}
                    >
                      {selectedApiKeyLine}
                    </span>
                  )}
                </div>
                <div className="account-sub-line codex-provider-inline-line codex-oauth-binding-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={oauthBindingLine}
                  >
                    {oauthBindingLine}
                  </span>
                  <button
                    type="button"
                    className="codex-provider-inline-switch codex-oauth-binding-action"
                    onClick={() => {
                      setPickerSearchQuery("");
                      setProviderOauthPickerId(provider.id);
                    }}
                    title={t("codex.api.oauthBinding.action", "绑定 OAuth")}
                  >
                    <Link2 size={11} />
                    {resolveBoundOAuthAccount(provider)
                      ? t("common.detail", "详情")
                      : t("codex.api.oauthBinding.actionShort", "绑定")}
                  </button>
                </div>
                <div className="account-sub-line codex-provider-inline-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={providerLine}
                  >
                    {providerLine}
                  </span>
                  <button
                    type="button"
                    className="codex-provider-inline-switch"
                    onClick={() => setProviderDetailId(provider.id)}
                    title={t("codex.quickSwitch.inlineAction", "切换")}
                  >
                    {t("codex.quickSwitch.inlineAction", "切换")}
                  </button>
                </div>
                <div className="account-sub-line">
                  <span className="codex-login-subline" title={apiBaseUrlLine}>
                    {apiBaseUrlLine}
                  </span>
                </div>
                <div className="codex-quota-section">
                  {usageMode === "deepseek" ? (
                    <div className="codex-api-key-usage-panel sub2api">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.totalBalance", "总余额")}</span>
                          <strong>{usagePrimaryText}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.grantedBalance", "赠金余额")}</span>
                          <strong>{deepSeekDetailValue("grantedBalance")}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.toppedUpBalance", "充值余额")}</span>
                          <strong>{deepSeekDetailValue("toppedUpBalance")}</strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "cockpit_tools" ? (
                    <div className="codex-api-key-usage-panel cockpit-tools">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.fiveHourRemainingPercent",
                              "5h 剩余",
                            )}
                          </span>
                          <strong>
                            {cockpitToolsDetailValue(
                              "fiveHourRemainingPercent",
                              true,
                            )}
                          </strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.apiKeyBalance",
                              "API_KEY 余额",
                            )}
                          </span>
                          <strong>
                            {cockpitToolsDetailValue("apiKeyBalance")}
                          </strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.weeklyRemainingPercent",
                              "周剩余",
                            )}
                          </span>
                          <strong>
                            {cockpitToolsDetailValue(
                              "weeklyRemainingPercent",
                              true,
                            )}
                          </strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.accountCount",
                              "账号池",
                            )}
                          </span>
                          <strong>{cockpitToolsDetailValue("accountCount")}</strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.availableAccountCount",
                              "可用账号",
                            )}
                          </span>
                          <strong>
                            {cockpitToolsDetailValue("availableAccountCount")}
                          </strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "token_plan" ? (
                    <div className="codex-api-key-usage-panel token-plan">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.remaining",
                              "Remaining",
                            )}
                          </span>
                          <strong>{usagePrimaryText}</strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.planName",
                              "Plan",
                            )}
                          </span>
                          <strong>{usageSummary?.planName || "-"}</strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.expiresAt",
                              "Next Reset",
                            )}
                          </span>
                          <strong>{tokenPlanResetText}</strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "sub2api" ? (
                    <div className="codex-api-key-usage-panel sub2api">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>{t("codex.modelProviders.usage.accountBalance", "账户余额")}</span>
                          <strong>{usagePrimaryText}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.todayRequests", "今日请求")}</span>
                          <strong>{usageRequestText}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.todayTokens", "今日 Token")}</span>
                          <strong>
                            {typeof usageSummary?.todayTotalTokens === "number"
                              ? usageSummary.todayTotalTokens.toLocaleString("en-US")
                              : "-"}
                          </strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "new_api" ? (
                    <div className="codex-api-key-usage-panel">
                      <div
                        className="quota-item codex-api-key-quota-item new-api"
                        title={`${t("codex.cockpitApi.balance", "额度")}：${
                          usageSummary?.quotaUnlimited
                            ? t("codex.newApi.quota.unlimited", "不限量")
                            : totalAvailable != null && totalGranted != null
                              ? `${formatUsageMoney(totalAvailable, usageSummary?.unit)} / ${formatUsageMoney(totalGranted, usageSummary?.unit)}`
                              : totalAvailable != null
                                ? formatUsageMoney(totalAvailable, usageSummary?.unit)
                                : "-"
                        }`}
                      >
                        <div className="quota-header">
                          <Database size={14} />
                          <span className="quota-label">
                            {t("codex.cockpitApi.balance", "额度")}
                          </span>
                          <span className="quota-pct high">
                            {usageSummary?.quotaUnlimited
                              ? t("codex.newApi.quota.unlimited", "不限量")
                              : totalAvailable != null && totalGranted != null
                                ? `${formatUsageMoney(totalAvailable, usageSummary?.unit)} / ${formatUsageMoney(totalGranted, usageSummary?.unit)}`
                                : totalAvailable != null
                                  ? formatUsageMoney(totalAvailable, usageSummary?.unit)
                                : "-"}
                          </span>
                        </div>
                        <div className="quota-bar-track">
                          <div
                            className="quota-bar high"
                            style={{ width: `${progressPercent}%` }}
                          />
                        </div>
                        <span className="quota-reset">
                          {expiresAt != null && expiresAt > 0
                            ? `${t("codex.modelProviders.usage.fields.expiresAt", "过期时间")} ${new Date(expiresAt * 1000).toLocaleDateString()}`
                            : t("dashboard.noData", "暂无数据")}
                        </span>
                      </div>
                    </div>
                  ) : (
                    <div className="codex-api-key-usage-panel empty">
                      {t("codex.modelProviders.usage.noKey", "暂无可查询额度")}
                    </div>
                  )}
                </div>
                <div className="codex-card-bottom">
                  <span className="card-date">
                    {new Date(provider.updatedAt || provider.createdAt).toLocaleString()}
                  </span>
                  <button
                    type="button"
                    className="codex-speed-select codex-provider-instance-select codex-provider-instance-trigger"
                    onClick={() => {
                      setPickerSearchQuery("");
                      setInstancePickerProviderId(provider.id);
                    }}
                    title={targetInstanceName}
                  >
                    <span>{targetInstanceName}</span>
                    <ChevronDown size={12} />
                  </button>
                  <div className="card-footer">
                    <div className="card-actions">
                      <button
                        className="card-action-btn"
                        onClick={() => setProviderDetailId(provider.id)}
                        title={t("codex.modelProviders.usage.detailTitle", "服务面板")}
                      >
                        <Database size={14} />
                      </button>
                      <button
                        className="card-action-btn"
                        disabled={!primaryApiKey || usageState?.loading}
                        onClick={() =>
                          primaryApiKey &&
                          void refreshProviderUsage(provider, primaryApiKey)
                        }
                        title={t("common.shared.refreshQuota", "刷新配额")}
                      >
                        <RefreshCw
                          size={14}
                          className={usageState?.loading ? "loading-spinner" : undefined}
                        />
                      </button>
                      <button
                        className="card-action-btn success"
                        disabled={!primaryApiKey || enabling || !targetInstanceReady}
                        title={
                          targetInstanceReady
                            ? t("codex.modelProviders.enableAndStart", "启用并启动")
                            : t(
                                "codex.modelProviders.instance.uninitializedHint",
                                "目标实例尚未初始化，请先到应用多开页启动一次。",
                              )
                        }
                        onClick={() =>
                          primaryApiKey &&
                          void handleEnableProvider(
                            provider,
                            primaryApiKey,
                            targetInstanceId,
                            targetInstanceName,
                          )
                        }
                      >
                        {enabling ? (
                          <RefreshCw size={14} className="loading-spinner" />
                        ) : (
                          <Play size={14} />
                        )}
                      </button>
                      {capabilityProfile.wireApi === "chat_completions" && (
                        <button
                          className="card-action-btn"
                          disabled={!primaryApiKey || testing}
                          onClick={() =>
                            primaryApiKey &&
                            void handleTestProvider(
                              provider,
                              primaryApiKey,
                              capabilityProfile.wireApi,
                            )
                          }
                          title={t("codex.localAccess.testAction", "测试")}
                        >
                          <Activity size={14} />
                        </button>
                      )}
                      <button
                        className="card-action-btn"
                        onClick={() => openEditModal(provider)}
                        title={t("instances.actions.edit", "编辑")}
                      >
                        <Pencil size={14} />
                      </button>
                      <button
                        className="card-action-btn"
                        onClick={() => {
                          const targetUrl = normalizeApiKeyFunOfficialUrl(
                            provider.website ||
                            provider.apiKeyUrl ||
                            provider.baseUrl,
                          );
                          if (!targetUrl) return;
                          window.open(targetUrl, "_blank", "noopener,noreferrer");
                        }}
                        title={t("codex.modelProviders.website", "官网")}
                        disabled={!(provider.website || provider.apiKeyUrl || provider.baseUrl)}
                      >
                        <ExternalLink size={14} />
                      </button>
                      <button
                        className="card-action-btn danger"
                        onClick={() => void handleDeleteProvider(provider)}
                        title={t("common.delete", "删除")}
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {batchTestModalOpen && (
        <div className="modal-overlay codex-provider-batch-test-overlay">
          <div
            className={`modal codex-wakeup-modal codex-provider-batch-test-modal ${
              batchTestStep === "results" ? "codex-wakeup-results-modal" : ""
            }`}
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("codex.modelProviders.batchTest.title", "一键测试模型供应商")}</h2>
              <button
                className="modal-close"
                onClick={closeBatchTestModal}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body codex-provider-batch-test-body">
              {batchTestError && (
                <div className="add-status error">
                  <CircleAlert size={16} />
                  <span>{batchTestError}</span>
                </div>
              )}

              {batchTestStep === "select" ? (
                <>
                  <div className="codex-provider-batch-test-copy">
                    <strong>
                      {t(
                        "codex.modelProviders.batchTest.selectTitle",
                        "选择要测试的供应商",
                      )}
                    </strong>
                    <span>
                      {t(
                        "codex.modelProviders.batchTest.selectDesc",
                        "会把选中的供应商临时接入本地网关发起对话测试，并按供应商协议能力转发到上游。",
                      )}
                    </span>
                  </div>
                  <div className="form-group codex-provider-batch-test-model">
                    <label>
                      {t(
                        "codex.modelProviders.batchTest.modelLabel",
                        "测试模型",
                      )}
                    </label>
                    <SingleSelectDropdown
                      value={batchTestModelId}
                      options={batchTestModelOptions}
                      onChange={setBatchTestModelId}
                      ariaLabel={t(
                        "codex.modelProviders.batchTest.modelLabel",
                        "测试模型",
                      )}
                      placeholder={t(
                        "codex.modelProviders.batchTest.modelAuto",
                        "自动选择（按目录/探测）",
                      )}
                    />
                    {batchTestModelId === "__custom__" && (
                      <input
                        className="form-input"
                        type="text"
                        value={batchTestModelCustom}
                        onChange={(event) =>
                          setBatchTestModelCustom(event.target.value)
                        }
                        placeholder={t(
                          "codex.modelProviders.batchTest.modelCustomPlaceholder",
                          "输入模型 ID，例如 gpt-4.1-mini",
                        )}
                        style={{ marginTop: 8 }}
                      />
                    )}
                    <p className="codex-provider-batch-test-model-hint">
                      {t(
                        "codex.modelProviders.batchTest.modelHint",
                        "可选统一模型。留空则按各供应商目录或探测结果自动选择。",
                      )}
                    </p>
                  </div>
                  <div className="search-box codex-provider-batch-test-search">
                    <Search className="search-icon" size={16} />
                    <input
                      type="text"
                      placeholder={t("common.search", "搜索...")}
                      value={batchTestSearchQuery}
                      onChange={(event) => setBatchTestSearchQuery(event.target.value)}
                    />
                  </div>
                  <div className="codex-wakeup-account-selection-bar">
                    <label className="codex-overview-select-all">
                      <input
                        type="checkbox"
                        checked={isAllBatchTestProvidersSelected}
                        onChange={toggleAllVisibleBatchTestProviders}
                        disabled={providerBatchTestSelectableIds.length === 0}
                      />
                      <span>{t("common.selectAll", "全选")}</span>
                    </label>
                    <span className="codex-wakeup-account-selection-summary">
                      {t("codex.modelProviders.batchTest.selectionSummary", {
                        defaultValue: "已选 {{selected}} / 可测 {{total}}",
                        selected: batchTestSelectedCount,
                        total: providerBatchTestSelectableIds.length,
                      })}
                    </span>
                  </div>
                  <div className="codex-provider-batch-test-list">
                    {providerBatchTestVisibleProviders.map((provider) => {
                      const apiKey = getSelectedProviderApiKey(provider);
                      const wireApi = resolveProviderWireApi(provider);
                      const selected = batchTestSelectedProviderIds.has(provider.id);
                      const disabled = !apiKey;
                      return (
                        <label
                          key={provider.id}
                          className={`codex-provider-batch-test-row ${
                            selected ? "selected" : ""
                          } ${disabled ? "disabled" : ""}`}
                        >
                          <input
                            type="checkbox"
                            checked={selected}
                            disabled={disabled}
                            onChange={() => toggleBatchTestProvider(provider.id)}
                          />
                          <div className="codex-provider-batch-test-row-copy">
                            <strong>{provider.name}</strong>
                            <span>{provider.baseUrl}</span>
                            <div className="codex-provider-batch-test-row-meta">
                              <span>
                                {apiKey
                                  ? resolveProviderApiKeyLabel(
                                      apiKey,
                                      provider.name,
                                      t("codex.modelProviders.unnamedKey", "未命名 Key"),
                                    )
                                  : t(
                                      "codex.modelProviders.batchTest.noApiKey",
                                      "缺少 API Key",
                                    )}
                              </span>
                              <span>
                                {wireApi === "chat_completions"
                                  ? t(
                                      "codex.modelProviders.batchTest.gatewayProtocol",
                                      "Chat Completions · 网关",
                                    )
                                  : t(
                                      "codex.modelProviders.batchTest.directProtocol",
                                      "Responses · 网关",
                                    )}
                              </span>
                            </div>
                          </div>
                        </label>
                      );
                    })}
                    {providerBatchTestVisibleProviders.length === 0 && (
                      <p className="wakeup-hint">{t("common.none", "暂无")}</p>
                    )}
                  </div>
                </>
              ) : (
                <>
                  {batchTestSession && (
                    <>
                      <section className="codex-wakeup-results-summary-bar">
                        <div className="codex-wakeup-results-summary-copy">
                          <div className="codex-wakeup-results-summary-head">
                            <span className="codex-wakeup-results-kicker">
                              {t("codex.modelProviders.batchTest.kicker", "PROVIDER TEST")}
                            </span>
                            <h3>
                              {t(
                                "codex.modelProviders.batchTest.resultsTitle",
                                "供应商对话测试结果",
                              )}
                            </h3>
                          </div>
                          <div className="codex-wakeup-results-summary-meta">
                            <span>
                              {batchTestSession.cancelled
                                ? t("common.cancelled", "已取消")
                                : batchTestSession.running
                                ? t(
                                    "codex.modelProviders.batchTest.status.running",
                                    "测试中",
                                  )
                                : t(
                                    "codex.modelProviders.batchTest.status.completed",
                                    "已完成",
                                  )}
                            </span>
                            <span>
                              {t("codex.modelProviders.batchTest.totalCount", {
                                defaultValue: "供应商 {{count}} 个",
                                count: batchTestSession.total,
                              })}
                            </span>
                          </div>
                        </div>
                        <div className="codex-wakeup-results-summary-progress">
                          <strong>
                            {batchTestSession.completed}/{batchTestSession.total}
                          </strong>
                          <span>
                            {batchTestSession.cancelled
                              ? t("common.cancelled", "已取消")
                              : batchTestSession.running
                              ? t(
                                  "codex.modelProviders.batchTest.status.running",
                                  "测试中",
                                )
                              : t(
                                  "codex.modelProviders.batchTest.status.completed",
                                  "已完成",
                                )}
                          </span>
                        </div>
                      </section>

                      <section className="codex-wakeup-results-progress-strip">
                        <div className="codex-wakeup-results-progress-head">
                          <span>
                            {t(
                              "codex.modelProviders.batchTest.progress",
                              "测试进度",
                            )}
                          </span>
                          <strong>
                            {batchTestSession.completed}/{batchTestSession.total}
                          </strong>
                        </div>
                        <div className="codex-wakeup-results-progress-track">
                          <div
                            className="codex-wakeup-results-progress-fill"
                            style={{
                              width: `${
                                batchTestSession.total > 0
                                  ? (batchTestSession.completed /
                                      batchTestSession.total) *
                                    100
                                  : 0
                              }%`,
                            }}
                          />
                        </div>
                      </section>

                      <div className="codex-wakeup-results-filter-bar">
                        {providerBatchTestFilterOptions.map((option) => (
                          <button
                            key={option.key}
                            type="button"
                            className={`codex-wakeup-results-filter-chip ${
                              batchTestFilter === option.key ? "active" : ""
                            } tone-${option.tone}`}
                            onClick={() => setBatchTestFilter(option.key)}
                          >
                            <span>{option.label}</span>
                            <strong>{option.count}</strong>
                          </button>
                        ))}
                      </div>

                      <div className="codex-provider-batch-test-result-toolbar">
                        <label className="codex-overview-select-all">
                          <input
                            type="checkbox"
                            checked={
                              filteredProviderBatchTestRecords.length > 0 &&
                              filteredProviderBatchTestRecords.every((record) =>
                                batchTestResultSelectedProviderIds.has(record.providerId),
                              )
                            }
                            onChange={toggleAllVisibleBatchTestResults}
                            disabled={filteredProviderBatchTestRecords.length === 0}
                          />
                          <span>
                            {t("codex.modelProviders.batchTest.selectVisible", "选择当前结果")}
                          </span>
                        </label>
                        <div className="codex-provider-batch-test-result-actions">
                          <span className="codex-overview-selected-count">
                            {t("codex.modelProviders.batchTest.resultSelectedCount", {
                              defaultValue: "已选 {{count}} 个",
                              count: batchTestResultSelectedProviderIds.size,
                            })}
                          </span>
                          <button
                            type="button"
                            className="btn btn-secondary"
                            onClick={selectFailedBatchTestResults}
                            disabled={providerBatchTestCounts.error === 0}
                          >
                            {t("codex.modelProviders.batchTest.selectFailed", "选择失败")}
                          </button>
                          <button
                            type="button"
                            className="btn btn-danger"
                            onClick={() => void handleDeleteBatchTestResults()}
                            disabled={
                              batchTestResultSelectedProviderIds.size === 0 ||
                              batchTestSession.running ||
                              batchTestDeleting
                            }
                          >
                            {batchTestDeleting ? (
                              <RefreshCw size={14} className="loading-spinner" />
                            ) : (
                              <Trash2 size={14} />
                            )}
                            {t("common.delete", "删除")}
                          </button>
                        </div>
                      </div>

                      <div className="codex-wakeup-results-list">
                        {filteredProviderBatchTestRecords.map((record) => {
                          const selected = batchTestResultSelectedProviderIds.has(
                            record.providerId,
                          );
                          const recordMessage =
                            record.status === "pending"
                              ? t(
                                  "codex.modelProviders.batchTest.pendingDesc",
                                  "等待开始测试。",
                                )
                              : record.status === "running"
                                ? t(
                                    "codex.modelProviders.batchTest.runningDesc",
                                    "正在发送对话请求。",
                                  )
                              : record.status === "cancelled"
                                ? t("common.cancelled", "已取消")
                              : record.status === "success"
                                  ? record.reply ||
                                    t(
                                      "codex.modelProviders.batchTest.noReply",
                                      "上游未返回可读回复。",
                                    )
                                  : formatProviderBatchTestErrorMessage(
                                      record.error,
                                    );
                          const statusLabel =
                            record.status === "success"
                              ? t(
                                  "codex.modelProviders.batchTest.status.success",
                                  "成功",
                                )
                              : record.status === "error"
                                ? t(
                                    "codex.modelProviders.batchTest.status.error",
                                    "失败",
                                  )
                              : record.status === "cancelled"
                                ? t("common.cancelled", "已取消")
                              : record.status === "running"
                                  ? t(
                                      "codex.modelProviders.batchTest.status.running",
                                      "测试中",
                                    )
                                  : t(
                                      "codex.modelProviders.batchTest.status.pending",
                                      "等待中",
                                    );
                          return (
                            <article
                              key={record.providerId}
                              className={`codex-wakeup-execution-row codex-provider-batch-test-result-row is-${record.status}`}
                            >
                              <div className="codex-wakeup-execution-row-head">
                                <div className="codex-provider-batch-test-result-title">
                                  <label className="codex-provider-batch-test-result-check">
                                    <input
                                      type="checkbox"
                                      checked={selected}
                                      onChange={() =>
                                        toggleBatchTestResultProvider(record.providerId)
                                      }
                                      disabled={record.status === "running"}
                                    />
                                  </label>
                                  <div>
                                    <h4 className="codex-wakeup-execution-row-title">
                                      {record.providerName}
                                    </h4>
                                    <span className="codex-wakeup-execution-row-subtitle">
                                      {record.apiKeyName ||
                                        t(
                                          "codex.modelProviders.unnamedKey",
                                          "未命名 Key",
                                        )}
                                    </span>
                                  </div>
                                </div>
                                <span className={`codex-wakeup-execution-badge is-${record.status}`}>
                                  {record.status === "running" && (
                                    <RefreshCw size={14} className="loading-spinner" />
                                  )}
                                  {statusLabel}
                                </span>
                              </div>
                              <div className="codex-wakeup-execution-row-prompt">
                                {t("codex.modelProviders.batchTest.protocolSummary", {
                                  defaultValue:
                                    "{{protocol}} · {{mode}} · {{model}}",
                                  protocol:
                                    record.wireApi === "chat_completions"
                                      ? t(
                                          "codex.modelProviders.wireApi.chatCompletions",
                                          "Chat Completions",
                                        )
                                      : t(
                                          "codex.modelProviders.wireApi.responses",
                                          "Responses 原生",
                                        ),
                                  mode:
                                    record.accessMode === "gateway"
                                      ? t(
                                          "codex.modelProviders.enableMode.gatewayMode",
                                          "网关模式",
                                        )
                                      : t(
                                          "codex.modelProviders.enableMode.directMode",
                                          "直连模式",
                                        ),
                                  model:
                                    record.modelId ||
                                    t(
                                      "codex.modelProviders.batchTest.modelUnknown",
                                      "模型待定",
                                    ),
                                })}
                              </div>
                              {record.prompt && (
                                <div className="codex-wakeup-execution-row-prompt">
                                  {t("codex.modelProviders.batchTest.promptLabel", "提示词")}：
                                  {record.prompt}
                                </div>
                              )}
                              <p
                                className="codex-wakeup-execution-row-message"
                                title={
                                  record.status === "error" && record.error
                                    ? record.error
                                    : undefined
                                }
                              >
                                {recordMessage}
                              </p>
                              <div className="codex-wakeup-execution-row-meta">
                                <span>{formatDateTime(record.timestamp)}</span>
                                <span>{formatDurationMs(record.durationMs)}</span>
                              </div>
                            </article>
                          );
                        })}
                        {filteredProviderBatchTestRecords.length === 0 && (
                          <p className="wakeup-hint">{t("common.none", "暂无")}</p>
                        )}
                      </div>
                    </>
                  )}
                </>
              )}
            </div>
            <div className="modal-footer">
              {batchTestStep === "results" && !batchTestSession?.running && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setBatchTestStep("select")}
                  disabled={batchTestDeleting}
                >
                  {t("common.back", "返回")}
                </button>
              )}
              <button
                type="button"
                className="btn btn-secondary"
                onClick={closeBatchTestModal}
              >
                {t("common.close", "关闭")}
              </button>
              {batchTestStep === "results" && batchTestSession?.running && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() =>
                    void requestBatchTestCancellation(batchTestSession.runId)
                  }
                  disabled={batchTestCancelling}
                >
                  {batchTestCancelling && (
                    <RefreshCw size={14} className="loading-spinner" />
                  )}
                  {batchTestCancelling
                    ? t("common.cancelling", "正在取消...")
                    : t("common.cancel", "取消")}
                </button>
              )}
              {batchTestStep === "select" && (
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => void handleStartBatchProviderTest()}
                  disabled={batchTestSelectedCount === 0}
                >
                  <Activity size={14} />
                  {t("codex.modelProviders.batchTest.start", "开始测试")}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {showProviderCustomSortModal && (
        <div className="modal-overlay">
          <div
            className="modal codex-custom-sort-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <div>
                <h2>
                  {t(
                    "codex.modelProviders.sort.customModalTitle",
                    "自定义供应商排序",
                  )}
                </h2>
                <p className="codex-custom-sort-modal-desc">
                  {t(
                    "codex.modelProviders.sort.customModalDesc",
                    "拖动供应商或使用上下按钮调整展示顺序。",
                  )}
                </p>
              </div>
              <button
                className="modal-close"
                onClick={() => setShowProviderCustomSortModal(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div
                className={`codex-custom-sort-list ${
                  draggedProviderCustomSortId ? "is-sorting" : ""
                }`}
                onMouseUp={stopProviderCustomSortDragging}
                onMouseLeave={stopProviderCustomSortDragging}
              >
                {providerCustomSortProviders.map((provider, index) => {
                  const wireApi = resolveProviderWireApi(provider);
                  const rowClass = [
                    "codex-custom-sort-row",
                    draggedProviderCustomSortId === provider.id
                      ? "is-dragging"
                      : "",
                    draggedProviderCustomSortId &&
                    draggedProviderCustomSortId !== provider.id
                      ? "is-drop-candidate"
                      : "",
                    draggedProviderCustomSortId &&
                    draggedProviderCustomSortId !== provider.id &&
                    providerCustomSortDropTargetId === provider.id
                      ? "is-drop-target"
                      : "",
                  ]
                    .join(" ")
                    .trim();

                  return (
                    <div
                      key={provider.id}
                      className={rowClass}
                      onMouseEnter={() =>
                        handleProviderCustomSortDragMove(provider.id)
                      }
                    >
                      <div className="codex-custom-sort-row-main">
                        <button
                          type="button"
                          className="codex-custom-sort-drag-handle"
                          onMouseDown={(event) =>
                            handleProviderCustomSortDragStart(
                              event,
                              provider.id,
                            )
                          }
                          title={t(
                            "codex.modelProviders.sort.customDragHandle",
                            "拖拽排序",
                          )}
                          aria-label={t(
                            "codex.modelProviders.sort.customDragHandle",
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
                            <span title={provider.name}>{provider.name}</span>
                            <span className="mini-tag">
                              {wireApi === "chat_completions"
                                ? t(
                                    "codex.modelProviders.batchTest.gatewayProtocol",
                                    "Chat Completions · 网关",
                                  )
                                : t(
                                    "codex.modelProviders.batchTest.directProtocol",
                                    "Responses · 网关",
                                  )}
                            </span>
                          </div>
                          <div className="codex-custom-sort-quota-line codex-custom-sort-provider-meta">
                            <span title={provider.baseUrl}>
                              {provider.baseUrl}
                            </span>
                            <span>
                              {t("codex.modelProviders.apiKeysCount", {
                                defaultValue: "API Key {{count}} 个",
                                count: provider.apiKeys.length,
                              })}
                            </span>
                            <span>
                              {t("codex.modelProviders.referencesCount", {
                                defaultValue: "引用账号 {{count}} 个",
                                count: providerReferenceMap.get(provider.id) ?? 0,
                              })}
                            </span>
                          </div>
                        </div>
                      </div>
                      <div className="codex-custom-sort-row-actions">
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            moveProviderCustomSortProvider(provider.id, "up")
                          }
                          disabled={index === 0}
                          title={t(
                            "codex.modelProviders.sort.customMoveUp",
                            "上移",
                          )}
                          aria-label={t(
                            "codex.modelProviders.sort.customMoveUp",
                            "上移",
                          )}
                        >
                          <ArrowUp size={14} />
                        </button>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            moveProviderCustomSortProvider(provider.id, "down")
                          }
                          disabled={
                            index === providerCustomSortProviders.length - 1
                          }
                          title={t(
                            "codex.modelProviders.sort.customMoveDown",
                            "下移",
                          )}
                          aria-label={t(
                            "codex.modelProviders.sort.customMoveDown",
                            "下移",
                          )}
                        >
                          <ArrowDown size={14} />
                        </button>
                      </div>
                    </div>
                  );
                })}
                {providerCustomSortProviders.length === 0 && (
                  <p className="wakeup-hint">{t("common.none", "暂无")}</p>
                )}
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={resetProviderCustomSortOrder}
              >
                <RotateCw size={14} />
                {t(
                  "codex.modelProviders.sort.customReset",
                  "重置自定义顺序",
                )}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => setShowProviderCustomSortModal(false)}
              >
                {t("common.confirm", "确认")}
              </button>
            </div>
          </div>
        </div>
      )}

      {showModal && (
        <div className="modal-overlay">
          <div
            className="modal codex-provider-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {form.providerId
                  ? t("codex.modelProviders.editTitle", "编辑模型供应商")
                  : t("codex.modelProviders.createTitle", "新增模型供应商")}
              </h2>
              <button
                className="modal-close"
                onClick={closeModal}
                aria-label={t("common.close", "关闭")}
                disabled={saving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="form-group">
                <label>{t("codex.api.provider.label", "供应商")}</label>
                <div className="api-provider-chip-list">
                  <button
                    className={`api-provider-chip ${selectedPresetId === CODEX_API_PROVIDER_CUSTOM_ID && !selectedSponsorTemplateId ? "active" : ""}`}
                    onClick={() =>
                      handleSelectProviderPreset(CODEX_API_PROVIDER_CUSTOM_ID)
                    }
                    type="button"
                    disabled={saving}
                  >
                    <span>{t("codex.api.provider.custom", "自定义")}</span>
                  </button>
                  {sponsorProviderTemplates.map((template) => (
                    <button
                      key={template.id}
                      className={`api-provider-chip sponsor ${selectedSponsorTemplateId === template.id ? "active" : ""}`}
                      onClick={() => handleSelectSponsorTemplate(template)}
                      type="button"
                      disabled={saving}
                    >
                      <span>{template.name}</span>
                      <Star size={12} className="api-provider-chip-badge" />
                    </button>
                  ))}
                  {CODEX_API_PROVIDER_PRESETS.filter(
                    (preset) => !preset.isService,
                  ).map((preset) => (
                    <button
                      key={preset.id}
                      className={`api-provider-chip ${selectedPresetId === preset.id ? "active" : ""}`}
                      onClick={() => handleSelectProviderPreset(preset.id)}
                      type="button"
                      disabled={saving}
                    >
                      <span>
                        {t(
                          `codex.api.providers.${preset.id}.name`,
                          preset.name,
                        )}
                      </span>
                      {preset.isPartner && (
                        <Star size={12} className="api-provider-chip-badge" />
                      )}
                    </button>
                  ))}
                </div>
              </div>
              {selectedPreset && selectedPreset.baseUrls.length > 1 && (
                <div className="form-group">
                  <label>
                    {t("codex.api.provider.endpoint", "供应商端点")}
                  </label>
                  <div className="api-provider-endpoint-list">
                    {selectedPreset.baseUrls.map((baseUrl) => (
                      <button
                        key={baseUrl}
                        className={`api-provider-endpoint-chip ${form.baseUrl === baseUrl ? "active" : ""}`}
                        onClick={() => handleSelectPresetEndpoint(baseUrl)}
                        type="button"
                        disabled={saving}
                      >
                        {baseUrl}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {selectedPreset && (
                <div className="api-provider-hint-block">
                  <p className="api-provider-hint">
                    {t(
                      "codex.api.provider.hint",
                      "已自动填写兼容 Base URL，可继续手动调整。",
                    )}
                  </p>
                  <div className="api-provider-links">
                    {selectedPreset.website && (
                      <a
                        className="btn btn-secondary"
                        href={selectedPreset.website}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} />
                        {t("codex.api.provider.website", "官网")}
                      </a>
                    )}
                    {selectedPreset.apiKeyUrl && (
                      <a
                        className="btn btn-secondary"
                        href={selectedPreset.apiKeyUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <KeyRound size={14} />
                        {t("codex.api.provider.apiKeyPage", "API Key 页面")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              {selectedSponsorTemplate && (
                <div className="api-provider-hint-block sponsor">
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.sponsorHint",
                      "已按专属中转站配置自动填写兼容服务地址。输入 API Key 后，卡片会自动查询余额和用量。",
                    )}
                  </p>
                  <div className="api-provider-links">
                    {selectedSponsorTemplate.website && (
                      <a
                        className="btn btn-secondary"
                        href={selectedSponsorTemplate.website}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} />
                        {t("codex.api.provider.website", "官网")}
                      </a>
                    )}
                    {selectedSponsorTemplate.apiKeyUrl && (
                      <a
                        className="btn btn-secondary"
                        href={selectedSponsorTemplate.apiKeyUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <KeyRound size={14} />
                        {t("codex.api.provider.apiKeyPage", "API Key 页面")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.name", "供应商名称")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.name}
                  onChange={(event) => mutateForm({ name: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.baseUrl", "Base URL")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.baseUrl}
                  onChange={(event) =>
                    mutateForm({ baseUrl: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label className="codex-provider-label-with-help">
                  <span>{t("codex.modelProviders.fields.wireApi", "协议")}</span>
                  <span
                    className="codex-provider-inline-help"
                    title={t(
                      "codex.modelProviders.wireApi.help",
                      "大多数供应商请选择 Responses；仅当供应商明确只支持 Chat Completions 时再切换。如果不确定，优先选 Responses。",
                    )}
                    aria-label={t(
                      "codex.modelProviders.wireApi.helpAria",
                      "查看协议说明",
                    )}
                  >
                    <HelpCircle size={14} />
                  </span>
                </label>
                <div className="api-provider-chip-list">
                  <button
                    type="button"
                    className={`api-provider-chip ${form.wireApi === "responses" ? "active" : ""}`}
                    onClick={() =>
                      mutateForm({
                        wireApi: "responses",
                        enableModePreference:
                          resolveEnableModePreferenceForWireApi(
                            "responses",
                            selectedPresetId,
                          ),
                      })
                    }
                    disabled={saving}
                  >
                    <span>
                      {t("codex.modelProviders.wireApi.responses", "Responses 原生")}
                    </span>
                  </button>
                  <button
                    type="button"
                    className={`api-provider-chip ${form.wireApi === "chat_completions" ? "active" : ""}`}
                    onClick={() =>
                      mutateForm({
                        wireApi: "chat_completions",
                        supportsWebsockets: false,
                        enableModePreference:
                          resolveEnableModePreferenceForWireApi(
                            "chat_completions",
                            selectedPresetId,
                          ),
                      })
                    }
                    disabled={saving}
                  >
                    <span>
                      {t(
                        "codex.modelProviders.wireApi.chatCompletions",
                        "Chat Completions 协议",
                      )}
                    </span>
                  </button>
                </div>
                {selectedPresetId === DEEPSEEK_API_PROVIDER_ID && (
                  <p className="api-provider-hint">
                    {form.wireApi === "responses"
                      ? t(
                          "codex.modelProviders.wireApi.deepseekResponsesHint",
                          "原生 Responses 直连官方 API，写入官方 models.json（工具/shell/apply_patch），默认模型 deepseek-v4-flash。",
                        )
                      : t(
                          "codex.modelProviders.wireApi.deepseekChatHint",
                          "DeepSeek Chat Completions 走本地网关协议转换，适合兼容旧链路；需要官方 Codex 工具形态时请选 Responses。",
                        )}
                  </p>
                )}
              </div>
              {form.wireApi === "responses" && (
                <div className="form-group">
                  <label>
                    {t(
                      "codex.modelProviders.fields.supportsWebsockets",
                      "WebSocket 传输",
                    )}
                  </label>
                  <label className="provider-vision-toggle">
                    <span className="provider-vision-toggle-copy">
                      <span className="provider-vision-toggle-title">
                        {t(
                          "codex.modelProviders.websockets.title",
                          "允许 Codex 使用 Responses WebSocket",
                        )}
                      </span>
                      <span className="provider-vision-toggle-desc">
                        {t(
                          "codex.modelProviders.websockets.help",
                          "仅在供应商明确支持 Responses WebSocket 时开启；连接方式可通过 Codex 或代理服务日志确认。",
                        )}
                      </span>
                    </span>
                    <span className="provider-vision-switch">
                      <input
                        type="checkbox"
                        checked={form.supportsWebsockets}
                        onChange={(event) =>
                          mutateForm({ supportsWebsockets: event.target.checked })
                        }
                        disabled={
                          saving ||
                          selectedPresetId === "openai_official" ||
                          selectedPresetId === DEEPSEEK_API_PROVIDER_ID
                        }
                      />
                      <span className="provider-vision-switch-track" />
                    </span>
                  </label>
                </div>
              )}
              {form.wireApi === "chat_completions" && (
                <>
                  <div className="form-group">
                    <label>
                      {t("codex.modelProviders.fields.modelCatalog", "模型目录")}
                    </label>
                    <textarea
                      className="form-input"
                      rows={4}
                      value={form.modelCatalogText}
                      onChange={(event) =>
                        mutateForm({ modelCatalogText: event.target.value })
                      }
                      placeholder={"deepseek-v4-flash\ndeepseek-v4-pro"}
                      disabled={saving}
                    />
                    <CodexModelContextWindowTable
                      models={parseModelCatalogText(form.modelCatalogText)}
                      drafts={form.modelContextWindowsDraft}
                      onChange={(model, value) =>
                        mutateForm({
                          modelContextWindowsDraft: {
                            ...form.modelContextWindowsDraft,
                            [model]: value,
                          },
                        })
                      }
                      disabled={saving}
                    />
                  </div>
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.fields.visionCapability",
                        "图片输入能力",
                      )}
                    </label>
                    <label className="provider-vision-toggle">
                      <span className="provider-vision-toggle-copy">
                        <span className="provider-vision-toggle-title">
                          {t(
                            "codex.modelProviders.vision.providerDefault",
                            "该供应商默认支持图片输入",
                          )}
                        </span>
                        <span className="provider-vision-toggle-desc">
                          {t(
                            "codex.modelProviders.vision.providerDefaultHint",
                            "关闭时，只有下方列出的模型会允许图片输入；其他模型会在本地网关直接提示不支持。",
                          )}
                        </span>
                      </span>
                      <span className="provider-vision-switch">
                        <input
                          type="checkbox"
                          checked={form.supportsVision}
                          onChange={(event) =>
                            mutateForm({ supportsVision: event.target.checked })
                          }
                          disabled={saving}
                        />
                        <span className="provider-vision-switch-track" />
                      </span>
                    </label>
                  </div>
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.fields.visionModels",
                        "支持图片的模型",
                      )}
                    </label>
                    <textarea
                      className="form-input"
                      rows={3}
                      value={form.visionModelText}
                      onChange={(event) =>
                        mutateForm({ visionModelText: event.target.value })
                      }
                      placeholder={"qwen-vl-plus\ngpt-4o"}
                      disabled={saving}
                    />
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.vision.modelsHint",
                      "每行一个模型名。适合同一供应商里只有部分视觉模型支持粘贴图片的情况。",
                    )}
                  </p>
                </div>
                <div className="form-group">
                  <label>
                    {t(
                      "codex.modelProviders.fields.visionRoutingModel",
                      "图片请求默认模型",
                    )}
                  </label>
                  <input
                    className="form-input"
                    value={form.visionRoutingModel}
                    onChange={(event) =>
                      mutateForm({ visionRoutingModel: event.target.value })
                    }
                    placeholder={"mimo-v2.5"}
                    disabled={saving}
                  />
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.vision.routingModelHint",
                      "当前模型不支持图片时，带图片的请求会改用该模型；留空则直接提示不支持。",
                    )}
                  </p>
                </div>
                <p className="api-provider-hint">
                  {t(
                    "codex.modelProviders.gatewayHint",
                      "第三方供应商启动时会使用本地网关隔离实例并完成协议转换；OpenAI 官方供应商保持直连。",
                    )}
                  </p>
                </>
              )}
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.website", "官网（可选）")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.website}
                  onChange={(event) =>
                    mutateForm({ website: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t(
                    "codex.modelProviders.fields.apiKeyUrl",
                    "API Key 页面（可选）",
                  )}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.apiKeyUrl}
                  onChange={(event) =>
                    mutateForm({ apiKeyUrl: event.target.value })
                  }
                  disabled={saving}
                />
              </div>

              {currentEditingProvider &&
                currentEditingProvider.apiKeys.length > 0 && (
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.existingApiKeys",
                        "现有 API Keys",
                      )}
                    </label>
                    {currentEditingProvider.apiKeys.length > 5 && (
                      <div className="search-box codex-provider-key-search">
                        <Search className="search-icon" size={16} />
                        <input
                          type="text"
                          placeholder={t(
                            "codex.modelProviders.existingApiKeysSearch",
                            "搜索已有 API Key…",
                          )}
                          value={existingApiKeySearchQuery}
                          onChange={(event) =>
                            setExistingApiKeySearchQuery(event.target.value)
                          }
                          disabled={saving}
                        />
                      </div>
                    )}
                    <div className="codex-provider-key-list inline">
                      {currentEditingProvider.apiKeys
                        .filter((item) => {
                          const query = existingApiKeySearchQuery
                            .trim()
                            .toLowerCase();
                          if (!query) return true;
                          const label = (
                            item.name ||
                            t("codex.modelProviders.unnamedKey", "未命名 Key")
                          ).toLowerCase();
                          const masked = maskApiKey(item.apiKey).toLowerCase();
                          return (
                            label.includes(query) ||
                            masked.includes(query) ||
                            item.apiKey.toLowerCase().includes(query)
                          );
                        })
                        .map((item) => {
                          const isEditing = editingApiKey?.apiKeyId === item.id;
                          if (isEditing && editingApiKey) {
                            return (
                              <div
                                className="codex-provider-key-row is-editing"
                                key={item.id}
                              >
                                <div className="codex-provider-key-edit-fields">
                                  <input
                                    className="form-input"
                                    type="text"
                                    value={editingApiKey.name}
                                    onChange={(event) =>
                                      setEditingApiKey((current) =>
                                        current
                                          ? { ...current, name: event.target.value }
                                          : current,
                                      )
                                    }
                                    placeholder={t(
                                      "codex.modelProviders.fields.newApiKeyName",
                                      "API Key name (optional)",
                                    )}
                                    disabled={saving}
                                  />
                                  <input
                                    className="form-input"
                                    type="password"
                                    value={editingApiKey.apiKey}
                                    onChange={(event) =>
                                      setEditingApiKey((current) =>
                                        current
                                          ? { ...current, apiKey: event.target.value }
                                          : current,
                                      )
                                    }
                                    placeholder={t(
                                      "codex.modelProviders.fields.apiKey",
                                      "API Key",
                                    )}
                                    autoComplete="off"
                                    disabled={saving}
                                  />
                                </div>
                                <div className="codex-provider-key-edit-actions">
                                  <button
                                    type="button"
                                    className="action-btn success"
                                    onClick={() => void handleSaveApiKeyEdit()}
                                    disabled={saving}
                                    title={t("common.save", "Save")}
                                  >
                                    <Check size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    className="action-btn"
                                    onClick={() => setEditingApiKey(null)}
                                    disabled={saving}
                                    title={t("common.cancel", "Cancel")}
                                  >
                                    <X size={12} />
                                  </button>
                                </div>
                              </div>
                            );
                          }
                          return (
                            <div className="codex-provider-key-row" key={item.id}>
                          <div className="codex-provider-key-text">
                            <span className="codex-provider-key-name">
                              {item.name ||
                                t(
                                  "codex.modelProviders.unnamedKey",
                                  "未命名 Key",
                                )}
                            </span>
                            <code>{maskApiKey(item.apiKey)}</code>
                          </div>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() =>
                              setEditingApiKey({
                                providerId: currentEditingProvider.id,
                                apiKeyId: item.id,
                                originalApiKey: item.apiKey,
                                apiKey: item.apiKey,
                                name: item.name,
                              })
                            }
                            disabled={saving}
                            title={t(
                              "codex.modelProviders.editApiKey",
                              "Edit API Key",
                            )}
                          >
                            <KeyRound size={12} />
                          </button>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() =>
                              void handleRenameApiKey(
                                currentEditingProvider,
                                item,
                              )
                            }
                            disabled={saving}
                            title={t("common.rename", "重命名")}
                          >
                            <Pencil size={12} />
                          </button>
                          <button
                            type="button"
                            className="action-btn danger"
                            onClick={() =>
                              void handleDeleteApiKey(
                                currentEditingProvider,
                                item,
                              )
                            }
                            disabled={saving}
                            title={t("common.delete", "删除")}
                          >
                            <Trash2 size={12} />
                          </button>
                            </div>
                          );
                        })}
                    </div>
                  </div>
                )}

              <div className="form-group">
                <label>
                  {t(
                    "codex.modelProviders.fields.newApiKeyName",
                    "新增 Key 名称（可选）",
                  )}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKeyName}
                  onChange={(event) =>
                    mutateForm({ newApiKeyName: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.newApiKey", "新增 API Key")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKey}
                  onChange={(event) =>
                    mutateForm({ newApiKey: event.target.value })
                  }
                  disabled={saving}
                />
              </div>

              <div className="provider-save-preview">
                <div className="provider-save-preview-header">
                  <div className="provider-save-preview-title">
                    {t("codex.modelProviders.preview.title", "保存预览")}
                  </div>
                  <span className="provider-save-preview-chip primary">
                    {t("codex.modelProviders.preview.writeNow", "会写入")}
                  </span>
                </div>
                <p className="provider-save-preview-desc">
                  {t(
                    "codex.modelProviders.preview.desc",
                    "保存供应商时会先更新供应商仓库；不会因为这次操作立刻切换官方 Codex 的当前配置。",
                  )}
                </p>
                <div className="provider-save-preview-list">
                  <div className="provider-save-preview-item primary">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          "codex.modelProviders.preview.providerStoreTitle",
                          "模型供应商仓库",
                        )}
                      </span>
                      <span className="provider-save-preview-chip primary">
                        {t("codex.modelProviders.preview.writeNow", "会写入")}
                      </span>
                    </div>
                    <code>{previewPaths.providerStorePath}</code>
                    <p>
                      {t(
                        "codex.modelProviders.preview.providerStoreDesc",
                        "保存供应商名称、Base URL、官网/API Key 页面链接，以及本弹框新增的 API Key。",
                      )}
                    </p>
                  </div>

                  <div className="provider-save-preview-item muted">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          "codex.modelProviders.preview.codexConfigTitle",
                          "当前 Codex 配置",
                        )}
                      </span>
                      <span className="provider-save-preview-chip muted">
                        {t(
                          "codex.modelProviders.preview.noImmediateChange",
                          "不会立即修改",
                        )}
                      </span>
                    </div>
                    <code>{previewPaths.codexConfigPath}</code>
                    <p>
                      {t(
                        "codex.modelProviders.preview.codexConfigDesc",
                        "不会立即改动当前 provider 或 Base URL；只有在保存或切换 Codex API Key 账号时才会更新。",
                      )}
                    </p>
                  </div>

                  <div className="provider-save-preview-item muted">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          "codex.modelProviders.preview.authFileTitle",
                          "当前 Codex 登录凭据",
                        )}
                      </span>
                      <span className="provider-save-preview-chip muted">
                        {t(
                          "codex.modelProviders.preview.noImmediateChange",
                          "不会立即修改",
                        )}
                      </span>
                    </div>
                    <code>{previewPaths.codexAuthPath}</code>
                    <p>
                      {t(
                        "codex.modelProviders.preview.authFileDesc",
                        "不会因为保存供应商而覆盖当前 auth.json 中的 OPENAI_API_KEY。",
                      )}
                    </p>
                  </div>
                </div>
              </div>

              {formError && (
                <div className="add-status error">
                  <CircleAlert size={16} />
                  <span>{formError}</span>
                </div>
              )}
            </div>

            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={closeModal}
                disabled={saving}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                className="btn btn-primary codex-provider-save-button"
                onClick={() => void handleSaveProvider()}
                disabled={saving}
              >
                <span className="codex-provider-save-button-label">
                  <span aria-hidden={saving}>
                    {t("common.save", "保存")}
                  </span>
                  <span aria-hidden={!saving}>
                    {t("common.saving", "保存中...")}
                  </span>
                </span>
              </button>
            </div>
          </div>
        </div>
      )}

      {apiKeyPickerProviderId && (() => {
        const provider = providers.find((item) => item.id === apiKeyPickerProviderId);
        if (!provider) return null;
        const unnamedKeyLabel = t(
          "codex.modelProviders.unnamedKey",
          "未命名 Key",
        );
        const filteredApiKeys = provider.apiKeys.filter((item) =>
          resolveProviderApiKeyLabel(item, provider.name, unnamedKeyLabel)
            .toLowerCase()
            .includes(pickerSearchQuery.trim().toLowerCase()),
        );
        return (
          <div className="modal-overlay">
            <div
              className="modal codex-provider-picker-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <h2>{t("codex.modelProviders.existingApiKeys", "已有 API Key")}</h2>
                <button
                  className="modal-close"
                  onClick={() => setApiKeyPickerProviderId(null)}
                  aria-label={t("common.close", "关闭")}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body codex-provider-picker-body">
                <div className="search-box codex-provider-picker-search">
                  <Search className="search-icon" size={16} />
                  <input
                    type="text"
                    placeholder={t("common.search", "搜索...")}
                    value={pickerSearchQuery}
                    onChange={(event) => setPickerSearchQuery(event.target.value)}
                  />
                </div>
                <div className="codex-provider-picker-list">
                  {filteredApiKeys.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`codex-provider-picker-item ${selectedProviderApiKeyMap[provider.id] === item.id || (!selectedProviderApiKeyMap[provider.id] && provider.apiKeys[0]?.id === item.id) ? "active" : ""}`}
                      onClick={() => {
                        setSelectedProviderApiKeyMap((previous) => ({
                          ...previous,
                          [provider.id]: item.id,
                        }));
                        setApiKeyPickerProviderId(null);
                      }}
                    >
                      <span>{resolveProviderApiKeyLabel(item, provider.name, unnamedKeyLabel)}</span>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>
        );
      })()}

      {instancePickerProviderId && (() => {
        const provider = providers.find((item) => item.id === instancePickerProviderId);
        if (!provider) return null;
        const filteredInstances = displayInstances.filter((instance) =>
          getInstanceName(instance)
            .toLowerCase()
            .includes(pickerSearchQuery.trim().toLowerCase()),
        );
        return (
          <div className="modal-overlay">
            <div
              className="modal codex-provider-picker-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <h2>{t("codex.modelProviders.instance.shortLabel", "实例")}</h2>
                <button
                  className="modal-close"
                  onClick={() => setInstancePickerProviderId(null)}
                  aria-label={t("common.close", "关闭")}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body codex-provider-picker-body">
                <div className="search-box codex-provider-picker-search">
                  <Search className="search-icon" size={16} />
                  <input
                    type="text"
                    placeholder={t("common.search", "搜索...")}
                    value={pickerSearchQuery}
                    onChange={(event) => setPickerSearchQuery(event.target.value)}
                  />
                </div>
                <div className="codex-provider-picker-list">
                  {filteredInstances.map((instance) => (
                    <button
                      key={instance.id}
                      type="button"
                      className={`codex-provider-picker-item ${getProviderInstanceId(provider) === instance.id ? "active" : ""}`}
                      onClick={() => {
                        void handleProviderInstanceChange(provider, instance.id);
                        setInstancePickerProviderId(null);
                      }}
                    >
                      <span>
                        {getInstanceName(instance)}
                        {instance.running
                          ? ` · ${t("codex.modelProviders.instance.running", "运行中")}`
                          : ""}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>
        );
      })()}

      {providerOauthTarget && (
        <div
          className="modal-overlay"
        >
          <div
            className="modal-content codex-add-modal codex-oauth-binding-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("codex.api.oauthBinding.title", "绑定 OAuth 账号")}</h2>
              <button
                className="modal-close"
                onClick={() => !providerOauthSaving && setProviderOauthPickerId(null)}
                aria-label={t("common.close", "关闭")}
                disabled={providerOauthSaving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-section">
                <div className="codex-oauth-binding-context">
                  <p className="section-desc codex-oauth-binding-desc">
                    {t(
                      "codex.modelProviders.oauthBinding.desc",
                      "可选绑定。只要 OAuth 账号带 refresh_token 即可选择；未绑定时供应商仍按原 API Key 逻辑运行。",
                    )}
                  </p>
                  <div className="section-desc codex-oauth-binding-current-target">
                    {t("codex.modelProviders.oauthBinding.currentProvider", {
                      defaultValue: "供应商：{{name}}",
                      name: providerOauthTarget.name,
                    })}
                  </div>
                </div>
                <div className="codex-oauth-binding-picker">
                  <div className="codex-oauth-binding-picker-header">
                    <label>
                      {t("codex.api.oauthBinding.selectLabel", "选择 OAuth 账号")}
                    </label>
                  </div>
                  {providerOauthAccounts.length === 0 ? (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>
                        {t(
                          "codex.api.oauthBinding.empty",
                          "暂无 OAuth 账号，请先添加 OAuth 授权账号。",
                        )}
                      </span>
                    </div>
                  ) : providerOauthEligibleAccounts.length === 0 ? (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>
                        {t(
                          "codex.api.oauthBinding.emptyEligible",
                          "没有带 refresh_token 的 OAuth 账号，请重新 OAuth 授权或添加符合条件的 OAuth 账号。",
                        )}
                      </span>
                    </div>
                  ) : (
                    <>
                      <div className="codex-oauth-binding-toolbar">
                        <div className="search-box codex-oauth-binding-search">
                          <Search size={16} className="search-icon" />
                          <input
                            type="text"
                            placeholder={t("common.shared.search", "搜索账号...")}
                            value={providerOauthSearchQuery}
                            onChange={(event) =>
                              setProviderOauthSearchQuery(event.target.value)
                            }
                            disabled={providerOauthSaving}
                          />
                        </div>
                        <MultiSelectFilterDropdown
                          options={providerOauthTierFilterOptions}
                          selectedValues={providerOauthFilterTypes}
                          allLabel={t("common.shared.filter.all", {
                            count: providerOauthTierCounts.all,
                          })}
                          filterLabel={t("common.shared.filterLabel", "筛选")}
                          clearLabel={t("accounts.clearFilter", "清空筛选")}
                          emptyLabel={t("common.none", "暂无")}
                          ariaLabel={t("common.shared.filterLabel", "筛选")}
                          onToggleValue={toggleProviderOAuthFilterTypeValue}
                          onClear={() => setProviderOauthFilterTypes([])}
                        />
                        <AccountTagFilterDropdown
                          availableTags={providerOauthAvailableTags}
                          selectedTags={providerOauthTagFilter}
                          onToggleTag={toggleProviderOAuthTagFilterValue}
                          onClear={() => setProviderOauthTagFilter([])}
                        />
                        <SingleSelectFilterDropdown
                          value={providerOauthSortBy}
                          options={[
                            {
                              value: "last_used",
                              label: t("accounts.columns.lastUsed", "最后使用"),
                            },
                            {
                              value: "created_at",
                              label: t("common.shared.sort.createdAt", "按创建时间"),
                            },
                            {
                              value: "account",
                              label: t("common.shared.columns.account", "账号"),
                            },
                            {
                              value: "plan",
                              label: t("accounts.sort.plan", "按套餐"),
                            },
                          ]}
                          ariaLabel={t("common.shared.sortLabel", "排序")}
                          icon={<ArrowDownWideNarrow size={14} />}
                          disabled={providerOauthSaving}
                          onChange={(value) =>
                            setProviderOauthSortBy(value as OAuthBindingSortBy)
                          }
                        />
                        <button
                          type="button"
                          className="sort-direction-btn"
                          onClick={() =>
                            setProviderOauthSortDirection((prev) =>
                              prev === "desc" ? "asc" : "desc",
                            )
                          }
                          disabled={providerOauthSaving}
                          title={
                            providerOauthSortDirection === "desc"
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
                          {providerOauthSortDirection === "desc" ? (
                            <ArrowDown size={15} />
                          ) : (
                            <ArrowUp size={15} />
                          )}
                        </button>
                      </div>
                      {providerOauthFilteredAccounts.length === 0 ? (
                        <div className="group-account-empty">
                          <span>
                            {t("common.shared.noMatch.title", "没有匹配的账号")}
                          </span>
                        </div>
                      ) : (
                        <div className="codex-oauth-binding-list">
                          {providerOauthPagination.pageItems.map((account) => {
                            const presentation = resolvePresentation(account);
                            const subscriptionInfo =
                              getCodexSubscriptionPresentation(
                                account.subscription_active_until,
                                t,
                              );
                            const selected =
                              providerOauthSelectedAccountId === account.id;
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
                                aria-disabled={providerOauthSaving}
                                onClick={(event) => {
                                  if (providerOauthSaving) {
                                    event.preventDefault();
                                    return;
                                  }
                                  setProviderOauthSelectedAccountId(account.id);
                                }}
                              >
                                <input
                                  type="radio"
                                  name="codex-provider-oauth-binding-account"
                                  checked={selected}
                                  onChange={() =>
                                    setProviderOauthSelectedAccountId(account.id)
                                  }
                                  disabled={providerOauthSaving}
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
                                      {t("codex.subscription.label", "有效期")}
                                    </span>
                                    <strong>{subscriptionInfo.valueText}</strong>
                                    <span>{subscriptionInfo.detailText}</span>
                                  </span>
                                </div>
                              </label>
                            );
                          })}
                        </div>
                      )}
                      <PaginationControls
                        totalItems={providerOauthPagination.totalItems}
                        currentPage={providerOauthPagination.currentPage}
                        totalPages={providerOauthPagination.totalPages}
                        pageSize={providerOauthPagination.pageSize}
                        pageSizeOptions={providerOauthPagination.pageSizeOptions}
                        rangeStart={providerOauthPagination.rangeStart}
                        rangeEnd={providerOauthPagination.rangeEnd}
                        canGoPrevious={providerOauthPagination.canGoPrevious}
                        canGoNext={providerOauthPagination.canGoNext}
                        onPageSizeChange={providerOauthPagination.setPageSize}
                        onPreviousPage={providerOauthPagination.goToPreviousPage}
                        onNextPage={providerOauthPagination.goToNextPage}
                      />
                    </>
                  )}
                </div>
                <div className="api-key-edit-actions">
                  {providerOauthHasExistingBinding && (
                    <button
                      className="btn btn-secondary codex-oauth-binding-clear"
                      onClick={() =>
                        void handleProviderOauthBindingChange(
                          providerOauthTarget,
                          null,
                        )
                      }
                      disabled={providerOauthSaving}
                    >
                      {t("codex.api.oauthBinding.clearAction", "解除绑定")}
                    </button>
                  )}
                  <button
                    className="btn btn-secondary"
                    onClick={() => setProviderOauthPickerId(null)}
                    disabled={providerOauthSaving}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() =>
                      selectedProviderOauthAccount &&
                      void handleProviderOauthBindingChange(
                        providerOauthTarget,
                        selectedProviderOauthAccount.id,
                      )
                    }
                    disabled={
                      providerOauthSaving ||
                      !selectedProviderOauthAccount ||
                      providerOauthEligibleAccounts.length === 0
                    }
                  >
                    {providerOauthSaving
                      ? t("common.saving", "保存中...")
                      : t("common.save", "保存")}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {providerDetailId && (() => {
        const provider = providers.find((item) => item.id === providerDetailId);
        if (!provider) return null;
        const usageState = providerUsageMap[provider.id];
        const primaryApiKey = getSelectedProviderApiKey(provider);
        const usageSummary = usageState?.summary;
        const resolvedWireApi = resolveProviderWireApi(provider);
        const providerUsageDetailValue = (key: string) => {
          const item = usageSummary?.details?.find((detail) => detail.key === key);
          return item ? formatUsageDetailValue(item, usageSummary?.unit) : "-";
        };
        const providerUsagePercentDetailValue = (key: string) => {
          const value = providerUsageDetailValue(key);
          return value === "-" ? value : `${value}%`;
        };
        const usageMode =
          usageSummary?.mode === "new_api" ||
          usageSummary?.mode === "sub2api" ||
          usageSummary?.mode === "cockpit_tools" ||
          usageSummary?.mode === "deepseek" ||
          usageSummary?.mode === "token_plan"
            ? usageSummary.mode
            : provider.integrationType ?? null;
        const coreDetailKeys =
          usageMode === "new_api"
            ? new Set(["mode", "totalGranted", "totalAvailable", "expiresAt"])
            : usageMode === "sub2api"
              ? new Set(["mode", "remaining", "todayRequests", "todayTokens"])
              : usageMode === "deepseek"
                ? new Set([
                    "isAvailable",
                    "currency",
                    "totalBalance",
                    "grantedBalance",
                    "toppedUpBalance",
                  ])
                : usageMode === "cockpit_tools"
                  ? new Set([
                      "mode",
                      "scope",
                      "fiveHourRemainingPercent",
                      "weeklyRemainingPercent",
                      "apiKeyBalance",
                      "accountCount",
                      "availableAccountCount",
                      "abnormalAccountCount",
                      "cooldownAccountCount",
                    ])
                  : usageMode === "token_plan"
                    ? new Set([
                        "mode",
                        "remaining",
                        "planName",
                        "expiresAt",
                      ])
                    : new Set<string>();
        const detailMetrics: CodexServicePanelMetricItem[] = [
          {
            key: "wireApi",
            label: t("codex.modelProviders.fields.wireApi", "协议"),
            value:
              resolvedWireApi === "chat_completions"
                ? t(
                    "codex.modelProviders.wireApi.chatCompletions",
                    "Chat Completions 协议",
                  )
                : t(
                    "codex.modelProviders.wireApi.responses",
                    "Responses 原生",
                  ),
            rawKey: "wireApi",
          },
          {
            key: "supportsWebsockets",
            label: t(
              "codex.modelProviders.fields.supportsWebsockets",
              "WebSocket 传输",
            ),
            value:
              resolvedWireApi === "responses" && provider.supportsWebsockets
                ? t("codex.modelProviders.websockets.enabled", "已启用")
                : t("codex.modelProviders.websockets.disabled", "已停用"),
            rawKey: "supportsWebsockets",
          },
          {
            key: "oauthBinding",
            label: t("codex.api.oauthBinding.label", "OAuth 绑定"),
            value:
              resolveBoundOAuthAccount(provider)?.account_name ||
              resolveBoundOAuthAccount(provider)?.email ||
              resolveBoundOAuthAccount(provider)?.id ||
              t("codex.api.oauthBinding.unbound", "未绑定"),
            rawKey: "boundOauthAccountId",
          },
          ...(resolveCodexApiProviderPresetId(provider.baseUrl) ===
          DEEPSEEK_API_PROVIDER_ID
            ? [
                {
                  key: "enableMode",
                  label: t("codex.modelProviders.enableMode.label", "接入方式"),
                  value: t(
                    "codex.deepSeek.start.chooseAtStart",
                    "启动时选择",
                  ),
                  rawKey: "enableMode",
                },
              ]
            : resolvedWireApi === "chat_completions"
              ? [
                  {
                    key: "enableMode",
                    label: t("codex.modelProviders.enableMode.label", "接入方式"),
                    value: t(
                      "codex.modelProviders.enableMode.gatewayMode",
                      "网关模式",
                    ),
                    rawKey: "enableMode",
                  },
                ]
              : [
                  {
                    key: "enableMode",
                    label: t("codex.modelProviders.enableMode.label", "接入方式"),
                    value: t(
                      "codex.modelProviders.enableMode.directMode",
                      "直连模式",
                    ),
                    rawKey: "enableMode",
                  },
                ]),
          {
            key: "vision",
            label: t("codex.modelProviders.vision.allModels", "图片输入"),
            value: provider.supportsVision
              ? t("common.yes", "是")
              : t("common.no", "否"),
            rawKey: "supportsVision",
          },
          {
            key: "modelCatalog",
            label: t("codex.modelProviders.modelCatalog", "模型"),
            value:
              (provider.modelCatalog?.length ?? 0) > 0
                ? (provider.modelCatalog ?? []).join(", ")
                : t("codex.modelProviders.modelCatalogEmpty", "未配置模型目录"),
            rawKey: "modelCatalog",
          },
          ...((usageSummary?.details ?? [])
            .filter((item) => !coreDetailKeys.has(item.key))
            .map((item) => ({
              key: item.key,
              label: formatUsageDetailLabel(item.key, item.label),
              value: formatUsageDetailValue(item, usageSummary?.unit),
              rawKey: item.key,
            })) as CodexServicePanelMetricItem[]),
        ];

        const newApiQuota = resolveNewApiQuotaSnapshot(usageSummary);
        const coreMetrics: CodexServicePanelMetricItem[] =
          usageMode === "deepseek"
            ? [
                "isAvailable",
                "currency",
                "totalBalance",
                "grantedBalance",
                "toppedUpBalance",
              ].map((key) => {
                const item = usageSummary?.details?.find((detail) => detail.key === key);
                return {
                  key,
                  label: formatUsageDetailLabel(key, key),
                  value: item ? formatUsageDetailValue(item, usageSummary?.unit) : "-",
                };
              })
            : usageMode === "cockpit_tools"
              ? [
                  {
                    key: "fiveHourRemainingPercent",
                    label: t(
                      "codex.modelProviders.usage.fields.fiveHourRemainingPercent",
                      "5h 剩余",
                    ),
                    value: providerUsagePercentDetailValue(
                      "fiveHourRemainingPercent",
                    ),
                  },
                  {
                    key: "weeklyRemainingPercent",
                    label: t(
                      "codex.modelProviders.usage.fields.weeklyRemainingPercent",
                      "周剩余",
                    ),
                    value: providerUsagePercentDetailValue(
                      "weeklyRemainingPercent",
                    ),
                  },
                  {
                    key: "apiKeyBalance",
                    label: t(
                      "codex.modelProviders.usage.fields.apiKeyBalance",
                      "API_KEY 余额",
                    ),
                    value: providerUsageDetailValue("apiKeyBalance"),
                  },
                  {
                    key: "accountCount",
                    label: t(
                      "codex.modelProviders.usage.fields.accountCount",
                      "账号池",
                    ),
                    value: providerUsageDetailValue("accountCount"),
                  },
                  {
                    key: "availableAccountCount",
                    label: t(
                      "codex.modelProviders.usage.fields.availableAccountCount",
                      "可用账号",
                    ),
                    value: providerUsageDetailValue("availableAccountCount"),
                  },
                ]
              : usageMode === "new_api"
                ? [
                {
                  key: "totalGranted",
                  label: t("codex.modelProviders.usage.fields.totalGranted", "授予额度"),
                  value: formatUsageDetailValue(
                    {
                      key: "totalGranted",
                      value: String(newApiQuota.granted ?? "-"),
                    },
                    usageSummary?.unit,
                  ),
                },
                {
                  key: "totalAvailable",
                  label: t("codex.modelProviders.usage.fields.totalAvailable", "可用额度"),
                  value: formatUsageDetailValue(
                    {
                      key: "totalAvailable",
                      value: String(newApiQuota.available ?? "-"),
                    },
                    usageSummary?.unit,
                  ),
                },
                {
                  key: "expiresAt",
                  label: t("codex.modelProviders.usage.fields.expiresAt", "过期时间"),
                  value: formatUsageDetailValue(
                    {
                      key: "expiresAt",
                      value: String(newApiQuota.expiresAt ?? "-"),
                    },
                    usageSummary?.unit,
                  ),
                },
                ]
                : usageMode === "sub2api"
                  ? [
                  {
                    key: "accountBalance",
                    label: t("codex.modelProviders.usage.accountBalance", "账户余额"),
                    value: formatUsageQuotaValue(
                      usageSummary,
                      usageSummary?.remaining ??
                        usageSummary?.balance ??
                        usageSummary?.quotaRemaining,
                    ),
                  },
                  {
                    key: "todayRequests",
                    label: t("codex.modelProviders.usage.fields.todayRequests", "今日请求"),
                    value: String(usageSummary?.todayRequests ?? 0),
                  },
                  {
                    key: "todayTokens",
                    label: t("codex.modelProviders.usage.fields.todayTokens", "今日 Token"),
                    value: (usageSummary?.todayTotalTokens ?? 0).toLocaleString("en-US"),
                  },
                    ]
                  : [];

        const actions: CodexServicePanelActionItem[] = [
          {
            key: "refresh",
            label: t("common.shared.refreshQuota", "刷新配额"),
            variant: "secondary",
            icon: (
              <RefreshCw
                size={14}
                className={usageState?.loading ? "loading-spinner" : ""}
              />
            ),
            disabled: !primaryApiKey || usageState?.loading,
            onClick: () => {
              if (primaryApiKey) {
                void refreshProviderUsage(provider, primaryApiKey);
              }
            },
          },
          {
            key: "edit",
            label: t("instances.actions.edit", "编辑"),
            variant: "secondary",
            icon: <Pencil size={14} />,
            onClick: () => {
              setProviderDetailId(null);
              openEditModal(provider);
            },
          },
          {
            key: "oauth",
            label: t("codex.api.oauthBinding.action", "绑定 OAuth"),
            variant: "secondary",
            icon: <Link2 size={14} />,
            onClick: () => {
              setProviderDetailId(null);
              setProviderOauthPickerId(provider.id);
            },
          },
        ];

        if (provider.website || provider.apiKeyUrl || provider.baseUrl) {
          actions.push({
            key: "website",
            label: t("codex.modelProviders.website", "官网"),
            variant: "secondary",
            icon: <ExternalLink size={14} />,
            onClick: () => {
              const targetUrl = normalizeApiKeyFunOfficialUrl(
                provider.website || provider.apiKeyUrl || provider.baseUrl,
              );
              if (!targetUrl) return;
              window.open(targetUrl, "_blank", "noopener,noreferrer");
            },
          });
        }

        return (
          <CodexServicePanelModal
            open={true}
            title={t("codex.modelProviders.usage.detailTitle", "服务面板")}
            subtitle={provider.name}
            baseUrl={provider.baseUrl}
            apiKeyDisplay={primaryApiKey ? maskApiKey(primaryApiKey.apiKey) : "-"}
            rawApiKey={primaryApiKey?.apiKey}
            coreMetrics={coreMetrics}
            detailMetrics={detailMetrics}
            actions={actions}
            onClose={() => setProviderDetailId(null)}
            emptyDetailText={t("codex.cockpitApi.noStats", "暂无统计")}
          />
        );
      })()}
      {deepSeekStart.modal}
    </div>
  );
}
