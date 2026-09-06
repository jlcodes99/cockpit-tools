import { Activity, BadgeDollarSign, ChevronDown, Check, CircleAlert, Copy, Eye, EyeOff, FolderPlus, Gauge, Image, Pin, PinOff, Play, Plus, Power, RefreshCw, Route, Send, ShieldCheck, SlidersHorizontal, Trash2, Undo2, Wrench, X } from "lucide-react";
import { CodexIcon } from "../components/icons/CodexIcon";
import { ManualHelpIconButton } from "../components/ManualHelpIconButton";
import { PlatformGroupSwitcher } from "../components/platform/PlatformGroupSwitcher";
import { resolveGroupChildName } from "../stores/usePlatformLayoutStore";
import { getPlatformLabel } from "../utils/platformMeta";
import { isCodexApiKeyScopeAccountActive, selectCodexApiKeyScopeAccounts } from "../utils/codexApiKeyAccountScope";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { formatCodexQuotaPoolPercent, formatCodexQuotaPoolWindowLabel } from "../utils/codexQuotaPool";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { CodexLocalAccessModal } from "../components/CodexLocalAccessModal";
import { CodexAccountPoolHealthModal } from "../components/CodexAccountPoolHealthModal";
import { CodexStatsRangePicker } from "../components/CodexStatsRangePicker";
import { CodexUsageTrend } from "../components/codex/CodexUsageTrend";
import { PaginationControls } from "../components/PaginationControls";
import type {
  CodexLocalAccessCustomRoutingRule,
  CodexLocalAccessScope,
} from "../types/codexLocalAccess";
import "./CodexApiServicePage.css";
import type {
  CopyField,
  RequestLogGatewayModeFilter,
  RequestLogKindFilter,
  RequestLogStatusFilter,
  useCodexApiServicePageController,
} from "./CodexApiServicePage";

export type CodexApiServiceViewProps = ReturnType<typeof useCodexApiServicePageController>;

/** 渲染 CodexApiServicePage 的界面；业务状态与动作统一由 Controller 提供。 */
export function CodexApiServiceView(props: CodexApiServiceViewProps) {
  const {
    accessScope,
    accessScopeOptions,
    accountDisplayNames,
    accountModelMappingDrafts,
    accountModelMappingError,
    accountModelMappingsOpen,
    accountModelRuleAllSelected,
    accountModelRuleBulkText,
    accountModelRuleCount,
    accountModelRuleDrafts,
    accountModelRuleSelected,
    accountModelRulesOpen,
    accounts,
    accountsLoaded,
    activating,
    activeTab,
    addAccountModelMappingRow,
    addressKind,
    apiKeyDrafts,
    apiKeyHasFixedAccountScope,
    apiKeyInheritsAccountPool,
    apiKeyPolicyDraftFromValue,
    apiKeyPolicyDraftIsDirty,
    apiKeyPolicyDrafts,
    apiKeyStatsById,
    apiServiceIsCurrent,
    applyTimeoutPreset,
    availableAccountCount,
    busy,
    cleanRequestLogErrorDetail,
    clearRequestLogFilters,
    clearTestChat,
    clientBaseUrlHost,
    clientBaseUrlHostOptions,
    codexInstances,
    collection,
    compatibilityExamples,
    cooldownCount,
    copiedField,
    currentGroup,
    currentPlatformId,
    disableCoolingDraft,
    displayBaseUrl,
    error,
    excludedModelsText,
    expandedApiKeyPolicyIds,
    fillDeepSeekAccountModelMappings,
    formatAccountTokenUsage,
    formatCompactNumber,
    formatDateTime,
    formatLatencyMs,
    formatRequestResultDetail,
    formatUsdCost,
    gatewayModeLabel,
    groups,
    handleActivateService,
    handleApplyAccountModelRuleBulk,
    handleClearStats,
    handleRefreshStatsDetails,
    handleCloseAccountModelMappings,
    handleCloseAccountModelRules,
    handleCloseTestDialog,
    handleCopy,
    handleCreateApiKey,
    handleCreateTimeoutPreset,
    handleCustomStatsRangeApply,
    handleDeleteApiKey,
    handleDeleteTimeoutPreset,
    handleKillPort,
    handleOpenAccountModelMappings,
    handleOpenAccountModelRules,
    handleOpenAddAccount,
    handleOpenPricingModal,
    handleOpenTestDialog,
    handleRecoverAccounts,
    handleRemoveMember,
    handleRepriceRequestLogs,
    handleResetApiKeyPolicy,
    handleResetTimeoutDrafts,
    handleRestartSidecar,
    handleRotateApiKey,
    handleSaveAccountModelMappings,
    handleSaveAccountModelRules,
    handleSaveApiKeyLabel,
    handleSaveApiKeyPolicy,
    handleSaveMembersFromModal,
    handleSaveModelPricings,
    handleSaveModelRules,
    handleSavePort,
    handleSaveProxy,
    handleSaveRoutingOptions,
    handleSaveTimeouts,
    handleSendTestChatMessage,
    handleSetApiKeyAccountPriority,
    handleStatsPresetChange,
    handleToggleApiKey,
    handleToggleEnabled,
    handleUpdateAccessScope,
    handleUpdateClientBaseUrlHost,
    handleUpdateRouting,
    handleUpdateTimeoutPreset,
    hasRequestLogFilters,
    healthByAccountId,
    healthModalOpen,
    imageUnavailableCount,
    immediateSseResponseDraft,
    keyVisible,
    localAccessAccounts,
    mappingDraftsFromAccount,
    mappingMemberAccounts,
    maskAccountText,
    maxConcurrentImageRequestsDraft,
    maxRetryCredentialsDraft,
    maxRetryIntervalDraft,
    memberAccountIds,
    memberAccounts,
    memberIds,
    memberModalOpen,
    memberView,
    modelAliasesText,
    modelIds,
    normalizeAddressKind,
    normalizeRequestLogPageSize,
    notice,
    parseModelRuleText,
    portInput,
    portKilling,
    pricingDrafts,
    pricingError,
    pricingModalOpen,
    pricingRepriceActive,
    pricingRepricePercent,
    pricingRepriceProgress,
    pricingRepriceStatusText,
    proxyInput,
    quotaPoolSummary,
    reloadState,
    removeAccountModelMappingRow,
    REQUEST_LOG_PAGE_SIZE_OPTIONS,
    requestKindLabel,
    requestLogAccountQuery,
    requestLogApiKeyQuery,
    requestLogCurrentPage,
    requestLogError,
    requestLogErrorQuery,
    requestLogEvents,
    requestLogGatewayModeFilter,
    requestLogGatewayModeOptions,
    requestLogInstanceOptions,
    requestLogInstanceQuery,
    requestLogKindFilter,
    requestLogKindOptions,
    requestLogLoading,
    statsDetailsRefreshing,
    requestLogModelQuery,
    requestLogPageSize,
    requestLogRangeEnd,
    requestLogRangeStart,
    requestLogStatusFilter,
    requestLogStatusOptions,
    requestLogTotal,
    requestLogTotalPages,
    resetPricingDraft,
    resolveClientInstanceLabel,
    responsesWebsocketsEnabledDraft,
    routingOptions,
    routingStrategy,
    selectedModelId,
    selectedStatsRangeTitle,
    selectedStatsWindow,
    selectedTimeoutPresetId,
    selectedTimeoutPresetIsCustom,
    serviceTabs,
    sessionAffinityDraft,
    sessionAffinityTtlDraft,
    setAccountModelRuleBulkText,
    setAccountModelRuleDrafts,
    setAccountModelRuleSelected,
    setActiveTab,
    setAddressKind,
    setApiKeyDrafts,
    setApiKeyPolicyDrafts,
    setDisableCoolingDraft,
    setError,
    setExcludedModelsText,
    setHealthModalOpen,
    setImmediateSseResponseDraft,
    setKeyVisible,
    setMaxConcurrentImageRequestsDraft,
    setMaxRetryCredentialsDraft,
    setMaxRetryIntervalDraft,
    setMemberModalOpen,
    setModelAliasesText,
    setNotice,
    setPortInput,
    setPricingModalOpen,
    setProxyInput,
    setRequestLogAccountQuery,
    setRequestLogApiKeyQuery,
    setRequestLogErrorQuery,
    setRequestLogGatewayModeFilter,
    setRequestLogInstanceQuery,
    setRequestLogKindFilter,
    setRequestLogModelQuery,
    setRequestLogPage,
    setRequestLogPageSize,
    setRequestLogStatusFilter,
    setResponsesWebsocketsEnabledDraft,
    setSelectedModelId,
    setSelectedTimeoutPresetId,
    setSessionAffinityDraft,
    setSessionAffinityTtlDraft,
    setState,
    setStatsLogTab,
    setTestChatInput,
    setTimeoutDrafts,
    setTimeoutPresetNameDraft,
    setTimeoutsError,
    setTimeoutsModalOpen,
    sidecarRestarting,
    state,
    stats,
    statsLogTab,
    statsLogTabs,
    statsRange,
    statsRangeError,
    statsTimeRange,
    summaryCards,
    switchOptions,
    t,
    testChatInput,
    testChatMessages,
    testChatScrollRef,
    testDialogError,
    testDialogOpen,
    testDialogRunning,
    timeoutDrafts,
    timeoutDraftsFromValue,
    timeoutPresetNameDraft,
    timeoutPresetOptions,
    timeoutsError,
    timeoutsModalOpen,
    toggleApiKeyPolicyExpanded,
    toggleStringSelection,
    truncateRequestLogErrorDetail,
    updateAccountModelMappingDraft,
    updatePricingDraft,
    updateTimeoutDraft,
  } = props;
  return (
    <div className="codex-api-service-page">
      <div className="page-top-strip">
        <div className="page-top-strip-left">
          <span className="page-top-strip-label">
            {t("settings.general.account", "Accounts")}
          </span>
          <ManualHelpIconButton className="platform-header-help" />
        </div>
        <div className="page-top-strip-right-placeholder" aria-hidden="true" />
      </div>

      <div className="page-tabs-row page-tabs-center page-tabs-row-with-leading">
        <div className="page-tabs-leading">
          <PlatformGroupSwitcher
            currentPlatformId={currentPlatformId}
            currentLabel={
              currentGroup
                ? resolveGroupChildName(
                    currentGroup,
                    currentPlatformId,
                    getPlatformLabel(currentPlatformId, t),
                  )
                : getPlatformLabel(currentPlatformId, t)
            }
            options={switchOptions}
            currentGroupId={currentGroup?.id ?? null}
          />
        </div>
        <div className="page-tabs filter-tabs">
          {serviceTabs.map((tab) => (
            <button
              key={tab.key}
              className={`filter-tab${activeTab === tab.key ? " active" : ""}`}
              onClick={() => setActiveTab(tab.key)}
            >
              {tab.icon}
              <span>{tab.label}</span>
            </button>
          ))}
        </div>
      </div>

      <main className="codex-api-service-content">
        <section className="codex-api-service-hero">
          <div className="codex-api-service-hero-main">
            <div className="codex-api-service-title-row">
              <span className="codex-api-service-title-icon" aria-hidden="true">
                <CodexIcon size={24} />
              </span>
              <div className="codex-api-service-title-copy">
                <div className="codex-api-service-title-line">
                  <h1>{t("codex.apiService.title", "Codex API 服务")}</h1>
                  {apiServiceIsCurrent && (
                    <span className="codex-api-service-current-tag">
                      {t("codex.current", "当前")}
                    </span>
                  )}
                  <span
                    className={`codex-api-service-status ${state?.running ? "running" : collection?.enabled ? "stopped" : "disabled"}`}
                  >
                    {collection?.enabled
                      ? state?.preparing
                        ? t("instances.status.starting", "启动中")
                        : state?.running
                        ? t("codex.localAccess.statusRunning", "运行中")
                        : t("codex.localAccess.statusStopped", "未运行")
                      : t("codex.localAccess.statusDisabled", "已停用")}
                  </span>
                  {state?.preparing && state.preparationTotal > 0 && (
                    <span className="codex-api-service-current-tag">
                      {t("common.loading", "加载中...")} {state.preparationCompleted}/
                      {state.preparationTotal}
                    </span>
                  )}
                  {state?.refreshingAccounts && state.accountRefreshTotal > 0 && (
                    <span className="codex-api-service-current-tag">
                      {t("common.loading", "加载中...")} {state.accountRefreshCompleted}/
                      {state.accountRefreshTotal}
                    </span>
                  )}
                  <span className="codex-api-service-current-tag">
                    {t("codex.localAccess.title", "API 服务")}
                  </span>
                </div>
              </div>
            </div>
          </div>
          <div className="codex-api-service-hero-actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void reloadState()}
              disabled={busy || activating || testDialogRunning || sidecarRestarting}
            >
              <RefreshCw size={14} />
              {t("codex.localAccess.refreshStats", "刷新统计")}
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void handleRestartSidecar()}
              disabled={
                !collection ||
                busy ||
                activating ||
                testDialogRunning ||
                sidecarRestarting
              }
              title={t("codex.localAccess.restartAction", "重启 Sidecar")}
            >
              <RefreshCw
                size={14}
                className={sidecarRestarting ? "loading-spinner" : ""}
              />
              {t("codex.localAccess.restartAction", "重启 Sidecar")}
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={handleOpenTestDialog}
              disabled={
                !collection ||
                busy ||
                activating ||
                state?.preparing ||
                testDialogRunning
              }
            >
              <ShieldCheck
                size={14}
                className={testDialogRunning ? "loading-spinner" : ""}
              />
              {t("codex.localAccess.testAction", "测试")}
            </button>
            <button
              type="button"
              className={`btn ${apiServiceIsCurrent ? "btn-secondary" : "btn-primary"}`}
              onClick={() => void handleActivateService()}
              disabled={
                !collection ||
                busy ||
                activating ||
                state?.preparing ||
                testDialogRunning
              }
              title={t("codex.localAccess.activateAction", "启动 API 服务")}
            >
              {activating ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <Play size={14} />
              )}
              {t("codex.localAccess.activateAction", "启动 API 服务")}
            </button>
            <button
              type="button"
              className={`btn ${collection?.enabled ? "btn-danger" : "btn-secondary"}`}
              onClick={() => void handleToggleEnabled()}
              disabled={!collection || busy || activating || testDialogRunning}
            >
              <Power size={14} />
              {collection?.enabled
                ? t("codex.localAccess.disableService", "停用服务")
                : t("codex.localAccess.enableService", "启用服务")}
            </button>
          </div>
        </section>

        {(error || notice || state?.lastError || pricingRepriceActive) && (
          <div className="codex-api-service-message-stack">
            {error && (
              <div className="codex-api-service-message error">
                <CircleAlert size={15} />
                <span>{error}</span>
                <button
                  type="button"
                  className="codex-api-service-message-dismiss"
                  onClick={() => setError("")}
                  aria-label={t("common.close", "关闭")}
                  title={t("common.close", "关闭")}
                >
                  <X size={14} />
                </button>
              </div>
            )}
            {state?.lastError && (
              <div className="codex-api-service-message error">
                <CircleAlert size={15} />
                <span>{state.lastError}</span>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void handleKillPort()}
                  disabled={portKilling || busy || activating}
                >
                  <Wrench size={13} />
                  {t("codex.localAccess.killPortAction", "清理端口")}
                </button>
              </div>
            )}
            {notice && (
              <div className="codex-api-service-message success">
                <Check size={15} />
                <span>{notice}</span>
                <button
                  type="button"
                  className="codex-api-service-message-dismiss"
                  onClick={() => setNotice("")}
                  aria-label={t("common.close", "关闭")}
                  title={t("common.close", "关闭")}
                >
                  <X size={14} />
                </button>
              </div>
            )}
            {pricingRepriceActive && (
              <div className="codex-api-service-message codex-api-service-reprice-message">
                <RefreshCw size={15} />
                <div className="codex-api-service-reprice-status">
                  <span>{pricingRepriceStatusText}</span>
                  <div className="codex-api-service-reprice-track">
                    <div
                      className="codex-api-service-reprice-fill"
                      style={{ width: `${pricingRepricePercent}%` }}
                    />
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        <section className="codex-api-service-usage-toolbar">
          <div className="codex-api-service-usage-context">
            <Activity size={16} />
            <div>
              <strong>
                {t("codex.apiService.usage.title", "Usage Stats")}
              </strong>
              <span>
                {selectedStatsRangeTitle}
                {stats?.updatedAt
                  ? ` · ${t("codex.apiService.usage.lastRecorded", "Last recorded")} ${formatDateTime(stats.updatedAt)}`
                  : ""}
              </span>
            </div>
          </div>
          <CodexStatsRangePicker
            value={statsRange}
            range={statsTimeRange}
            onPresetChange={handleStatsPresetChange}
            onCustomApply={handleCustomStatsRangeApply}
            disabled={busy}
            error={statsRangeError}
            compact
          />
        </section>

        <section className="codex-api-service-summary-grid">
          {summaryCards.map((item) => (
            <div key={item.key} className="codex-api-service-summary-card">
              <span>{item.label}</span>
              <strong>{item.value}</strong>
              <small>{item.detail}</small>
            </div>
          ))}
        </section>

        <CodexUsageTrend statsWindow={selectedStatsWindow} />

        {activeTab === "overview" && (
          <div className="codex-api-service-grid two">
            <section className="codex-api-service-panel">
              <div className="codex-api-service-panel-head">
                <h2>{t("codex.localAccess.configTitle", "服务配置")}</h2>
              </div>
              <div className="codex-api-service-config-list">
                <label>
                  <span>Base URL</span>
                  <div className="codex-api-service-copy-row">
                    <code>{displayBaseUrl}</code>
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() => void handleCopy("baseUrl", displayBaseUrl)}
                      disabled={!displayBaseUrl}
                    >
                      {copiedField === "baseUrl" ? (
                        <Check size={14} />
                      ) : (
                        <Copy size={14} />
                      )}
                    </button>
                  </div>
                </label>
                <label>
                  <span>
                    {t(
                      "codex.localAccess.clientBaseUrlHostLabel",
                      "客户端地址",
                    )}
                  </span>
                  <div className="codex-api-service-input-row codex-api-service-stacked-control">
                    <SingleSelectDropdown
                      value={clientBaseUrlHost}
                      options={clientBaseUrlHostOptions}
                      onChange={(value) =>
                        void handleUpdateClientBaseUrlHost(value)
                      }
                      className="codex-api-service-client-host-select"
                      menuClassName="codex-api-service-client-host-menu"
                      disabled={busy || !collection}
                      ariaLabel={t(
                        "codex.localAccess.clientBaseUrlHostLabel",
                        "客户端地址",
                      )}
                    />
                    <small className="codex-api-service-field-hint">
                      {t(
                        "codex.localAccess.clientBaseUrlHostDesc",
                        "仅影响写入 Codex Provider 和复制给客户端的 Base URL，不改变服务监听地址。",
                      )}
                    </small>
                  </div>
                </label>
                <label>
                  <span>{t("codex.localAccess.apiKey", "密钥")}</span>
                  <div className="codex-api-service-copy-row">
                    <code title={collection?.apiKey || "-"}>
                      {collection
                        ? keyVisible
                          ? collection.apiKey
                          : `${collection.apiKey.slice(0, 10)}••••••••••••`
                        : "-"}
                    </code>
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() => setKeyVisible((current) => !current)}
                      disabled={!collection}
                    >
                      {keyVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() =>
                        void handleCopy("apiKey", collection?.apiKey || "")
                      }
                      disabled={!collection}
                    >
                      {copiedField === "apiKey" ? (
                        <Check size={14} />
                      ) : (
                        <Copy size={14} />
                      )}
                    </button>
                  </div>
                </label>
                <label>
                  <span>{t("codex.localAccess.portLabel", "服务端口")}</span>
                  <div className="codex-api-service-input-row">
                    <input
                      type="number"
                      min={1}
                      max={65535}
                      value={portInput}
                      onChange={(event) => setPortInput(event.target.value)}
                      disabled={busy}
                    />
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => void handleSavePort()}
                      disabled={busy}
                    >
                      {t("codex.localAccess.portSave", "保存端口")}
                    </button>
                  </div>
                </label>
                <label>
                  <span>
                    {t("codex.localAccess.upstreamProxyLabel", "API 代理地址")}
                  </span>
                  <div className="codex-api-service-input-row codex-api-service-proxy-input-row">
                    <input
                      type="text"
                      value={proxyInput}
                      onChange={(event) => setProxyInput(event.target.value)}
                      placeholder={t(
                        "codex.localAccess.upstreamProxyUrlPlaceholderSidecar",
                        "留空用全局代理",
                      )}
                      disabled={busy}
                    />
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => void handleSaveProxy()}
                      disabled={busy}
                    >
                      {t(
                        "codex.localAccess.upstreamProxySaveAction",
                        "保存代理",
                      )}
                    </button>
                  </div>
                </label>
                <label>
                  <span>
                    {t("codex.apiService.timeouts.entryLabel", "高级参数")}
                  </span>
                  <div className="codex-api-service-input-row">
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => {
                        setTimeoutsError("");
                        setSelectedTimeoutPresetId(
                          collection?.activeTimeoutPresetId || "long_wait",
                        );
                        setTimeoutPresetNameDraft("");
                        setTimeoutDrafts(
                          timeoutDraftsFromValue(collection?.timeouts),
                        );
                        setTimeoutsModalOpen(true);
                      }}
                      disabled={!collection}
                    >
                      <SlidersHorizontal size={14} />
                      {t("codex.apiService.timeouts.openAction", "超时与重试")}
                    </button>
                  </div>
                </label>
              </div>
            </section>

            <section className="codex-api-service-panel">
              <div className="codex-api-service-panel-head">
                <h2>{t("codex.apiService.healthTitle", "服务健康")}</h2>
              </div>
              <div className="codex-api-service-health-grid">
                <button
                  type="button"
                  className="codex-api-service-health-action"
                  onClick={() => setHealthModalOpen(true)}
                  aria-label={t(
                    "codex.localAccess.accountPoolHealth.openDetails",
                    "查看异常账号详情",
                  )}
                >
                  <span>
                    {t("codex.apiService.health.availableAccounts", "可用账号")}
                  </span>
                  <strong>
                    {availableAccountCount}/{memberAccounts.length}
                  </strong>
                </button>
                <div>
                  <span>{t("codex.apiService.health.cooldowns", "冷却")}</span>
                  <strong>{cooldownCount}</strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.apiService.health.imageUnavailable",
                      "图片不可用",
                    )}
                  </span>
                  <strong>{imageUnavailableCount}</strong>
                </div>
                <div>
                  <span>{t("codex.apiService.health.keys", "客户端 Key")}</span>
                  <strong>{collection?.apiKeys.length ?? 0}</strong>
                </div>
              </div>
              <div className="codex-api-service-quota-strip">
                {quotaPoolSummary.visiblePlans.length === 0 ? (
                  <span>
                    {t("codex.localAccess.emptyMembers", "当前集合暂无账号")}
                  </span>
                ) : (
                  quotaPoolSummary.visiblePlans.map((item) => (
                    <span key={item.key}>
                      {item.key} ({item.count})
                      {item.windows.length > 0
                        ? ` · ${item.windows
                            .map(
                              (window) =>
                                `${formatCodexQuotaPoolWindowLabel(
                                  window.label,
                                  t("codex.localAccess.quotaPool.weeklyShort", "周"),
                                )} ${formatCodexQuotaPoolPercent(window.percentage)}`,
                            )
                            .join(" · ")}`
                        : ""}
                    </span>
                  ))
                )}
              </div>
            </section>

            <section className="codex-api-service-panel codex-api-service-compat-panel">
              <div className="codex-api-service-panel-head">
                <div>
                  <h2>
                    {t(
                      "codex.apiService.compat.title",
                      "协议兼容",
                    )}
                  </h2>
                  <p className="codex-api-service-panel-desc">
                    {t(
                      "codex.apiService.compat.desc",
                      "同一个 API 服务地址支持 OpenAI Chat、Responses、Anthropic Messages、Gemini 和 Ollama 入口。",
                    )}
                  </p>
                </div>
              </div>
              <div className="codex-api-service-compat-grid">
                {compatibilityExamples.map((item) => {
                  const copyField: CopyField = `compat:${item.id}`;
                  return (
                    <div key={item.id} className="codex-api-service-compat-item">
                      <div className="codex-api-service-compat-item-head">
                        <div>
                          <strong>{item.title}</strong>
                          <span>{item.endpoint}</span>
                        </div>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() => void handleCopy(copyField, item.value)}
                          disabled={!displayBaseUrl}
                          title={t("common.copy", "复制")}
                        >
                          {copiedField === copyField ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </div>
                      <code>{item.value}</code>
                      <small>{item.note}</small>
                    </div>
                  );
                })}
              </div>
              <div className="codex-api-service-compat-models">
                <span>
                  {t(
                    "codex.apiService.compat.modelCatalogLabel",
                    "模型目录",
                  )}
                </span>
                <code>/v1/models · /v1beta/models · /api/tags</code>
              </div>
            </section>
          </div>
        )}

        {activeTab === "keys" && (
          <section className="codex-api-service-panel">
            <div className="codex-api-service-panel-head">
              <h2>{t("codex.localAccess.apiKeysTitle", "客户端 Key")}</h2>
                <div className="codex-api-service-head-actions">
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={() => void handleCreateApiKey()}
                  disabled={busy || !collection}
                >
                  <Plus size={14} />
                  {t("codex.localAccess.apiKeyAdd", "新增 Key")}
                </button>
              </div>
            </div>
            <div className="codex-api-service-table">
              {(collection?.apiKeys ?? []).map((apiKey) => {
                const labelDraft = apiKeyDrafts[apiKey.id] ?? apiKey.label;
                const policyDraft =
                  apiKeyPolicyDrafts[apiKey.id] ??
                  apiKeyPolicyDraftFromValue(apiKey);
                const persistedInheritAccountPool =
                  apiKeyInheritsAccountPool(apiKey);
                const persistedAccountIds = apiKey.accountIds ?? [];
                const accountScopeLocked = apiKeyHasFixedAccountScope(
                  apiKey,
                  collection,
                );
                const keySelectableAccounts = selectCodexApiKeyScopeAccounts({
                  accounts: localAccessAccounts,
                  restrictFreeAccounts: collection?.restrictFreeAccounts ?? true,
                  scopedAccountIds: apiKey.accountIds ?? [],
                });
                const keySelectableAccountIds = keySelectableAccounts.map(
                  (account) => account.id,
                );
                const keySelectableAccountIdSet = new Set(
                  keySelectableAccounts.map((account) => account.id),
                );
                const policyDirty = apiKeyPolicyDraftIsDirty(
                  apiKey,
                  policyDraft,
                );
                const customScopeInvalid =
                  !policyDraft.inheritAccountPool &&
                  policyDraft.accountIds.length === 0;
                const keyStats = apiKeyStatsById.get(apiKey.id);
                const keyUsage = keyStats?.usage;
                const tokenLimit = apiKey.tokenLimit ?? 0;
                const tokenUsed = apiKey.tokenUsed ?? 0;
                const tokenLimitReached =
                  tokenLimit > 0 && tokenUsed >= tokenLimit;
                const tokenLimitProgress =
                  tokenLimit > 0
                    ? Math.min(100, Math.max(0, (tokenUsed / tokenLimit) * 100))
                    : 0;
                const keySuccessRate =
                  keyUsage && keyUsage.requestCount > 0
                    ? Math.round(
                        (keyUsage.successCount / keyUsage.requestCount) * 100,
                      )
                    : 0;
                const policyExpanded = expandedApiKeyPolicyIds.has(apiKey.id);
                return (
                  <div key={apiKey.id} className="codex-api-service-key-card">
                    <div className="codex-api-service-key-main">
                      <input
                        value={labelDraft}
                        onChange={(event) =>
                          setApiKeyDrafts((drafts) => ({
                            ...drafts,
                            [apiKey.id]: event.target.value,
                          }))
                        }
                        onBlur={() =>
                          void handleSaveApiKeyLabel(apiKey.id, apiKey.label)
                        }
                        disabled={busy}
                        aria-label={t(
                          "codex.localAccess.apiKeyLabel",
                          "Key 名称",
                        )}
                      />
                      <code title={apiKey.key}>
                        {keyVisible
                          ? apiKey.key
                          : `${apiKey.key.slice(0, 10)}••••••••••••`}
                      </code>
                      <span
                        className={`codex-api-service-pill ${apiKey.enabled ? "success" : "muted"}`}
                      >
                        {apiKey.enabled
                          ? t("common.enabled", "Enabled")
                          : t("common.disabled", "Disabled")}
                      </span>
                      <span className="codex-api-service-key-last-used">
                        <small>
                          {t("codex.apiService.keys.lastUsed", "Last used")}
                        </small>
                        <strong>{formatDateTime(apiKey.lastUsedAt)}</strong>
                      </span>
                      <div className="codex-api-service-row-actions">
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            void handleCopy(`apiKey:${apiKey.id}`, apiKey.key)
                          }
                          title={t("common.copy", "Copy")}
                        >
                          {copiedField === `apiKey:${apiKey.id}` ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            void handleToggleApiKey(apiKey.id, !apiKey.enabled)
                          }
                          disabled={busy}
                          title={
                            apiKey.enabled
                              ? t("common.disable", "Disable")
                              : t("common.enable", "Enable")
                          }
                        >
                          <Power size={14} />
                        </button>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() => void handleRotateApiKey(apiKey.id)}
                          disabled={busy}
                          title={t(
                            "codex.localAccess.apiKeyRotate",
                            "Rotate Key",
                          )}
                        >
                          <RefreshCw size={14} />
                        </button>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() => void handleDeleteApiKey(apiKey.id)}
                          disabled={
                            busy || (collection?.apiKeys.length ?? 0) <= 1
                          }
                          title={t("common.delete", "Delete")}
                        >
                          <Trash2 size={14} />
                        </button>
                      </div>
                    </div>
                    <div className="api-key-details-row">
                      <div
                        className={`api-key-routing-summary ${
                          !persistedInheritAccountPool &&
                          persistedAccountIds.length === 0
                            ? "warning"
                            : ""
                        }`}
                      >
                        <Route size={16} />
                        <div>
                          <span>
                            {t("codex.apiService.keys.routingAccounts", "Routing Accounts")}
                          </span>
                          <strong>
                            {persistedInheritAccountPool
                              ? t(
                                  "codex.apiService.keys.accountScopeInheritedCount",
                                  "Account pool: inheriting {{count}}",
                                  { count: memberIds.length },
                                )
                              : persistedAccountIds.length === 0
                                ? t(
                                    "codex.apiService.keys.accountScopeUnavailable",
                                    "Account pool: no available accounts",
                                  )
                                : t(
                                    "codex.apiService.keys.accountScopeCount",
                                    "Account pool: {{selected}}/{{total}}",
                                    {
                                      selected: persistedAccountIds.length,
                                      total: keySelectableAccountIds.length,
                                    },
                                  )}
                          </strong>
                        </div>
                      </div>
                      <div
                        className={`api-key-token-limit-summary${
                          tokenLimitReached ? " limit-reached" : ""
                        }`}
                      >
                        <Gauge size={16} />
                        <div>
                          <span>
                            {t(
                              "codex.apiService.keys.tokenLimitUsage",
                              "Token limit",
                            )}
                          </span>
                          <strong>
                            {tokenLimit > 0
                              ? `${formatCompactNumber(tokenUsed)} / ${formatCompactNumber(tokenLimit)}`
                              : t(
                                  "codex.apiService.keys.tokenLimitUnlimited",
                                  "Unlimited",
                                )}
                          </strong>
                          {tokenLimit > 0 && (
                            <span
                              className="api-key-token-limit-progress"
                              aria-label={`${Math.round(tokenLimitProgress)}%`}
                            >
                              <span
                                style={{ width: `${tokenLimitProgress}%` }}
                              />
                            </span>
                          )}
                        </div>
                      </div>
                      <div
                        key={`${statsRange}:${statsTimeRange.startAt}:${statsTimeRange.endAt}`}
                        className="api-key-usage-grid"
                        aria-live="polite"
                        aria-label={`${selectedStatsRangeTitle} Key Usage`}
                      >
                        <div className="api-key-usage-grid-head">
                          <Activity size={14} />
                          <span>{selectedStatsRangeTitle}</span>
                        </div>
                        <div>
                          <span>
                            {t("codex.localAccess.stats.requests", "Requests")}
                          </span>
                          <strong>
                            {formatCompactNumber(keyUsage?.requestCount ?? 0)}
                          </strong>
                        </div>
                        <div>
                          <span>Token</span>
                          <strong>{formatAccountTokenUsage(keyUsage)}</strong>
                        </div>
                        <div>
                          <span>
                            {t("codex.localAccess.stats.successRateLabel", "Success Rate")}
                          </span>
                          <strong>{keySuccessRate}%</strong>
                        </div>
                        <div>
                          <span>
                            {t("codex.localAccess.stats.estimatedCost", "Estimated Cost")}
                          </span>
                          <strong>
                            {formatUsdCost(keyUsage?.estimatedCostUsd ?? 0)}
                          </strong>
                        </div>
                      </div>
                    </div>
                    <button
                      type="button"
                      className="codex-api-service-key-advanced-toggle"
                      aria-expanded={policyExpanded}
                      onClick={() => toggleApiKeyPolicyExpanded(apiKey.id)}
                    >
                      <span className="codex-api-service-section-title">
                        <SlidersHorizontal size={14} />
                        <span>
                          {t(
                            "codex.apiService.keys.advancedPolicyTitle",
                            "账号池与模型策略",
                          )}
                        </span>
                      </span>
                      <span className="codex-api-service-key-advanced-state">
                        {policyDirty && (
                          <span className="api-key-policy-dirty">
                            {t(
                              "codex.apiService.keys.unsaved",
                              "未保存",
                            )}
                          </span>
                        )}
                        {policyExpanded
                          ? t("common.collapse", "收起")
                          : t("common.expand", "展开")}
                        <ChevronDown size={14} />
                      </span>
                    </button>
                    {policyExpanded && (
                      <div className="codex-api-service-key-policy">
                        <div className="api-key-token-limit-editor">
                          <label>
                            <span>
                              {t(
                                "codex.apiService.keys.tokenLimit",
                                "Total token limit",
                              )}
                            </span>
                            <input
                              type="text"
                              inputMode="decimal"
                              value={policyDraft.tokenLimit}
                              onChange={(event) =>
                                setApiKeyPolicyDrafts((drafts) => ({
                                  ...drafts,
                                  [apiKey.id]: {
                                    ...(drafts[apiKey.id] ?? policyDraft),
                                    tokenLimit: event.target.value,
                                  },
                                }))
                              }
                              placeholder={t(
                                "codex.apiService.keys.tokenLimitPlaceholder",
                                "Example: 10m",
                              )}
                              disabled={busy}
                            />
                          </label>
                          <div>
                            <strong>
                              {t(
                                "codex.apiService.keys.tokenLimitCurrentUsage",
                                "Used: {{used}} tokens",
                                { used: formatCompactNumber(tokenUsed) },
                              )}
                            </strong>
                            <small>
                              {t(
                                "codex.apiService.keys.tokenLimitHint",
                                "Leave blank for unlimited. Input and output tokens are combined across all models.",
                              )}
                            </small>
                          </div>
                        </div>
                        <div className="api-key-account-scope">
                          <div className="api-key-account-scope-header">
                            <span>
                              {t(
                                "codex.apiService.keys.accountScope",
                                "账号轮转范围",
                              )}
                            </span>
                            <span className="api-key-account-scope-hint">
                              {accountScopeLocked
                                ? t(
                                    "codex.apiService.keys.accountScopeFixed",
                                    "此 Key 已固定绑定账号，不能继承服务池或清空账号范围",
                                  )
                                : policyDraft.inheritAccountPool
                                  ? t(
                                      "codex.apiService.keys.accountScopeInherit",
                                      "随服务账号池自动更新",
                                    )
                                  : customScopeInvalid
                                    ? t(
                                        "codex.apiService.keys.accountScopeRequired",
                                        "自定义账号池至少需要选择 1 个账号",
                                      )
                                    : t(
                                        "codex.apiService.keys.accountScopeSelected",
                                        "已选择 {{count}} 个账号",
                                        { count: policyDraft.accountIds.length },
                                      )}
                            </span>
                          </div>
                          <div
                            className="api-key-account-scope-mode"
                            role="group"
                            aria-label={t(
                              "codex.apiService.keys.accountScope",
                              "账号轮转范围",
                            )}
                          >
                            <button
                              type="button"
                              className={
                                policyDraft.inheritAccountPool ? "active" : ""
                              }
                              aria-pressed={policyDraft.inheritAccountPool}
                              onClick={() =>
                                setApiKeyPolicyDrafts((drafts) => ({
                                  ...drafts,
                                  [apiKey.id]: {
                                    ...(drafts[apiKey.id] ?? policyDraft),
                                    inheritAccountPool: true,
                                  },
                                }))
                              }
                              disabled={busy || accountScopeLocked}
                              title={
                                accountScopeLocked
                                  ? t(
                                      "codex.apiService.keys.accountScopeFixed",
                                      "此 Key 已固定绑定账号，不能继承服务池或清空账号范围",
                                    )
                                  : undefined
                              }
                            >
                              {t(
                                "codex.apiService.keys.accountScopeModeInherit",
                                "继承服务池",
                              )}
                            </button>
                            <button
                              type="button"
                              className={
                                policyDraft.inheritAccountPool ? "" : "active"
                              }
                              aria-pressed={!policyDraft.inheritAccountPool}
                              onClick={() =>
                                setApiKeyPolicyDrafts((drafts) => {
                                  const currentDraft =
                                    drafts[apiKey.id] ?? policyDraft;
                                  const reusableAccountIds =
                                    currentDraft.accountIds.filter((accountId) =>
                                      keySelectableAccountIdSet.has(accountId),
                                    );
                                  return {
                                    ...drafts,
                                    [apiKey.id]: {
                                      ...currentDraft,
                                      inheritAccountPool: false,
                                      accountIds:
                                        reusableAccountIds.length > 0
                                          ? reusableAccountIds
                                          : memberAccountIds,
                                    },
                                  };
                                })
                              }
                              disabled={
                                busy ||
                                accountScopeLocked ||
                                keySelectableAccounts.length === 0
                              }
                            >
                              {t(
                                "codex.apiService.keys.accountScopeModeCustom",
                                "自定义账号池",
                              )}
                            </button>
                          </div>
                          {keySelectableAccounts.length === 0 ? (
                            <div className="codex-api-service-empty">
                              {t(
                                "codex.localAccess.emptyMembers",
                                "当前集合暂无账号",
                              )}
                            </div>
                          ) : (
                            <div className="api-key-account-scope-grid">
                              {keySelectableAccounts.map((account) => {
                                const presentation =
                                  buildCodexAccountPresentation(account, t);
                                const accountSelected =
                                  isCodexApiKeyScopeAccountActive({
                                    accountId: account.id,
                                    inheritAccountPool:
                                      policyDraft.inheritAccountPool,
                                    accountIds: policyDraft.accountIds,
                                    inheritedAccountIds: memberAccountIds,
                                  });
                                const canPinAccount =
                                  !policyDraft.inheritAccountPool &&
                                  !accountScopeLocked &&
                                  !policyDirty &&
                                  apiKey.accountIds?.includes(account.id);
                                const priorityRank = (
                                  apiKey.priorityAccountIds ?? []
                                ).indexOf(account.id);
                                const isPrioritizedAccount = priorityRank >= 0;
                                const isTopPriorityAccount = priorityRank === 0;
                                return (
                                  <div
                                    key={account.id}
                                    className={`api-key-account-scope-item${
                                      isPrioritizedAccount ? " is-preferred" : ""
                                    }`}
                                  >
                                    <label className="api-key-account-scope-selection">
                                      <input
                                        type="checkbox"
                                        checked={accountSelected}
                                        onChange={() =>
                                          setApiKeyPolicyDrafts((drafts) => {
                                            const currentDraft =
                                              drafts[apiKey.id] ?? policyDraft;
                                            const accountIds = toggleStringSelection(
                                              currentDraft.accountIds,
                                              account.id,
                                            );
                                            return {
                                              ...drafts,
                                              [apiKey.id]: {
                                                ...currentDraft,
                                                accountIds,
                                              },
                                            };
                                          })
                                        }
                                        disabled={
                                          busy ||
                                          policyDraft.inheritAccountPool ||
                                          accountScopeLocked
                                        }
                                      />
                                      <span>
                                        <strong title={presentation.displayName}>
                                          {maskAccountText(
                                            presentation.displayName,
                                          )}
                                        </strong>
                                        <small>{presentation.planLabel}</small>
                                      </span>
                                    </label>
                                    <button
                                      type="button"
                                      className="api-key-account-priority-btn"
                                      data-priority-rank={
                                        isPrioritizedAccount
                                          ? priorityRank + 1
                                          : undefined
                                      }
                                      aria-label={
                                        isTopPriorityAccount
                                          ? t(
                                              "codex.apiService.keys.accountPriorityClear",
                                              "取消置顶账号",
                                            )
                                          : isPrioritizedAccount
                                            ? t(
                                                "codex.apiService.keys.accountPriorityPromote",
                                                "提升为最高优先级",
                                              )
                                          : t(
                                              "codex.apiService.keys.accountPrioritySet",
                                              "置顶账号优先调用",
                                            )
                                      }
                                      aria-pressed={isPrioritizedAccount}
                                      title={
                                        isTopPriorityAccount
                                          ? t(
                                              "codex.apiService.keys.accountPriorityClear",
                                              "取消置顶账号",
                                            )
                                          : isPrioritizedAccount
                                            ? t(
                                                "codex.apiService.keys.accountPriorityPromote",
                                                "提升为最高优先级",
                                              )
                                          : t(
                                              "codex.apiService.keys.accountPrioritySet",
                                              "置顶账号优先调用",
                                            )
                                      }
                                      onClick={() =>
                                        void handleSetApiKeyAccountPriority(
                                          apiKey,
                                          policyDraft,
                                          account.id,
                                        )
                                      }
                                      disabled={busy || !canPinAccount}
                                    >
                                      {isTopPriorityAccount ? (
                                        <PinOff size={14} />
                                      ) : (
                                        <Pin size={14} />
                                      )}
                                    </button>
                                  </div>
                                );
                              })}
                            </div>
                          )}
                        </div>
                        <div className="codex-api-service-policy-grid">
                          <label>
                            <span>
                              {t(
                                "codex.apiService.keys.modelPrefix",
                                "模型前缀",
                              )}
                            </span>
                            <input
                              value={policyDraft.modelPrefix}
                              onChange={(event) =>
                                setApiKeyPolicyDrafts((drafts) => ({
                                  ...drafts,
                                  [apiKey.id]: {
                                    ...(drafts[apiKey.id] ?? policyDraft),
                                    modelPrefix: event.target.value,
                                  },
                                }))
                              }
                              placeholder={t(
                                "codex.apiService.keys.modelPrefixPlaceholder",
                                "例如 codex",
                              )}
                              disabled={busy}
                            />
                          </label>
                          <label>
                            <span>
                              {t(
                                "codex.apiService.keys.allowedModels",
                                "允许模型",
                              )}
                            </span>
                            <textarea
                              value={policyDraft.allowedModels}
                              onChange={(event) =>
                                setApiKeyPolicyDrafts((drafts) => ({
                                  ...drafts,
                                  [apiKey.id]: {
                                    ...(drafts[apiKey.id] ?? policyDraft),
                                    allowedModels: event.target.value,
                                  },
                                }))
                              }
                              placeholder={t(
                                "codex.apiService.keys.allowedModelsPlaceholder",
                                "留空允许全部；每行一个模型或通配符",
                              )}
                              disabled={busy}
                            />
                          </label>
                          <label>
                            <span>
                              {t(
                                "codex.apiService.keys.excludedModels",
                                "排除模型",
                              )}
                            </span>
                            <textarea
                              value={policyDraft.excludedModels}
                              onChange={(event) =>
                                setApiKeyPolicyDrafts((drafts) => ({
                                  ...drafts,
                                  [apiKey.id]: {
                                    ...(drafts[apiKey.id] ?? policyDraft),
                                    excludedModels: event.target.value,
                                  },
                                }))
                              }
                              placeholder={t(
                                "codex.apiService.keys.excludedModelsPlaceholder",
                                "每行一个模型或通配符",
                              )}
                              disabled={busy}
                            />
                          </label>
                          <div className="codex-api-service-policy-actions">
                            <button
                              type="button"
                              className="btn btn-ghost btn-sm"
                              onClick={() => handleResetApiKeyPolicy(apiKey)}
                              disabled={busy || !policyDirty}
                            >
                              <Undo2 size={14} />
                              {t(
                                "codex.apiService.keys.resetPolicy",
                                "撤销修改",
                              )}
                            </button>
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              onClick={() =>
                                void handleSaveApiKeyPolicy(apiKey.id)
                              }
                              disabled={
                                busy || !policyDirty || customScopeInvalid
                              }
                            >
                              <Check size={14} />
                              {t(
                                "codex.apiService.keys.savePolicy",
                                "保存策略",
                              )}
                            </button>
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {activeTab === "accounts" && (
          <div className="codex-api-service-grid accounts">
            <section className="codex-api-service-panel">
              <div className="codex-api-service-panel-head">
                <h2>
                  {t("codex.localAccess.accountStatsTitle", "按账号统计")}
                </h2>
                <div className="codex-api-service-head-actions">
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={handleOpenAddAccount}
                    disabled={busy || activating || testDialogRunning}
                    title={t("common.shared.addAccount", "添加账号")}
                  >
                    <Plus size={14} />
                    {t("common.shared.addAccount", "添加账号")}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={handleOpenAccountModelMappings}
                    disabled={
                      busy || !collection || mappingMemberAccounts.length === 0
                    }
                  >
                    <Route size={14} />
                    {t(
                      "codex.apiService.accountModelMappings.action",
                      "模型映射",
                    )}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={handleOpenAccountModelRules}
                    disabled={busy || !collection || memberAccounts.length === 0}
                  >
                    <Wrench size={14} />
                    {t(
                      "codex.apiService.accountModelRules.action",
                      "禁用模型",
                    )}
                    {accountModelRuleCount > 0 ? ` ${accountModelRuleCount}` : ""}
                  </button>
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    onClick={() => setMemberModalOpen(true)}
                    disabled={busy || !collection}
                  >
                    <FolderPlus size={14} />
                    {t("codex.localAccess.modal.manageMembers", "管理成员")}
                  </button>
                </div>
              </div>
              <div className="codex-api-service-account-grid">
                {memberAccounts.length === 0 ? (
                  <div className="codex-api-service-empty codex-api-service-empty-with-action">
                    <span>
                      {t("codex.localAccess.emptyMembers", "当前集合暂无账号")}
                    </span>
                    <div className="codex-api-service-empty-actions">
                      <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        onClick={handleOpenAddAccount}
                        disabled={busy || activating || testDialogRunning}
                      >
                        <Plus size={14} />
                        {t("common.shared.addAccount", "添加账号")}
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => setMemberModalOpen(true)}
                        disabled={busy || !collection}
                      >
                        <FolderPlus size={14} />
                        {t("codex.localAccess.modal.manageMembers", "管理成员")}
                      </button>
                    </div>
                  </div>
                ) : (
                  memberAccounts.map((account) => {
                    const presentation = buildCodexAccountPresentation(
                      account,
                      t,
                    );
                    const health = healthByAccountId.get(account.id);
                    const stat = selectedStatsWindow?.accounts.find(
                      (item) => item.accountId === account.id,
                    );
                    const disabledModelCount =
                      parseModelRuleText(
                        accountModelRuleDrafts[account.id] ?? "",
                      ).length;
                    return (
                      <div
                        key={account.id}
                        className="codex-api-service-account-card"
                      >
                        <div>
                          <strong title={presentation.displayName}>
                            {maskAccountText(presentation.displayName)}
                          </strong>
                          <span
                            className={`tier-badge ${presentation.planClass}`}
                          >
                            {presentation.planLabel}
                          </span>
                        </div>
                        <div className="codex-api-service-account-meta">
                          <span>
                            {t("codex.localAccess.stats.accountRequests", {
                              count: stat?.usage.requestCount ?? 0,
                              defaultValue: "{{count}} 次",
                            })}
                          </span>
                          <span className="codex-api-service-account-meta-token">
                            {formatAccountTokenUsage(stat?.usage)}
                          </span>
                          <span>{formatRequestResultDetail(stat?.usage)}</span>
                          <span>
                            {formatUsdCost(stat?.usage.estimatedCostUsd ?? 0)}
                          </span>
                          <span>
                            {t("codex.apiService.accountHealth.failures", {
                              count: health?.consecutiveFailures ?? 0,
                              defaultValue: "连续失败 {{count}}",
                            })}
                          </span>
                          <span>
                            {health?.cooldowns.length
                              ? t("codex.localAccess.healthCooldown", {
                                  count: health.cooldowns.length,
                                  defaultValue: "冷却 {{count}}",
                                })
                              : health && !health.available
                                ? t(
                                    health.schedulerReason === "unauthorized"
                                      ? "codex.apiService.accountHealth.authError"
                                      : "codex.apiService.accountHealth.unavailable",
                                    health.schedulerReason === "unauthorized"
                                      ? "鉴权异常"
                                      : "暂不可用",
                                  )
                                : t("codex.localAccess.healthAvailable", "可用")}
                          </span>
                          <span>
                            {t("codex.apiService.accountHealth.image", {
                              status:
                                health?.imageGenerationStatus ?? "unknown",
                              defaultValue: "图片 {{status}}",
                            })}
                          </span>
                          {(account.api_model_mappings?.length ?? 0) > 0 && (
                            <span>
                              {t(
                                "codex.apiService.accountModelMappings.cardCount",
                                {
                                  count: account.api_model_mappings?.length ?? 0,
                                  defaultValue: "映射 {{count}}",
                                },
                              )}
                            </span>
                          )}
                          {disabledModelCount > 0 && (
                            <span>
                              {t(
                                "codex.apiService.accountModelRules.cardCount",
                                {
                                  count: disabledModelCount,
                                  defaultValue: "禁用 {{count}}",
                                },
                              )}
                            </span>
                          )}
                        </div>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() => void handleRemoveMember(account.id)}
                          disabled={busy}
                        >
                          <Trash2 size={14} />
                        </button>
                      </div>
                    );
                  })
                )}
              </div>
            </section>

            <section className="codex-api-service-panel">
              <div className="codex-api-service-panel-head">
                <h2 className="codex-api-service-title-with-icon">
                  <Route size={16} />
                  {t("codex.apiService.routing.optionsTitle", "调度选项")}
                </h2>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void handleSaveRoutingOptions()}
                  disabled={busy || !collection}
                >
                  <Check size={14} />
                  {t("codex.apiService.routing.saveOptions", "保存选项")}
                </button>
              </div>
              <div className="codex-api-service-config-list codex-api-service-routing-form">
                <label>
                  <span>{t("codex.localAccess.routingLabel", "调度策略")}</span>
                  <SingleSelectDropdown
                    value={routingStrategy}
                    options={routingOptions}
                    onChange={(value) => void handleUpdateRouting(value)}
                    disabled={busy || !collection}
                    ariaLabel={t("codex.localAccess.routingLabel", "调度策略")}
                  />
                </label>
                <label>
                  <span>
                    {t(
                      "codex.apiService.routing.sessionAffinity",
                      "会话亲和",
                    )}
                  </span>
                  <input
                    type="checkbox"
                    checked={sessionAffinityDraft}
                    onChange={(event) =>
                      setSessionAffinityDraft(event.target.checked)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t(
                      "codex.apiService.routing.sessionAffinityTtl",
                      "过期时间（秒）",
                    )}
                  </span>
                  <input
                    type="number"
                    min={60}
                    max={86400}
                    value={sessionAffinityTtlDraft}
                    onChange={(event) =>
                      setSessionAffinityTtlDraft(event.target.value)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t(
                      "codex.apiService.routing.responsesWebsockets",
                      "Responses WebSocket",
                    )}
                  </span>
                  <input
                    type="checkbox"
                    checked={responsesWebsocketsEnabledDraft}
                    onChange={(event) =>
                      setResponsesWebsocketsEnabledDraft(event.target.checked)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t(
                      "codex.apiService.routing.maxRetryCredentials",
                      "重试账号数",
                    )}
                  </span>
                  <input
                    type="number"
                    min={0}
                    max={8}
                    value={maxRetryCredentialsDraft}
                    onChange={(event) =>
                      setMaxRetryCredentialsDraft(event.target.value)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t("codex.apiService.routing.maxRetryInterval", "重试等待")}
                  </span>
                  <input
                    type="number"
                    min={0}
                    max={30}
                    value={maxRetryIntervalDraft}
                    onChange={(event) =>
                      setMaxRetryIntervalDraft(event.target.value)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t("codex.apiService.routing.disableCooling", "禁用冷却")}
                  </span>
                  <input
                    type="checkbox"
                    checked={disableCoolingDraft}
                    onChange={(event) =>
                      setDisableCoolingDraft(event.target.checked)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t(
                      "codex.apiService.routing.immediateSseResponse",
                      "SSE 立即返回 200",
                    )}
                  </span>
                  <input
                    type="checkbox"
                    checked={immediateSseResponseDraft}
                    onChange={(event) =>
                      setImmediateSseResponseDraft(event.target.checked)
                    }
                    disabled={busy || !collection}
                  />
                </label>
                <label>
                  <span>
                    {t(
                      "codex.apiService.routing.maxConcurrentImageRequests",
                      "Image requests per account",
                    )}
                  </span>
                  <input
                    type="number"
                    min={1}
                    max={16}
                    value={maxConcurrentImageRequestsDraft}
                    onChange={(event) =>
                      setMaxConcurrentImageRequestsDraft(event.target.value)
                    }
                    disabled={busy || !collection}
                  />
                </label>
              </div>
            </section>
          </div>
        )}

        {activeTab === "models" && (
          <div className="codex-api-service-grid two">
            <section className="codex-api-service-panel">
              <div className="codex-api-service-panel-head">
                <h2>
                  {t("codex.apiService.models.availableTitle", "可用模型")}
                </h2>
                <div className="codex-api-service-head-actions">
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={handleOpenPricingModal}
                    disabled={!collection}
                  >
                    <BadgeDollarSign size={14} />
                    {t("codex.apiService.models.pricingAction", "价格设置")}
                  </button>
                  <button
                    type="button"
                    className="folder-icon-btn"
                    onClick={() => void handleCopy("modelId", selectedModelId)}
                    disabled={!selectedModelId}
                  >
                    {copiedField === "modelId" ? (
                      <Check size={14} />
                    ) : (
                      <Copy size={14} />
                    )}
                  </button>
                </div>
              </div>
              <div className="codex-api-service-model-list">
                {modelIds.map((modelId) => (
                  <button
                    key={modelId}
                    type="button"
                    className={selectedModelId === modelId ? "active" : ""}
                    onClick={() => setSelectedModelId(modelId)}
                  >
                    <span>{modelId}</span>
                    {modelId === "gpt-image-2" && <Image size={14} />}
                  </button>
                ))}
              </div>
            </section>
            <div className="codex-api-service-panel-stack">
              <section className="codex-api-service-panel">
                <div className="codex-api-service-panel-head">
                  <h2>
                    {t("codex.apiService.models.capabilityTitle", "能力开关")}
                  </h2>
                </div>
                <div className="codex-api-service-config-list">
                  <label>
                    <span>
                      {t("codex.localAccess.accessScopeLabel", "访问范围")}
                    </span>
                    <SingleSelectDropdown
                      value={accessScope}
                      options={accessScopeOptions}
                      onChange={(value) => void handleUpdateAccessScope(value)}
                      disabled={busy || !collection}
                      ariaLabel={t(
                        "codex.localAccess.accessScopeLabel",
                        "访问范围",
                      )}
                    />
                  </label>
                  <p className="codex-api-service-muted">
                    {t(
                      "codex.apiService.models.capabilityDesc",
                      "gpt-image-2 会根据服务开关、账号套餐和已记录的图片能力状态自动暴露或隐藏。",
                    )}
                  </p>
                </div>
              </section>
              <section className="codex-api-service-panel">
                <div className="codex-api-service-panel-head">
                  <h2>{t("codex.apiService.models.rulesTitle", "模型规则")}</h2>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={() => void handleSaveModelRules()}
                    disabled={busy || !collection}
                  >
                    <Check size={14} />
                    {t("codex.apiService.models.saveRules", "保存规则")}
                  </button>
                </div>
                <div className="codex-api-service-policy-grid model-rules">
                  <label>
                    <span>
                      {t("codex.apiService.models.aliasTitle", "模型别名")}
                    </span>
                    <textarea
                      value={modelAliasesText}
                      onChange={(event) =>
                        setModelAliasesText(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.models.aliasPlaceholder",
                        "gpt-5 => g5；保留原模型加 +",
                      )}
                      disabled={busy || !collection}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.models.excludedTitle", "隐藏模型")}
                    </span>
                    <textarea
                      value={excludedModelsText}
                      onChange={(event) =>
                        setExcludedModelsText(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.models.excludedPlaceholder",
                        "每行一个模型或通配符，例如 gpt-5-*",
                      )}
                      disabled={busy || !collection}
                    />
                  </label>
                </div>
              </section>
            </div>
          </div>
        )}

        {activeTab === "logs" && (
          <section className="codex-api-service-panel">
            <div className="codex-api-service-panel-head codex-api-service-log-panel-head">
              <div
                className="codex-api-service-subtabs"
                role="tablist"
                aria-label={t("codex.apiService.tabs.logs", "统计与日志")}
              >
                {statsLogTabs.map((tab) => (
                  <button
                    key={tab.key}
                    type="button"
                    role="tab"
                    aria-selected={statsLogTab === tab.key}
                    className={statsLogTab === tab.key ? "active" : ""}
                    onClick={() => setStatsLogTab(tab.key)}
                  >
                    {tab.label}
                  </button>
                ))}
              </div>
              <div className="codex-api-service-head-actions">
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void handleRefreshStatsDetails()}
                  disabled={busy || statsDetailsRefreshing}
                  title={t("codex.localAccess.refreshStats", "刷新统计")}
                >
                  <RefreshCw
                    size={14}
                    className={statsDetailsRefreshing ? "loading-spinner" : ""}
                  />
                  {t("common.refresh", "刷新")}
                </button>
                <button
                  type="button"
                  className="btn btn-danger btn-sm"
                  onClick={() => void handleClearStats()}
                  disabled={busy}
                >
                  <Trash2 size={14} />
                  {t("codex.localAccess.clearStats", "清除统计")}
                </button>
              </div>
            </div>

            {statsLogTab === "accounts" && (
              <div className="codex-api-service-account-grid codex-api-service-stats-account-grid">
                {memberAccounts.length === 0 ? (
                  <div className="codex-api-service-empty">
                    {t("codex.localAccess.emptyMembers", "当前集合暂无账号")}
                  </div>
                ) : (
                  memberAccounts.map((account) => {
                    const presentation = buildCodexAccountPresentation(
                      account,
                      t,
                    );
                    const health = healthByAccountId.get(account.id);
                    const stat = selectedStatsWindow?.accounts.find(
                      (item) => item.accountId === account.id,
                    );
                    return (
                      <div
                        key={account.id}
                        className="codex-api-service-account-card"
                      >
                        <div>
                          <strong title={presentation.displayName}>
                            {maskAccountText(presentation.displayName)}
                          </strong>
                          <span
                            className={`tier-badge ${presentation.planClass}`}
                          >
                            {presentation.planLabel}
                          </span>
                        </div>
                        <div className="codex-api-service-account-meta">
                          <span>
                            {t("codex.localAccess.stats.accountRequests", {
                              count: stat?.usage.requestCount ?? 0,
                              defaultValue: "{{count}} 次",
                            })}
                          </span>
                          <span className="codex-api-service-account-meta-token">
                            {formatAccountTokenUsage(stat?.usage)}
                          </span>
                          <span>{formatRequestResultDetail(stat?.usage)}</span>
                          <span>
                            {formatUsdCost(stat?.usage.estimatedCostUsd ?? 0)}
                          </span>
                          <span>
                            {t("codex.apiService.accountHealth.failures", {
                              count: health?.consecutiveFailures ?? 0,
                              defaultValue: "连续失败 {{count}}",
                            })}
                          </span>
                          <span>
                            {health?.cooldowns.length
                              ? t("codex.localAccess.healthCooldown", {
                                  count: health.cooldowns.length,
                                  defaultValue: "冷却 {{count}}",
                                })
                              : health && !health.available
                                ? t(
                                    health.schedulerReason === "unauthorized"
                                      ? "codex.apiService.accountHealth.authError"
                                      : "codex.apiService.accountHealth.unavailable",
                                    health.schedulerReason === "unauthorized"
                                      ? "鉴权异常"
                                      : "暂不可用",
                                  )
                                : t("codex.localAccess.healthAvailable", "可用")}
                          </span>
                          <span>
                            {t("codex.apiService.accountHealth.image", {
                              status:
                                health?.imageGenerationStatus ?? "unknown",
                              defaultValue: "图片 {{status}}",
                            })}
                          </span>
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
            )}

            {statsLogTab === "models" && (
              <div className="codex-api-service-log-list">
                {(selectedStatsWindow?.models?.length ?? 0) === 0 ? (
                  <div className="codex-api-service-empty">
                    {t("codex.localAccess.statsEmpty", "当前还没有统计数据")}
                  </div>
                ) : (
                  selectedStatsWindow?.models.map((item) => (
                    <div
                      key={item.modelId}
                      className="codex-api-service-log-row codex-api-service-stat-row"
                    >
                      <div>
                        <strong>{item.modelId}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.localAccess.stats.accountRequests", {
                            count: item.usage.requestCount,
                            defaultValue: "{{count}} 次",
                          })}
                        </span>
                        <span>{formatRequestResultDetail(item.usage)}</span>
                        <span>
                          {formatCompactNumber(item.usage.totalTokens)} Tokens
                        </span>
                        <span>
                          {formatUsdCost(item.usage.estimatedCostUsd)}
                        </span>
                      </div>
                    </div>
                  ))
                )}
              </div>
            )}

            {statsLogTab === "keys" && (
              <div className="codex-api-service-log-list">
                {(selectedStatsWindow?.apiKeys?.length ?? 0) === 0 ? (
                  <div className="codex-api-service-empty">
                    {t("codex.localAccess.statsEmpty", "当前还没有统计数据")}
                  </div>
                ) : (
                  selectedStatsWindow?.apiKeys.map((item) => (
                    <div
                      key={item.apiKeyId}
                      className="codex-api-service-log-row codex-api-service-stat-row"
                    >
                      <div>
                        <strong title={item.label || item.apiKeyId}>
                          {item.label || item.apiKeyId}
                        </strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.localAccess.stats.accountRequests", {
                            count: item.usage.requestCount,
                            defaultValue: "{{count}} 次",
                          })}
                        </span>
                        <span>{formatRequestResultDetail(item.usage)}</span>
                        <span>
                          {formatCompactNumber(item.usage.totalTokens)} Tokens
                        </span>
                        <span>
                          {formatUsdCost(item.usage.estimatedCostUsd)}
                        </span>
                      </div>
                    </div>
                  ))
                )}
              </div>
            )}

            {statsLogTab === "logs" && (
              <>
                <div className="codex-api-service-log-filters">
                  <label>
                    <span>
                      {t("codex.apiService.logs.modelFilter", "模型")}
                    </span>
                    <input
                      value={requestLogModelQuery}
                      onChange={(event) =>
                        setRequestLogModelQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.modelPlaceholder",
                        "Model ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.accountFilter", "Account")}
                    </span>
                    <input
                      value={requestLogAccountQuery}
                      onChange={(event) =>
                        setRequestLogAccountQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.accountPlaceholder",
                        "Email or account ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.apiKeyFilter", "API Key")}
                    </span>
                    <input
                      value={requestLogApiKeyQuery}
                      onChange={(event) =>
                        setRequestLogApiKeyQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.apiKeyPlaceholder",
                        "Name or ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.instanceFilter", "Instance")}
                    </span>
                    <SingleSelectDropdown
                      value={requestLogInstanceQuery}
                      options={requestLogInstanceOptions}
                      onChange={setRequestLogInstanceQuery}
                      ariaLabel={t(
                        "codex.apiService.logs.instanceFilter",
                        "Instance",
                      )}
                      placeholder={t(
                        "codex.apiService.logs.allInstances",
                        "All Instances",
                      )}
                    />
                  </label>
                  <label>
                    <span>{t("codex.apiService.logs.kindFilter", "Type")}</span>
                    <SingleSelectDropdown
                      value={requestLogKindFilter}
                      options={requestLogKindOptions}
                      onChange={(value) =>
                        setRequestLogKindFilter(value as RequestLogKindFilter)
                      }
                      ariaLabel={t("codex.apiService.logs.kindFilter", "Type")}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.statusFilter", "Status")}
                    </span>
                    <SingleSelectDropdown
                      value={requestLogStatusFilter}
                      options={requestLogStatusOptions}
                      onChange={(value) =>
                        setRequestLogStatusFilter(
                          value as RequestLogStatusFilter,
                        )
                      }
                      ariaLabel={t(
                        "codex.apiService.logs.statusFilter",
                        "Status",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.gatewayModeFilter", "Mode")}
                    </span>
                    <SingleSelectDropdown
                      value={requestLogGatewayModeFilter}
                      options={requestLogGatewayModeOptions}
                      onChange={(value) =>
                        setRequestLogGatewayModeFilter(
                          value as RequestLogGatewayModeFilter,
                        )
                      }
                      ariaLabel={t(
                        "codex.apiService.logs.gatewayModeFilter",
                        "Mode",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.errorFilter", "Error")}
                    </span>
                    <input
                      value={requestLogErrorQuery}
                      onChange={(event) =>
                        setRequestLogErrorQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.errorPlaceholder",
                        "Error category",
                      )}
                    />
                  </label>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={clearRequestLogFilters}
                    disabled={!hasRequestLogFilters}
                  >
                    {t("codex.apiService.logs.clearFilters", "Clear Filters")}
                  </button>
                </div>
                <div className="codex-api-service-log-list">
                  {requestLogError && (
                    <div className="codex-api-service-message error">
                      <CircleAlert size={15} />
                      <span>{requestLogError}</span>
                    </div>
                  )}
                  {requestLogLoading && requestLogEvents.length === 0 && (
                    <div className="codex-api-service-empty">
                      {t("codex.apiService.logs.loading", "正在加载请求日志")}
                    </div>
                  )}
                  {requestLogEvents.map((event, index) => {
                    const fullErrorDetail = cleanRequestLogErrorDetail(
                      event.errorMessage,
                    );
                    const errorDetail =
                      truncateRequestLogErrorDetail(fullErrorDetail);
                    const accountDisplayName =
                      accountDisplayNames.get((event.accountId || "").trim()) ||
                      accountDisplayNames.get((event.email || "").trim()) ||
                      event.email ||
                      event.accountId ||
                      "-";
                    return (
                      <div
                        key={`${event.timestamp}-${event.requestId || event.apiKeyId}-${index}`}
                        className="codex-api-service-log-row"
                      >
                        <div>
                          <strong>{event.modelId || "--"}</strong>
                          <span
                            className={`codex-api-service-pill ${event.success ? "success" : "error"}`}
                          >
                            {event.success
                              ? t("codex.localAccess.requestLogSuccess", "成功")
                              : t("codex.localAccess.requestLogFailed", "失败")}
                          </span>
                          {event.reasoningEffort ? (
                            <span
                              className="codex-api-service-pill muted"
                              title={t(
                                "codex.apiService.logs.reasoningEffort",
                                "思考强度",
                              )}
                            >
                              {t("codex.apiService.logs.reasoningEffortValue", {
                                effort: event.reasoningEffort,
                                defaultValue: "思考 {{effort}}",
                              })}
                            </span>
                          ) : null}
                          {event.serviceTier ? (
                            <span
                              className="codex-api-service-pill muted"
                              title={t(
                                "codex.apiService.logs.serviceTier",
                                "服务等级",
                              )}
                            >
                              {t("codex.apiService.logs.serviceTierValue", {
                                tier: event.serviceTier,
                                defaultValue: "Tier {{tier}}",
                              })}
                            </span>
                          ) : null}
                          <span
                            className={`codex-api-service-pill ${
                              event.gatewayMode === "legacy"
                                ? "mode-legacy"
                                : event.gatewayMode === "sidecar"
                                  ? "mode-sidecar"
                                  : "muted"
                            }`}
                          >
                            {gatewayModeLabel(event.gatewayMode, t)}
                          </span>
                        </div>
                        <div>
                          <span>{formatDateTime(event.timestamp)}</span>
                          <span>{requestKindLabel(event.requestKind, t)}</span>
                          <span>
                            {event.apiKeyLabel || event.apiKeyId || "-"}
                          </span>
                          <span
                            title={
                              event.clientInstanceId
                                ? `${resolveClientInstanceLabel(
                                    event.clientInstanceId,
                                    codexInstances,
                                    t,
                                  )} (${event.clientInstanceId})`
                                : undefined
                            }
                          >
                            {resolveClientInstanceLabel(
                              event.clientInstanceId,
                              codexInstances,
                              t,
                            )}
                          </span>
                          <span>
                            {maskAccountText(accountDisplayName)}
                          </span>
                          <span>{formatLatencyMs(event.latencyMs)}</span>
                          <span>
                            {formatCompactNumber(event.totalTokens)} Tokens
                          </span>
                          <span>{formatUsdCost(event.estimatedCostUsd)}</span>
                          {event.requestId ? (
                            <span>
                              {t("codex.apiService.logs.requestIdShort", {
                                id: event.requestId,
                                defaultValue: "ID {{id}}",
                              })}
                            </span>
                          ) : null}
                          {event.httpStatus ? (
                            <span>
                              {t("codex.apiService.logs.httpStatus", {
                                status: event.httpStatus,
                                defaultValue: "HTTP {{status}}",
                              })}
                            </span>
                          ) : null}
                          {event.errorCategory ? (
                            <span>{event.errorCategory}</span>
                          ) : null}
                          {errorDetail ? (
                            <span
                              className="codex-api-service-log-error-detail"
                              title={fullErrorDetail}
                            >
                              {errorDetail}
                            </span>
                          ) : null}
                        </div>
                      </div>
                    );
                  })}
                  {!requestLogLoading &&
                    !requestLogError &&
                    requestLogEvents.length === 0 && (
                      <div className="codex-api-service-empty">
                        {t("codex.localAccess.requestLogEmpty", "暂无请求日志")}
                      </div>
                    )}
                </div>
                <PaginationControls
                  totalItems={requestLogTotal}
                  currentPage={requestLogCurrentPage}
                  totalPages={requestLogTotalPages}
                  pageSize={requestLogPageSize}
                  pageSizeOptions={REQUEST_LOG_PAGE_SIZE_OPTIONS}
                  rangeStart={requestLogRangeStart}
                  rangeEnd={requestLogRangeEnd}
                  canGoPrevious={requestLogCurrentPage > 1}
                  canGoNext={requestLogCurrentPage < requestLogTotalPages}
                  onPageSizeChange={(pageSize) => {
                    setRequestLogPageSize(
                      normalizeRequestLogPageSize(pageSize),
                    );
                    setRequestLogPage(1);
                  }}
                  onPreviousPage={() =>
                    setRequestLogPage((page) => Math.max(1, page - 1))
                  }
                  onNextPage={() =>
                    setRequestLogPage((page) =>
                      Math.min(requestLogTotalPages, page + 1),
                    )
                  }
                />
              </>
            )}
          </section>
        )}
      </main>

      {timeoutsModalOpen && (
        <div
          className="modal-overlay codex-api-service-pricing-overlay"
          role="presentation"
        >
          <div
            className="modal codex-api-service-timeouts-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-api-service-timeouts-title"
          >
            <div className="modal-header">
              <div>
                <h2 id="codex-api-service-timeouts-title">
                  {t("codex.apiService.timeouts.title", "超时与重试")}
                </h2>
                <p className="codex-api-service-pricing-desc">
                  {t(
                    "codex.apiService.timeouts.desc",
                    "单位为秒，保存后会按当前网关模式重启或重载 API 服务。",
                  )}
                </p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={() => setTimeoutsModalOpen(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body codex-api-service-timeouts-body">
              {timeoutsError && (
                <div className="codex-api-service-message error">
                  <CircleAlert size={15} />
                  <span>{timeoutsError}</span>
                </div>
              )}
              <section className="codex-api-service-timeout-section codex-api-service-timeout-preset-section">
                <h3>
                  {t("codex.apiService.timeouts.presetTitle", "参数方案")}
                </h3>
                <div className="codex-api-service-timeout-preset-row">
                  <label className="codex-api-service-timeout-preset-select">
                    <span>
                      {t("codex.apiService.timeouts.presetSelect", "选择方案")}
                    </span>
                    <select
                      value={selectedTimeoutPresetId}
                      onChange={(event) =>
                        applyTimeoutPreset(event.target.value)
                      }
                    >
                      {timeoutPresetOptions.map((preset) => (
                        <option key={preset.id} value={preset.id}>
                          {preset.name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="codex-api-service-timeout-preset-name">
                    <span>
                      {t("codex.apiService.timeouts.presetName", "方案名称")}
                    </span>
                    <input
                      type="text"
                      maxLength={40}
                      value={timeoutPresetNameDraft}
                      onChange={(event) => {
                        setTimeoutsError("");
                        setTimeoutPresetNameDraft(event.target.value);
                      }}
                      placeholder={t(
                        "codex.apiService.timeouts.presetNamePlaceholder",
                        "新的自定义方案",
                      )}
                    />
                  </label>
                </div>
                <div className="codex-api-service-timeout-preset-actions">
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={() => void handleCreateTimeoutPreset()}
                    disabled={!collection || busy}
                  >
                    <Plus size={14} />
                    {t("codex.apiService.timeouts.saveAsPreset", "另存为方案")}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={() => void handleUpdateTimeoutPreset()}
                    disabled={
                      !collection || !selectedTimeoutPresetIsCustom || busy
                    }
                  >
                    <Check size={14} />
                    {t(
                      "codex.apiService.timeouts.updatePreset",
                      "更新当前方案",
                    )}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm danger"
                    onClick={() => void handleDeleteTimeoutPreset()}
                    disabled={
                      !collection || !selectedTimeoutPresetIsCustom || busy
                    }
                  >
                    <Trash2 size={14} />
                    {t("codex.apiService.timeouts.deletePreset", "删除方案")}
                  </button>
                </div>
              </section>
              <section className="codex-api-service-timeout-section">
                <h3>
                  {t("codex.apiService.timeouts.sidecarTitle", "新 API 服务")}
                </h3>
                <div className="codex-api-service-policy-grid">
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.streamOpen", "流打开超时")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.sidecarStreamOpenTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarStreamOpenTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.streamIdle", "流空闲超时")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.sidecarStreamIdleTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarStreamIdleTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.imageOpen", "图片流打开")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.sidecarImageStreamOpenTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarImageStreamOpenTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.imageIdle", "图片流空闲")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.sidecarImageStreamIdleTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarImageStreamIdleTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.openAttempts", "打开尝试")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={3}
                      value={timeoutDrafts.sidecarStreamOpenMaxAttempts}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarStreamOpenMaxAttempts",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.keepalive", "Keep-alive")}
                    </span>
                    <input
                      type="number"
                      min={0}
                      max={300}
                      value={timeoutDrafts.sidecarStreamKeepaliveSeconds}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarStreamKeepaliveSeconds",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.bootstrapRetries",
                        "启动重试",
                      )}
                    </span>
                    <input
                      type="number"
                      min={0}
                      max={5}
                      value={timeoutDrafts.sidecarStreamingBootstrapRetries}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "sidecarStreamingBootstrapRetries",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                </div>
              </section>
              <section className="codex-api-service-timeout-section">
                <h3>
                  {t(
                    "codex.apiService.timeouts.retryTitle",
                    "发送与账号重试",
                  )}
                </h3>
                <div className="codex-api-service-policy-grid">
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.sendRetryAttempts",
                        "发送重试",
                      )}
                    </span>
                    <input
                      type="number"
                      min={0}
                      max={5}
                      value={timeoutDrafts.upstreamSendRetryAttempts}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "upstreamSendRetryAttempts",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.sendRetryBaseDelay",
                        "发送基础延迟(ms)",
                      )}
                    </span>
                    <input
                      type="number"
                      min={50}
                      max={10000}
                      value={timeoutDrafts.upstreamSendRetryBaseDelayMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "upstreamSendRetryBaseDelayMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.sendRetryMaxDelay",
                        "发送最大延迟(ms)",
                      )}
                    </span>
                    <input
                      type="number"
                      min={50}
                      max={10000}
                      value={timeoutDrafts.upstreamSendRetryMaxDelayMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "upstreamSendRetryMaxDelayMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.singleStatusAttempts",
                        "单账号重试",
                      )}
                    </span>
                    <input
                      type="number"
                      min={0}
                      max={5}
                      value={timeoutDrafts.singleAccountStatusRetryAttempts}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "singleAccountStatusRetryAttempts",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.singleStatusBaseDelay",
                        "单账号基础延迟(ms)",
                      )}
                    </span>
                    <input
                      type="number"
                      min={50}
                      max={10000}
                      value={timeoutDrafts.singleAccountStatusRetryBaseDelayMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "singleAccountStatusRetryBaseDelayMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.singleStatusMaxDelay",
                        "单账号最大延迟(ms)",
                      )}
                    </span>
                    <input
                      type="number"
                      min={50}
                      max={10000}
                      value={timeoutDrafts.singleAccountStatusRetryMaxDelayMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "singleAccountStatusRetryMaxDelayMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                </div>
              </section>
              <section className="codex-api-service-timeout-section">
                <h3>
                  {t(
                    "codex.apiService.timeouts.websocketTitle",
                    "WebSocket 设置",
                  )}
                </h3>
                <div className="codex-api-service-policy-grid">
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.websocketConnect",
                        "连接超时",
                      )}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.websocketConnectTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "websocketConnectTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.websocketInitial",
                        "首包超时",
                      )}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.websocketInitialMessageTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "websocketInitialMessageTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.websocketIdle", "空闲超时")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={1800}
                      value={timeoutDrafts.websocketIdleTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "websocketIdleTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t(
                        "codex.apiService.timeouts.websocketHeartbeat",
                        "心跳间隔",
                      )}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.websocketHeartbeatIntervalMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "websocketHeartbeatIntervalMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                </div>
              </section>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => void handleRepriceRequestLogs()}
                disabled={busy}
              >
                <RefreshCw size={15} />
                {t("codex.apiService.models.pricingReprice", "重算历史估值")}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={handleResetTimeoutDrafts}
              >
                {t("codex.apiService.timeouts.resetDefaults", "恢复默认")}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setTimeoutsModalOpen(false)}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void handleSaveTimeouts()}
                disabled={busy}
              >
                <Check size={15} />
                {t("common.save", "保存")}
              </button>
            </div>
          </div>
        </div>
      )}

      {accountModelRulesOpen && (
        <div
          className="modal-overlay codex-api-service-pricing-overlay"
          role="presentation"
        >
          <div
            className="modal codex-api-service-pricing-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-api-service-account-model-rules-title"
          >
            <div className="modal-header">
              <div>
                <h2 id="codex-api-service-account-model-rules-title">
                  {t(
                    "codex.apiService.accountModelRules.title",
                    "账号禁用模型",
                  )}
                </h2>
                <p className="codex-api-service-pricing-desc">
                  {t(
                    "codex.apiService.accountModelRules.desc",
                    "命中规则的账号不会参与该模型请求；每行一个模型或通配符。",
                  )}
                </p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={handleCloseAccountModelRules}
                aria-label={t("common.close", "关闭")}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body codex-api-service-pricing-body">
              <div className="codex-api-service-policy-actions">
                <label className="codex-api-service-account-model-bulk">
                  <span>
                    {t(
                      "codex.apiService.accountModelRules.bulkLabel",
                      "批量规则",
                    )}
                  </span>
                  <textarea
                    value={accountModelRuleBulkText}
                    onChange={(event) =>
                      setAccountModelRuleBulkText(event.target.value)
                    }
                    placeholder={t(
                      "codex.apiService.accountModelRules.placeholder",
                      "gpt-5.4-mini\ngpt-5.3-*",
                    )}
                  />
                </label>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={handleApplyAccountModelRuleBulk}
                  disabled={busy || accountModelRuleSelected.size === 0}
                >
                  {t(
                    "codex.apiService.accountModelRules.applySelected",
                    "应用到已选",
                  )}
                </button>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() =>
                    setAccountModelRuleSelected(
                      accountModelRuleAllSelected
                        ? new Set()
                        : new Set(memberAccounts.map((account) => account.id)),
                    )
                  }
                  disabled={memberAccounts.length === 0}
                >
                  {accountModelRuleAllSelected
                    ? t(
                        "codex.apiService.accountModelRules.clearSelection",
                        "清除选择",
                      )
                    : t(
                        "codex.apiService.accountModelRules.selectAll",
                        "全选账号",
                      )}
                </button>
              </div>
              <div className="codex-api-service-pricing-table">
                {memberAccounts.map((account) => {
                  const presentation = buildCodexAccountPresentation(
                    account,
                    t,
                  );
                  return (
                    <div
                      key={account.id}
                      className="codex-api-service-account-model-row"
                    >
                      <label className="codex-api-service-account-model-check">
                        <input
                          type="checkbox"
                          checked={accountModelRuleSelected.has(account.id)}
                          onChange={(event) => {
                            setAccountModelRuleSelected((selected) => {
                              const next = new Set(selected);
                              if (event.target.checked) {
                                next.add(account.id);
                              } else {
                                next.delete(account.id);
                              }
                              return next;
                            });
                          }}
                        />
                        <span>
                          <strong title={presentation.displayName}>
                            {maskAccountText(presentation.displayName)}
                          </strong>
                          <small>{presentation.planLabel}</small>
                        </span>
                      </label>
                      <textarea
                        value={accountModelRuleDrafts[account.id] ?? ""}
                        onChange={(event) =>
                          setAccountModelRuleDrafts((drafts) => ({
                            ...drafts,
                            [account.id]: event.target.value,
                          }))
                        }
                        placeholder={t(
                          "codex.apiService.accountModelRules.placeholder",
                          "gpt-5.4-mini\ngpt-5.3-*",
                        )}
                        disabled={busy}
                      />
                    </div>
                  );
                })}
              </div>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={handleCloseAccountModelRules}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void handleSaveAccountModelRules()}
                disabled={busy}
              >
                <Check size={15} />
                {t("common.save", "保存")}
              </button>
            </div>
          </div>
        </div>
      )}

      {accountModelMappingsOpen && (
        <div
          className="modal-overlay codex-api-service-pricing-overlay"
          role="presentation"
        >
          <div
            className="modal codex-api-service-pricing-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-api-service-account-model-mappings-title"
          >
            <div className="modal-header">
              <div>
                <h2 id="codex-api-service-account-model-mappings-title">
                  {t(
                    "codex.apiService.accountModelMappings.title",
                    "账号模型映射",
                  )}
                </h2>
                <p className="codex-api-service-pricing-desc">
                  {t(
                    "codex.apiService.accountModelMappings.desc",
                    "只改这个账号被抽中后发给上游的模型名。左侧是调用方请求的模型，右侧是实际上游模型。上下文窗口可选，官方 / DeepSeek 留空则保留厂商默认值。",
                  )}
                </p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={handleCloseAccountModelMappings}
                aria-label={t("common.close", "关闭")}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body codex-api-service-pricing-body">
              {accountModelMappingError && (
                <div className="codex-api-service-message error">
                  <CircleAlert size={15} />
                  <span>{accountModelMappingError}</span>
                </div>
              )}
              {mappingMemberAccounts.length === 0 ? (
                <div className="codex-api-service-empty">
                  {t(
                    "codex.apiService.accountModelMappings.empty",
                    "当前成员里没有可映射的 API Key 账号",
                  )}
                </div>
              ) : (
                mappingMemberAccounts.map((account) => {
                  const presentation = buildCodexAccountPresentation(account, t);
                  const rows =
                    accountModelMappingDrafts[account.id] ??
                    mappingDraftsFromAccount(account);
                  return (
                    <div
                      key={account.id}
                      className="codex-api-service-mapping-account"
                    >
                      <div className="codex-api-service-mapping-account-head">
                        <strong title={presentation.displayName}>
                          {maskAccountText(presentation.displayName)}
                        </strong>
                        <div className="codex-api-service-head-actions">
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={() =>
                              fillDeepSeekAccountModelMappings(account.id)
                            }
                            disabled={busy}
                          >
                            {t(
                              "codex.apiService.accountModelMappings.fillDeepSeek",
                              "填入 DeepSeek",
                            )}
                          </button>
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={() => addAccountModelMappingRow(account.id)}
                            disabled={busy}
                          >
                            <Plus size={14} />
                            {t(
                              "codex.apiService.accountModelMappings.addRow",
                              "添加一行",
                            )}
                          </button>
                        </div>
                      </div>
                      <div className="codex-api-service-mapping-rows">
                        {rows.map((row, index) => (
                          <div
                            key={`${account.id}-${index}`}
                            className="codex-api-service-mapping-row"
                          >
                            <label>
                              <span>
                                {t(
                                  "codex.apiService.accountModelMappings.clientModel",
                                  "请求模型",
                                )}
                              </span>
                              <input
                                value={row.clientModel}
                                onChange={(event) =>
                                  updateAccountModelMappingDraft(
                                    account.id,
                                    index,
                                    "clientModel",
                                    event.target.value,
                                  )
                                }
                                placeholder="gpt-5.6-sol"
                                disabled={busy}
                              />
                            </label>
                            <span className="codex-api-service-mapping-arrow">
                              →
                            </span>
                            <label>
                              <span>
                                {t(
                                  "codex.apiService.accountModelMappings.upstreamModel",
                                  "发送模型",
                                )}
                              </span>
                              <input
                                value={row.upstreamModel}
                                onChange={(event) =>
                                  updateAccountModelMappingDraft(
                                    account.id,
                                    index,
                                    "upstreamModel",
                                    event.target.value,
                                  )
                                }
                                placeholder="deepseek-v4-flash"
                                disabled={busy}
                              />
                            </label>
                            <label>
                              <span>
                                {t(
                                  "codex.api.modelCatalog.contextWindow",
                                  "上下文窗口",
                                )}
                              </span>
                              <input
                                value={row.contextWindow ?? ""}
                                onChange={(event) =>
                                  updateAccountModelMappingDraft(
                                    account.id,
                                    index,
                                    "contextWindow",
                                    event.target.value,
                                  )
                                }
                                placeholder={t(
                                  "codex.api.modelCatalog.contextWindowPlaceholder",
                                  "留空=默认",
                                )}
                                inputMode="numeric"
                                disabled={busy}
                              />
                            </label>
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() =>
                                removeAccountModelMappingRow(account.id, index)
                              }
                              disabled={busy}
                              aria-label={t("common.delete", "删除")}
                            >
                              <Trash2 size={14} />
                            </button>
                          </div>
                        ))}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={handleCloseAccountModelMappings}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void handleSaveAccountModelMappings()}
                disabled={busy || mappingMemberAccounts.length === 0}
              >
                <Check size={15} />
                {t("common.save", "保存")}
              </button>
            </div>
          </div>
        </div>
      )}

      {pricingModalOpen && (
        <div
          className="modal-overlay codex-api-service-pricing-overlay"
          role="presentation"
        >
          <div
            className="modal codex-api-service-pricing-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-api-service-pricing-title"
          >
            <div className="modal-header">
              <div>
                <h2 id="codex-api-service-pricing-title">
                  {t("codex.apiService.models.pricingTitle", "模型价格设置")}
                </h2>
                <p className="codex-api-service-pricing-desc">
                  {t(
                    "codex.apiService.models.pricingDesc",
                    "单位为 USD / 1M tokens，仅用于本地价值统计。",
                  )}
                </p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={() => setPricingModalOpen(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body codex-api-service-pricing-body">
              {pricingError && (
                <div className="codex-api-service-message error">
                  <CircleAlert size={15} />
                  <span>{pricingError}</span>
                </div>
              )}
              {pricingRepriceProgress && (
                <div
                  className={`codex-api-service-pricing-reprice is-${pricingRepriceProgress.phase}`}
                >
                  <div className="codex-api-service-pricing-reprice-head">
                    <span>{pricingRepriceStatusText}</span>
                    <strong>{pricingRepricePercent}%</strong>
                  </div>
                  <div className="codex-api-service-reprice-track">
                    <div
                      className="codex-api-service-reprice-fill"
                      style={{ width: `${pricingRepricePercent}%` }}
                    />
                  </div>
                </div>
              )}
              <div className="codex-api-service-pricing-table">
                <div className="codex-api-service-pricing-head">
                  <span>
                    {t("codex.apiService.models.pricingModel", "模型")}
                  </span>
                  <span>
                    {t(
                      "codex.apiService.models.pricingLongThreshold",
                      "长上下文阶梯阈值（tokens）",
                    )}
                  </span>
                  <span className="codex-api-service-pricing-head-tier">
                    {t(
                      "codex.apiService.models.pricingStandard",
                      "标准",
                    )}
                    <span className="codex-api-service-pricing-head-fields">
                      <span>{t("codex.apiService.models.pricingInput", "输入")}</span>
                      <span>
                        {t("codex.apiService.models.pricingCache", "缓存输入")}
                      </span>
                      <span>{t("codex.apiService.models.pricingOutput", "输出")}</span>
                    </span>
                  </span>
                  <span className="codex-api-service-pricing-head-tier">
                    {t(
                      "codex.apiService.models.pricingStandardLong",
                      "标准（长上下文）",
                    )}
                    <span className="codex-api-service-pricing-head-fields">
                      <span>{t("codex.apiService.models.pricingInput", "输入")}</span>
                      <span>
                        {t("codex.apiService.models.pricingCache", "缓存输入")}
                      </span>
                      <span>{t("codex.apiService.models.pricingOutput", "输出")}</span>
                    </span>
                  </span>
                  <span className="codex-api-service-pricing-head-tier">
                    {t(
                      "codex.apiService.models.pricingPriority",
                      "快速",
                    )}
                    <span className="codex-api-service-pricing-head-fields">
                      <span>{t("codex.apiService.models.pricingInput", "输入")}</span>
                      <span>
                        {t("codex.apiService.models.pricingCache", "缓存输入")}
                      </span>
                      <span>{t("codex.apiService.models.pricingOutput", "输出")}</span>
                    </span>
                  </span>
                  <div className="codex-api-service-pricing-cell">
                    <span>
                      {t("codex.apiService.models.pricingSource", "来源")}
                    </span>
                  </div>
                  <div className="codex-api-service-pricing-cell">
                    <span>
                      {t("codex.apiService.models.pricingActions", "操作")}
                    </span>
                  </div>
                </div>
                <div className="codex-api-service-pricing-rows">
                  {pricingDrafts.map((draft) => (
                    <div
                      key={draft.modelId}
                      className="codex-api-service-pricing-row"
                    >
                      <strong>{draft.modelId}</strong>
                      <div className="codex-api-service-pricing-inputs compact">
                        <input
                          type="number"
                          min={1}
                          step={1}
                          value={draft.longContextThresholdTokens}
                          placeholder={t(
                            "codex.apiService.models.pricingLongThreshold",
                            "长上下文阶梯阈值（tokens）",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingLongThreshold",
                            "长上下文阶梯阈值（tokens）",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "longContextThresholdTokens",
                              event.target.value,
                            )
                          }
                        />
                      </div>
                      <div className="codex-api-service-pricing-inputs">
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.inputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingInput",
                            "输入",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "inputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.cachedInputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingCache",
                            "缓存输入",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "cachedInputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.outputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingOutput",
                            "输出",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "outputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                      </div>
                      <div className="codex-api-service-pricing-inputs">
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.standardLongInputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingStandardLongInput",
                            "标准（长上下文）输入",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "standardLongInputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.standardLongCachedInputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingStandardLongCache",
                            "标准（长上下文）缓存输入",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "standardLongCachedInputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.standardLongOutputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingStandardLongOutput",
                            "标准（长上下文）输出",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "standardLongOutputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                      </div>
                      <div className="codex-api-service-pricing-inputs">
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.priorityInputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingPriorityInput",
                            "快速 输入",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "priorityInputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.priorityCachedInputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingPriorityCache",
                            "快速 缓存输入",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "priorityCachedInputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                        <input
                          type="number"
                          min={0}
                          step="0.000001"
                          value={draft.priorityOutputUsdPerMillion}
                          placeholder={t(
                            "codex.apiService.models.pricingOfficialMissing",
                            "-",
                          )}
                          aria-label={t(
                            "codex.apiService.models.pricingPriorityOutput",
                            "快速 输出",
                          )}
                          onChange={(event) =>
                            updatePricingDraft(
                              draft.modelId,
                              "priorityOutputUsdPerMillion",
                              event.target.value,
                            )
                          }
                        />
                      </div>
                      <div className="codex-api-service-pricing-cell">
                        <span
                          className={`codex-api-service-pill ${draft.custom ? "success" : "muted"}`}
                        >
                          {draft.custom
                            ? t(
                                "codex.apiService.models.pricingCustom",
                                "自定义",
                              )
                            : draft.hasPreset
                              ? t(
                                  "codex.apiService.models.pricingPreset",
                                  "预设",
                                )
                              : t(
                                  "codex.apiService.models.pricingUnset",
                                  "未设置",
                                )}
                        </span>
                      </div>
                      <div className="codex-api-service-pricing-cell">
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm"
                          onClick={() => resetPricingDraft(draft.modelId)}
                        >
                          {t("codex.apiService.models.pricingReset", "重置")}
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setPricingModalOpen(false)}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void handleSaveModelPricings()}
                disabled={busy || pricingRepriceActive}
              >
                <Check size={15} />
                {t("common.save", "保存")}
              </button>
            </div>
          </div>
        </div>
      )}

      {testDialogOpen && (
        <div
          className="modal-overlay codex-local-access-test-dialog-overlay"
        >
          <div
            className="modal codex-local-access-test-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-api-service-test-dialog-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header codex-local-access-test-dialog-header">
              <div>
                <h3 id="codex-api-service-test-dialog-title">
                  <ShieldCheck size={18} />
                  <span>
                    {t("codex.localAccess.testDialogTitle", "测试 API 服务")}
                  </span>
                </h3>
                <p>
                  {t(
                    "codex.localAccess.testDialogDesc",
                    "像游乐场一样通过当前 API 服务发起真实对话，便于检查模型、账号路由和上游响应。",
                  )}
                </p>
              </div>
              <button
                className="modal-close codex-local-access-test-dialog-close"
                onClick={handleCloseTestDialog}
                disabled={testDialogRunning}
                aria-label={t("common.close")}
              >
                <X size={18} />
              </button>
            </div>

            <div className="modal-body codex-local-access-test-dialog-body">
              <div className="codex-local-access-test-chat-toolbar">
                <div className="codex-local-access-test-chat-model">
                  <span>{t("codex.localAccess.testChatModel", "模型")}</span>
                  <SingleSelectDropdown
                    value={selectedModelId}
                    options={modelIds.map((modelId) => ({
                      value: modelId,
                      label: modelId,
                    }))}
                    onChange={setSelectedModelId}
                    disabled={modelIds.length === 0 || testDialogRunning}
                    ariaLabel={t("codex.localAccess.testChatModel", "模型")}
                    placeholder={t(
                      "codex.localAccess.modelIdPlaceholder",
                      "选择模型 ID",
                    )}
                    menuPlacement="down"
                    menuMaxHeight={240}
                  />
                </div>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={clearTestChat}
                  disabled={testDialogRunning || testChatMessages.length === 0}
                >
                  {t("codex.localAccess.testChatClear", "清空对话")}
                </button>
              </div>

              <div
                className="codex-local-access-test-chat"
                ref={testChatScrollRef}
              >
                {testChatMessages.length === 0 ? (
                  <div className="codex-local-access-test-chat-empty">
                    {t(
                      "codex.localAccess.testChatEmpty",
                      "输入一条消息后，会通过当前 API 服务发起真实对话。",
                    )}
                  </div>
                ) : (
                  testChatMessages.map((message) => (
                    <div
                      key={message.id}
                      className={`codex-local-access-test-chat-message is-${message.role}${
                        message.failureTitle ? " is-error" : ""
                      }`}
                    >
                      <div className="codex-local-access-test-chat-bubble">
                        {message.failureTitle && (
                          <strong className="codex-local-access-test-chat-error-title">
                            {message.failureTitle}
                          </strong>
                        )}
                        <p>{message.content}</p>
                        {message.failureDetail && (
                          <span className="codex-local-access-test-chat-meta">
                            {message.failureDetail}
                          </span>
                        )}
                        {typeof message.latencyMs === "number" && (
                          <span className="codex-local-access-test-chat-meta">
                            {t("codex.localAccess.testChatLatency", {
                              latency: formatLatencyMs(message.latencyMs),
                              defaultValue: "耗时 {{latency}}",
                            })}
                          </span>
                        )}
                      </div>
                    </div>
                  ))
                )}
                {testDialogRunning && (
                  <div className="codex-local-access-test-chat-message is-assistant">
                    <div className="codex-local-access-test-chat-bubble">
                      <span className="codex-local-access-test-chat-loading">
                        <RefreshCw size={14} className="loading-spinner" />
                        {t(
                          "codex.localAccess.testChatSending",
                          "正在请求 API 服务",
                        )}
                      </span>
                    </div>
                  </div>
                )}
              </div>

              {testDialogError && (
                <div
                  className="codex-local-access-inline-error"
                  aria-live="assertive"
                >
                  <CircleAlert size={14} />
                  <span>{testDialogError}</span>
                </div>
              )}
            </div>

            <div className="modal-footer codex-local-access-test-dialog-footer">
              <textarea
                className="codex-local-access-test-chat-input"
                value={testChatInput}
                onChange={(event) => setTestChatInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey) {
                    event.preventDefault();
                    void handleSendTestChatMessage();
                  }
                }}
                disabled={testDialogRunning}
                rows={2}
                placeholder={t(
                  "codex.localAccess.testChatInputPlaceholder",
                  "输入测试消息，Enter 发送",
                )}
              />
              <button
                className="btn btn-primary codex-local-access-test-chat-send"
                onClick={() => void handleSendTestChatMessage()}
                disabled={
                  testDialogRunning || !testChatInput.trim() || !selectedModelId
                }
              >
                <Send size={15} />
                {t("codex.localAccess.testChatSend", "发送")}
              </button>
              <button
                className="btn btn-secondary"
                onClick={handleCloseTestDialog}
                disabled={testDialogRunning}
              >
                {t("common.close")}
              </button>
            </div>
          </div>
        </div>
      )}

      <CodexAccountPoolHealthModal
        isOpen={healthModalOpen}
        accountIds={memberIds}
        accounts={accounts}
        accountHealth={state?.accountHealth ?? []}
        accountPoolHealth={state?.accountPoolHealth ?? []}
        actionBusy={busy}
        maskAccountText={maskAccountText}
        onClose={() => setHealthModalOpen(false)}
        onRecover={(accountId) => handleRecoverAccounts([accountId])}
        onRecoverAll={handleRecoverAccounts}
      />

      <CodexLocalAccessModal
        isOpen={memberModalOpen}
        mode="members"
        state={state}
        addressKind={addressKind}
        addressOptions={[
          {
            value: "local",
            label: t("codex.localAccess.addressLocal", "本机"),
          },
          ...(state?.lanBaseUrl
            ? [
                {
                  value: "lan",
                  label: t("codex.localAccess.addressLan", "局域网"),
                },
              ]
            : []),
        ]}
        onAddressKindChange={(value) =>
          setAddressKind(normalizeAddressKind(value))
        }
        accounts={accounts}
        accountsLoaded={accountsLoaded}
        accountGroups={groups}
        memberView={memberView}
        initialSelectedIds={memberIds}
        maskAccountText={maskAccountText}
        onClose={() => setMemberModalOpen(false)}
        onSaveAccounts={async ({
          accountIds,
          restrictFreeAccounts,
          backupAccountIds,
          preferredAccountIds,
          sessionAffinity,
          sessionAffinityTtlMs,
          imageGenerationAccountPolicies,
        }) => {
          await handleSaveMembersFromModal(
            accountIds,
            restrictFreeAccounts,
            backupAccountIds,
            preferredAccountIds,
            imageGenerationAccountPolicies,
          );
          if (collection) {
            const next = await codexLocalAccessService.updateCodexLocalAccessRoutingOptions({
              sessionAffinity,
              sessionAffinityTtlMs,
              responsesWebsocketsEnabled: collection.responsesWebsocketsEnabled,
              maxRetryCredentials: collection.maxRetryCredentials,
              maxRetryIntervalMs: collection.maxRetryIntervalMs,
              disableCooling: collection.disableCooling,
              immediateSseResponse: collection.immediateSseResponse,
              maxConcurrentImageRequests: collection.maxConcurrentImageRequests,
            });
            setState(next);
          }
        }}
        onClearStats={() =>
          codexLocalAccessService.clearCodexLocalAccessStats().then(setState)
        }
        onRefreshStats={reloadState}
        onUpdatePort={(port) =>
          codexLocalAccessService
            .updateCodexLocalAccessPort(port)
            .then(setState)
        }
        onUpdateRoutingStrategy={(strategy) =>
          codexLocalAccessService
            .updateCodexLocalAccessRoutingStrategy(strategy)
            .then(setState)
        }
        onUpdateCustomRouting={(rules: CodexLocalAccessCustomRoutingRule[]) =>
          codexLocalAccessService
            .updateCodexLocalAccessCustomRouting(rules)
            .then(setState)
        }
        onUpdateAccessScope={(scope: CodexLocalAccessScope) =>
          codexLocalAccessService
            .updateCodexLocalAccessAccessScope(scope)
            .then(setState)
        }
        onUpdateDebugLogs={(debugLogs) =>
          codexLocalAccessService
            .updateCodexLocalAccessDebugLogs(debugLogs)
            .then(setState)
        }
        onUpdateUpstreamProxyConfig={(url) =>
          codexLocalAccessService
            .updateCodexLocalAccessUpstreamProxyConfig(url)
            .then(setState)
        }
        onRotateApiKey={() =>
          codexLocalAccessService.rotateCodexLocalAccessApiKey().then(setState)
        }
        onRestartSidecar={handleRestartSidecar}
        onKillPort={handleKillPort}
        onToggleEnabled={handleToggleEnabled}
        onRecoverAccounts={handleRecoverAccounts}
        healthActionBusy={busy}
        onStreamTestMessage={({ sessionId, modelId, messages }) =>
          codexLocalAccessService.streamCodexLocalAccessChatTest(
            sessionId,
            modelId,
            messages,
          )
        }
        saving={busy}
        testing={testDialogRunning}
        starting={false}
        portCleanupBusy={portKilling}
        sidecarRestarting={sidecarRestarting}
      />
    </div>
  );
}
