import { useEffect, type ReactElement } from "react";
import { RefreshCw, Upload, Trash2, X, Power, Database, Copy, Check, Play, RotateCw, CircleAlert, Info, Calendar, Tag, Eye, EyeOff, FileText, ExternalLink, Pencil, FolderOpen, FolderPlus, ChevronRight, LogOut, Wrench, Terminal, Link2 } from "lucide-react";
import { isCodexGroupQuotaRefreshInherit, resolveCodexGroupQuotaAutoRefreshMinutes } from "../services/codexAccountGroupService";
import { isCodexApiKeyAccount, isCodexAgentIdentityAccount, isCodexChatCompletionsApiKeyAccount, isCodexNewApiAccount } from "../types/codex";
import { isVerboseCodexQuotaErrorMessage, summarizeCodexQuotaErrorMessage } from "../utils/codexQuotaError";
import { CodexQuotaMiniRows } from "../components/codex/CodexQuotaMiniRows";
import { isCodexClientReauthNoticeOnly, isCodexRefreshTokenNoticeOnly, isCodexRefreshTokenReusedAccount, isCodexServerRevokedReauth } from "../utils/codexSwitchAuthFailure";
import { DEFAULT_CODEX_INSTANCE_ID } from "../components/codex/CodexLaunchPreviewModal";
import { isDeepSeekAccount, isCodexTokenPlanAccount, shouldShowCodexApiKeyUsagePanel } from "../utils/codexDeepSeekAccess";
import { CodexSpeedSelect } from "../components/codex/CodexSpeedSelect";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { CODEX_API_SERVICE_BIND_ID } from "../types/instance";
import { COCKPIT_API_BASE_URL } from "../utils/codexProviderPresets";
import { formatCodexQuotaPoolBalance, formatCodexQuotaPoolPercent, formatCodexQuotaPoolWindowLabel } from "../utils/codexQuotaPool";
import { resolveNewApiQuotaSnapshot } from "../services/modelProviderUsageService";
import { CODEX_LOCAL_ACCESS_FALLBACK_API_KEY_MASK, formatCockpitApiInteger, formatCockpitApiTokenCount, getCockpitApiStatsRecord, getCockpitApiUsageRecord, getCodexAccountNoteTitle, hasCodexAccountNoteDetails, isPendingOAuthCodexAccount, isSponsorModelProvider, readCockpitApiNumber, readCockpitApiString, resolveApiKeyUsageMode, toCockpitApiRecord, type CockpitApiJsonRecord } from "./codexAccountsControllerModel";
import type { useCodexAccountsBaseController } from "./useCodexAccountsBaseController";
import type { useCodexAccountsOAuthController } from "./useCodexAccountsOAuthController";
import type { useCodexAccountsAccessController } from "./useCodexAccountsAccessController";
import type { useCodexAccountsLocalAccessController } from "./useCodexAccountsLocalAccessController";
import type { useCodexAccountsOverviewController } from "./useCodexAccountsOverviewController";

/** 封装 useCodexAccountsPageController 的 useCodexAccountsRenderers 业务域状态与动作。 */
export function useCodexAccountsRenderers(context: Pick<ReturnType<typeof useCodexAccountsBaseController> & ReturnType<typeof useCodexAccountsOAuthController> & ReturnType<typeof useCodexAccountsAccessController> & ReturnType<typeof useCodexAccountsLocalAccessController> & ReturnType<typeof useCodexAccountsOverviewController>,
  | "accountIdLabel"
  | "activeGroupId"
  | "addingLocalAccessAccountId"
  | "apiKeyUsageDetailAccount"
  | "apiKeyUsageMap"
  | "apiServiceAppSpeed"
  | "applyWindowStatsToQuotaItems"
  | "batchImportOpen"
  | "boundLocalAccessOAuthAccount"
  | "canDirectlyAddLocalAccessAccount"
  | "canRefreshApiKeyUsage"
  | "clearInlineRename"
  | "cliLaunchingAccountId"
  | "closeExternalImportProgressModal"
  | "cockpitApiPanelAccount"
  | "codexGroups"
  | "editingApiKeyNameId"
  | "editingApiKeyNameValue"
  | "externalImportProgress"
  | "filteredAccounts"
  | "findApiKeyUsageDetail"
  | "formatApiKeyUsageDetailByKey"
  | "formatApiKeyUsageDetailLabel"
  | "formatApiKeyUsageDetailValue"
  | "formatApiKeyUsageMoney"
  | "formatApiKeyUsagePercent"
  | "formatApiKeyUsageQuotaValue"
  | "formatDate"
  | "getCodexSwitchOrLaunchBlockedReason"
  | "groupByTag"
  | "handleAccountNameDoubleClick"
  | "handleAddLocalAccessAccount"
  | "handleApiServiceAppSpeedChange"
  | "handleCopyLocalAccessValue"
  | "handleDelete"
  | "handleEnterGroup"
  | "handleExportByIds"
  | "handleHideLocalAccessEntry"
  | "handleKillLocalAccessPort"
  | "handleLaunchCodexCli"
  | "handleLaunchLocalAccessCli"
  | "handleLocalAccessAddressKindChange"
  | "handleQuickRefreshLocalAccessQuota"
  | "handleQuickToggleLocalAccessEnabled"
  | "handleRefresh"
  | "handleRefreshGroup"
  | "handleRefreshSubscriptionInfo"
  | "handleRemoveLocalAccessAccount"
  | "handleRemoveSingleFromGroup"
  | "handleSubmitInlineRename"
  | "handleSwitch"
  | "handleToggleOverviewAccount"
  | "hideRelayQuota"
  | "importApiServiceGuideCount"
  | "inlineRenameDiscardRef"
  | "localAccessAccountIdSet"
  | "localAccessAccountPoolHealthHasIssue"
  | "localAccessAccountPoolHealthSummary"
  | "localAccessAddressOptions"
  | "localAccessBusy"
  | "localAccessCollection"
  | "localAccessCopiedField"
  | "localAccessDetailsExpanded"
  | "localAccessEntryVisible"
  | "localAccessKeyVisible"
  | "localAccessLaunchCurrent"
  | "localAccessPortKilling"
  | "localAccessQuotaHiddenCount"
  | "localAccessQuotaPoolLabels"
  | "localAccessQuotaPreviewItems"
  | "localAccessRefreshing"
  | "localAccessScopeLabel"
  | "localAccessStarting"
  | "localAccessState"
  | "maskAccountText"
  | "openAccountNoteModal"
  | "openApiKeyCredentialsModal"
  | "openCodexAddModal"
  | "openCodexApiServicePage"
  | "openLocalAccessMemberPicker"
  | "openLocalAccessOAuthBindingModal"
  | "openLocalAccessPanel"
  | "openQuickSwitchProviderModal"
  | "openQuotaErrorDetail"
  | "openTagModal"
  | "overviewCurrentAccountId"
  | "overviewLayoutMode"
  | "paginatedAccounts"
  | "refreshApiKeyUsage"
  | "refreshApiKeyUsageByAccountId"
  | "refreshing"
  | "refreshingAll"
  | "refreshingGroupId"
  | "refreshingSubscriptionAccountId"
  | "removingGroupAccountIds"
  | "renderAccountNoteButton"
  | "renderAccountSpeedSelect"
  | "renderAddLocalAccessAccountButton"
  | "renderApiKeyRevealLine"
  | "renderApiKeyUsagePanel"
  | "renderOAuthBindingLine"
  | "renderQuotaErrorInline"
  | "renderResetCreditControls"
  | "requestDeleteGroup"
  | "resolveAccountMeta"
  | "resolveApiKeyDisplayText"
  | "resolveApiProviderDisplayName"
  | "resolveCockpitApiAccountBalanceText"
  | "resolveGroupAccounts"
  | "resolveLocalAccessBaseUrl"
  | "resolvePresentation"
  | "resolveQuotaErrorMeta"
  | "resolveSingleExportBaseName"
  | "resolveSubscriptionPresentation"
  | "resolveUsageProviderForApiKeyAccount"
  | "resolveVisibleQuotaItems"
  | "savingApiKeyNameId"
  | "savingAppSpeedId"
  | "selected"
  | "selectedLocalAccessAddressKind"
  | "setActiveTab"
  | "setApiKeyUsageDetailAccountId"
  | "setCockpitApiPanelAccountId"
  | "setEditingApiKeyNameValue"
  | "setExternalImportSyncError"
  | "setGroupQuickAddGroupId"
  | "setImportApiServiceGuideCount"
  | "setLaunchPreviewInstanceId"
  | "setLocalAccessDetailsExpanded"
  | "setLocalAccessKeyVisible"
  | "setLocalAccessLaunchPreviewOpen"
  | "setMessage"
  | "setShowCodexGroupModal"
  | "setShowLocalAccessHealthModal"
  | "setShowLocalAccessQuotaStatsModal"
  | "shouldOfferReauthorizeAction"
  | "sponsorApiProviderTemplates"
  | "switching"
  | "t"
  | "toggleAccountApiKeyVisible"
  | "visibleApiKeyAccountIds"
>) {
  const {
    accountIdLabel,
    activeGroupId,
    addingLocalAccessAccountId,
    apiKeyUsageDetailAccount,
    apiKeyUsageMap,
    apiServiceAppSpeed,
    applyWindowStatsToQuotaItems,
    batchImportOpen,
    boundLocalAccessOAuthAccount,
    canDirectlyAddLocalAccessAccount,
    canRefreshApiKeyUsage,
    clearInlineRename,
    cliLaunchingAccountId,
    closeExternalImportProgressModal,
    cockpitApiPanelAccount,
    codexGroups,
    editingApiKeyNameId,
    editingApiKeyNameValue,
    externalImportProgress,
    filteredAccounts,
    findApiKeyUsageDetail,
    formatApiKeyUsageDetailByKey,
    formatApiKeyUsageDetailLabel,
    formatApiKeyUsageDetailValue,
    formatApiKeyUsageMoney,
    formatApiKeyUsagePercent,
    formatApiKeyUsageQuotaValue,
    formatDate,
    getCodexSwitchOrLaunchBlockedReason,
    groupByTag,
    handleAccountNameDoubleClick,
    handleAddLocalAccessAccount,
    handleApiServiceAppSpeedChange,
    handleCopyLocalAccessValue,
    handleDelete,
    handleEnterGroup,
    handleExportByIds,
    handleHideLocalAccessEntry,
    handleKillLocalAccessPort,
    handleLaunchCodexCli,
    handleLaunchLocalAccessCli,
    handleLocalAccessAddressKindChange,
    handleQuickRefreshLocalAccessQuota,
    handleQuickToggleLocalAccessEnabled,
    handleRefresh,
    handleRefreshGroup,
    handleRefreshSubscriptionInfo,
    handleRemoveLocalAccessAccount,
    handleRemoveSingleFromGroup,
    handleSubmitInlineRename,
    handleSwitch,
    handleToggleOverviewAccount,
    hideRelayQuota,
    importApiServiceGuideCount,
    inlineRenameDiscardRef,
    localAccessAccountIdSet,
    localAccessAccountPoolHealthHasIssue,
    localAccessAccountPoolHealthSummary,
    localAccessAddressOptions,
    localAccessBusy,
    localAccessCollection,
    localAccessCopiedField,
    localAccessDetailsExpanded,
    localAccessEntryVisible,
    localAccessKeyVisible,
    localAccessLaunchCurrent,
    localAccessPortKilling,
    localAccessQuotaHiddenCount,
    localAccessQuotaPoolLabels,
    localAccessQuotaPreviewItems,
    localAccessRefreshing,
    localAccessScopeLabel,
    localAccessStarting,
    localAccessState,
    maskAccountText,
    openAccountNoteModal,
    openApiKeyCredentialsModal,
    openCodexAddModal,
    openCodexApiServicePage,
    openLocalAccessMemberPicker,
    openLocalAccessOAuthBindingModal,
    openLocalAccessPanel,
    openQuickSwitchProviderModal,
    openQuotaErrorDetail,
    openTagModal,
    overviewCurrentAccountId,
    overviewLayoutMode,
    paginatedAccounts,
    refreshApiKeyUsage,
    refreshApiKeyUsageByAccountId,
    refreshing,
    refreshingAll,
    refreshingGroupId,
    refreshingSubscriptionAccountId,
    removingGroupAccountIds,
    renderAccountNoteButton,
    renderAccountSpeedSelect,
    renderAddLocalAccessAccountButton,
    renderApiKeyRevealLine,
    renderApiKeyUsagePanel,
    renderOAuthBindingLine,
    renderQuotaErrorInline,
    renderResetCreditControls,
    requestDeleteGroup,
    resolveAccountMeta,
    resolveApiKeyDisplayText,
    resolveApiProviderDisplayName,
    resolveCockpitApiAccountBalanceText,
    resolveGroupAccounts,
    resolveLocalAccessBaseUrl,
    resolvePresentation,
    resolveQuotaErrorMeta,
    resolveSingleExportBaseName,
    resolveSubscriptionPresentation,
    resolveUsageProviderForApiKeyAccount,
    resolveVisibleQuotaItems,
    savingApiKeyNameId,
    savingAppSpeedId,
    selected,
    selectedLocalAccessAddressKind,
    setActiveTab,
    setApiKeyUsageDetailAccountId,
    setCockpitApiPanelAccountId,
    setEditingApiKeyNameValue,
    setExternalImportSyncError,
    setGroupQuickAddGroupId,
    setImportApiServiceGuideCount,
    setLaunchPreviewInstanceId,
    setLocalAccessDetailsExpanded,
    setLocalAccessKeyVisible,
    setLocalAccessLaunchPreviewOpen,
    setMessage,
    setShowCodexGroupModal,
    setShowLocalAccessHealthModal,
    setShowLocalAccessQuotaStatsModal,
    shouldOfferReauthorizeAction,
    sponsorApiProviderTemplates,
    switching,
    t,
    toggleAccountApiKeyVisible,
    visibleApiKeyAccountIds,
  } = context;
  const renderCompactRows = (
      items: typeof filteredAccounts,
      groupKey?: string,
    ) =>
      items.map((account) => {
        const presentation = resolvePresentation(account);
        const isCurrent = overviewCurrentAccountId === account.id;
        const isSelected = selected.has(account.id);
        const isApiKeyAccount = isCodexApiKeyAccount(account);
        const serverRevokedReauth = isCodexServerRevokedReauth(account);
        const refreshTokenReusedState = isCodexRefreshTokenReusedAccount(account);
        const reauthNoticeOnly =
          !serverRevokedReauth &&
          !refreshTokenReusedState &&
          isCodexClientReauthNoticeOnly(account);
        const clientAuthRequired =
          !isApiKeyAccount &&
          !account.requires_reauth &&
          !serverRevokedReauth &&
          account.client_auth_status === "login_required";
        const isAgentIdentityAccount = isCodexAgentIdentityAccount(account);
        const switchOrLaunchBlockedReason =
          getCodexSwitchOrLaunchBlockedReason(account);
        const isChatCompletionsApiKey =
          isCodexChatCompletionsApiKeyAccount(account);
        const compactOfficialQuotaItems = applyWindowStatsToQuotaItems(
          account,
          presentation.quotaItems,
        ).filter((item) => item.key === "primary" || item.key === "secondary");
        const compactDeepSeekSummary = isDeepSeekAccount(account)
          ? apiKeyUsageMap[account.id]?.summary
          : undefined;
        const compactTokenPlanSummary = isCodexTokenPlanAccount(account)
          ? apiKeyUsageMap[account.id]?.summary
          : undefined;
        const subscriptionInfo = resolveSubscriptionPresentation(account);
        const showCompactExpiry =
          !isApiKeyAccount &&
          !isAgentIdentityAccount &&
          subscriptionInfo.bucket !== "active";
        const showSubscriptionRefreshAction =
          !isApiKeyAccount &&
          !isAgentIdentityAccount &&
          (subscriptionInfo.bucket === "missing" ||
            subscriptionInfo.bucket === "expired");
        const isSubscriptionRefreshPending =
          refreshingSubscriptionAccountId === account.id ||
          refreshing === account.id;
        return (
          <div
            key={groupKey ? `${groupKey}-${account.id}` : account.id}
            className={`codex-compact-row ${isCurrent ? "current" : ""} ${isSelected ? "selected" : ""}`}
          >
            <div className="codex-compact-select">
              <input
                type="checkbox"
                checked={isSelected}
                onChange={() => handleToggleOverviewAccount(account.id)}
              />
            </div>
            <span
              className="codex-compact-email"
              title={maskAccountText(presentation.displayName)}
            >
              {maskAccountText(presentation.displayName)}
            </span>
            {!isApiKeyAccount &&
              !refreshTokenReusedState &&
              (account.requires_reauth || serverRevokedReauth) && (
              <span
                className={`codex-status-pill ${reauthNoticeOnly ? "quota-refresh" : "quota-error"} codex-client-auth-status-pill`}
                title={account.reauth_reason || t("codex.authError.badge", "授权异常")}
              >
                <CircleAlert size={11} />
                {reauthNoticeOnly
                  ? t("codex.switchAuth.apiOnlyBadge", "客户端需授权")
                  : t("codex.authError.badge", "授权异常")}
              </span>
            )}
            {clientAuthRequired && (
              <span
                className="codex-status-pill quota-refresh codex-client-auth-status-pill"
                title={t(
                  "codex.switchAuth.apiOnlyDescription",
                  "检测到当前账号的客户端需要重新授权，API 服务仍可用。",
                )}
              >
                <CircleAlert size={11} />
                {t("codex.switchAuth.apiOnlyBadge", "客户端需授权")}
              </span>
            )}
            <div className="codex-compact-quotas">
              {isDeepSeekAccount(account) ? (
                <>
                  {(
                    [
                      [
                        "totalBalance",
                        t(
                          "codex.modelProviders.usage.fields.totalBalance",
                          "总余额",
                        ),
                        formatApiKeyUsageMoney(
                          compactDeepSeekSummary?.balance,
                          compactDeepSeekSummary?.unit,
                        ),
                      ],
                      [
                        "grantedBalance",
                        t(
                          "codex.modelProviders.usage.fields.grantedBalance",
                          "赠金余额",
                        ),
                        formatApiKeyUsageDetailByKey(
                          compactDeepSeekSummary,
                          "grantedBalance",
                        ),
                      ],
                      [
                        "toppedUpBalance",
                        t(
                          "codex.modelProviders.usage.fields.toppedUpBalance",
                          "充值余额",
                        ),
                        formatApiKeyUsageDetailByKey(
                          compactDeepSeekSummary,
                          "toppedUpBalance",
                        ),
                      ],
                    ] as const
                  ).map(([key, label, value]) => (
                    <span
                      key={`${account.id}-${key}`}
                      className={`codex-compact-quota codex-compact-quota-${key}`}
                      title={`${label} ${value}`}
                    >
                      <span className="codex-compact-dot" />
                      <span className="codex-compact-quota-value high">
                        {value}
                      </span>
                    </span>
                  ))}
                </>
              ) : isCodexTokenPlanAccount(account) ? (
                <>
                  {(
                    [
                      [
                        "remaining",
                        t(
                          "codex.modelProviders.usage.fields.remaining",
                          "Remaining",
                        ),
                        formatApiKeyUsageMoney(
                          compactTokenPlanSummary?.quotaRemaining ??
                            compactTokenPlanSummary?.remaining,
                          compactTokenPlanSummary?.unit,
                        ),
                      ],
                      [
                        "planName",
                        t("codex.modelProviders.usage.fields.planName", "Plan"),
                        compactTokenPlanSummary?.planName || "-",
                      ],
                    ] as const
                  ).map(([key, label, value]) => (
                    <span
                      key={`${account.id}-${key}`}
                      className={`codex-compact-quota codex-compact-quota-${key}`}
                      title={`${label} ${value}`}
                    >
                      <span className="codex-compact-dot" />
                      <span className="codex-compact-quota-value high">
                        {value}
                      </span>
                    </span>
                  ))}
                </>
              ) : (
                !isChatCompletionsApiKey && (
                  <CodexQuotaMiniRows items={compactOfficialQuotaItems} t={t} />
                )
              )}
              {showCompactExpiry && (
                <span className="codex-compact-expiry-wrap">
                  <span
                    className={`codex-compact-expiry ${subscriptionInfo.tone}`}
                    title={subscriptionInfo.titleText}
                  >
                    {subscriptionInfo.valueText}
                  </span>
                  {showSubscriptionRefreshAction && (
                    <button
                      type="button"
                      className="codex-subscription-refresh-btn"
                      onClick={() =>
                        void handleRefreshSubscriptionInfo(account.id)
                      }
                      disabled={isSubscriptionRefreshPending}
                      title={t("common.refresh", "刷新")}
                      aria-label={t("common.refresh", "刷新")}
                    >
                      {t("common.refresh", "刷新")}
                    </button>
                  )}
                </span>
              )}
            </div>
            {renderAccountSpeedSelect(account, true)}
            {renderAddLocalAccessAccountButton(
              account,
              "codex-compact-api-service-btn",
              13,
            )}
            {clientAuthRequired && (
              <button
                type="button"
                className="codex-compact-note-btn codex-compact-reauthorize-btn"
                onClick={() => openCodexAddModal("oauth", account)}
                title={t("common.reauthorize", "重新授权")}
                aria-label={t("common.reauthorize", "重新授权")}
              >
                <RefreshCw size={13} />
              </button>
            )}
            {!isApiKeyAccount && (
              <button
                className={`codex-compact-note-btn ${hasCodexAccountNoteDetails(account) ? "has-note" : ""}`}
                onClick={() => openAccountNoteModal(account)}
                title={
                  getCodexAccountNoteTitle(account, "") ||
                  t("codex.accountNote.emptyTitle", "填写账号备注")
                }
                aria-label={t("codex.accountNote.title", "账号备注")}
              >
                <FileText size={13} />
              </button>
            )}
            <button
              className={`codex-compact-switch-btn ${!isCurrent ? "success" : ""}`}
              onClick={() => handleSwitch(account.id)}
              disabled={!!switching || Boolean(switchOrLaunchBlockedReason)}
              title={switchOrLaunchBlockedReason || t("codex.switch", "切换")}
            >
              {switching === account.id ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <Play size={14} />
              )}
            </button>
          </div>
        );
      });
  
    const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
      items.map((account) => {
        const presentation = resolvePresentation(account);
        const meta = resolveAccountMeta(account);
        const isCurrent = overviewCurrentAccountId === account.id;
        const isApiKeyAccount = isCodexApiKeyAccount(account);
        const serverRevokedReauth = isCodexServerRevokedReauth(account);
        const refreshTokenReusedState = isCodexRefreshTokenReusedAccount(account);
        const clientAuthRequired =
          !isApiKeyAccount &&
          !account.requires_reauth &&
          !refreshTokenReusedState &&
          account.client_auth_status === "login_required";
        const clientAuthNoticeText = t(
          "codex.switchAuth.apiOnlyDescription",
          "检测到当前账号的客户端需要重新授权，API 服务仍可用。",
        );
        const clientAuthDetailText = [
          t(
            "codex.switchAuth.observationStatus",
            "检测到跳转到登录页面",
          ),
          account.last_client_launch_at
            ? t("codex.switchAuth.launchTime", {
                time: new Date(account.last_client_launch_at * 1000).toLocaleString(),
                defaultValue: "本次实例启动时间：{{time}}",
              })
            : "",
          account.last_client_login_redirect_at
            ? t("codex.switchAuth.loginRedirectTime", {
                time: new Date(
                  account.last_client_login_redirect_at * 1000,
                ).toLocaleString(),
                defaultValue: "最近一次跳转登录页：{{time}}",
              })
            : "",
          account.last_client_auth_observed_at
            ? t("codex.switchAuth.observationTime", {
                time: new Date(
                  account.last_client_auth_observed_at * 1000,
                ).toLocaleString(),
                defaultValue: "最近一次检测时间：{{time}}",
              })
            : "",
          account.last_client_auth_instance_id
            ? t("codex.switchAuth.observationInstance", {
                instance: account.last_client_auth_instance_id,
                defaultValue: "检测实例：{{instance}}",
              })
            : "",
        ]
          .filter(Boolean)
          .join("\n");
        const switchOrLaunchBlockedReason =
          getCodexSwitchOrLaunchBlockedReason(account);
        const isPendingOAuthAccount = isPendingOAuthCodexAccount(account);
        const isNewApiAccount = isCodexNewApiAccount(account);
        const isEditingApiKeyName =
          isApiKeyAccount && editingApiKeyNameId === account.id;
        const isSavingApiKeyName = savingApiKeyNameId === account.id;
        const planClass = presentation.planClass || "unknown";
        const isSelected = selected.has(account.id);
        const quotaItems = applyWindowStatsToQuotaItems(
          account,
          resolveVisibleQuotaItems(
            presentation,
            isApiKeyAccount,
            isNewApiAccount,
          ),
        );
        const reauthErrorMeta = resolveQuotaErrorMeta(
          !refreshTokenReusedState && account.requires_reauth && account.reauth_reason
            ? {
                message: account.reauth_reason,
                timestamp: account.token_updated_at || account.last_used,
              }
            : undefined,
        );
        const quotaErrorMeta = resolveQuotaErrorMeta(
          refreshTokenReusedState ? undefined : account.quota_error,
        );
        const accountIssueMeta = reauthErrorMeta.rawMessage
          ? reauthErrorMeta
          : quotaErrorMeta;
        const hasQuotaError = Boolean(accountIssueMeta.rawMessage);
        const isRefreshTokenNotice = isCodexRefreshTokenNoticeOnly(account);
        const isClientReauthNotice =
          !serverRevokedReauth &&
          !refreshTokenReusedState &&
          isCodexClientReauthNoticeOnly(account);
        const isQuotaRefreshNotice =
          isClientReauthNotice ||
          (!reauthErrorMeta.rawMessage &&
            quotaErrorMeta.isRefreshRequestFailure &&
            !quotaErrorMeta.statusCode &&
            !quotaErrorMeta.errorCode);
        const accountIssueDisplayText = isRefreshTokenNotice
          ? t(
              "codex.quotaError.authRefreshDeferred",
              "refresh_token 已失效；当前 access_token 仍可用于 API 服务，但不能切换到官方客户端，请重新授权后再切号。",
            )
          : accountIssueMeta.displayText;
        const accountIssueBadge = isRefreshTokenNotice
          ? t("codex.authError.badge", "授权异常")
          : isQuotaRefreshNotice
            ? t("codex.quotaError.refreshFailedBadge", "刷新失败")
            : reauthErrorMeta.rawMessage
              ? t("codex.authError.badge", "授权异常")
              : accountIssueMeta.statusCode ||
                t("codex.quotaError.badge", "配额异常");
        const showReauthorizeAction =
          !isApiKeyAccount &&
          !isRefreshTokenNotice &&
          (isPendingOAuthAccount ||
            (hasQuotaError && shouldOfferReauthorizeAction(accountIssueMeta)));
        const accountIdText =
          meta.chatgptAccountId &&
          meta.chatgptAccountId !== t("common.none", "暂无")
            ? meta.chatgptAccountId
            : meta.userId;
        const signInLine = `${meta.signedInWithText} | ${accountIdLabel}: ${accountIdText}`;
        const apiProviderName = resolveApiProviderDisplayName(account);
        const apiProviderLine = `${t("codex.api.provider.label", "供应商")}：${apiProviderName}`;
        const apiBaseUrlText = (account.api_base_url || "").trim() || "-";
        const apiBaseUrlLine = `${t("codex.api.baseUrl", "Base URL")}：${apiBaseUrlText}`;
        const apiKeyUsageProvider = resolveUsageProviderForApiKeyAccount(account);
        const isSponsorApiKeyAccount =
          isApiKeyAccount &&
          isSponsorModelProvider(
            apiKeyUsageProvider,
            sponsorApiProviderTemplates,
          );
        const apiKeyUsageMode = resolveApiKeyUsageMode(
          apiKeyUsageMap[account.id]?.summary,
        );
        const showApiKeyUsagePanel = shouldShowCodexApiKeyUsagePanel(
          account,
          hideRelayQuota,
        );
        const isDetailedUsageAccount =
          showApiKeyUsagePanel &&
          (apiKeyUsageMode === "sub2api" ||
            apiKeyUsageMode === "cockpit_tools" ||
            apiKeyUsageProvider?.integrationType === "sub2api" ||
            apiKeyUsageProvider?.integrationType === "cockpit_tools");
        const isTokenPlanUsageAccount =
          showApiKeyUsagePanel && apiKeyUsageMode === "token_plan";
        const isQuotaAwareApiKeyAccount =
          showApiKeyUsagePanel &&
          !isSponsorApiKeyAccount &&
          (apiKeyUsageMode !== null ||
            isDeepSeekAccount(account) ||
            apiKeyUsageProvider?.integrationType === "new_api" ||
            apiKeyUsageProvider?.integrationType === "sub2api" ||
            apiKeyUsageProvider?.integrationType === "cockpit_tools");
        const shouldRenderQuotaSection =
          showApiKeyUsagePanel ||
          !isApiKeyAccount ||
          (isNewApiAccount && !hideRelayQuota);
        const displayPlanClass = isSponsorApiKeyAccount
          ? "sponsor-api"
          : isQuotaAwareApiKeyAccount
            ? "new-api-exclusive"
            : planClass;
        const displayPlanLabel = isSponsorApiKeyAccount
          ? apiProviderName
          : presentation.planLabel;
        const cockpitApiAccountBalanceText =
          isNewApiAccount && !hideRelayQuota
            ? resolveCockpitApiAccountBalanceText(account)
            : null;
        const accountTags = (account.tags || [])
          .map((tag) => tag.trim())
          .filter(Boolean);
        // 充分利用卡片横向空间：最多展示 8 个，避免 3 个标签就被 +N 收起（#962）
        const visibleTags = accountTags.slice(0, 8);
        const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
        const isInLocalAccess = localAccessAccountIdSet.has(account.id);
        const canAddToLocalAccess = canDirectlyAddLocalAccessAccount(account);
        const subscriptionInfo = resolveSubscriptionPresentation(account);
        const isSubscriptionInfoMissing = subscriptionInfo.bucket === "missing";
        const isAccessTokenOnlySubscription =
          subscriptionInfo.bucket === "access_token_only";
        const showSubscriptionRefreshAction =
          !isApiKeyAccount &&
          !isPendingOAuthAccount &&
          (subscriptionInfo.bucket === "missing" ||
            subscriptionInfo.bucket === "expired");
        const isSubscriptionRefreshPending =
          refreshingSubscriptionAccountId === account.id ||
          refreshing === account.id;
        const resetCreditControls = renderResetCreditControls(account);
        return (
          <div
            key={groupKey ? `${groupKey}-${account.id}` : account.id}
            className={`codex-account-card ${isCurrent ? "current" : ""} ${isSelected ? "selected" : ""} ${isPendingOAuthAccount ? "pending-auth" : ""} ${isNewApiAccount ? "new-api-exclusive" : ""} ${isQuotaAwareApiKeyAccount ? "api-key-usage-account" : ""} ${isSponsorApiKeyAccount ? "sponsor-api-account" : ""}`}
          >
            <div className="card-top">
              <div className="card-select">
                <input
                  type="checkbox"
                  checked={isSelected}
                  onChange={() => handleToggleOverviewAccount(account.id)}
                />
              </div>
              {isEditingApiKeyName ? (
                <input
                  className="account-email inline-name-editor"
                  value={editingApiKeyNameValue}
                  onChange={(event) =>
                    setEditingApiKeyNameValue(event.target.value)
                  }
                  onBlur={() => void handleSubmitInlineRename(account)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void handleSubmitInlineRename(account);
                    } else if (event.key === "Escape") {
                      event.preventDefault();
                      inlineRenameDiscardRef.current = true;
                      clearInlineRename();
                    }
                  }}
                  disabled={isSavingApiKeyName}
                  autoFocus
                />
              ) : (
                <span
                  className={`account-email ${isApiKeyAccount ? "editable" : ""}`}
                  title={maskAccountText(presentation.displayName)}
                  onDoubleClick={() => handleAccountNameDoubleClick(account)}
                >
                  {maskAccountText(presentation.displayName)}
                </span>
              )}
              {isCurrent && (
                <span className="current-tag">{t("codex.current", "当前")}</span>
              )}
              {clientAuthRequired && (
                <span
                  className="codex-status-pill quota-refresh codex-client-auth-status-pill"
                  title={t(
                    "codex.switchAuth.apiOnlyDescription",
                    "检测到当前账号的客户端需要重新授权，API 服务仍可用。",
                  )}
                >
                  <CircleAlert size={12} />
                  {t("codex.switchAuth.apiOnlyBadge", "客户端需授权")}
                </span>
              )}
              {hasQuotaError && (
                <span
                  className={`codex-status-pill ${isQuotaRefreshNotice ? "quota-refresh" : "quota-error"}`}
                  title={accountIssueDisplayText}
                >
                  {isQuotaRefreshNotice ? (
                    <Info size={12} />
                  ) : (
                    <CircleAlert size={12} />
                  )}
                  {accountIssueBadge}
                </span>
              )}
              <span className={`tier-badge ${displayPlanClass}`}>
                {displayPlanLabel}
              </span>
            </div>
            {(meta.accountContextText ||
              isInLocalAccess ||
              canAddToLocalAccess ||
              (!isApiKeyAccount && hasCodexAccountNoteDetails(account)) ||
              resetCreditControls) && (
              <div className="account-sub-line">
                {meta.accountContextText && (
                  <span
                    className="codex-login-subline"
                    title={meta.accountContextText}
                  >
                    Team Name：{meta.accountContextText}
                  </span>
                )}
                {isInLocalAccess && (
                  <button
                    type="button"
                    className="group-account-badge codex-local-access-inline-remove"
                    onClick={(event) => {
                      event.stopPropagation();
                      void handleRemoveLocalAccessAccount(account.id);
                    }}
                    disabled={addingLocalAccessAccountId !== null}
                    title={t("codex.localAccess.removeAction", "移除 API 服务")}
                    aria-label={t(
                      "codex.localAccess.removeAction",
                      "移除 API 服务",
                    )}
                  >
                    {addingLocalAccessAccountId === account.id ? (
                      <RefreshCw size={11} className="loading-spinner" />
                    ) : (
                      <Link2 size={11} />
                    )}
                    {t("codex.localAccess.removeAction", "移除 API 服务")}
                  </button>
                )}
                {!isInLocalAccess && canAddToLocalAccess && (
                  <button
                    type="button"
                    className="group-account-badge codex-local-access-inline-add"
                    onClick={() => void handleAddLocalAccessAccount(account.id)}
                    disabled={addingLocalAccessAccountId !== null}
                    title={t("codex.localAccess.entryAction", "添加至 API 服务")}
                  >
                    <Link2 size={11} />
                    {t("codex.localAccess.entryAction", "添加至 API 服务")}
                  </button>
                )}
                {!isApiKeyAccount && renderAccountNoteButton(account)}
                {resetCreditControls}
              </div>
            )}
            {!isApiKeyAccount && (
              <div className="account-sub-line">
                <span className="codex-login-subline" title={signInLine}>
                  {meta.signedInWithText} | {accountIdLabel}:{" "}
                  {maskAccountText(accountIdText)}
                </span>
              </div>
            )}
            {isApiKeyAccount && (
              <>
                <div className="account-sub-line">
                  {renderApiKeyRevealLine(account)}
                </div>
                {renderOAuthBindingLine(account)}
                <div className="account-sub-line codex-provider-inline-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={apiProviderLine}
                  >
                    {apiProviderLine}
                  </span>
                  {!isNewApiAccount && (
                    <button
                      type="button"
                      className="codex-provider-inline-switch"
                      onClick={() => openQuickSwitchProviderModal(account)}
                      title={t("codex.quickSwitch.action", "快速切换供应商")}
                    >
                      {t("codex.quickSwitch.inlineAction", "切换")}
                    </button>
                  )}
                </div>
                <div className="account-sub-line codex-provider-inline-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={apiBaseUrlLine}
                  >
                    {apiBaseUrlLine}
                  </span>
                  {(isDetailedUsageAccount || isTokenPlanUsageAccount) && (
                    <button
                      type="button"
                      className="codex-provider-inline-switch"
                      onClick={() => setApiKeyUsageDetailAccountId(account.id)}
                      title={t(
                        "codex.modelProviders.usage.detailTitle",
                        "服务面板",
                      )}
                    >
                      {t("common.detail", "详情")}
                    </button>
                  )}
                </div>
              </>
            )}
            {accountTags.length > 0 && (
              <div className="card-tags">
                {visibleTags.map((tag, idx) => (
                  <span key={`${account.id}-${tag}-${idx}`} className="tag-pill">
                    {tag}
                  </span>
                ))}
                {moreTagCount > 0 && (
                  <span className="tag-pill more">+{moreTagCount}</span>
                )}
              </div>
            )}
            {shouldRenderQuotaSection && (
              <div className="codex-quota-section">
                {showApiKeyUsagePanel ? (
                  renderApiKeyUsagePanel(account, apiKeyUsageProvider)
                ) : (
                  <>
                    {clientAuthRequired &&
                      renderQuotaErrorInline({
                        accountName: presentation.displayName,
                        displayText: clientAuthNoticeText,
                        rawMessage: clientAuthDetailText,
                        isVerbose: false,
                        detailSummary: clientAuthNoticeText,
                        detailReauthorizeAccountId: account.id,
                        clearClientAuthObservationAccountId: account.id,
                        detailTitle: t(
                          "codex.switchAuth.apiOnlyTitle",
                          "客户端需授权",
                        ),
                        isRefreshNotice: true,
                        showReauthorize: true,
                        onReauthorize: () =>
                          openCodexAddModal("oauth", account),
                      })}
                    {!isPendingOAuthAccount &&
                      hasQuotaError &&
                      renderQuotaErrorInline({
                        accountName: presentation.displayName,
                        displayText: accountIssueDisplayText,
                        rawMessage: accountIssueMeta.rawMessage,
                        isVerbose: accountIssueMeta.isVerbose,
                        detailSummary: isRefreshTokenNotice
                          ? accountIssueDisplayText
                          : undefined,
                        detailReauthorizeAccountId:
                          isRefreshTokenNotice || showReauthorizeAction
                            ? account.id
                            : undefined,
                        isRefreshNotice: isQuotaRefreshNotice,
                        showReauthorize: showReauthorizeAction,
                        onReauthorize: () => openCodexAddModal("oauth", account),
                      })}
                    {cockpitApiAccountBalanceText && (
                      <div className="codex-account-balance-line">
                        <span>
                          {t(
                            "codex.modelProviders.usage.accountBalance",
                            "账户余额",
                          )}
                          ：
                        </span>
                        <strong>{cockpitApiAccountBalanceText}</strong>
                      </div>
                    )}
                    <CodexQuotaMiniRows items={quotaItems} t={t} />
                    {quotaItems.length === 0 && !cockpitApiAccountBalanceText && (
                      <div className="quota-empty">
                        {t("common.shared.quota.noData", "暂无配额数据")}
                      </div>
                    )}
                    {isPendingOAuthAccount && (
                      <div className="codex-card-action-inline">
                        <button
                          className="btn btn-sm btn-outline"
                          onClick={() => openCodexAddModal("oauth", account)}
                          title={t("common.shared.addModal.oauth", "OAuth 授权")}
                        >
                          {t("common.shared.addModal.oauth", "OAuth 授权")}
                        </button>
                      </div>
                    )}
                  </>
                )}
              </div>
            )}
            {!isApiKeyAccount && (
              <div
                className={`codex-subscription-footer ${subscriptionInfo.tone}`}
                title={subscriptionInfo.titleText}
              >
                <div className="codex-subscription-footer-main">
                  <Calendar size={14} />
                  {isSubscriptionInfoMissing || isAccessTokenOnlySubscription ? (
                    <strong>{subscriptionInfo.valueText}</strong>
                  ) : (
                    <>
                      <span>{t("codex.subscription.label", "有效期")}</span>
                      <strong>{subscriptionInfo.valueText}</strong>
                    </>
                  )}
                </div>
                {(subscriptionInfo.timestampMs != null ||
                  showSubscriptionRefreshAction) && (
                  <div className="codex-subscription-footer-side">
                    {subscriptionInfo.timestampMs != null && (
                      <span className="codex-subscription-footer-date">
                        {subscriptionInfo.detailText}
                      </span>
                    )}
                    {showSubscriptionRefreshAction && (
                      <button
                        type="button"
                        className="codex-subscription-refresh-btn"
                        onClick={() =>
                          void handleRefreshSubscriptionInfo(account.id)
                        }
                        disabled={isSubscriptionRefreshPending}
                        title={t("common.refresh", "刷新")}
                        aria-label={t("common.refresh", "刷新")}
                      >
                        {t("common.refresh", "刷新")}
                      </button>
                    )}
                  </div>
                )}
              </div>
            )}
            <div className="codex-card-bottom">
              <span className="card-date">{formatDate(account.created_at)}</span>
              {renderAccountSpeedSelect(account)}
              <div className="card-footer">
                <div className="card-actions">
                  <button
                    className="card-action-btn"
                    onClick={() => void handleLaunchCodexCli(account)}
                    disabled={
                      cliLaunchingAccountId === account.id ||
                      Boolean(switchOrLaunchBlockedReason)
                    }
                    title={
                      switchOrLaunchBlockedReason ||
                      t("codex.cli.quickLaunch", "CLI 快速启动")
                    }
                  >
                    {cliLaunchingAccountId === account.id ? (
                      <RefreshCw size={14} className="loading-spinner" />
                    ) : (
                      <Terminal size={14} />
                    )}
                  </button>
                  {isNewApiAccount && (
                    <button
                      className="card-action-btn"
                      onClick={() => setCockpitApiPanelAccountId(account.id)}
                      title={t("codex.cockpitApi.servicePanel", "服务面板")}
                    >
                      <Database size={14} />
                    </button>
                  )}
                  <button
                    className="card-action-btn"
                    onClick={() => openTagModal(account.id)}
                    title={t("accounts.editTags", "编辑标签")}
                  >
                    <Tag size={14} />
                  </button>
                  {!isApiKeyAccount && !isNewApiAccount && (
                    <button
                      className={`card-action-btn ${hasCodexAccountNoteDetails(account) ? "active" : ""}`}
                      onClick={() => openAccountNoteModal(account)}
                      title={
                        getCodexAccountNoteTitle(account, "") ||
                        t("codex.accountNote.emptyTitle", "填写账号备注")
                      }
                      aria-label={t("codex.accountNote.title", "账号备注")}
                    >
                      <FileText size={14} />
                    </button>
                  )}
                  {isApiKeyAccount && !isNewApiAccount && (
                    <button
                      className="card-action-btn"
                      onClick={() => openApiKeyCredentialsModal(account)}
                      title={t("instances.actions.edit", "编辑")}
                    >
                      <Pencil size={14} />
                    </button>
                  )}
                  <button
                    className={`card-action-btn ${!isCurrent ? "success" : ""}`}
                    onClick={() => handleSwitch(account.id)}
                    disabled={!!switching || Boolean(switchOrLaunchBlockedReason)}
                    title={
                      switchOrLaunchBlockedReason || t("codex.switch", "切换")
                    }
                  >
                    {switching === account.id ? (
                      <RefreshCw size={14} className="loading-spinner" />
                    ) : (
                      <Play size={14} />
                    )}
                  </button>
                  {!isPendingOAuthAccount &&
                    (!isApiKeyAccount ||
                      isNewApiAccount ||
                      canRefreshApiKeyUsage(account, apiKeyUsageProvider)) && (
                      <button
                        className="card-action-btn"
                        onClick={() =>
                          canRefreshApiKeyUsage(account, apiKeyUsageProvider)
                            ? void refreshApiKeyUsage(
                                account,
                                apiKeyUsageProvider,
                              )
                            : handleRefresh(account.id)
                        }
                        disabled={
                          canRefreshApiKeyUsage(account, apiKeyUsageProvider)
                            ? apiKeyUsageMap[account.id]?.loading === true
                            : refreshing === account.id
                        }
                        title={t("common.shared.refreshQuota", "刷新配额")}
                      >
                        <RotateCw
                          size={14}
                          className={
                            canRefreshApiKeyUsage(account, apiKeyUsageProvider)
                              ? apiKeyUsageMap[account.id]?.loading === true
                                ? "loading-spinner"
                                : ""
                              : refreshing === account.id
                                ? "loading-spinner"
                                : ""
                          }
                        />
                      </button>
                    )}
                  <button
                    className="card-action-btn export-btn"
                    onClick={() =>
                      handleExportByIds(
                        [account.id],
                        resolveSingleExportBaseName(account),
                      )
                    }
                    title={t("common.shared.export.title", "导出")}
                  >
                    <Upload size={14} />
                  </button>
                  <button
                    className="card-action-btn danger"
                    onClick={() => handleDelete(account.id)}
                    title={t("common.delete", "删除")}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          </div>
        );
      });
  
    const renderLocalAccessInlineCard = () => {
      if (!localAccessEntryVisible) {
        return null;
      }
  
      const isGridLocalAccessCard = overviewLayoutMode === "grid";
      const showLocalAccessDetails = isGridLocalAccessCard
        ? true
        : localAccessDetailsExpanded;
      const baseUrl = resolveLocalAccessBaseUrl();
      const apiKeyDisplay = !localAccessCollection
        ? CODEX_LOCAL_ACCESS_FALLBACK_API_KEY_MASK
        : localAccessKeyVisible
          ? localAccessCollection.apiKey
          : `${localAccessCollection.apiKey.slice(0, 10)}••••••••••••`;
      const localAccessOAuthBindingLabel = t(
        "codex.api.oauthBinding.label",
        "OAuth 绑定",
      );
      const localAccessOAuthBindingValue = boundLocalAccessOAuthAccount
        ? maskAccountText(
            boundLocalAccessOAuthAccount.account_name ||
              boundLocalAccessOAuthAccount.email ||
              boundLocalAccessOAuthAccount.id,
          )
        : t("codex.api.oauthBinding.unbound", "未绑定");
      const localAccessOAuthBindingLine = `${localAccessOAuthBindingLabel}：${localAccessOAuthBindingValue}`;
      const localAccessBoundOAuthNeedsReauth = Boolean(
        boundLocalAccessOAuthAccount?.requires_reauth,
      );
      const localAccessBoundOAuthIssueText =
        boundLocalAccessOAuthAccount?.reauth_reason?.trim() ||
        t(
          "codex.switchAuth.reauthorizeDescription",
          "当前登录凭据无法自动更新，请重新授权后继续使用。",
        );
      const quotaReserveStatus = localAccessState?.quotaReserveStatus ?? null;
      const quotaReserveWarningLine =
        quotaReserveStatus?.warning &&
        quotaReserveStatus.effectiveWindow &&
        quotaReserveStatus.effectiveRemainingPercent != null &&
        quotaReserveStatus.effectiveReservePercent != null
          ? `${
              quotaReserveStatus.effectiveWindow === "weekly"
                ? t(
                    "codex.localAccess.oauthBinding.quotaReserveWeeklyLabel",
                    "周保留",
                  )
                : t(
                    "codex.localAccess.oauthBinding.quotaReserveHourlyLabel",
                    "5 小时保留",
                  )
            }：${quotaReserveStatus.effectiveRemainingPercent}% / ${quotaReserveStatus.effectiveReservePercent}%`
          : null;
      const localAccessStatusTone = !localAccessCollection
        ? "disabled"
        : localAccessState?.running
          ? "running"
          : localAccessCollection.enabled
            ? "stopped"
            : "disabled";
      const localAccessStatusText = !localAccessCollection
        ? t("codex.localAccess.statusDisabled", "已停用")
        : localAccessState?.running
          ? t("codex.localAccess.statusRunning", "运行中")
          : localAccessCollection.enabled
            ? t("codex.localAccess.statusStopped", "未运行")
            : t("codex.localAccess.statusDisabled", "已停用");
      const isLocalAccessCurrent = localAccessLaunchCurrent;
      const localAccessMemberCountLabel = t("codex.localAccess.accountCount", {
        count: localAccessState?.memberCount ?? 0,
        defaultValue: "{{count}} 个账号",
      });
      return (
        <div
          key="codex-local-access-card"
          className={`codex-account-card folder-inline-card codex-local-access-card codex-local-access-card--${overviewLayoutMode} ${
            isLocalAccessCurrent ? "current" : ""
          } ${showLocalAccessDetails ? "is-expanded" : "is-collapsed"}`}
        >
          <div className="folder-inline-header codex-local-access-header">
            {isGridLocalAccessCard ? (
              <>
                <div className="folder-inline-info">
                  <div className="codex-local-access-title-row">
                    <span className="codex-local-access-current-mode">
                      {t("codex.localAccess.title", "API 服务")}
                    </span>
                  </div>
                </div>
              </>
            ) : (
              <div
                className="codex-local-access-summary-trigger"
                role="button"
                tabIndex={0}
                onClick={() =>
                  setLocalAccessDetailsExpanded((current) => !current)
                }
                onKeyDown={(event) => {
                  if (event.key !== "Enter" && event.key !== " ") return;
                  event.preventDefault();
                  setLocalAccessDetailsExpanded((current) => !current);
                }}
                title={
                  showLocalAccessDetails
                    ? t("codex.localAccess.collapseDetails", "收起详情")
                    : t("codex.localAccess.expandDetails", "展开详情")
                }
              >
                <div className="folder-inline-info">
                  <div className="codex-local-access-title-row">
                    <span className="codex-local-access-current-mode">
                      {t("codex.localAccess.title", "API 服务")}
                    </span>
                    <span className="codex-local-access-summary-text">
                      {localAccessMemberCountLabel}
                    </span>
                  </div>
                </div>
              </div>
            )}
            <div className="codex-local-access-header-actions">
              {isLocalAccessCurrent && (
                <span className="current-tag">{t("codex.current", "当前")}</span>
              )}
              <span
                className={`codex-local-access-status ${localAccessStatusTone}`}
              >
                {localAccessStatusText}
              </span>
              {!isGridLocalAccessCard && (
                <button
                  type="button"
                  className="folder-icon-btn codex-local-access-toggle-btn"
                  onClick={() =>
                    setLocalAccessDetailsExpanded((current) => !current)
                  }
                  title={
                    showLocalAccessDetails
                      ? t("codex.localAccess.collapseDetails", "收起详情")
                      : t("codex.localAccess.expandDetails", "展开详情")
                  }
                  aria-label={
                    showLocalAccessDetails
                      ? t("codex.localAccess.collapseDetails", "收起详情")
                      : t("codex.localAccess.expandDetails", "展开详情")
                  }
                >
                  <ChevronRight
                    size={16}
                    className={`codex-local-access-toggle-icon ${
                      showLocalAccessDetails ? "is-open" : ""
                    }`}
                  />
                </button>
              )}
              <button
                type="button"
                className="folder-icon-btn codex-local-access-close-btn"
                onClick={() => void handleHideLocalAccessEntry()}
                title={t(
                  "codex.localAccess.hideEntryAction",
                  "关闭 API 服务入口",
                )}
                aria-label={t(
                  "codex.localAccess.hideEntryAction",
                  "关闭 API 服务入口",
                )}
              >
                <X size={14} />
              </button>
            </div>
          </div>
  
          {showLocalAccessDetails && (
            <>
              <div className="codex-local-access-meta">
                <div className="codex-local-access-row">
                  <div className="codex-local-access-label codex-local-access-address-select">
                    <SingleSelectDropdown
                      value={selectedLocalAccessAddressKind}
                      options={localAccessAddressOptions}
                      onChange={handleLocalAccessAddressKindChange}
                      menuClassName="codex-local-access-address-menu"
                      menuWidth={92}
                      menuMaxHeight={120}
                      disabled={localAccessAddressOptions.length < 2}
                      ariaLabel={t("codex.localAccess.addressKind", "地址类型")}
                    />
                  </div>
                  <code className="codex-local-access-code" title={baseUrl}>
                    {baseUrl || "-"}
                  </code>
                  <div className="codex-local-access-row-actions">
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() =>
                        void handleCopyLocalAccessValue("baseUrl", baseUrl)
                      }
                      title={t("common.copy", "复制")}
                      disabled={!baseUrl}
                    >
                      {localAccessCopiedField === "baseUrl" ? (
                        <Check size={14} />
                      ) : (
                        <Copy size={14} />
                      )}
                    </button>
                  </div>
                </div>
                <div className="codex-local-access-row">
                  <span className="codex-local-access-label">
                    {t("codex.localAccess.apiKey", "密钥")}
                  </span>
                  <code
                    className="codex-local-access-code"
                    title={localAccessCollection?.apiKey || "-"}
                  >
                    {apiKeyDisplay}
                  </code>
                  <div className="codex-local-access-row-actions">
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() =>
                        setLocalAccessKeyVisible((current) => !current)
                      }
                      title={
                        localAccessKeyVisible
                          ? t("codex.localAccess.hideKey", "隐藏密钥")
                          : t("codex.localAccess.showKey", "显示密钥")
                      }
                      disabled={!localAccessCollection}
                    >
                      {localAccessKeyVisible ? (
                        <EyeOff size={14} />
                      ) : (
                        <Eye size={14} />
                      )}
                    </button>
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() =>
                        void handleCopyLocalAccessValue(
                          "apiKey",
                          localAccessCollection?.apiKey || "",
                        )
                      }
                      title={t("common.copy", "复制")}
                      disabled={!localAccessCollection}
                    >
                      {localAccessCopiedField === "apiKey" ? (
                        <Check size={14} />
                      ) : (
                        <Copy size={14} />
                      )}
                    </button>
                  </div>
                </div>
                <div className="account-sub-line codex-provider-inline-line codex-oauth-binding-line codex-local-access-oauth-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={localAccessOAuthBindingLine}
                  >
                    {localAccessOAuthBindingLine}
                  </span>
                  {localAccessBoundOAuthNeedsReauth && (
                    <span
                      className="codex-status-pill quota-error"
                      title={localAccessBoundOAuthIssueText}
                    >
                      <CircleAlert size={12} />
                      {t("codex.authError.badge", "授权异常")}
                    </span>
                  )}
                  {localAccessBoundOAuthNeedsReauth &&
                    boundLocalAccessOAuthAccount && (
                      <button
                        type="button"
                        className="codex-provider-inline-switch codex-oauth-binding-action"
                        onClick={() =>
                          openCodexAddModal(
                            "oauth",
                            boundLocalAccessOAuthAccount,
                          )
                        }
                        title={t("common.reauthorize", "重新授权")}
                        disabled={localAccessBusy}
                      >
                        <RefreshCw size={11} />
                        {t("common.reauthorize", "重新授权")}
                      </button>
                    )}
                  <button
                    type="button"
                    className="codex-provider-inline-switch codex-oauth-binding-action"
                    onClick={() => openLocalAccessOAuthBindingModal()}
                    title={t("codex.api.oauthBinding.action", "绑定 OAuth")}
                    disabled={localAccessBusy}
                  >
                    <Link2 size={11} />
                    {t("codex.api.oauthBinding.actionShort", "绑定")}
                  </button>
                </div>
                {quotaReserveWarningLine && (
                  <div
                    className={`codex-local-access-quota-reserve-warning ${
                      quotaReserveStatus?.blocked ? "is-blocked" : "is-near"
                    }`}
                    title={t(
                      "codex.localAccess.oauthBinding.quotaReserveDesc",
                      "API 服务仅在 5 小时和周剩余额度均高于保留值时使用该 OAuth 账号。",
                    )}
                  >
                    <CircleAlert size={13} />
                    <span>{quotaReserveWarningLine}</span>
                  </div>
                )}
              </div>
  
              {localAccessQuotaPreviewItems.length > 0 && (
                <div
                  className="codex-local-access-pool-row"
                  aria-label={localAccessQuotaPoolLabels.title}
                >
                  {localAccessQuotaPreviewItems.map((item) => (
                    <div key={item.key} className="codex-local-access-pool-pill">
                      <strong>
                        {item.key} ({item.count})
                      </strong>
                      {item.windows.map((window) => (
                        <span key={window.key}>
                          {formatCodexQuotaPoolWindowLabel(
                            window.label,
                            localAccessQuotaPoolLabels.weekly,
                          )}{" "}
                          {formatCodexQuotaPoolPercent(window.percentage)}
                        </span>
                      ))}
                      {item.balance != null && (
                        <span>
                          {t("codex.localAccess.quotaPool.balance", "余额")}{" "}
                          {formatCodexQuotaPoolBalance(item.balance)}
                        </span>
                      )}
                    </div>
                  ))}
                  {localAccessQuotaHiddenCount > 0 && (
                    <button
                      type="button"
                      className="codex-local-access-pool-more"
                      onClick={() => setShowLocalAccessQuotaStatsModal(true)}
                      title={t(
                        "codex.localAccess.quotaPool.viewFull",
                        "查看完整统计",
                      )}
                      aria-label={t(
                        "codex.localAccess.quotaPool.viewFull",
                        "查看完整统计",
                      )}
                    >
                      +{localAccessQuotaHiddenCount}
                    </button>
                  )}
                </div>
              )}
  
              {localAccessAccountPoolHealthSummary.total > 0 && (
                <button
                  type="button"
                  className={`codex-local-access-health-summary${
                    localAccessAccountPoolHealthHasIssue ? " has-issue" : ""
                  }`}
                  title={t("codex.localAccess.accountPoolHealth.detail", {
                    available: localAccessAccountPoolHealthSummary.available,
                    total: localAccessAccountPoolHealthSummary.total,
                    abnormal: localAccessAccountPoolHealthSummary.abnormal,
                    cooldown: localAccessAccountPoolHealthSummary.cooldown,
                    missing: localAccessAccountPoolHealthSummary.missing,
                    authError: localAccessAccountPoolHealthSummary.authError,
                    quotaLimited:
                      localAccessAccountPoolHealthSummary.quotaLimited,
                    poolUnavailable:
                      localAccessAccountPoolHealthSummary.poolUnavailable,
                    defaultValue:
                      "可用 {{available}}/{{total}}，异常 {{abnormal}}，冷却 {{cooldown}}，缺失 {{missing}}，鉴权 {{authError}}，额度 {{quotaLimited}}",
                  })}
                  onClick={() => setShowLocalAccessHealthModal(true)}
                  aria-label={t(
                    "codex.localAccess.accountPoolHealth.openDetails",
                    "查看异常账号详情",
                  )}
                >
                  <span className="codex-local-access-health-summary-title">
                    {t("codex.localAccess.accountPoolHealth.title", "账号池")}
                  </span>
                  <span className="codex-local-access-health-summary-value">
                    {localAccessAccountPoolHealthSummary.available ===
                      localAccessAccountPoolHealthSummary.total &&
                    localAccessAccountPoolHealthSummary.abnormal === 0 &&
                    localAccessAccountPoolHealthSummary.cooldown === 0 &&
                    localAccessAccountPoolHealthSummary.poolUnavailable === 0
                      ? t("codex.localAccess.accountPoolHealth.allAvailable", {
                          count: localAccessAccountPoolHealthSummary.total,
                          defaultValue: "全部可用 {{count}}",
                        })
                      : t("codex.localAccess.accountPoolHealth.availableRatio", {
                          available:
                            localAccessAccountPoolHealthSummary.available,
                          total: localAccessAccountPoolHealthSummary.total,
                          defaultValue: "可用 {{available}}/{{total}}",
                        })}
                  </span>
                  {(localAccessAccountPoolHealthSummary.abnormal > 0 ||
                    localAccessAccountPoolHealthSummary.cooldown > 0 ||
                    localAccessAccountPoolHealthSummary.poolUnavailable > 0) && (
                    <span className="codex-local-access-health-summary-value">
                      {t("codex.localAccess.accountPoolHealth.issueSummary", {
                        abnormal: localAccessAccountPoolHealthSummary.abnormal,
                        cooldown: localAccessAccountPoolHealthSummary.cooldown,
                        poolUnavailable:
                          localAccessAccountPoolHealthSummary.poolUnavailable,
                        defaultValue:
                          "异常 {{abnormal}} · 池异常 {{poolUnavailable}} · 冷却 {{cooldown}}",
                      })}
                    </span>
                  )}
                </button>
              )}
  
              {localAccessState?.lastError && (
                <div className="quota-error-inline">
                  <CircleAlert size={14} />
                  <span
                    className="quota-error-inline-text"
                    title={summarizeCodexQuotaErrorMessage(
                      localAccessState.lastError,
                    )}
                  >
                    {summarizeCodexQuotaErrorMessage(localAccessState.lastError)}
                  </span>
                  {isVerboseCodexQuotaErrorMessage(
                    localAccessState.lastError,
                  ) && (
                    <button
                      type="button"
                      className="btn btn-sm btn-outline quota-error-action"
                      onClick={() =>
                        openQuotaErrorDetail(
                          t("codex.localAccess.title", "API 服务"),
                          localAccessState.lastError || "",
                        )
                      }
                      title={t("codex.quotaError.viewDetails", "查看详情")}
                    >
                      {t("codex.quotaError.viewDetails", "查看详情")}
                    </button>
                  )}
                  <button
                    type="button"
                    className="folder-icon-btn codex-local-access-error-action"
                    onClick={() => void handleKillLocalAccessPort()}
                    title={t("codex.localAccess.killPortAction", "清理端口")}
                    aria-label={t("codex.localAccess.killPortAction", "清理端口")}
                    disabled={localAccessBusy || !localAccessCollection}
                  >
                    {localAccessPortKilling ? (
                      <RefreshCw size={14} className="loading-spinner" />
                    ) : (
                      <Wrench size={14} />
                    )}
                  </button>
                </div>
              )}
  
              <div className="codex-card-bottom codex-local-access-card-bottom">
                <span className="card-date">
                  {t("codex.localAccess.footerHint", {
                    scope: localAccessScopeLabel,
                    defaultValue: "监听范围：{{scope}}",
                  })}
                </span>
                <CodexSpeedSelect
                  value={apiServiceAppSpeed}
                  onChange={handleApiServiceAppSpeedChange}
                  busy={savingAppSpeedId === CODEX_API_SERVICE_BIND_ID}
                  preferredPlacement="top"
                  ariaLabel={t("codex.speed.title", "速度")}
                />
                <div
                  className={`card-footer codex-local-access-footer ${
                    importApiServiceGuideCount !== null &&
                    !batchImportOpen &&
                    !externalImportProgress.visible
                      ? "has-import-guide"
                      : ""
                  }`}
                >
                  <div className="card-actions">
                    <button
                      className="card-action-btn"
                      onClick={openLocalAccessMemberPicker}
                      title={t("common.shared.addAccount", "添加账号")}
                      disabled={localAccessBusy}
                    >
                      <FolderPlus size={14} />
                    </button>
                    <button
                      className="card-action-btn"
                      onClick={() => void handleLaunchLocalAccessCli()}
                      title={t("codex.cli.quickLaunch", "CLI 快速启动")}
                      disabled={
                        localAccessBusy ||
                        !localAccessCollection ||
                        cliLaunchingAccountId === CODEX_API_SERVICE_BIND_ID
                      }
                    >
                      {cliLaunchingAccountId === CODEX_API_SERVICE_BIND_ID ? (
                        <RefreshCw size={14} className="loading-spinner" />
                      ) : (
                        <Terminal size={14} />
                      )}
                    </button>
                    <button
                      className="card-action-btn"
                      onClick={openLocalAccessPanel}
                      title={t("codex.localAccess.dashboardAction", "服务面板")}
                      disabled={localAccessBusy}
                    >
                      <Database size={14} />
                    </button>
                    <button
                      className="card-action-btn"
                      onClick={openCodexApiServicePage}
                      title={t("codex.apiService.openPage", "进入 API 服务")}
                      disabled={localAccessBusy}
                    >
                      <ExternalLink size={14} />
                    </button>
                    <button
                      className="card-action-btn"
                      onClick={() => void handleQuickRefreshLocalAccessQuota()}
                      title={t("common.shared.refreshQuota", "刷新配额")}
                      disabled={localAccessBusy || !localAccessCollection}
                    >
                      <RotateCw
                        size={14}
                        className={localAccessRefreshing ? "loading-spinner" : ""}
                      />
                    </button>
                    <div className="codex-import-api-service-guide-anchor">
                      {importApiServiceGuideCount !== null &&
                        !batchImportOpen &&
                        !externalImportProgress.visible && (
                          <div
                            className="codex-local-access-gateway-guide codex-import-api-service-anchor-guide"
                            role="dialog"
                            aria-label={t(
                              "codex.importApiService.guideTitle",
                              "账号已加入 API 服务",
                            )}
                            onClick={(event) => event.stopPropagation()}
                          >
                            <button
                              type="button"
                              className="codex-local-access-gateway-guide-close"
                              onClick={() => setImportApiServiceGuideCount(null)}
                              aria-label={t("common.close", "关闭")}
                            >
                              <X size={12} />
                            </button>
                            <div className="codex-local-access-gateway-guide-title">
                              {t(
                                "codex.importApiService.guideTitle",
                                "账号已加入 API 服务",
                              )}
                            </div>
                            <p>
                              {t(
                                "codex.importApiService.guideDescription",
                                "已将 {{count}} 个账号加入 API 服务。点击“启动 API 服务”即可切换并使用。",
                              ).replace(
                                "{{count}}",
                                String(importApiServiceGuideCount),
                              )}
                            </p>
                            <button
                              type="button"
                              className="codex-local-access-gateway-guide-action"
                              onClick={() => setImportApiServiceGuideCount(null)}
                            >
                              {t("codex.importApiService.later", "稍后")}
                            </button>
                          </div>
                        )}
                      <button
                        className="card-action-btn success"
                        onClick={() => {
                          setImportApiServiceGuideCount(null);
                          setLaunchPreviewInstanceId(DEFAULT_CODEX_INSTANCE_ID);
                          setLocalAccessLaunchPreviewOpen(true);
                        }}
                        title={t(
                          "codex.localAccess.activateAction",
                          "启动 API 服务",
                        )}
                        disabled={localAccessBusy || !localAccessCollection}
                      >
                        {localAccessStarting ? (
                          <RefreshCw size={14} className="loading-spinner" />
                        ) : (
                          <Play size={14} />
                        )}
                      </button>
                    </div>
                    <button
                      className={`card-action-btn ${localAccessCollection?.enabled ? "" : "success"}`}
                      onClick={() => void handleQuickToggleLocalAccessEnabled()}
                      title={
                        localAccessCollection?.enabled
                          ? t("codex.localAccess.disableService", "停用服务")
                          : t("codex.localAccess.enableService", "启用服务")
                      }
                      disabled={localAccessBusy || !localAccessCollection}
                    >
                      <Power size={14} />
                    </button>
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
      );
    };
  
    const renderInlineFolderCards = () => {
      const cards: ReactElement[] = [];
      const localAccessCard = renderLocalAccessInlineCard();
      if (localAccessCard) {
        cards.push(localAccessCard);
      }
  
      if (!activeGroupId && !groupByTag) {
        cards.push(
          ...codexGroups.map((group) => {
            const groupAccounts = resolveGroupAccounts(group);
            const previewAccounts = groupAccounts.slice(0, 4);
            const hiddenCount = Math.max(
              0,
              groupAccounts.length - previewAccounts.length,
            );
            const refreshableCount = groupAccounts.filter(
              (account) =>
                !isCodexApiKeyAccount(account) || isCodexNewApiAccount(account),
            ).length;
            const isGroupRefreshing = refreshingGroupId === group.id;
            const groupRefreshDisabled =
              refreshingAll ||
              Boolean(refreshingGroupId) ||
              refreshableCount === 0;
  
            return (
              <div
                key={`codex-folder-${group.id}`}
                className="codex-account-card folder-inline-card codex-group-folder-card"
                onClick={() => handleEnterGroup(group.id)}
              >
                <div className="folder-inline-header">
                  <div className="folder-inline-icon">
                    <FolderOpen size={24} />
                  </div>
                  <div className="folder-inline-info">
                    <span className="folder-inline-name">{group.name}</span>
                    <span className="folder-inline-count">
                      {t("accounts.groups.accountCount", {
                        count: groupAccounts.length,
                      })}
                      {(() => {
                        const minutes =
                          resolveCodexGroupQuotaAutoRefreshMinutes(group);
                        if (minutes === null) return null;
                        const label =
                          minutes === -1
                            ? t("accounts.groups.quotaRefreshOffBadge", "不刷新")
                            : t("accounts.groups.quotaRefreshMinutesBadge", {
                                count: minutes,
                                defaultValue: "{{count}} 分钟",
                              });
                        return (
                          <span
                            className="folder-inline-quota-meta"
                            title={t(
                              "accounts.groups.quotaRefreshPolicyHint",
                              "分组额度刷新为最高优先级；可继承平台设置、自定义间隔或不刷新",
                            )}
                          >
                            · {label}
                          </span>
                        );
                      })()}
                    </span>
                  </div>
                  <button
                    className="folder-icon-btn"
                    title={
                      refreshableCount === 0
                        ? t(
                            "accounts.groups.refreshEmpty",
                            "当前分组没有可刷新的账号",
                          )
                        : !isCodexGroupQuotaRefreshInherit(group)
                          ? t(
                              "accounts.groups.refreshForceHint",
                              "本组自动额度策略非继承时，仍可手动刷新本组",
                            )
                          : t("accounts.groups.refresh", "刷新分组")
                    }
                    aria-label={t("accounts.groups.refresh", "刷新分组")}
                    disabled={groupRefreshDisabled}
                    onClick={(event) => {
                      event.stopPropagation();
                      void handleRefreshGroup(group);
                    }}
                  >
                    <RefreshCw
                      size={14}
                      className={isGroupRefreshing ? "loading-spinner" : ""}
                    />
                  </button>
                  <button
                    className="folder-icon-btn"
                    title={t("accounts.groups.addAccounts")}
                    onClick={(event) => {
                      event.stopPropagation();
                      setGroupQuickAddGroupId(group.id);
                    }}
                  >
                    <FolderPlus size={14} />
                  </button>
                  <button
                    className="folder-icon-btn"
                    title={t("accounts.groups.editTitle")}
                    onClick={(event) => {
                      event.stopPropagation();
                      setShowCodexGroupModal(true);
                    }}
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    className="folder-icon-btn folder-delete-btn"
                    title={t("accounts.groups.deleteTitle")}
                    onClick={(event) => {
                      event.stopPropagation();
                      requestDeleteGroup(group.id, group.name);
                    }}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
                <div className="folder-inline-preview">
                  {previewAccounts.length === 0 ? (
                    <div className="folder-preview-item more">
                      {t("accounts.groups.accountPickerEmpty")}
                    </div>
                  ) : (
                    previewAccounts.map((account) => {
                      const presentation = resolvePresentation(account);
                      return (
                        <div
                          key={`${group.id}-${account.id}`}
                          className="folder-preview-item"
                        >
                          <span
                            className="folder-preview-email"
                            title={maskAccountText(presentation.displayName)}
                          >
                            {maskAccountText(presentation.displayName)}
                          </span>
                          <span
                            className={`tier-badge ${presentation.planClass || "unknown"}`}
                          >
                            {presentation.planLabel}
                          </span>
                          <button
                            type="button"
                            className="folder-preview-remove-btn"
                            onClick={(event) => {
                              event.stopPropagation();
                              void handleRemoveSingleFromGroup(
                                group.id,
                                account.id,
                              );
                            }}
                            title={t("accounts.groups.removeFromGroup")}
                            aria-label={`${t("accounts.groups.removeFromGroup")}: ${maskAccountText(presentation.displayName)}`}
                            disabled={removingGroupAccountIds.has(account.id)}
                          >
                            <LogOut size={12} />
                          </button>
                        </div>
                      );
                    })
                  )}
                  {hiddenCount > 0 && (
                    <div className="folder-preview-item more">+{hiddenCount}</div>
                  )}
                </div>
              </div>
            );
          }),
        );
      }
  
      return cards.length > 0 ? cards : null;
    };
  
    const renderTableRows = (items: typeof filteredAccounts, groupKey?: string) =>
      items.map((account) => {
        const presentation = resolvePresentation(account);
        const meta = resolveAccountMeta(account);
        const isCurrent = overviewCurrentAccountId === account.id;
        const isApiKeyAccount = isCodexApiKeyAccount(account);
        const serverRevokedReauth = isCodexServerRevokedReauth(account);
        const refreshTokenReusedState = isCodexRefreshTokenReusedAccount(account);
        const switchOrLaunchBlockedReason =
          getCodexSwitchOrLaunchBlockedReason(account);
        const isPendingOAuthAccount = isPendingOAuthCodexAccount(account);
        const isNewApiAccount = isCodexNewApiAccount(account);
        const isChatCompletionsApiKey =
          isCodexChatCompletionsApiKeyAccount(account);
        const isEditingApiKeyName =
          isApiKeyAccount && editingApiKeyNameId === account.id;
        const isSavingApiKeyName = savingApiKeyNameId === account.id;
        const planClass = presentation.planClass || "unknown";
        const quotaItems = applyWindowStatsToQuotaItems(
          account,
          resolveVisibleQuotaItems(
            presentation,
            isApiKeyAccount,
            isNewApiAccount,
          ),
        );
        const reauthErrorMeta = resolveQuotaErrorMeta(
          !refreshTokenReusedState && account.requires_reauth && account.reauth_reason
            ? {
                message: account.reauth_reason,
                timestamp: account.token_updated_at || account.last_used,
              }
            : undefined,
        );
        const quotaErrorMeta = resolveQuotaErrorMeta(
          refreshTokenReusedState ? undefined : account.quota_error,
        );
        const accountIssueMeta = reauthErrorMeta.rawMessage
          ? reauthErrorMeta
          : quotaErrorMeta;
        const hasQuotaError = Boolean(accountIssueMeta.rawMessage);
        const isRefreshTokenNotice = isCodexRefreshTokenNoticeOnly(account);
        const isClientReauthNotice =
          !serverRevokedReauth &&
          !refreshTokenReusedState &&
          isCodexClientReauthNoticeOnly(account);
        const isQuotaRefreshNotice =
          isClientReauthNotice ||
          (!reauthErrorMeta.rawMessage &&
            quotaErrorMeta.isRefreshRequestFailure &&
            !quotaErrorMeta.statusCode &&
            !quotaErrorMeta.errorCode);
        const accountIssueDisplayText = isRefreshTokenNotice
          ? t(
              "codex.quotaError.authRefreshDeferred",
              "refresh_token 已失效；当前 access_token 仍可用于 API 服务，但不能切换到官方客户端，请重新授权后再切号。",
            )
          : accountIssueMeta.displayText;
        const accountIssueBadge = isRefreshTokenNotice
          ? t("codex.authError.badge", "授权异常")
          : isQuotaRefreshNotice
            ? t("codex.quotaError.refreshFailedBadge", "刷新失败")
            : reauthErrorMeta.rawMessage
              ? t("codex.authError.badge", "授权异常")
              : accountIssueMeta.statusCode ||
                t("codex.quotaError.badge", "配额异常");
        const showReauthorizeAction =
          !isApiKeyAccount &&
          !isRefreshTokenNotice &&
          (isPendingOAuthAccount ||
            (hasQuotaError && shouldOfferReauthorizeAction(accountIssueMeta)));
        const accountIdText =
          meta.chatgptAccountId &&
          meta.chatgptAccountId !== t("common.none", "暂无")
            ? meta.chatgptAccountId
            : meta.userId;
        const signInLine = `${meta.signedInWithText} | ${accountIdLabel}: ${accountIdText}`;
        const apiProviderName = resolveApiProviderDisplayName(account);
        const apiProviderLine = `${t("codex.api.provider.label", "供应商")}：${apiProviderName}`;
        const apiBaseUrlText = (account.api_base_url || "").trim() || "-";
        const apiBaseUrlLine = `${t("codex.api.baseUrl", "Base URL")}：${apiBaseUrlText}`;
        const apiKeyUsageProvider = resolveUsageProviderForApiKeyAccount(account);
        const isSponsorApiKeyAccount =
          isApiKeyAccount &&
          isSponsorModelProvider(
            apiKeyUsageProvider,
            sponsorApiProviderTemplates,
          );
        const apiKeyUsageMode = resolveApiKeyUsageMode(
          apiKeyUsageMap[account.id]?.summary,
        );
        const showApiKeyUsagePanel = shouldShowCodexApiKeyUsagePanel(
          account,
          hideRelayQuota,
        );
        const isDetailedUsageAccount =
          showApiKeyUsagePanel &&
          (apiKeyUsageMode === "sub2api" ||
            apiKeyUsageMode === "cockpit_tools" ||
            apiKeyUsageProvider?.integrationType === "sub2api" ||
            apiKeyUsageProvider?.integrationType === "cockpit_tools");
        const isTokenPlanUsageAccount =
          showApiKeyUsagePanel && apiKeyUsageMode === "token_plan";
        const isQuotaAwareApiKeyAccount =
          showApiKeyUsagePanel &&
          !isSponsorApiKeyAccount &&
          (apiKeyUsageMode !== null ||
            isDeepSeekAccount(account) ||
            apiKeyUsageProvider?.integrationType === "new_api" ||
            apiKeyUsageProvider?.integrationType === "sub2api" ||
            apiKeyUsageProvider?.integrationType === "cockpit_tools");
        const displayPlanClass = isSponsorApiKeyAccount
          ? "sponsor-api"
          : isQuotaAwareApiKeyAccount
            ? "new-api-exclusive"
            : planClass;
        const displayPlanLabel = isSponsorApiKeyAccount
          ? apiProviderName
          : presentation.planLabel;
        const cockpitApiAccountBalanceText =
          isNewApiAccount && !hideRelayQuota
            ? resolveCockpitApiAccountBalanceText(account)
            : null;
        const isInLocalAccess = localAccessAccountIdSet.has(account.id);
        const subscriptionInfo = resolveSubscriptionPresentation(account);
        const showSubscriptionRefreshAction =
          !isApiKeyAccount &&
          !isPendingOAuthAccount &&
          (subscriptionInfo.bucket === "missing" ||
            subscriptionInfo.bucket === "expired");
        const isSubscriptionRefreshPending =
          refreshingSubscriptionAccountId === account.id ||
          refreshing === account.id;
        const resetCreditControls = renderResetCreditControls(account);
        return (
          <tr
            key={groupKey ? `${groupKey}-${account.id}` : account.id}
            className={`${isCurrent ? "current" : ""} ${isPendingOAuthAccount ? "pending-auth" : ""} ${isNewApiAccount ? "new-api-exclusive" : ""} ${isQuotaAwareApiKeyAccount ? "api-key-usage-account" : ""} ${isSponsorApiKeyAccount ? "sponsor-api-account" : ""}`}
          >
            <td>
              <input
                type="checkbox"
                checked={selected.has(account.id)}
                onChange={() => handleToggleOverviewAccount(account.id)}
              />
            </td>
            <td>
              <div className="account-cell">
                <div className="account-main-line">
                  {isEditingApiKeyName ? (
                    <input
                      className="account-email-text inline-name-editor"
                      value={editingApiKeyNameValue}
                      onChange={(event) =>
                        setEditingApiKeyNameValue(event.target.value)
                      }
                      onBlur={() => void handleSubmitInlineRename(account)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          void handleSubmitInlineRename(account);
                        } else if (event.key === "Escape") {
                          event.preventDefault();
                          inlineRenameDiscardRef.current = true;
                          clearInlineRename();
                        }
                      }}
                      disabled={isSavingApiKeyName}
                      autoFocus
                    />
                  ) : (
                    <span
                      className={`account-email-text ${isApiKeyAccount ? "editable" : ""}`}
                      title={maskAccountText(presentation.displayName)}
                      onDoubleClick={() => handleAccountNameDoubleClick(account)}
                    >
                      {maskAccountText(presentation.displayName)}
                    </span>
                  )}
                  {isCurrent && (
                    <span className="mini-tag current">
                      {t("codex.current", "当前")}
                    </span>
                  )}
                  {renderAccountSpeedSelect(account, true)}
                </div>
                {(meta.accountContextText ||
                  isInLocalAccess ||
                  (!isApiKeyAccount && hasCodexAccountNoteDetails(account)) ||
                  resetCreditControls) && (
                  <div className="account-sub-line codex-account-meta-inline">
                    {meta.accountContextText && (
                      <span
                        className="codex-login-subline"
                        title={meta.accountContextText}
                      >
                        Team Name：{meta.accountContextText}
                      </span>
                    )}
                    {isInLocalAccess && (
                      <button
                        type="button"
                        className="group-account-badge codex-local-access-inline-remove"
                        onClick={(event) => {
                          event.stopPropagation();
                          void handleRemoveLocalAccessAccount(account.id);
                        }}
                        disabled={addingLocalAccessAccountId !== null}
                        title={t(
                          "codex.localAccess.removeAction",
                          "移除 API 服务",
                        )}
                        aria-label={t(
                          "codex.localAccess.removeAction",
                          "移除 API 服务",
                        )}
                      >
                        {addingLocalAccessAccountId === account.id ? (
                          <RefreshCw size={11} className="loading-spinner" />
                        ) : (
                          <Link2 size={11} />
                        )}
                        {t("codex.localAccess.removeAction", "移除 API 服务")}
                      </button>
                    )}
                    {!isApiKeyAccount && renderAccountNoteButton(account)}
                    {resetCreditControls}
                  </div>
                )}
                {!isApiKeyAccount && (
                  <div className="account-sub-line codex-account-meta-inline">
                    <span className="codex-login-subline" title={signInLine}>
                      {meta.signedInWithText} | {accountIdLabel}:{" "}
                      {maskAccountText(accountIdText)}
                    </span>
                  </div>
                )}
                {isApiKeyAccount && (
                  <>
                    <div className="account-sub-line codex-account-meta-inline">
                      {renderApiKeyRevealLine(account)}
                    </div>
                    {renderOAuthBindingLine(account)}
                    <div className="account-sub-line codex-account-meta-inline codex-provider-inline-line">
                      <span
                        className="codex-login-subline codex-provider-inline-text"
                        title={apiProviderLine}
                      >
                        {apiProviderLine}
                      </span>
                      {!isNewApiAccount && (
                        <button
                          type="button"
                          className="codex-provider-inline-switch"
                          onClick={() => openQuickSwitchProviderModal(account)}
                          title={t("codex.quickSwitch.action", "快速切换供应商")}
                        >
                          {t("codex.quickSwitch.inlineAction", "切换")}
                        </button>
                      )}
                    </div>
                    <div className="account-sub-line codex-account-meta-inline codex-provider-inline-line">
                      <span
                        className="codex-login-subline codex-provider-inline-text"
                        title={apiBaseUrlLine}
                      >
                        {apiBaseUrlLine}
                      </span>
                      {(isDetailedUsageAccount || isTokenPlanUsageAccount) && (
                        <button
                          type="button"
                          className="codex-provider-inline-switch"
                          onClick={() =>
                            setApiKeyUsageDetailAccountId(account.id)
                          }
                          title={t(
                            "codex.modelProviders.usage.detailTitle",
                            "服务面板",
                          )}
                        >
                          {t("common.detail", "详情")}
                        </button>
                      )}
                    </div>
                  </>
                )}
                {hasQuotaError && (
                  <div className="account-sub-line">
                    <span
                      className={`codex-status-pill ${isQuotaRefreshNotice ? "quota-refresh" : "quota-error"}`}
                      title={accountIssueDisplayText}
                    >
                      {isQuotaRefreshNotice ? (
                        <Info size={12} />
                      ) : (
                        <CircleAlert size={12} />
                      )}
                      {accountIssueBadge}
                    </span>
                  </div>
                )}
              </div>
            </td>
            <td>
              <span className={`tier-badge ${displayPlanClass}`}>
                {displayPlanLabel}
              </span>
            </td>
            <td>
              {isApiKeyAccount ? (
                isNewApiAccount ? (
                  <div
                    className="codex-subscription-table-cell"
                    title={presentation.planLabel}
                  >
                    <span className="codex-subscription-badge new-api-exclusive">
                      {presentation.planLabel}
                    </span>
                  </div>
                ) : (
                  <span className="codex-subscription-table-empty">-</span>
                )
              ) : (
                <div
                  className="codex-subscription-table-cell"
                  title={subscriptionInfo.titleText}
                >
                  <div className="codex-subscription-table-head">
                    <span
                      className={`codex-subscription-badge ${subscriptionInfo.tone}`}
                    >
                      {subscriptionInfo.valueText}
                    </span>
                    {showSubscriptionRefreshAction && (
                      <button
                        type="button"
                        className="codex-subscription-refresh-btn"
                        onClick={() =>
                          void handleRefreshSubscriptionInfo(account.id)
                        }
                        disabled={isSubscriptionRefreshPending}
                        title={t("common.refresh", "刷新")}
                        aria-label={t("common.refresh", "刷新")}
                      >
                        {t("common.refresh", "刷新")}
                      </button>
                    )}
                  </div>
                  {subscriptionInfo.timestampMs != null && (
                    <span className="codex-subscription-date">
                      {subscriptionInfo.detailText}
                    </span>
                  )}
                </div>
              )}
            </td>
            <td>
              {showApiKeyUsagePanel ? (
                renderApiKeyUsagePanel(account, apiKeyUsageProvider, "table")
              ) : isChatCompletionsApiKey ? (
                <span className="codex-subscription-table-empty">-</span>
              ) : (
                <>
                  <div className="quota-grid">
                    {cockpitApiAccountBalanceText && (
                      <div className="codex-account-balance-line table">
                        <span>
                          {t(
                            "codex.modelProviders.usage.accountBalance",
                            "账户余额",
                          )}
                          ：
                        </span>
                        <strong>{cockpitApiAccountBalanceText}</strong>
                      </div>
                    )}
                    <CodexQuotaMiniRows items={quotaItems} t={t} />
                    {quotaItems.length === 0 && !cockpitApiAccountBalanceText && (
                      <span style={{ color: "var(--text-muted)", fontSize: 13 }}>
                        {t("common.shared.quota.noData", "暂无配额数据")}
                      </span>
                    )}
                  </div>
                  {!isPendingOAuthAccount &&
                    hasQuotaError &&
                    renderQuotaErrorInline({
                      accountName: presentation.displayName,
                      displayText: accountIssueDisplayText,
                      rawMessage: accountIssueMeta.rawMessage,
                      isVerbose: accountIssueMeta.isVerbose,
                      detailSummary: isRefreshTokenNotice
                        ? accountIssueDisplayText
                        : undefined,
                      detailReauthorizeAccountId:
                        isRefreshTokenNotice || showReauthorizeAction
                          ? account.id
                          : undefined,
                      isRefreshNotice: isQuotaRefreshNotice,
                      showReauthorize: showReauthorizeAction,
                      onReauthorize: () => openCodexAddModal("oauth", account),
                      table: true,
                    })}
                  {isPendingOAuthAccount && (
                    <div className="quota-error-inline table quota-refresh-notice">
                      <Info size={12} />
                      <span>
                        {t("common.shared.quota.noData", "暂无配额数据")}
                      </span>
                      <button
                        className="btn btn-sm btn-outline"
                        onClick={() => openCodexAddModal("oauth", account)}
                        title={t("common.shared.addModal.oauth", "OAuth 授权")}
                      >
                        {t("common.shared.addModal.oauth", "OAuth 授权")}
                      </button>
                    </div>
                  )}
                </>
              )}
            </td>
            <td className="sticky-action-cell table-action-cell">
              <div className="action-buttons">
                <button
                  className="action-btn"
                  onClick={() => void handleLaunchCodexCli(account)}
                  disabled={
                    cliLaunchingAccountId === account.id ||
                    Boolean(switchOrLaunchBlockedReason)
                  }
                  title={
                    switchOrLaunchBlockedReason ||
                    t("codex.cli.quickLaunch", "CLI 快速启动")
                  }
                >
                  {cliLaunchingAccountId === account.id ? (
                    <RefreshCw size={14} className="loading-spinner" />
                  ) : (
                    <Terminal size={14} />
                  )}
                </button>
                {renderAddLocalAccessAccountButton(account, "action-btn")}
                {isNewApiAccount && (
                  <button
                    className="action-btn"
                    onClick={() => setCockpitApiPanelAccountId(account.id)}
                    title={t("codex.cockpitApi.servicePanel", "服务面板")}
                  >
                    <Database size={14} />
                  </button>
                )}
                <button
                  className="action-btn"
                  onClick={() => openTagModal(account.id)}
                  title={t("accounts.editTags", "编辑标签")}
                >
                  <Tag size={14} />
                </button>
                {!isApiKeyAccount && !isNewApiAccount && (
                  <button
                    className={`action-btn ${hasCodexAccountNoteDetails(account) ? "active" : ""}`}
                    onClick={() => openAccountNoteModal(account)}
                    title={
                      getCodexAccountNoteTitle(account, "") ||
                      t("codex.accountNote.emptyTitle", "填写账号备注")
                    }
                    aria-label={t("codex.accountNote.title", "账号备注")}
                  >
                    <FileText size={14} />
                  </button>
                )}
                {isApiKeyAccount && !isNewApiAccount && (
                  <button
                    className="action-btn"
                    onClick={() => openApiKeyCredentialsModal(account)}
                    title={t("instances.actions.edit", "编辑")}
                  >
                    <Pencil size={14} />
                  </button>
                )}
                <button
                  className={`action-btn ${!isCurrent ? "success" : ""}`}
                  onClick={() => handleSwitch(account.id)}
                  disabled={!!switching || Boolean(switchOrLaunchBlockedReason)}
                  title={switchOrLaunchBlockedReason || t("codex.switch", "切换")}
                >
                  {switching === account.id ? (
                    <RefreshCw size={14} className="loading-spinner" />
                  ) : (
                    <Play size={14} />
                  )}
                </button>
                {!isPendingOAuthAccount &&
                  (!isApiKeyAccount ||
                    isNewApiAccount ||
                    canRefreshApiKeyUsage(account, apiKeyUsageProvider)) && (
                    <button
                      className="action-btn"
                      onClick={() =>
                        canRefreshApiKeyUsage(account, apiKeyUsageProvider)
                          ? void refreshApiKeyUsage(account, apiKeyUsageProvider)
                          : handleRefresh(account.id)
                      }
                      disabled={
                        canRefreshApiKeyUsage(account, apiKeyUsageProvider)
                          ? apiKeyUsageMap[account.id]?.loading === true
                          : refreshing === account.id
                      }
                      title={t("common.shared.refreshQuota", "刷新配额")}
                    >
                      <RotateCw
                        size={14}
                        className={
                          canRefreshApiKeyUsage(account, apiKeyUsageProvider)
                            ? apiKeyUsageMap[account.id]?.loading === true
                              ? "loading-spinner"
                              : ""
                            : refreshing === account.id
                              ? "loading-spinner"
                              : ""
                        }
                      />
                    </button>
                  )}
                <button
                  className="action-btn"
                  onClick={() =>
                    handleExportByIds(
                      [account.id],
                      resolveSingleExportBaseName(account),
                    )
                  }
                  title={t("common.shared.export.title", "导出")}
                >
                  <Upload size={14} />
                </button>
                <button
                  className="action-btn danger"
                  onClick={() => handleDelete(account.id)}
                  title={t("common.delete", "删除")}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </td>
          </tr>
        );
      });
  
    const renderGroupTableRows = () => {
      if (activeGroupId || groupByTag) return null;
  
      const rows: ReactElement[] = codexGroups.map((group) => {
        const groupAccounts = resolveGroupAccounts(group);
        const refreshableCount = groupAccounts.filter(
          (account) =>
            !isCodexApiKeyAccount(account) || isCodexNewApiAccount(account),
        ).length;
        const isGroupRefreshing = refreshingGroupId === group.id;
        const groupRefreshDisabled =
          refreshingAll || Boolean(refreshingGroupId) || refreshableCount === 0;
        return (
          <tr
            key={`folder-row-${group.id}`}
            className="folder-table-row"
            style={{ cursor: "pointer" }}
            onClick={() => handleEnterGroup(group.id)}
          >
            <td />
            <td colSpan={4}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <FolderOpen size={16} style={{ color: "var(--primary)" }} />
                <strong>{group.name}</strong>
                <span style={{ color: "var(--text-muted)", fontSize: 12 }}>
                  {t("accounts.groups.accountCount", {
                    count: groupAccounts.length,
                  })}
                  {(() => {
                    const minutes =
                      resolveCodexGroupQuotaAutoRefreshMinutes(group);
                    if (minutes === null) return null;
                    const label =
                      minutes === -1
                        ? t("accounts.groups.quotaRefreshOffBadge", "不刷新")
                        : t("accounts.groups.quotaRefreshMinutesBadge", {
                            count: minutes,
                            defaultValue: "{{count}} 分钟",
                          });
                    return (
                      <span
                        className="folder-inline-quota-meta"
                        title={t(
                          "accounts.groups.quotaRefreshPolicyHint",
                          "分组额度刷新为最高优先级；可继承平台设置、自定义间隔或不刷新",
                        )}
                      >
                        {" "}
                        · {label}
                      </span>
                    );
                  })()}
                </span>
              </div>
            </td>
            <td>
              <div className="folder-table-actions">
                <button
                  className="folder-icon-btn"
                  title={
                    refreshableCount === 0
                      ? t(
                          "accounts.groups.refreshEmpty",
                          "当前分组没有可刷新的账号",
                        )
                      : !isCodexGroupQuotaRefreshInherit(group)
                        ? t(
                            "accounts.groups.refreshForceHint",
                            "本组自动额度策略非继承时，仍可手动刷新本组",
                          )
                        : t("accounts.groups.refresh", "刷新分组")
                  }
                  aria-label={t("accounts.groups.refresh", "刷新分组")}
                  disabled={groupRefreshDisabled}
                  onClick={(event) => {
                    event.stopPropagation();
                    void handleRefreshGroup(group);
                  }}
                >
                  <RefreshCw
                    size={14}
                    className={isGroupRefreshing ? "loading-spinner" : ""}
                  />
                </button>
                <button
                  className="folder-icon-btn"
                  title={t("accounts.groups.addAccounts")}
                  onClick={(event) => {
                    event.stopPropagation();
                    setGroupQuickAddGroupId(group.id);
                  }}
                >
                  <FolderPlus size={14} />
                </button>
                <button
                  className="folder-icon-btn"
                  title={t("accounts.groups.editTitle")}
                  onClick={(event) => {
                    event.stopPropagation();
                    setShowCodexGroupModal(true);
                  }}
                >
                  <Pencil size={14} />
                </button>
                <button
                  className="folder-icon-btn folder-delete-btn"
                  title={t("accounts.groups.deleteTitle")}
                  onClick={(event) => {
                    event.stopPropagation();
                    requestDeleteGroup(group.id, group.name);
                  }}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </td>
          </tr>
        );
      });
  
      return rows.length > 0 ? rows : null;
    };
  
    const inlineFolderCards = renderInlineFolderCards();
    const hasGroupEntryCards = Boolean(
      inlineFolderCards && inlineFolderCards.length > 0,
    );
    const showOverviewSelectionBar = paginatedAccounts.length > 0;
    const externalImportRunning = [
      "receiving",
      "fetching",
      "parsing",
      "importing",
      "refreshing",
    ].includes(externalImportProgress.status);
    const externalImportStepIndex = (() => {
      switch (externalImportProgress.status) {
        case "receiving":
          return 0;
        case "fetching":
          return 1;
        case "parsing":
          return 2;
        case "importing":
          return 3;
        case "refreshing":
          return 4;
        case "success":
        case "partial":
        case "error":
          return 5;
        default:
          return -1;
      }
    })();
    const externalImportSteps = [
      t("common.shared.externalImport.stepReceive", "接收导入请求"),
      t("common.shared.externalImport.stepFetch", "获取导入包"),
      t("common.shared.externalImport.stepParse", "解析 Codex JSON"),
      t("common.shared.externalImport.stepImport", "导入账号"),
      t("common.shared.externalImport.stepRefresh", "刷新账号列表"),
    ];
    const externalImportPercent = Math.max(
      0,
      Math.min(100, Math.round(externalImportProgress.progress)),
    );
    const handleCopyExternalImportErrors = async () => {
      const content = externalImportProgress.failures
        .map((item) => `${item.index}. ${item.label}: ${item.error}`)
        .join("\n");
      if (!content) return;
      await navigator.clipboard.writeText(content).catch(() => {});
      setMessage({
        text: t("common.shared.externalImport.copied", "已复制"),
        tone: "success",
      });
    };
    const handleViewExternalImportAccounts = () => {
      setActiveTab("overview");
      closeExternalImportProgressModal();
    };
  
    useEffect(() => {
      if (externalImportRunning) {
        setExternalImportSyncError(null);
      }
    }, [externalImportRunning]);
  
    useEffect(() => {
      if (importApiServiceGuideCount === null) return;
      setActiveTab("overview");
      setLocalAccessDetailsExpanded(true);
    }, [importApiServiceGuideCount]);
  
    const renderApiKeyUsageDetailModal = () => {
      const account = apiKeyUsageDetailAccount;
      if (!account) return null;
      const state = apiKeyUsageMap[account.id];
      const summary = state?.summary;
      const provider = resolveUsageProviderForApiKeyAccount(account);
      const usageMode =
        resolveApiKeyUsageMode(summary) ??
        (provider?.integrationType === "sub2api" ||
        provider?.integrationType === "cockpit_tools"
          ? provider.integrationType
          : null);
      if (!usageMode) return null;
      const formatCockpitToolsPercent = (key: string) => {
        const value = formatApiKeyUsageDetailByKey(summary, key);
        return value === "-" ? value : `${value}%`;
      };
      const coreDetailKeys =
        usageMode === "new_api"
          ? new Set(["mode", "totalGranted", "totalAvailable", "expiresAt"])
          : usageMode === "sub2api"
            ? new Set(["mode", "remaining", "todayRequests", "todayTokens"])
            : usageMode === "deepseek"
              ? new Set([
                  "mode",
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
                  ? new Set(["mode", "remaining", "planName", "expiresAt"])
                  : new Set<string>();
      const details = (summary?.details ?? []).filter(
        (item) => !coreDetailKeys.has(item.key),
      );
      const visible = visibleApiKeyAccountIds.has(account.id);
      const apiKeyDisplay = resolveApiKeyDisplayText(account, visible);
      const baseUrl =
        provider?.baseUrl.trim() || (account.api_base_url || "").trim() || "-";
      const usedPercent = formatApiKeyUsagePercent(summary);
      const newApiQuota = resolveNewApiQuotaSnapshot(summary);
      const summaryDetails =
        usageMode === "new_api"
          ? [
              {
                key: "totalGranted",
                label: t(
                  "codex.modelProviders.usage.fields.totalGranted",
                  "授予额度",
                ),
                value: formatApiKeyUsageMoney(newApiQuota.granted, summary?.unit),
              },
              {
                key: "totalAvailable",
                label: t(
                  "codex.modelProviders.usage.fields.totalAvailable",
                  "可用额度",
                ),
                value: formatApiKeyUsageMoney(
                  newApiQuota.available,
                  summary?.unit,
                ),
              },
              {
                key: "expiresAt",
                label: t(
                  "codex.modelProviders.usage.fields.expiresAt",
                  "过期时间",
                ),
                value:
                  newApiQuota.expiresAt != null
                    ? formatApiKeyUsageDetailValue({
                        key: "expiresAt",
                        value: String(newApiQuota.expiresAt),
                      })
                    : "-",
              },
            ]
          : usageMode === "cockpit_tools"
            ? [
                {
                  key: "fiveHourRemainingPercent",
                  label: t(
                    "codex.modelProviders.usage.fields.fiveHourRemainingPercent",
                    "5h 剩余",
                  ),
                  value: formatCockpitToolsPercent(
                    "fiveHourRemainingPercent",
                  ),
                },
                {
                  key: "weeklyRemainingPercent",
                  label: t(
                    "codex.modelProviders.usage.fields.weeklyRemainingPercent",
                    "周剩余",
                  ),
                  value: formatCockpitToolsPercent(
                    "weeklyRemainingPercent",
                  ),
                },
                {
                  key: "apiKeyBalance",
                  label: t(
                    "codex.modelProviders.usage.fields.apiKeyBalance",
                    "API_KEY 余额",
                  ),
                  value: formatApiKeyUsageDetailByKey(summary, "apiKeyBalance"),
                },
                {
                  key: "accountCount",
                  label: t(
                    "codex.modelProviders.usage.fields.accountCount",
                    "账号池",
                  ),
                  value: formatApiKeyUsageDetailByKey(summary, "accountCount"),
                },
                {
                  key: "availableAccountCount",
                  label: t(
                    "codex.modelProviders.usage.fields.availableAccountCount",
                    "可用账号",
                  ),
                  value: formatApiKeyUsageDetailByKey(
                    summary,
                    "availableAccountCount",
                  ),
                },
              ]
            : usageMode === "token_plan"
              ? [
                  {
                    key: "remaining",
                    label: t(
                      "codex.modelProviders.usage.fields.remaining",
                      "Remaining",
                    ),
                    value: formatApiKeyUsageQuotaValue(
                      summary,
                      summary?.quotaRemaining ?? summary?.remaining,
                    ),
                  },
                  {
                    key: "planName",
                    label: t(
                      "codex.modelProviders.usage.fields.planName",
                      "Plan",
                    ),
                    value: summary?.planName || "-",
                  },
                  {
                    key: "expiresAt",
                    label: t(
                      "codex.modelProviders.usage.fields.expiresAt",
                      "Next Reset",
                    ),
                    value: formatApiKeyUsageDetailByKey(
                      summary,
                      findApiKeyUsageDetail(summary, "intervalExpiresAt")
                        ? "intervalExpiresAt"
                        : findApiKeyUsageDetail(summary, "weeklyExpiresAt")
                          ? "weeklyExpiresAt"
                          : "expiresAt",
                    ),
                  },
                ]
              : usageMode === "sub2api"
                ? [
                  {
                    key: "accountBalance",
                    label: t(
                      "codex.modelProviders.usage.accountBalance",
                      "账户余额",
                    ),
                    value: formatApiKeyUsageQuotaValue(
                      summary,
                      summary?.remaining ??
                        summary?.balance ??
                        summary?.quotaRemaining,
                    ),
                  },
                  {
                    key: "todayRequests",
                    label: t(
                      "codex.modelProviders.usage.fields.todayRequests",
                      "今日请求",
                    ),
                    value: summary
                      ? formatCockpitApiInteger(summary.todayRequests ?? 0)
                      : "-",
                  },
                  {
                    key: "todayTokens",
                    label: t(
                      "codex.modelProviders.usage.fields.todayTokens",
                      "今日 Token",
                    ),
                    value: summary
                      ? formatCockpitApiTokenCount(summary.todayTotalTokens ?? 0)
                      : "-",
                  },
                ]
              : usageMode === "deepseek"
                ? [
                    {
                      key: "totalBalance",
                      label: t(
                        "codex.modelProviders.usage.fields.totalBalance",
                        "总余额",
                      ),
                      value: formatApiKeyUsageMoney(
                        summary?.balance,
                        summary?.unit,
                      ),
                    },
                    {
                      key: "grantedBalance",
                      label: t(
                        "codex.modelProviders.usage.fields.grantedBalance",
                        "赠金余额",
                      ),
                      value: formatApiKeyUsageDetailByKey(
                        summary,
                        "grantedBalance",
                      ),
                    },
                    {
                      key: "toppedUpBalance",
                      label: t(
                        "codex.modelProviders.usage.fields.toppedUpBalance",
                        "充值余额",
                      ),
                      value: formatApiKeyUsageDetailByKey(
                        summary,
                        "toppedUpBalance",
                      ),
                    },
                  ]
                : [];
      const summaryGridClassName =
        usageMode === "sub2api" ||
        usageMode === "new_api" ||
        usageMode === "cockpit_tools" ||
        usageMode === "token_plan"
          ? "cockpit-api-summary-grid compact"
          : "cockpit-api-summary-grid";
  
      return (
        <div className="modal-overlay">
          <div
            className="modal-content cockpit-api-panel-modal codex-api-key-usage-detail-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header cockpit-api-panel-header">
              <div>
                <h2>{t("codex.modelProviders.usage.detailTitle", "服务面板")}</h2>
                <span className="cockpit-api-panel-subtitle">
                  {maskAccountText(resolvePresentation(account).displayName)}
                  {provider ? ` · ${provider.name}` : ""}
                </span>
              </div>
              <button
                className="modal-close"
                onClick={() => setApiKeyUsageDetailAccountId(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="cockpit-api-panel-body">
              <section className="cockpit-api-connection-card">
                <div className="cockpit-api-connection-row">
                  <span>{t("codex.localAccess.baseUrl", "地址")}</span>
                  <code title={baseUrl}>{baseUrl}</code>
                  <button
                    type="button"
                    className="folder-icon-btn cockpit-api-icon-btn"
                    onClick={() =>
                      void navigator.clipboard.writeText(baseUrl).catch(() => {})
                    }
                    title={t("common.copy", "复制")}
                  >
                    <Copy size={14} />
                  </button>
                </div>
                <div className="cockpit-api-connection-row">
                  <span>{t("codex.localAccess.apiKey", "密钥")}</span>
                  <code title={visible ? account.openai_api_key || "" : ""}>
                    {apiKeyDisplay}
                  </code>
                  <div className="cockpit-api-connection-actions">
                    <button
                      type="button"
                      className="folder-icon-btn cockpit-api-icon-btn"
                      onClick={() => toggleAccountApiKeyVisible(account.id)}
                      title={
                        visible
                          ? t("codex.localAccess.hideKey", "隐藏密钥")
                          : t("codex.localAccess.showKey", "显示密钥")
                      }
                    >
                      {visible ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                    <button
                      type="button"
                      className="folder-icon-btn cockpit-api-icon-btn"
                      onClick={() =>
                        void navigator.clipboard
                          .writeText(account.openai_api_key || "")
                          .catch(() => {})
                      }
                      title={t("common.copy", "复制")}
                      disabled={!account.openai_api_key}
                    >
                      <Copy size={14} />
                    </button>
                  </div>
                </div>
              </section>
              <section className={summaryGridClassName}>
                {summaryDetails.map((item) => (
                  <div
                    className="cockpit-api-stat-card cockpit-api-stat-card-center"
                    key={item.key}
                  >
                    <span className="cockpit-api-card-label">{item.label}</span>
                    <strong>{item.value}</strong>
                    {(item.key === "remaining" ||
                      item.key === "totalAvailable") &&
                      usageMode !== "new_api" &&
                      usageMode !== "sub2api" && (
                        <div>
                          <div className="cockpit-api-progress-row">
                            <div className="cockpit-api-progress-track">
                              <div
                                className="cockpit-api-progress-bar"
                                style={{ width: `${usedPercent}%` }}
                              />
                            </div>
                            <span>{usedPercent}%</span>
                          </div>
                        </div>
                      )}
                  </div>
                ))}
              </section>
              <section className="cockpit-api-panel-section">
                <div className="cockpit-api-section-head">
                  <strong>
                    {t("codex.modelProviders.usage.rawFields", "服务数据")}
                  </strong>
                </div>
                <div className="cockpit-api-usage-card-grid">
                  {details.length > 0 ? (
                    details.map((item) => (
                      <div className="cockpit-api-usage-card" key={item.key}>
                        <span className="cockpit-api-card-label">
                          {formatApiKeyUsageDetailLabel(item.key, item.label)}
                        </span>
                        <strong>
                          {formatApiKeyUsageDetailValue(item, summary?.unit)}
                        </strong>
                        <small>{item.key}</small>
                      </div>
                    ))
                  ) : (
                    <div className="cockpit-api-empty-row">
                      {t("codex.cockpitApi.noStats", "暂无统计")}
                    </div>
                  )}
                </div>
              </section>
            </div>
            <div className="modal-footer cockpit-api-panel-footer">
              <button
                className="btn btn-secondary"
                onClick={() => void refreshApiKeyUsageByAccountId(account.id)}
                disabled={state?.loading}
              >
                <RotateCw
                  size={14}
                  className={state?.loading ? "loading-spinner" : ""}
                />
                {t("common.shared.refreshQuota", "刷新配额")}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => openApiKeyCredentialsModal(account)}
              >
                <Pencil size={14} />
                {t("instances.actions.edit", "编辑")}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleLaunchCodexCli(account)}
                disabled={cliLaunchingAccountId === account.id}
              >
                {cliLaunchingAccountId === account.id ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Terminal size={14} />
                )}
                {t("codex.cli.quickLaunch", "CLI 快速启动")}
              </button>
            </div>
          </div>
        </div>
      );
    };
  
    const renderCockpitApiServicePanel = () => {
      const account = cockpitApiPanelAccount;
      if (!account) return null;
  
      const usage = getCockpitApiUsageRecord(account);
      const stats = getCockpitApiStatsRecord(account);
      const requests = toCockpitApiRecord(stats?.requests);
      const tokens = toCockpitApiRecord(stats?.tokens);
      const total = toCockpitApiRecord(stats?.total);
      const modelItems = (Array.isArray(stats?.models) ? stats.models : [])
        .map(toCockpitApiRecord)
        .filter((item): item is CockpitApiJsonRecord => Boolean(item))
        .slice(0, 8);
      const dailyItems = (Array.isArray(stats?.daily) ? stats.daily : [])
        .map(toCockpitApiRecord)
        .filter((item): item is CockpitApiJsonRecord => Boolean(item));
      const visible = visibleApiKeyAccountIds.has(account.id);
      const apiKeyDisplay = resolveApiKeyDisplayText(account, visible);
      const baseUrl = (account.api_base_url || "").trim() || COCKPIT_API_BASE_URL;
      const quotaText = readCockpitApiString(usage, "summary_display") || "-";
      const cockpitApiAccountBalanceText =
        resolveCockpitApiAccountBalanceText(account);
      const usedPercent = readCockpitApiNumber(usage, "used_percent");
      const requestCount = readCockpitApiNumber(requests, "total");
      const todayCount = readCockpitApiNumber(requests, "today");
      const last7Count = readCockpitApiNumber(requests, "last_7_days");
      const last30Count = readCockpitApiNumber(requests, "last_30_days");
      const totalTokens = readCockpitApiNumber(tokens, "total");
      const totalQuotaDisplay = readCockpitApiString(total, "quota_display");
      const panelDisplayName = resolvePresentation(account).displayName;
      const hasStats = Boolean(stats);
      const usedPercentText = `${formatCockpitApiInteger(usedPercent)}%`;
      const summaryItems = [
        {
          key: "requests",
          label: t("codex.cockpitApi.requests", "请求"),
          value: formatCockpitApiInteger(requestCount),
          meta: `${t("codex.cockpitApi.today", "今日")} ${formatCockpitApiInteger(todayCount)}`,
        },
        {
          key: "periods",
          label: t("codex.cockpitApi.periods", "周期"),
          value: `7d ${formatCockpitApiInteger(last7Count)}`,
          meta: `30d ${formatCockpitApiInteger(last30Count)}`,
        },
        {
          key: "tokens",
          label: t("codex.cockpitApi.tokens", "Tokens"),
          value: formatCockpitApiTokenCount(totalTokens),
          meta: `${t("codex.cockpitApi.quotaUsed", "消耗")} ${totalQuotaDisplay || "-"}`,
        },
      ];
  
      return (
        <div className="modal-overlay">
          <div
            className="modal-content cockpit-api-panel-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header cockpit-api-panel-header">
              <div>
                <h2>
                  {t("codex.cockpitApi.panelTitle", "Cockpit Api 服务面板")}
                </h2>
                <span className="cockpit-api-panel-subtitle">
                  {maskAccountText(panelDisplayName)}
                </span>
              </div>
              <button
                className="modal-close"
                onClick={() => setCockpitApiPanelAccountId(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
  
            <div className="cockpit-api-panel-body">
              <section className="cockpit-api-connection-card">
                <div className="cockpit-api-connection-row">
                  <span>{t("codex.localAccess.baseUrl", "地址")}</span>
                  <code title={baseUrl}>{baseUrl}</code>
                  <button
                    type="button"
                    className="folder-icon-btn cockpit-api-icon-btn"
                    onClick={() =>
                      void navigator.clipboard.writeText(baseUrl).catch(() => {})
                    }
                    title={t("common.copy", "复制")}
                  >
                    <Copy size={14} />
                  </button>
                </div>
                <div className="cockpit-api-connection-row">
                  <span>{t("codex.localAccess.apiKey", "密钥")}</span>
                  <code title={visible ? account.openai_api_key || "" : ""}>
                    {apiKeyDisplay}
                  </code>
                  <div className="cockpit-api-connection-actions">
                    <button
                      type="button"
                      className="folder-icon-btn cockpit-api-icon-btn"
                      onClick={() => toggleAccountApiKeyVisible(account.id)}
                      title={
                        visible
                          ? t("codex.localAccess.hideKey", "隐藏密钥")
                          : t("codex.localAccess.showKey", "显示密钥")
                      }
                    >
                      {visible ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                    <button
                      type="button"
                      className="folder-icon-btn cockpit-api-icon-btn"
                      onClick={() =>
                        void navigator.clipboard
                          .writeText(account.openai_api_key || "")
                          .catch(() => {})
                      }
                      title={t("common.copy", "复制")}
                      disabled={!account.openai_api_key}
                    >
                      <Copy size={14} />
                    </button>
                  </div>
                </div>
              </section>
  
              <section className="cockpit-api-summary-grid">
                <div className="cockpit-api-balance-card">
                  <span className="cockpit-api-card-label">
                    {t("codex.cockpitApi.balance", "额度")}
                  </span>
                  <strong>{quotaText}</strong>
                  {cockpitApiAccountBalanceText && (
                    <small className="cockpit-api-balance-meta">
                      {t("codex.modelProviders.usage.accountBalance", "账户余额")}
                      ：{cockpitApiAccountBalanceText}
                    </small>
                  )}
                  <div className="cockpit-api-progress-row">
                    <div className="cockpit-api-progress-track">
                      <div
                        className="cockpit-api-progress-bar"
                        style={{ width: usedPercentText }}
                      />
                    </div>
                    <span>{usedPercentText}</span>
                  </div>
                </div>
                {summaryItems.map((item) => (
                  <div className="cockpit-api-stat-card" key={item.key}>
                    <span className="cockpit-api-card-label">{item.label}</span>
                    <strong>{item.value}</strong>
                    <small>{item.meta}</small>
                  </div>
                ))}
              </section>
  
              {hasStats ? (
                <div className="cockpit-api-stats-grid">
                  <section className="cockpit-api-panel-section">
                    <div className="cockpit-api-section-head">
                      <strong>
                        {t("codex.cockpitApi.modelStats", "模型统计")}
                      </strong>
                    </div>
                    <div className="cockpit-api-usage-list">
                      {modelItems.length > 0 ? (
                        modelItems.map((item) => {
                          const modelName =
                            readCockpitApiString(item, "model_name") || "-";
                          const count = readCockpitApiNumber(
                            item,
                            "request_count",
                          );
                          const modelTokens = readCockpitApiNumber(
                            item,
                            "total_tokens",
                          );
                          const quotaDisplay = readCockpitApiString(
                            item,
                            "quota_display",
                          );
                          return (
                            <div
                              className="cockpit-api-usage-row"
                              key={modelName}
                            >
                              <div>
                                <span className="cockpit-api-usage-name">
                                  {modelName}
                                </span>
                                <small>
                                  {t("codex.cockpitApi.requests", "请求")}{" "}
                                  {formatCockpitApiInteger(count)}
                                </small>
                              </div>
                              <div className="cockpit-api-usage-values">
                                <span>
                                  {t("codex.cockpitApi.tokens", "Tokens")}{" "}
                                  {formatCockpitApiTokenCount(modelTokens)}
                                </span>
                                <strong>{quotaDisplay || "-"}</strong>
                              </div>
                            </div>
                          );
                        })
                      ) : (
                        <div className="cockpit-api-empty-row">
                          {t("codex.cockpitApi.noStats", "暂无统计")}
                        </div>
                      )}
                    </div>
                  </section>
  
                  <section className="cockpit-api-panel-section">
                    <div className="cockpit-api-section-head">
                      <strong>
                        {t("codex.cockpitApi.dailyStats", "每日统计")}
                      </strong>
                    </div>
                    <div className="cockpit-api-usage-list">
                      {dailyItems.length > 0 ? (
                        dailyItems.map((item) => {
                          const date = readCockpitApiString(item, "date") || "-";
                          const count = readCockpitApiNumber(
                            item,
                            "request_count",
                          );
                          const dayTokens = readCockpitApiNumber(
                            item,
                            "total_tokens",
                          );
                          const quotaDisplay = readCockpitApiString(
                            item,
                            "quota_display",
                          );
                          return (
                            <div className="cockpit-api-usage-row" key={date}>
                              <div>
                                <span className="cockpit-api-usage-name">
                                  {date}
                                </span>
                                <small>
                                  {t("codex.cockpitApi.requests", "请求")}{" "}
                                  {formatCockpitApiInteger(count)}
                                </small>
                              </div>
                              <div className="cockpit-api-usage-values">
                                <span>
                                  {t("codex.cockpitApi.tokens", "Tokens")}{" "}
                                  {formatCockpitApiTokenCount(dayTokens)}
                                </span>
                                <strong>{quotaDisplay || "-"}</strong>
                              </div>
                            </div>
                          );
                        })
                      ) : (
                        <div className="cockpit-api-empty-row">
                          {t("codex.cockpitApi.noStats", "暂无统计")}
                        </div>
                      )}
                    </div>
                  </section>
                </div>
              ) : (
                <div className="cockpit-api-empty-state">
                  {t(
                    "codex.cockpitApi.refreshHint",
                    "点击刷新后会同步当前 API key 的统计。",
                  )}
                </div>
              )}
            </div>
  
            <div className="modal-footer cockpit-api-panel-footer">
              <button
                className="btn btn-secondary"
                onClick={() => void handleRefresh(account.id)}
                disabled={refreshing === account.id}
              >
                <RotateCw
                  size={14}
                  className={refreshing === account.id ? "loading-spinner" : ""}
                />
                {t("common.shared.refreshQuota", "刷新配额")}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleLaunchCodexCli(account)}
                disabled={cliLaunchingAccountId === account.id}
              >
                {cliLaunchingAccountId === account.id ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Terminal size={14} />
                )}
                {t("codex.cli.quickLaunch", "CLI 快速启动")}
              </button>
            </div>
          </div>
        </div>
      );
    };
  return {
    externalImportPercent,
    externalImportRunning,
    externalImportStepIndex,
    externalImportSteps,
    handleCopyExternalImportErrors,
    handleViewExternalImportAccounts,
    hasGroupEntryCards,
    inlineFolderCards,
    renderApiKeyUsageDetailModal,
    renderCockpitApiServicePanel,
    renderCompactRows,
    renderGridCards,
    renderGroupTableRows,
    renderTableRows,
    showOverviewSelectionBar,
  };
}
