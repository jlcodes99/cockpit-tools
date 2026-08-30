import { useEffect, useMemo, useCallback, type MouseEvent as ReactMouseEvent } from "react";
import { RefreshCw, RotateCw } from "lucide-react";
import * as codexService from "../services/codexService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { type CodexAccountGroup } from "../services/codexAccountGroupService";
import { hasCodexAccountStructure, hasCodexAccountName, getCodexQuotaWindows, isCodexApiKeyAccount, isCodexAgentIdentityAccount, isCodexNewApiAccount, isCodexTeamLikePlan } from "../types/codex";
import { summarizeCodexQuotaErrorMessage } from "../utils/codexQuotaError";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { buildCodexAccountWindowStatQueries, formatCodexWindowStatsText, type CodexWindowStats } from "../utils/codexWindowStats";
import { type CodexLaunchPreviewAction, type CodexLaunchPreviewSummary } from "../components/codex/CodexLaunchPreviewModal";
import { CodexSpeedSelect } from "../components/codex/CodexSpeedSelect";
import { useEscClose } from "../hooks/useEscClose";
import { useEnterConfirm } from "../hooks/useEnterConfirm";
import type { CodexAccount } from "../types/codex";
import { CODEX_API_SERVICE_BIND_ID } from "../types/instance";
import { createCodexOverviewAccountComparator, filterAndSortCodexOverviewAccounts } from "../utils/codexAccountOverview";
import { buildPaginatedGroups, buildPaginationPageSizeStorageKey, isEveryIdSelected, usePagination } from "../hooks/usePagination";
import { formatCockpitApiInteger, formatCockpitApiTokenCount, getCodexAccountNoteTitle, isPendingOAuthCodexAccount, resolveApiKeyUsageMode, shouldAutoHideBatchDeleteJob } from "./codexAccountsControllerModel";
import type { useCodexAccountsBaseController } from "./useCodexAccountsBaseController";
import type { useCodexAccountsOAuthController } from "./useCodexAccountsOAuthController";
import type { useCodexAccountsAccessController } from "./useCodexAccountsAccessController";
import type { useCodexAccountsLocalAccessController } from "./useCodexAccountsLocalAccessController";

/** 封装 useCodexAccountsPageController 的 useCodexAccountsOverviewController 业务域状态与动作。 */
export function useCodexAccountsOverviewController(context: Pick<ReturnType<typeof useCodexAccountsBaseController> & ReturnType<typeof useCodexAccountsOAuthController> & ReturnType<typeof useCodexAccountsAccessController> & ReturnType<typeof useCodexAccountsLocalAccessController>,
  | "accounts"
  | "activeGroupId"
  | "activeTab"
  | "addingLocalAccessAccountId"
  | "apiKeyUsageMap"
  | "apiServiceAppSpeed"
  | "batchDeleteBusy"
  | "batchDeleteJob"
  | "batchDeleteRefreshedCompletedRef"
  | "batchDeleteRemoveIdsRef"
  | "buildResetCreditsTitle"
  | "canDirectlyAddLocalAccessAccount"
  | "canRefreshApiKeyUsage"
  | "cliLaunchingAccountId"
  | "codexGroups"
  | "confirmDeleteGroup"
  | "currentAccount"
  | "customSortOrder"
  | "deleteConfirm"
  | "deletingGroup"
  | "draggedCustomSortAccountId"
  | "fetchAccounts"
  | "fetchCurrentAccount"
  | "filterTypes"
  | "formatApiKeyUsageMoney"
  | "formatDate"
  | "fullQuotaWakeupOpenSignalRef"
  | "getResetCreditDetails"
  | "getResetCreditsAvailable"
  | "getScopedSelectedCount"
  | "groupByTag"
  | "groupDeleteConfirm"
  | "groupFilter"
  | "handleAddLocalAccessAccount"
  | "handleApiServiceAppSpeedChange"
  | "handleExportByIds"
  | "handleHideLocalAccessEntry"
  | "handleKillLocalAccessPort"
  | "handleLaunchCodexCli"
  | "handleLaunchLocalAccessCli"
  | "handleQuickRefreshLocalAccessQuota"
  | "handleQuickToggleLocalAccessEnabled"
  | "handleRefresh"
  | "handleRefreshSubscriptionInfo"
  | "handleRemoveLocalAccessAccount"
  | "hydrateAccountProfilesIfNeeded"
  | "isAbnormalAccount"
  | "isAllFilteredSelected"
  | "isAvailableResetCredit"
  | "localAccessAccountIdSet"
  | "localAccessCollection"
  | "localAccessLaunchCurrent"
  | "localAccessRefreshing"
  | "localAccessState"
  | "maskAccountText"
  | "normalizeTag"
  | "openAccountNoteModal"
  | "openApiKeyCredentialsModal"
  | "openCodexApiServicePage"
  | "openLocalAccessMemberPicker"
  | "openLocalAccessPanel"
  | "openOAuthBindingModal"
  | "openQuickSwitchProviderModal"
  | "openQuotaErrorDetail"
  | "openResetCreditConfirmModal"
  | "openTagModal"
  | "overviewAccounts"
  | "refreshAccountsAfterBatchDelete"
  | "refreshApiKeyUsage"
  | "refreshing"
  | "refreshingSubscriptionAccountId"
  | "renderAccountSpeedSelect"
  | "resettingResetCreditAccountId"
  | "resolveAccountMeta"
  | "resolveApiKeyDisplayText"
  | "resolveApiProviderDisplayName"
  | "resolveBoundOAuthAccount"
  | "resolveLocalAccessBaseUrl"
  | "resolvePresentation"
  | "resolveQuotaErrorMeta"
  | "resolveSingleExportBaseName"
  | "resolveSubscriptionPresentation"
  | "resolveUsageProviderForApiKeyAccount"
  | "savingAppSpeedId"
  | "searchQuery"
  | "selected"
  | "sessionWindowStats"
  | "setApiKeyUsageDetailAccountId"
  | "setBatchDeleteBusy"
  | "setBatchDeleteJob"
  | "setBatchDeleteModalError"
  | "setCockpitApiPanelAccountId"
  | "setCustomSortDropTargetId"
  | "setCustomSortOrder"
  | "setDeleteConfirm"
  | "setDraggedCustomSortAccountId"
  | "setFullQuotaWakeupOpenRequest"
  | "setGroupDeleteConfirm"
  | "setGroupDeleteError"
  | "setIsAllFilteredSelected"
  | "setMessage"
  | "setRefreshingGroupId"
  | "setSelected"
  | "setSessionWindowStats"
  | "setShowCustomSortModal"
  | "setSortBy"
  | "showAdditionalQuota"
  | "showCodeReviewQuota"
  | "sortBy"
  | "sortDirection"
  | "t"
  | "tagFilter"
  | "toggleAccountApiKeyVisible"
  | "toggleSelect"
  | "toggleSelectAll"
  | "untaggedKey"
  | "visibleApiKeyAccountIds"
>) {
  const {
    accounts,
    activeGroupId,
    activeTab,
    addingLocalAccessAccountId,
    apiKeyUsageMap,
    apiServiceAppSpeed,
    batchDeleteBusy,
    batchDeleteJob,
    batchDeleteRefreshedCompletedRef,
    batchDeleteRemoveIdsRef,
    buildResetCreditsTitle,
    canDirectlyAddLocalAccessAccount,
    canRefreshApiKeyUsage,
    cliLaunchingAccountId,
    codexGroups,
    confirmDeleteGroup,
    currentAccount,
    customSortOrder,
    deleteConfirm,
    deletingGroup,
    draggedCustomSortAccountId,
    fetchAccounts,
    fetchCurrentAccount,
    filterTypes,
    formatApiKeyUsageMoney,
    formatDate,
    fullQuotaWakeupOpenSignalRef,
    getResetCreditDetails,
    getResetCreditsAvailable,
    getScopedSelectedCount,
    groupByTag,
    groupDeleteConfirm,
    groupFilter,
    handleAddLocalAccessAccount,
    handleApiServiceAppSpeedChange,
    handleExportByIds,
    handleHideLocalAccessEntry,
    handleKillLocalAccessPort,
    handleLaunchCodexCli,
    handleLaunchLocalAccessCli,
    handleQuickRefreshLocalAccessQuota,
    handleQuickToggleLocalAccessEnabled,
    handleRefresh,
    handleRefreshSubscriptionInfo,
    handleRemoveLocalAccessAccount,
    hydrateAccountProfilesIfNeeded,
    isAbnormalAccount,
    isAllFilteredSelected,
    isAvailableResetCredit,
    localAccessAccountIdSet,
    localAccessCollection,
    localAccessLaunchCurrent,
    localAccessRefreshing,
    localAccessState,
    maskAccountText,
    normalizeTag,
    openAccountNoteModal,
    openApiKeyCredentialsModal,
    openCodexApiServicePage,
    openLocalAccessMemberPicker,
    openLocalAccessPanel,
    openOAuthBindingModal,
    openQuickSwitchProviderModal,
    openQuotaErrorDetail,
    openResetCreditConfirmModal,
    openTagModal,
    overviewAccounts,
    refreshAccountsAfterBatchDelete,
    refreshApiKeyUsage,
    refreshing,
    refreshingSubscriptionAccountId,
    renderAccountSpeedSelect,
    resettingResetCreditAccountId,
    resolveAccountMeta,
    resolveApiKeyDisplayText,
    resolveApiProviderDisplayName,
    resolveBoundOAuthAccount,
    resolveLocalAccessBaseUrl,
    resolvePresentation,
    resolveQuotaErrorMeta,
    resolveSingleExportBaseName,
    resolveSubscriptionPresentation,
    resolveUsageProviderForApiKeyAccount,
    savingAppSpeedId,
    searchQuery,
    selected,
    sessionWindowStats,
    setApiKeyUsageDetailAccountId,
    setBatchDeleteBusy,
    setBatchDeleteJob,
    setBatchDeleteModalError,
    setCockpitApiPanelAccountId,
    setCustomSortDropTargetId,
    setCustomSortOrder,
    setDeleteConfirm,
    setDraggedCustomSortAccountId,
    setFullQuotaWakeupOpenRequest,
    setGroupDeleteConfirm,
    setGroupDeleteError,
    setIsAllFilteredSelected,
    setMessage,
    setRefreshingGroupId,
    setSelected,
    setSessionWindowStats,
    setShowCustomSortModal,
    setSortBy,
    showAdditionalQuota,
    showCodeReviewQuota,
    sortBy,
    sortDirection,
    t,
    tagFilter,
    toggleAccountApiKeyVisible,
    toggleSelect,
    toggleSelectAll,
    untaggedKey,
    visibleApiKeyAccountIds,
  } = context;
  const overviewCurrentAccountId = localAccessLaunchCurrent
      ? null
      : (currentAccount?.id ?? null);
  
    useEffect(() => {
      if (activeTab !== "overview") {
        setSessionWindowStats({ ready: false, byAccountId: {} });
        return;
      }
      const memberAccounts = (localAccessCollection?.accountIds ?? [])
        .map((accountId) => accounts.find((account) => account.id === accountId))
        .filter((account): account is CodexAccount => Boolean(account));
      const now = Math.floor(Date.now() / 1000);
      const queries = memberAccounts.flatMap((account) =>
        buildCodexAccountWindowStatQueries(
          account.id,
          getCodexQuotaWindows(account.quota),
          now,
        ),
      );
      if (queries.length === 0) {
        setSessionWindowStats({ ready: false, byAccountId: {} });
        return;
      }
      let cancelled = false;
      void (async () => {
        try {
          const rows =
            await codexLocalAccessService.queryCodexLocalAccessAccountWindowStats(
              queries,
            );
          if (cancelled) return;
          const byAccountId: Record<
            string,
            { primary?: CodexWindowStats; secondary?: CodexWindowStats }
          > = {};
          rows.forEach((row) => {
            const stats: CodexWindowStats = {
              requestCount: row.requestCount,
              inputTokens: row.inputTokens,
              cachedInputTokens: row.cachedTokens,
              outputTokens: row.outputTokens,
              totalTokens: row.totalTokens,
              estimatedCostUsd: row.estimatedCostUsd,
            };
            const current = byAccountId[row.accountId] ?? {};
            if (row.windowKey === "secondary") {
              current.secondary = stats;
            } else if (row.windowKey === "primary") {
              current.primary = stats;
            }
            byAccountId[row.accountId] = current;
          });
          setSessionWindowStats({ ready: true, byAccountId });
        } catch {
          if (!cancelled) {
            setSessionWindowStats({ ready: false, byAccountId: {} });
          }
        }
      })();
      return () => {
        cancelled = true;
      };
    }, [accounts, activeTab, localAccessCollection?.accountIds]);
  
    const applyWindowStatsToQuotaItems = useCallback(
      (
        account: CodexAccount,
        items: ReturnType<typeof buildCodexAccountPresentation>["quotaItems"],
      ) => {
        const memberIds = localAccessCollection?.accountIds ?? [];
        if (!memberIds.includes(account.id)) {
          return items;
        }
        const accountStats = sessionWindowStats.byAccountId[account.id] ?? {};
        const emptyStats = {
          requestCount: 0,
          inputTokens: 0,
          cachedInputTokens: 0,
          outputTokens: 0,
          totalTokens: 0,
          estimatedCostUsd: 0,
        };
        return items.map((item) => {
          if (item.key !== "primary" && item.key !== "secondary") {
            return item;
          }
          const stats =
            (item.key === "primary"
              ? accountStats.primary
              : accountStats.secondary) ?? emptyStats;
          const windowStatsText = formatCodexWindowStatsText(stats);
          const hint = t(
            "codex.quota.windowStatsHint",
            "从本窗口满额到现在，该账号走 API 服务的请求数、token 和账号计费",
          );
          return {
            ...item,
            windowStats: stats,
            windowStatsText,
            hintText: [item.hintText, windowStatsText, hint]
              .filter(Boolean)
              .join("\n"),
          };
        });
      },
      [localAccessCollection?.accountIds, sessionWindowStats, t],
    );
  
    const buildAccountLaunchPreviewSummary = useCallback(
      (account: CodexAccount): CodexLaunchPreviewSummary => {
        const presentation = resolvePresentation(account);
        const isApiKey = isCodexApiKeyAccount(account);
        const isNewApi = isCodexNewApiAccount(account);
        const meta = resolveAccountMeta(account);
        const quotaItems = applyWindowStatsToQuotaItems(
          account,
          presentation.quotaItems,
        ).filter((item) => {
          if (isApiKey && !isNewApi) return false;
          if (!showCodeReviewQuota && item.key === "code_review") return false;
          if (!showAdditionalQuota && item.key.startsWith("additional:")) {
            return false;
          }
          return true;
        });
        const providerName = isApiKey
          ? resolveApiProviderDisplayName(account)
          : "";
        const usage = isApiKey ? apiKeyUsageMap[account.id]?.summary : undefined;
        const useTodayUsage =
          usage?.todayRequests != null ||
          usage?.todayTotalTokens != null ||
          usage?.todayCost != null;
        const requestCount = useTodayUsage
          ? usage?.todayRequests
          : usage?.totalRequests;
        const tokenCount = useTodayUsage
          ? usage?.todayTotalTokens
          : usage?.totalTotalTokens;
        const cost = useTodayUsage ? usage?.todayCost : usage?.totalCost;
        const subscription = !isApiKey
          ? resolveSubscriptionPresentation(account)
          : null;
  
        return {
          badgeLabel:
            account.plan_type?.trim() ||
            (isApiKey ? providerName : presentation.planLabel),
          contextText: isApiKey
            ? providerName
            : [meta.accountContextText, meta.signedInWithText]
                .filter(Boolean)
                .join(" · "),
          statusLabel: account.requires_reauth
            ? t("codex.authError.badge", "授权异常")
            : account.client_auth_status === "login_required"
              ? t("codex.switchAuth.apiOnlyBadge", "客户端需授权")
              : overviewCurrentAccountId === account.id
                ? t("codex.current", "当前")
                : undefined,
          statusTone: account.requires_reauth || account.client_auth_status === "login_required"
            ? "warning"
            : overviewCurrentAccountId === account.id
              ? "success"
              : undefined,
          facts: isApiKey
            ? [
                {
                  label: t("codex.api.provider.label", "供应商"),
                  value: providerName,
                },
                {
                  label: t("codex.api.modelCatalog.label", "模型列表"),
                  value: t("codex.api.modelCatalog.count", {
                    count: account.api_model_catalog?.length ?? 0,
                    defaultValue: "{{count}} 个模型",
                  }),
                },
                {
                  label: t("codex.api.baseUrl", "Base URL"),
                  value: account.api_base_url?.trim() || "-",
                  monospace: true,
                  wide: true,
                },
              ]
            : [
                {
                  label: t("kiro.account.userId", "用户 ID"),
                  value: meta.userId,
                  monospace: true,
                },
                {
                  label: t("codex.apiSwitchNotice.type.account", "账号"),
                  value: meta.chatgptAccountId,
                  monospace: true,
                },
                {
                  label: t("codex.subscription.label", "有效期"),
                  value: subscription
                    ? `${subscription.valueText}${
                        subscription.detailText
                          ? ` · ${subscription.detailText}`
                          : ""
                      }`
                    : t("common.none", "暂无"),
                },
              ],
          quotaItems,
          usage:
            isApiKey && usage
              ? {
                  label: useTodayUsage
                    ? t("codex.modelProviders.usage.today", "今日用量")
                    : t("codex.modelProviders.usage.total", "累计用量"),
                  requests:
                    requestCount != null
                      ? formatCockpitApiInteger(requestCount)
                      : null,
                  tokens:
                    tokenCount != null
                      ? formatCockpitApiTokenCount(tokenCount)
                      : null,
                  cost:
                    cost != null
                      ? formatApiKeyUsageMoney(cost, usage.unit)
                      : null,
                  extraLabel: usage.planName
                    ? t("codex.modelProviders.usage.fields.planName", "订阅")
                    : null,
                  extraValue: usage.planName || null,
                }
              : null,
          tags: account.tags,
          footerText: formatDate(account.created_at),
        };
      },
      [
        apiKeyUsageMap,
        applyWindowStatsToQuotaItems,
        overviewCurrentAccountId,
        resolveAccountMeta,
        resolveApiProviderDisplayName,
        resolvePresentation,
        resolveSubscriptionPresentation,
        showAdditionalQuota,
        showCodeReviewQuota,
        t,
      ],
    );
  
    const buildAccountLaunchPreviewActions = (
      account: CodexAccount,
    ): CodexLaunchPreviewAction[] => {
      const actions: CodexLaunchPreviewAction[] = [];
      const presentation = resolvePresentation(account);
      const isApiKey = isCodexApiKeyAccount(account);
      const isNewApi = isCodexNewApiAccount(account);
      const provider = isApiKey
        ? resolveUsageProviderForApiKeyAccount(account)
        : null;
      const providerName = isApiKey ? resolveApiProviderDisplayName(account) : "";
      const subscription = !isApiKey
        ? resolveSubscriptionPresentation(account)
        : null;
      const accountIssue = resolveQuotaErrorMeta(
        account.requires_reauth && account.reauth_reason
          ? {
              message: account.reauth_reason,
              timestamp: account.token_updated_at || account.last_used,
            }
          : account.quota_error,
      );
      const accountName =
        presentation.displayName ||
        account.email ||
        account.account_name ||
        account.id;
  
      if (accountIssue.rawMessage) {
        actions.push({
          id: "issue-detail",
          label: t("codex.quotaError.viewDetails", "查看详情"),
          description: accountIssue.displayText,
          actionLabel: t("common.detail", "详情"),
          onAction: () => {
            openQuotaErrorDetail(accountName, accountIssue.rawMessage);
          },
        });
      }
  
      if (
        subscription &&
        !isPendingOAuthCodexAccount(account) &&
        (subscription.bucket === "missing" || subscription.bucket === "expired")
      ) {
        actions.push({
          id: "subscription",
          label: t("codex.subscription.label", "有效期"),
          description: `${subscription.valueText}${
            subscription.detailText ? ` · ${subscription.detailText}` : ""
          }`,
          actionLabel: t("common.refresh", "刷新"),
          disabled:
            refreshingSubscriptionAccountId === account.id ||
            refreshing === account.id,
          onAction: async () => {
            await handleRefreshSubscriptionInfo(account.id);
          },
        });
      }
  
      if (localAccessAccountIdSet.has(account.id)) {
        actions.push({
          id: "api-service-membership",
          label: t("codex.localAccess.removeAction", "移除 API 服务"),
          description: `${t("codex.localAccess.title", "API 服务")} · ${accountName}`,
          actionLabel: t("codex.localAccess.removeAction", "移除 API 服务"),
          disabled: addingLocalAccessAccountId !== null,
          onAction: async () => {
            await handleRemoveLocalAccessAccount(account.id);
          },
        });
      } else if (canDirectlyAddLocalAccessAccount(account)) {
        actions.push({
          id: "api-service-membership",
          label: t("codex.localAccess.entryAction", "添加至 API 服务"),
          description: `${t("codex.localAccess.title", "API 服务")} · ${accountName}`,
          actionLabel: t("codex.localAccess.entryAction", "添加至 API 服务"),
          disabled: addingLocalAccessAccountId !== null,
          onAction: async () => {
            await handleAddLocalAccessAccount(account.id);
          },
        });
      }
  
      if (isApiKey) {
        const apiKeyVisible = visibleApiKeyAccountIds.has(account.id);
        actions.push({
          id: "api-key-visibility",
          label: apiKeyVisible
            ? t("codex.api.hideApiKey", "隐藏 API Key")
            : t("codex.api.showApiKey", "显示 API Key"),
          description: resolveApiKeyDisplayText(account, apiKeyVisible),
          actionLabel: apiKeyVisible
            ? t("codex.api.hideApiKey", "隐藏 API Key")
            : t("codex.api.showApiKey", "显示 API Key"),
          onAction: () => toggleAccountApiKeyVisible(account.id),
        });
  
        const boundOAuth = resolveBoundOAuthAccount(account);
        actions.push({
          id: "oauth-binding",
          label: t("codex.api.oauthBinding.action", "绑定 OAuth"),
          description: boundOAuth
            ? maskAccountText(
                boundOAuth.account_name || boundOAuth.email || boundOAuth.id,
              )
            : t("codex.api.oauthBinding.unbound", "未绑定"),
          actionLabel: t("codex.api.oauthBinding.actionShort", "绑定"),
          onAction: () => {
            openOAuthBindingModal(account);
          },
        });
  
        if (!isNewApi) {
          actions.push({
            id: "provider",
            label: t("codex.quickSwitch.action", "快速切换供应商"),
            description: `${providerName} · ${account.api_base_url?.trim() || "-"}`,
            actionLabel: t("codex.quickSwitch.inlineAction", "切换"),
            onAction: () => {
              openQuickSwitchProviderModal(account);
            },
          });
        }
  
        if (!isNewApi) {
          actions.push({
            id: "edit-credentials",
            label: t("instances.actions.edit", "编辑"),
            description: `${providerName} · ${account.api_model_catalog?.length ?? 0} ${t(
              "codex.api.modelCatalog.label",
              "模型列表",
            )}`,
            actionLabel: t("instances.actions.edit", "编辑"),
            onAction: () => {
              openApiKeyCredentialsModal(account);
            },
          });
        }
  
        if (isNewApi) {
          actions.push({
            id: "service-panel",
            label: t("codex.cockpitApi.servicePanel", "服务面板"),
            description: providerName,
            actionLabel: t("common.open", "打开"),
            onAction: () => {
              setCockpitApiPanelAccountId(account.id);
            },
          });
        } else if (resolveApiKeyUsageMode(apiKeyUsageMap[account.id]?.summary)) {
          actions.push({
            id: "usage-detail",
            label: t("codex.modelProviders.usage.detailTitle", "服务面板"),
            description:
              apiKeyUsageMap[account.id]?.summary?.planName || providerName,
            actionLabel: t("common.detail", "详情"),
            onAction: () => {
              setApiKeyUsageDetailAccountId(account.id);
            },
          });
        }
      } else {
        actions.push({
          id: "account-note",
          label: t("codex.accountNote.short", "账号备注"),
          description:
            getCodexAccountNoteTitle(account, "") ||
            t("codex.accountNote.emptyTitle", "填写账号备注"),
          actionLabel: t("instances.actions.edit", "编辑"),
          onAction: () => {
            openAccountNoteModal(account);
          },
        });
      }
  
      const resetCreditControls = renderResetCreditControls(account);
      if (resetCreditControls) {
        const resetCount = getResetCreditsAvailable(account) ?? 0;
        actions.push({
          id: "reset-credits",
          label: t("codex.quota.resetCredits", { count: resetCount }),
          description: t("codex.quota.resetCreditsTitle", {
            count: resetCount,
          }),
          actionLabel: t("codex.quota.resetCredits", { count: resetCount }),
          onAction: () => {
            openResetCreditConfirmModal(account);
          },
        });
      }
  
      actions.push(
        {
          id: "speed",
          label: t("codex.speed.title", "速度"),
          description:
            account.app_speed === "fast"
              ? t("codex.speed.fastDesc", "1.5 倍速，用量增加")
              : t("codex.speed.standardDesc", "默认速度，常规用量"),
          control: renderAccountSpeedSelect(account),
        },
        {
          id: "tags",
          label: t("accounts.editTags", "编辑标签"),
          description:
            account.tags && account.tags.length > 0
              ? account.tags.join(" · ")
              : t("common.none", "暂无"),
          actionLabel: t("instances.actions.edit", "编辑"),
          onAction: () => {
            openTagModal(account.id);
          },
        },
        {
          id: "cli",
          label: t("codex.cli.quickLaunch", "CLI 快速启动"),
          description: `${accountName} · ${t("instances.defaultName", "默认实例")}`,
          actionLabel: t("common.open", "打开"),
          disabled: cliLaunchingAccountId === account.id,
          onAction: async () => {
            await handleLaunchCodexCli(account);
          },
        },
      );
  
      if (
        !isPendingOAuthCodexAccount(account) &&
        (!isApiKey || isNewApi || canRefreshApiKeyUsage(account, provider))
      ) {
        const canRefreshUsage =
          isApiKey && canRefreshApiKeyUsage(account, provider);
        actions.push({
          id: "refresh-quota",
          label: t("common.shared.refreshQuota", "刷新配额"),
          description: presentation.quotaItems
            .slice(0, 2)
            .map((item) => `${item.label} ${item.valueText}`)
            .join(" · "),
          actionLabel: t("common.refresh", "刷新"),
          disabled: canRefreshUsage
            ? apiKeyUsageMap[account.id]?.loading === true
            : refreshing === account.id,
          onAction: async () => {
            if (canRefreshUsage) {
              await refreshApiKeyUsage(account, provider);
            } else {
              await handleRefresh(account.id);
            }
          },
        });
      }
  
      actions.push({
        id: "export",
        label: t("common.shared.export.title", "导出"),
        description: accountName,
        actionLabel: t("common.shared.export.title", "导出"),
        onAction: async () => {
          await handleExportByIds(
            [account.id],
            resolveSingleExportBaseName(account),
          );
        },
      });
  
      const actionOrder = new Map<string, number>([
        ["reauthorize", 10],
        ["issue-detail", 20],
        ["subscription", 30],
        ["api-service-membership", 40],
        ["api-key-visibility", 50],
        ["oauth-binding", 60],
        ["provider", 70],
        ["usage-detail", 80],
        ["reset-credits", 90],
        ["speed", 100],
        ["cli", 110],
        ["service-panel", 120],
        ["tags", 130],
        ["account-note", 140],
        ["edit-credentials", 140],
        ["refresh-quota", 160],
        ["export", 170],
      ]);
      return actions.sort(
        (left, right) =>
          (actionOrder.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
          (actionOrder.get(right.id) ?? Number.MAX_SAFE_INTEGER),
      );
    };
  
    const buildLocalAccessLaunchPreviewSummary =
      useCallback((): CodexLaunchPreviewSummary => {
        const collection = localAccessCollection;
        const totals = localAccessState?.stats.weekly.totals;
        const statusLabel = localAccessState?.running
          ? t("codex.localAccess.statusRunning", "运行中")
          : collection?.enabled
            ? t("codex.localAccess.statusStopped", "未运行")
            : t("codex.localAccess.statusDisabled", "已停用");
        const scopeLabel =
          collection?.accessScope === "lan"
            ? t("codex.localAccess.accessScopeLanShort", "本机+局域网")
            : t("codex.localAccess.accessScopeLocalhostShort", "仅本机");
        const routingStrategy = collection?.routingStrategy ?? "auto";
        const routingTranslationKey =
          {
            auto: "auto",
            random: "random",
            single_account: "singleAccount",
            quota_high_first: "quotaHighFirst",
            quota_low_first: "quotaLowFirst",
            plan_high_first: "planHighFirst",
            plan_low_first: "planLowFirst",
            expiry_soon_first: "expirySoonFirst",
            custom: "custom",
          }[routingStrategy] ?? "auto";
        const requestCount = totals?.requestCount ?? 0;
        const successRate =
          requestCount > 0
            ? `${Math.round(((totals?.successCount ?? 0) / requestCount) * 100)}%`
            : "-";
  
        return {
          badgeLabel: t("codex.apiSwitchNotice.type.apiKey", "API 密钥"),
          contextText: `${t("codex.apiSwitchNotice.type.apiKey", "API 密钥")} · ${t(
            "codex.localAccess.accountCount",
            {
              count: localAccessState?.memberCount ?? 0,
              defaultValue: "{{count}} 个账号",
            },
          )}`,
          statusLabel,
          statusTone: localAccessState?.running
            ? "success"
            : collection?.enabled
              ? "neutral"
              : "warning",
          facts: [
            {
              label: t("codex.localAccess.memberTitle", "集合成员"),
              value: t("codex.localAccess.accountCount", {
                count: localAccessState?.memberCount ?? 0,
                defaultValue: "{{count}} 个账号",
              }),
            },
            {
              label: t("codex.localAccess.routingLabel", "调度策略"),
              value: t(
                `codex.localAccess.routingStrategy.${routingTranslationKey}`,
                routingStrategy,
              ),
            },
            {
              label: t("codex.api.modelCatalog.label", "模型列表"),
              value: t("codex.api.modelCatalog.count", {
                count: localAccessState?.modelIds.length ?? 0,
                defaultValue: "{{count}} 个模型",
              }),
            },
            {
              label: t("codex.localAccess.baseUrl", "地址"),
              value: resolveLocalAccessBaseUrl() || "-",
              monospace: true,
              wide: true,
            },
            {
              label: t("codex.localAccess.accessScopeLabel", "访问范围"),
              value: scopeLabel,
            },
          ],
          usage: totals
            ? {
                label: t("codex.localAccess.statsRange.weekly", "本周"),
                requests: formatCockpitApiInteger(totals.requestCount),
                tokens: formatCockpitApiTokenCount(totals.totalTokens),
                cost: `$${totals.estimatedCostUsd.toFixed(2)}`,
                extraLabel: t(
                  "codex.localAccess.stats.successRateLabel",
                  "成功率",
                ),
                extraValue: successRate,
              }
            : null,
          footerText: t("codex.localAccess.footerHint", {
            scope: scopeLabel,
            defaultValue: "监听范围：{{scope}}",
          }),
        };
      }, [localAccessCollection, localAccessState, resolveLocalAccessBaseUrl, t]);
  
    const buildLocalAccessLaunchPreviewActions =
      (): CodexLaunchPreviewAction[] => {
        if (!localAccessCollection) return [];
        const baseUrl = resolveLocalAccessBaseUrl() || "-";
        const actions: CodexLaunchPreviewAction[] = [
          {
            id: "members",
            label: t("common.shared.addAccount", "添加账号"),
            description: t("codex.localAccess.accountCount", {
              count: localAccessState?.memberCount ?? 0,
              defaultValue: "{{count}} 个账号",
            }),
            actionLabel: t("common.shared.addAccount", "添加账号"),
            onAction: () => {
              openLocalAccessMemberPicker();
            },
          },
          {
            id: "cli",
            label: t("codex.cli.quickLaunch", "CLI 快速启动"),
            description: `${t("codex.localAccess.title", "API 服务")} · ${baseUrl}`,
            actionLabel: t("common.open", "打开"),
            disabled: cliLaunchingAccountId === CODEX_API_SERVICE_BIND_ID,
            onAction: async () => {
              await handleLaunchLocalAccessCli();
            },
          },
          {
            id: "dashboard",
            label: t("codex.localAccess.dashboardAction", "服务面板"),
            description: baseUrl,
            actionLabel: t("common.open", "打开"),
            onAction: () => {
              openLocalAccessPanel();
            },
          },
          {
            id: "full-page",
            label: t("codex.apiService.openPage", "进入 API 服务"),
            description: t("codex.apiService.openFullPage", "查看全部功能"),
            actionLabel: t("common.open", "打开"),
            onAction: () => {
              openCodexApiServicePage();
            },
          },
          {
            id: "refresh-quota",
            label: t("common.shared.refreshQuota", "刷新配额"),
            description: t("codex.localAccess.accountCount", {
              count: localAccessState?.memberCount ?? 0,
              defaultValue: "{{count}} 个账号",
            }),
            actionLabel: t("common.refresh", "刷新"),
            disabled: localAccessRefreshing,
            onAction: async () => {
              await handleQuickRefreshLocalAccessQuota();
            },
          },
          {
            id: "speed",
            label: t("codex.speed.title", "速度"),
            description:
              apiServiceAppSpeed === "fast"
                ? t("codex.speed.fastDesc", "1.5 倍速，用量增加")
                : t("codex.speed.standardDesc", "默认速度，常规用量"),
            control: (
              <CodexSpeedSelect
                value={apiServiceAppSpeed}
                onChange={handleApiServiceAppSpeedChange}
                busy={savingAppSpeedId === CODEX_API_SERVICE_BIND_ID}
                preferredPlacement="top"
                ariaLabel={t("codex.speed.title", "速度")}
              />
            ),
          },
          {
            id: "toggle-service",
            label: localAccessCollection.enabled
              ? t("codex.localAccess.disableService", "停用服务")
              : t("codex.localAccess.enableService", "启用服务"),
            description: localAccessState?.running
              ? t("codex.localAccess.statusRunning", "运行中")
              : t("codex.localAccess.statusStopped", "未运行"),
            actionLabel: localAccessCollection.enabled
              ? t("codex.localAccess.disableService", "停用服务")
              : t("codex.localAccess.enableService", "启用服务"),
            onAction: async () => {
              await handleQuickToggleLocalAccessEnabled();
            },
          },
          {
            id: "hide-entry",
            label: t("codex.localAccess.hideEntryAction", "关闭 API 服务入口"),
            description: t("codex.localAccess.title", "API 服务"),
            actionLabel: t("common.close", "关闭"),
            onAction: async () => {
              await handleHideLocalAccessEntry();
            },
          },
        ];
  
        if (localAccessState?.lastError) {
          actions.push({
            id: "clear-port",
            label: t("codex.localAccess.killPortAction", "清理端口"),
            description: summarizeCodexQuotaErrorMessage(
              localAccessState.lastError,
            ),
            actionLabel: t("codex.localAccess.killPortAction", "清理端口"),
            onAction: async () => {
              await handleKillLocalAccessPort();
            },
          });
        }
  
        const actionOrder = new Map<string, number>([
          ["clear-port", 80],
          ["speed", 90],
          ["members", 100],
          ["cli", 110],
          ["dashboard", 120],
          ["full-page", 130],
          ["refresh-quota", 140],
          ["toggle-service", 150],
          ["hide-entry", 160],
        ]);
        return actions.sort(
          (left, right) =>
            (actionOrder.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
            (actionOrder.get(right.id) ?? Number.MAX_SAFE_INTEGER),
        );
      };
  
    const compareAccountsBySort = useMemo(
      () =>
        createCodexOverviewAccountComparator({
          sortBy,
          sortDirection,
          customSortOrder,
          currentAccountId: overviewCurrentAccountId,
          resolveSubscriptionTimestamp: (account) =>
            isCodexApiKeyAccount(account)
              ? null
              : resolveSubscriptionPresentation(account).timestampMs,
        }),
      [
        customSortOrder,
        overviewCurrentAccountId,
        resolveSubscriptionPresentation,
        sortBy,
        sortDirection,
      ],
    );
  
    const sortedAccountsForInstances = useMemo(
      () => [...accounts].sort(compareAccountsBySort),
      [accounts, compareAccountsBySort],
    );
  
    const filteredAccounts = useMemo(
      () =>
        filterAndSortCodexOverviewAccounts({
          accounts: overviewAccounts,
          groups: codexGroups,
          searchQuery,
          filterTypes,
          tagFilter,
          groupFilter,
          activeGroupId,
          resolveDisplayName: (account) =>
            resolvePresentation(account).displayName,
          compareAccounts: compareAccountsBySort,
          isAbnormalAccount,
        }),
      [
        activeGroupId,
        codexGroups,
        compareAccountsBySort,
        filterTypes,
        groupFilter,
        isAbnormalAccount,
        overviewAccounts,
        resolvePresentation,
        searchQuery,
        tagFilter,
      ],
    );
  
    const filteredIds = useMemo(
      () => filteredAccounts.map((account) => account.id),
      [filteredAccounts],
    );
    const overviewTotalCount = overviewAccounts.length;
    const overviewVisibleCount = filteredAccounts.length;
    const hasActiveOverviewFilters =
      Boolean(searchQuery.trim()) ||
      filterTypes.length > 0 ||
      tagFilter.length > 0 ||
      groupFilter.length > 0 ||
      Boolean(activeGroupId);
    const showOverviewFilterBanner =
      hasActiveOverviewFilters && overviewVisibleCount !== overviewTotalCount;
    const overviewFilterChips = useMemo(() => {
      const chips: string[] = [];
      if (activeGroupId) {
        chips.push(t("codex.filters.chipFolder", "分组目录"));
      }
      if (groupFilter.length > 0) {
        chips.push(t("codex.filters.chipGroup", "分组"));
      }
      if (tagFilter.length > 0) {
        chips.push(t("codex.filters.chipTags", "标签"));
      }
      if (searchQuery.trim()) {
        chips.push(t("codex.filters.chipSearch", "搜索"));
      }
      if (filterTypes.length > 0) {
        chips.push(t("codex.filters.chipPlan", "套餐"));
      }
      return chips;
    }, [
      activeGroupId,
      filterTypes.length,
      groupFilter.length,
      searchQuery,
      t,
      tagFilter.length,
    ]);
    const errorAccountIds = useMemo(
      () =>
        filteredAccounts.filter(isAbnormalAccount).map((account) => account.id),
      [filteredAccounts, isAbnormalAccount],
    );
    // Full overview set of auth-failed accounts for export (#992), not limited to current page filter.
    const authFailedExportAccountIds = useMemo(
      () =>
        overviewAccounts.filter(isAbnormalAccount).map((account) => account.id),
      [isAbnormalAccount, overviewAccounts],
    );
    const handleExportAuthFailedAccounts = useCallback(() => {
      if (authFailedExportAccountIds.length === 0) return;
      void handleExportByIds(
        authFailedExportAccountIds,
        `codex_auth_failed_${authFailedExportAccountIds.length}`,
      );
    }, [authFailedExportAccountIds, handleExportByIds]);
    const hasDetectableFullQuotaWakeupAccounts = useMemo(
      () =>
        filteredAccounts.some(
          (account) =>
            !isCodexApiKeyAccount(account) &&
            Boolean(account.tokens.refresh_token?.trim()),
        ),
      [filteredAccounts],
    );
    const handleClearErrorAccounts = useCallback(() => {
      if (errorAccountIds.length === 0) return;
      setDeleteConfirm({
        ids: errorAccountIds,
        message: t("messages.cleanErrorAccountsConfirm", {
          count: errorAccountIds.length,
          defaultValue: "确定要删除当前范围内的 {{count}} 条 ERROR 账号吗？",
        }),
      });
    }, [errorAccountIds, setDeleteConfirm, t]);
    const openFullQuotaWakeupTestModal = useCallback(() => {
      if (!hasDetectableFullQuotaWakeupAccounts) {
        setMessage({
          text: t(
            "codex.wakeup.fullQuotaNoAccounts",
            "当前列表没有可唤醒的 OAuth 账号。",
          ),
          tone: "error",
        });
        return;
      }
      fullQuotaWakeupOpenSignalRef.current += 1;
      setFullQuotaWakeupOpenRequest({
        signal: fullQuotaWakeupOpenSignalRef.current,
        variant: "fullQuota",
        defaultSortBy: "hourly",
        defaultSortDirection: "desc",
      });
    }, [hasDetectableFullQuotaWakeupAccounts, setMessage, t]);
    const exportSelectionCount = getScopedSelectedCount(filteredIds);
    const pagination = usePagination({
      items: filteredAccounts,
      storageKey: buildPaginationPageSizeStorageKey("Codex"),
    });
    const paginatedAccounts = pagination.pageItems;
    const paginatedIds = useMemo(
      () => paginatedAccounts.map((account) => account.id),
      [paginatedAccounts],
    );
    const isCustomSortActive = sortBy === "custom";
    const customSortAccounts = useMemo(() => {
      const accountMap = new Map(
        accounts.map((account) => [account.id, account]),
      );
      const result: CodexAccount[] = [];
      const seen = new Set<string>();
  
      customSortOrder.forEach((accountId) => {
        const account = accountMap.get(accountId);
        if (!account || seen.has(accountId)) return;
        result.push(account);
        seen.add(accountId);
      });
  
      accounts.forEach((account) => {
        if (seen.has(account.id)) return;
        result.push(account);
        seen.add(account.id);
      });
  
      return result;
    }, [accounts, customSortOrder]);
    const customSortAccountIds = useMemo(
      () => customSortAccounts.map((account) => account.id),
      [customSortAccounts],
    );
    const moveCustomSortAccount = useCallback(
      (accountId: string, direction: "up" | "down") => {
        const currentIndex = customSortAccountIds.indexOf(accountId);
        if (currentIndex < 0) return;
        const targetIndex =
          direction === "up" ? currentIndex - 1 : currentIndex + 1;
        if (targetIndex < 0 || targetIndex >= customSortAccountIds.length) return;
        const next = [...customSortAccountIds];
        const [moved] = next.splice(currentIndex, 1);
        next.splice(targetIndex, 0, moved);
        setCustomSortOrder(next);
      },
      [customSortAccountIds],
    );
    const stopCustomSortDragging = useCallback(() => {
      setDraggedCustomSortAccountId(null);
      setCustomSortDropTargetId(null);
    }, []);
    const handleCustomSortDragStart = useCallback(
      (event: ReactMouseEvent, accountId: string) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        setDraggedCustomSortAccountId(accountId);
        setCustomSortDropTargetId(null);
      },
      [],
    );
    const handleCustomSortDragMove = useCallback(
      (targetAccountId: string) => {
        if (!draggedCustomSortAccountId) return;
        if (draggedCustomSortAccountId === targetAccountId) {
          setCustomSortDropTargetId(null);
          return;
        }
        const fromIndex = customSortAccountIds.indexOf(
          draggedCustomSortAccountId,
        );
        const toIndex = customSortAccountIds.indexOf(targetAccountId);
        if (fromIndex < 0 || toIndex < 0) return;
        setCustomSortDropTargetId(targetAccountId);
        const next = [...customSortAccountIds];
        const [moved] = next.splice(fromIndex, 1);
        next.splice(toIndex, 0, moved);
        setCustomSortOrder(next);
      },
      [customSortAccountIds, draggedCustomSortAccountId],
    );
    const resetCustomSortOrder = useCallback(() => {
      setCustomSortOrder(accounts.map((account) => account.id));
    }, [accounts]);
    const handleSortByChange = useCallback(
      (value: string) => {
        setSortBy(value);
        if (value === "custom") {
          setShowCustomSortModal(true);
        }
      },
      [setSortBy],
    );
    const isAllPaginatedSelected = useMemo(
      () => isEveryIdSelected(selected, paginatedIds),
      [paginatedIds, selected],
    );
    const isAllFilteredSelectionActive = useMemo(
      () =>
        isAllFilteredSelected &&
        filteredIds.length > 0 &&
        selected.size === filteredIds.length &&
        filteredIds.every((id) => selected.has(id)),
      [filteredIds, isAllFilteredSelected, selected],
    );
    const canSelectAllFilteredAccounts =
      !isAllFilteredSelectionActive &&
      isAllPaginatedSelected &&
      filteredIds.length > paginatedIds.length;
  
    useEffect(() => {
      if (isAllFilteredSelected && !isAllFilteredSelectionActive) {
        setIsAllFilteredSelected(false);
      }
    }, [isAllFilteredSelected, isAllFilteredSelectionActive]);
  
    const handleToggleOverviewAccount = useCallback(
      (accountId: string) => {
        setIsAllFilteredSelected(false);
        toggleSelect(accountId);
      },
      [toggleSelect],
    );
  
    const handleToggleSelectAllPaginated = useCallback(() => {
      setIsAllFilteredSelected(false);
      toggleSelectAll(paginatedIds);
    }, [paginatedIds, toggleSelectAll]);
  
    const handleSelectAllFilteredAccounts = useCallback(() => {
      if (filteredIds.length === 0) return;
      setSelected(new Set(filteredIds));
      setIsAllFilteredSelected(true);
    }, [filteredIds, setSelected]);
  
    const handleClearOverviewSelection = useCallback(() => {
      setSelected(new Set());
      setIsAllFilteredSelected(false);
    }, [setSelected]);
  
    const handleCodexBatchDelete = useCallback(() => {
      const ids = isAllFilteredSelectionActive
        ? filteredIds
        : Array.from(selected);
      if (ids.length === 0) return;
      setDeleteConfirm({
        ids,
        message: isAllFilteredSelectionActive
          ? t("messages.deleteFilteredAccountsConfirm", {
              count: ids.length,
              defaultValue:
                "将删除当前筛选条件下的 {{count}} 个 Codex 账号。此操作不会只删除当前页，确认继续？",
            })
          : t("messages.batchDeleteConfirm", { count: ids.length }),
      });
    }, [
      filteredIds,
      isAllFilteredSelectionActive,
      selected,
      setDeleteConfirm,
      t,
    ]);
  
    const confirmCodexDelete = useCallback(async () => {
      if (!deleteConfirm || batchDeleteBusy) return;
      setBatchDeleteBusy(true);
      setBatchDeleteModalError(null);
      batchDeleteRemoveIdsRef.current = new Set(deleteConfirm.ids);
      try {
        const job = await codexService.startCodexBatchDelete(deleteConfirm.ids);
        batchDeleteRefreshedCompletedRef.current = 0;
        if (shouldAutoHideBatchDeleteJob(job)) {
          await refreshAccountsAfterBatchDelete();
          try {
            await codexService.clearCodexBatchDelete(job.jobId);
          } catch (clearError) {
            console.warn(
              "[Codex Batch Delete] 自动清理已完成任务失败:",
              clearError,
            );
          }
          setBatchDeleteJob(null);
        } else {
          setBatchDeleteJob(job);
        }
        setSelected((prev) => {
          const next = new Set(prev);
          deleteConfirm.ids.forEach((id) => next.delete(id));
          return next;
        });
        setIsAllFilteredSelected(false);
        setDeleteConfirm(null);
        // 用成功提示覆盖页顶旧错误，避免删除后红色报错仍挂着（#1160）
        setMessage({
          text: t("codex.batchDelete.started", {
            count: deleteConfirm.ids.length,
          }),
          tone: "success",
        });
      } catch (error) {
        batchDeleteRemoveIdsRef.current = new Set();
        setBatchDeleteModalError(
          t("messages.actionFailed", {
            action: t("common.delete"),
            error: String(error),
          }),
        );
      } finally {
        setBatchDeleteBusy(false);
      }
    }, [
      batchDeleteBusy,
      deleteConfirm,
      refreshAccountsAfterBatchDelete,
      setDeleteConfirm,
      setMessage,
      setSelected,
      t,
    ]);
  
    useEscClose(Boolean(deleteConfirm) && !batchDeleteBusy, () => {
      setDeleteConfirm(null);
    });
    useEnterConfirm(Boolean(deleteConfirm) && !batchDeleteBusy, () => {
      void confirmCodexDelete();
    });
    useEscClose(Boolean(groupDeleteConfirm) && !deletingGroup, () => {
      setGroupDeleteConfirm(null);
      setGroupDeleteError(null);
    });
    useEnterConfirm(Boolean(groupDeleteConfirm) && !deletingGroup, () => {
      void confirmDeleteGroup();
    });
  
    const handlePauseBatchDelete = useCallback(async () => {
      if (!batchDeleteJob?.jobId || batchDeleteBusy) return;
      setBatchDeleteBusy(true);
      try {
        const job = await codexService.pauseCodexBatchDelete(
          batchDeleteJob.jobId,
        );
        setBatchDeleteJob(job);
        await refreshAccountsAfterBatchDelete();
      } catch (error) {
        setMessage({
          text: t("codex.batchDelete.actionFailed", {
            error: String(error),
          }),
          tone: "error",
        });
      } finally {
        setBatchDeleteBusy(false);
      }
    }, [
      batchDeleteBusy,
      batchDeleteJob?.jobId,
      refreshAccountsAfterBatchDelete,
      setMessage,
      t,
    ]);
  
    const handleResumeBatchDelete = useCallback(async () => {
      if (!batchDeleteJob?.jobId || batchDeleteBusy) return;
      setBatchDeleteBusy(true);
      try {
        setBatchDeleteJob(
          await codexService.resumeCodexBatchDelete(batchDeleteJob.jobId),
        );
      } catch (error) {
        setMessage({
          text: t("codex.batchDelete.actionFailed", {
            error: String(error),
          }),
          tone: "error",
        });
      } finally {
        setBatchDeleteBusy(false);
      }
    }, [batchDeleteBusy, batchDeleteJob?.jobId, setMessage, t]);
  
    const handleRetryFailedBatchDelete = useCallback(async () => {
      if (!batchDeleteJob?.jobId || batchDeleteBusy) return;
      setBatchDeleteBusy(true);
      try {
        const job = await codexService.retryFailedCodexBatchDelete(
          batchDeleteJob.jobId,
        );
        batchDeleteRefreshedCompletedRef.current = job.completed;
        setBatchDeleteJob(job);
      } catch (error) {
        setMessage({
          text: t("codex.batchDelete.actionFailed", {
            error: String(error),
          }),
          tone: "error",
        });
      } finally {
        setBatchDeleteBusy(false);
      }
    }, [batchDeleteBusy, batchDeleteJob?.jobId, setMessage, t]);
  
    const handleClearBatchDelete = useCallback(async () => {
      if (!batchDeleteJob?.jobId || batchDeleteBusy) return;
      setBatchDeleteBusy(true);
      try {
        await codexService.clearCodexBatchDelete(batchDeleteJob.jobId);
        await refreshAccountsAfterBatchDelete();
        batchDeleteRemoveIdsRef.current = new Set();
        batchDeleteRefreshedCompletedRef.current = 0;
        setBatchDeleteJob(null);
      } catch (error) {
        setMessage({
          text: t("codex.batchDelete.actionFailed", {
            error: String(error),
          }),
          tone: "error",
        });
      } finally {
        setBatchDeleteBusy(false);
      }
    }, [
      batchDeleteBusy,
      batchDeleteJob?.jobId,
      refreshAccountsAfterBatchDelete,
      setMessage,
      t,
    ]);
  
    const groupedAccounts = useMemo(() => {
      if (!groupByTag) return [] as Array<[string, typeof filteredAccounts]>;
      const groups = new Map<string, typeof filteredAccounts>();
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      filteredAccounts.forEach((a) => {
        const tags = (a.tags || []).map(normalizeTag).filter(Boolean);
        const matchedTags =
          selectedTags.size > 0
            ? tags.filter((tag) => selectedTags.has(tag))
            : tags;
        if (matchedTags.length === 0) {
          if (!groups.has(untaggedKey)) groups.set(untaggedKey, []);
          groups.get(untaggedKey)?.push(a);
          return;
        }
        matchedTags.forEach((tag) => {
          if (!groups.has(tag)) groups.set(tag, []);
          groups.get(tag)?.push(a);
        });
      });
      return Array.from(groups.entries()).sort(([a], [b]) => {
        if (a === untaggedKey) return -1;
        if (b === untaggedKey) return 1;
        return a.localeCompare(b);
      });
    }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);
  
    const paginatedGroupedAccounts = useMemo(
      () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
      [groupedAccounts, paginatedAccounts],
    );
  
    const accountsById = useMemo(
      () => new Map(overviewAccounts.map((account) => [account.id, account])),
      [overviewAccounts],
    );
  
    const resolveGroupAccounts = useCallback(
      (group: CodexAccountGroup) =>
        group.accountIds
          .map((accountId) => accountsById.get(accountId))
          .filter((account): account is CodexAccount => Boolean(account))
          .sort(compareAccountsBySort),
      [accountsById, compareAccountsBySort],
    );
  
    const handleRefreshGroup = useCallback(
      async (group: CodexAccountGroup) => {
        const groupAccounts = resolveGroupAccounts(group);
        const targetIds = groupAccounts
          .filter(
            (account) =>
              !isCodexApiKeyAccount(account) || isCodexNewApiAccount(account),
          )
          .map((account) => account.id);
  
        if (targetIds.length === 0) {
          setMessage({
            text: t("accounts.groups.refreshEmpty", "当前分组没有可刷新的账号"),
            tone: "error",
          });
          return;
        }
  
        setRefreshingGroupId(group.id);
        try {
          // 显式「刷新分组」：不遵守分组关闭策略，允许用户强制刷新
          const successCount = await codexService.refreshCodexQuotasBatch(
            targetIds,
            {
              respectGroupQuotaRefresh: false,
            },
          );
  
          await fetchAccounts();
          await fetchCurrentAccount();
  
          if (successCount === targetIds.length) {
            setMessage({
              text: t("codex.refreshAllSuccess", { count: successCount }),
            });
            return;
          }
  
          if (successCount > 0) {
            setMessage({
              text: t("codex.refreshAllPartialFailed", {
                success: successCount,
                total: targetIds.length,
              }),
              tone: "error",
            });
            return;
          }
  
          setMessage({
            text: t("codex.refreshFailed", {
              error: t("common.shared.quota.queryFailed", "配额查询失败"),
            }),
            tone: "error",
          });
        } catch (error) {
          setMessage({
            text: t("codex.refreshFailed", {
              error:
                String(error ?? "").replace(/^Error:\s*/, "") ||
                t("common.shared.quota.queryFailed", "配额查询失败"),
            }),
            tone: "error",
          });
        } finally {
          setRefreshingGroupId(null);
        }
      },
      [fetchAccounts, fetchCurrentAccount, resolveGroupAccounts, setMessage, t],
    );
  
    useEffect(() => {
      const teamAccountIds = paginatedAccounts
        .filter(
          (account) =>
            !hasCodexAccountStructure(account) ||
            (isCodexTeamLikePlan(account.plan_type) &&
              !hasCodexAccountName(account)),
        )
        .map((account) => account.id);
      if (teamAccountIds.length === 0) return;
      void hydrateAccountProfilesIfNeeded(teamAccountIds);
    }, [hydrateAccountProfilesIfNeeded, paginatedAccounts]);
  
    const resolveGroupLabel = (groupKey: string) =>
      groupKey === untaggedKey
        ? t("accounts.defaultGroup", "默认分组")
        : groupKey;
  
    const resolveVisibleQuotaItems = useCallback(
      (
        presentation: ReturnType<typeof buildCodexAccountPresentation>,
        isApiKeyAccount: boolean,
        isNewApiAccount: boolean,
      ) => {
        if (isApiKeyAccount && !isNewApiAccount) return [];
        return presentation.quotaItems.filter((item) => {
          if (!showCodeReviewQuota && item.key === "code_review") return false;
          if (!showAdditionalQuota && item.key.startsWith("additional:")) {
            return false;
          }
          return true;
        });
      },
      [showAdditionalQuota, showCodeReviewQuota],
    );
  
    const renderResetCreditControls = (account: CodexAccount) => {
      if (isCodexApiKeyAccount(account) || isCodexAgentIdentityAccount(account))
        return null;
  
      const creditDetails = getResetCreditDetails(account);
      const availableCount = getResetCreditsAvailable(account);
      if (availableCount == null && creditDetails.length === 0) return null;
  
      const displayCount =
        availableCount ?? creditDetails.filter(isAvailableResetCredit).length;
      const isResetting = resettingResetCreditAccountId === account.id;
      const isDisabled = isResetting;
      const titleText =
        displayCount > 0
          ? buildResetCreditsTitle(account, displayCount)
          : t("codex.quota.resetCreditDetailsTitle", "重置次数明细");
  
      return (
        <div className="codex-reset-credit-row inline">
          <button
            type="button"
            className={`codex-reset-credit-pill ${
              displayCount > 0 ? "is-available" : "is-unavailable"
            }`}
            onClick={() => openResetCreditConfirmModal(account)}
            disabled={isDisabled}
            title={titleText}
          >
            {isResetting ? (
              <RefreshCw size={13} className="loading-spinner" />
            ) : (
              <RotateCw size={13} />
            )}
            {t("codex.quota.resetCredits", { count: displayCount })}
          </button>
        </div>
      );
    };
  return {
    applyWindowStatsToQuotaItems,
    authFailedExportAccountIds,
    buildAccountLaunchPreviewActions,
    buildAccountLaunchPreviewSummary,
    buildLocalAccessLaunchPreviewActions,
    buildLocalAccessLaunchPreviewSummary,
    canSelectAllFilteredAccounts,
    confirmCodexDelete,
    customSortAccounts,
    errorAccountIds,
    exportSelectionCount,
    filteredAccounts,
    filteredIds,
    handleClearBatchDelete,
    handleClearErrorAccounts,
    handleClearOverviewSelection,
    handleCodexBatchDelete,
    handleCustomSortDragMove,
    handleCustomSortDragStart,
    handleExportAuthFailedAccounts,
    handlePauseBatchDelete,
    handleRefreshGroup,
    handleResumeBatchDelete,
    handleRetryFailedBatchDelete,
    handleSelectAllFilteredAccounts,
    handleSortByChange,
    handleToggleOverviewAccount,
    handleToggleSelectAllPaginated,
    hasActiveOverviewFilters,
    hasDetectableFullQuotaWakeupAccounts,
    isAllFilteredSelectionActive,
    isAllPaginatedSelected,
    isCustomSortActive,
    moveCustomSortAccount,
    openFullQuotaWakeupTestModal,
    overviewCurrentAccountId,
    overviewFilterChips,
    overviewTotalCount,
    overviewVisibleCount,
    paginatedAccounts,
    paginatedGroupedAccounts,
    pagination,
    renderResetCreditControls,
    resetCustomSortOrder,
    resolveGroupAccounts,
    resolveGroupLabel,
    resolveVisibleQuotaItems,
    showOverviewFilterBanner,
    sortedAccountsForInstances,
    stopCustomSortDragging,
  };
}
