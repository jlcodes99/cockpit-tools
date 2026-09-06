import { useState, useEffect, useMemo, useCallback } from "react";
import { createPortal } from "react-dom";
import { RefreshCw, X, CircleAlert, Info, Link2 } from "lucide-react";
import * as codexService from "../services/codexService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { presentWindowsOperationError } from "../utils/windowsOperationDialog";
import { assignAccountsToCodexGroup, deleteCodexGroup, removeAccountsFromCodexGroup } from "../services/codexAccountGroupService";
import { formatCodexLoginProvider, getCodexAuthMetadata, getCodexPlanFilterKey, getCodexSubscriptionPresentationForAccount, isCodexApiKeyAccount, isCodexNewApiAccount, isCodexTeamLikePlan, type CodexQuotaErrorInfo } from "../types/codex";
import { canAddCodexAccountToLocalAccess, filterCodexLocalAccessAccountIds } from "../utils/codexLocalAccessAccounts";
import { extractCodexQuotaErrorCode, extractCodexQuotaErrorStatusCode, isBlockingCodexAccountQuotaError, isVerboseCodexQuotaErrorMessage, summarizeCodexQuotaErrorMessage } from "../utils/codexQuotaError";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { isCodexIdTokenReauthReason } from "../utils/codexSwitchAuthFailure";
import { CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, getCodexPlanBadgeStyle, type CodexPlanBadgeStyle } from "../utils/codexPreferences";
import { invoke } from "@tauri-apps/api/core";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { DEFAULT_CODEX_INSTANCE_ID } from "../components/codex/CodexLaunchPreviewModal";
import type { MultiSelectFilterOption } from "../components/MultiSelectFilterDropdown";
import type { SingleSelectFilterOption } from "../components/SingleSelectFilterDropdown";
import type { CodexAccount } from "../types/codex";
import type { CodexLocalAccessAddressKind, CodexLocalAccessCustomRoutingRule, CodexLocalAccessImageGenerationPolicy, CodexLocalAccessRoutingStrategy, CodexLocalAccessScope } from "../types/codexLocalAccess";
import { CODEX_API_SERVICE_BIND_ID } from "../types/instance";
import { buildCodexOverviewGroupFilterOptions, buildCodexOverviewSortOptions, buildCodexPlanFilterOptions, createCodexOverviewAccountComparator, createCodexPlanFilterCounts, filterAndSortCodexOverviewAccounts, incrementCodexPlanFilterCount, isCodexOverviewAccountAbnormal, isCodexOverviewAccountSubscriptionExpired, isCodexOverviewAccountZeroQuota } from "../utils/codexAccountOverview";
import { summarizeCodexQuotaPool } from "../utils/codexQuotaPool";
import { buildValidAccountsFilterOption } from "../utils/accountValidityFilter";
import { buildPaginationPageSizeStorageKey, usePagination } from "../hooks/usePagination";
import { CODEX_LOCAL_ACCESS_FALLBACK_BASE_URL, isAbnormalLocalAccessAccountFailure, normalizeLocalAccessAddressKind, OAUTH_BINDING_PAGE_SIZE_OPTIONS, persistLocalAccessAddressKind, type LocalAccessAccountPoolHealthSummary } from "./codexAccountsControllerModel";
import type { useCodexAccountsBaseController } from "./useCodexAccountsBaseController";
import type { useCodexAccountsOAuthController } from "./useCodexAccountsOAuthController";
import type { useCodexAccountsAccessController } from "./useCodexAccountsAccessController";

/** 封装 useCodexAccountsPageController 的 useCodexAccountsLocalAccessController 业务域状态与动作。 */
export function useCodexAccountsLocalAccessController(context: Pick<ReturnType<typeof useCodexAccountsBaseController> & ReturnType<typeof useCodexAccountsOAuthController> & ReturnType<typeof useCodexAccountsAccessController>,
  | "accounts"
  | "activeGroupId"
  | "addingLocalAccessAccountId"
  | "batchImportTargetGroupId"
  | "clearGroupFilter"
  | "codexAccountsRef"
  | "codexAddTargetGroupId"
  | "codexGroups"
  | "codexInstanceStore"
  | "currentAccount"
  | "customSortOrder"
  | "deletingGroup"
  | "ensureLocalAccessEntryVisible"
  | "fetchAccounts"
  | "fetchCurrentAccount"
  | "filterTypes"
  | "formatCodexAuthFailureMessage"
  | "groupDeleteConfirm"
  | "groupFilter"
  | "groupQuickAddGroupId"
  | "launchPreviewInstanceId"
  | "localAccessAddressKind"
  | "localAccessCollection"
  | "localAccessHealthActionBusy"
  | "localAccessHideSubmitting"
  | "localAccessLaunchCurrent"
  | "localAccessPortKilling"
  | "localAccessRefreshing"
  | "localAccessSaving"
  | "localAccessSidecarRestarting"
  | "localAccessStarting"
  | "localAccessState"
  | "normalizeTag"
  | "oauthAccounts"
  | "oauthBindingAccountId"
  | "oauthBindingTargetActive"
  | "openCodexAddModal"
  | "quotaErrorDetail"
  | "reloadCodexGroups"
  | "reloadLocalAccessState"
  | "requestLocalAccessRiskNotice"
  | "searchQuery"
  | "selected"
  | "setActiveGroupId"
  | "setAddingLocalAccessAccountId"
  | "setBatchImportTargetGroupId"
  | "setCodexAddTargetGroupId"
  | "setDeletingGroup"
  | "setGroupDeleteConfirm"
  | "setGroupDeleteError"
  | "setGroupQuickAddGroupId"
  | "setLocalAccessAddressKind"
  | "setLocalAccessCopiedField"
  | "setLocalAccessEntryVisible"
  | "setLocalAccessHealthActionBusy"
  | "setLocalAccessHideSubmitting"
  | "setLocalAccessLaunchCurrent"
  | "setLocalAccessLaunchPreviewOpen"
  | "setLocalAccessModalMode"
  | "setLocalAccessPortKilling"
  | "setLocalAccessRefreshing"
  | "setLocalAccessSaving"
  | "setLocalAccessSidecarRestarting"
  | "setLocalAccessStarting"
  | "setLocalAccessState"
  | "setMessage"
  | "setQuotaErrorDetail"
  | "setRemovingGroupAccountIds"
  | "setSelected"
  | "setShowLocalAccessHideConfirm"
  | "setShowLocalAccessModal"
  | "sortBy"
  | "sortDirection"
  | "t"
  | "tagFilter"
>) {
  const {
    accounts,
    activeGroupId,
    addingLocalAccessAccountId,
    batchImportTargetGroupId,
    clearGroupFilter,
    codexAccountsRef,
    codexAddTargetGroupId,
    codexGroups,
    codexInstanceStore,
    currentAccount,
    customSortOrder,
    deletingGroup,
    ensureLocalAccessEntryVisible,
    fetchAccounts,
    fetchCurrentAccount,
    filterTypes,
    formatCodexAuthFailureMessage,
    groupDeleteConfirm,
    groupFilter,
    groupQuickAddGroupId,
    launchPreviewInstanceId,
    localAccessAddressKind,
    localAccessCollection,
    localAccessHealthActionBusy,
    localAccessHideSubmitting,
    localAccessLaunchCurrent,
    localAccessPortKilling,
    localAccessRefreshing,
    localAccessSaving,
    localAccessSidecarRestarting,
    localAccessStarting,
    localAccessState,
    normalizeTag,
    oauthAccounts,
    oauthBindingAccountId,
    oauthBindingTargetActive,
    openCodexAddModal,
    quotaErrorDetail,
    reloadCodexGroups,
    reloadLocalAccessState,
    requestLocalAccessRiskNotice,
    searchQuery,
    selected,
    setActiveGroupId,
    setAddingLocalAccessAccountId,
    setBatchImportTargetGroupId,
    setCodexAddTargetGroupId,
    setDeletingGroup,
    setGroupDeleteConfirm,
    setGroupDeleteError,
    setGroupQuickAddGroupId,
    setLocalAccessAddressKind,
    setLocalAccessCopiedField,
    setLocalAccessEntryVisible,
    setLocalAccessHealthActionBusy,
    setLocalAccessHideSubmitting,
    setLocalAccessLaunchCurrent,
    setLocalAccessLaunchPreviewOpen,
    setLocalAccessModalMode,
    setLocalAccessPortKilling,
    setLocalAccessRefreshing,
    setLocalAccessSaving,
    setLocalAccessSidecarRestarting,
    setLocalAccessStarting,
    setLocalAccessState,
    setMessage,
    setQuotaErrorDetail,
    setRemovingGroupAccountIds,
    setSelected,
    setShowLocalAccessHideConfirm,
    setShowLocalAccessModal,
    sortBy,
    sortDirection,
    t,
    tagFilter,
  } = context;
  const [clearClientAuthBusy, setClearClientAuthBusy] = useState(false);
  const [clearClientAuthError, setClearClientAuthError] = useState<string | null>(
    null,
  );
  const resolveQuotaErrorMeta = useCallback(
      (quotaError?: CodexQuotaErrorInfo) => {
        if (!quotaError?.message) {
          return {
            statusCode: "",
            errorCode: "",
            displayText: "",
            rawMessage: "",
            isRefreshRequestFailure: false,
            isVerbose: false,
          };
        }
        const rawMessage = quotaError.message;
        const normalizedRawMessage = rawMessage.trim();
        const lowerRawMessage = normalizedRawMessage.toLowerCase();
        const requestErrorIndex = lowerRawMessage.indexOf(
          "error sending request",
        );
        const isRefreshRequestFailure = requestErrorIndex >= 0;
        const requestErrorMessage = isRefreshRequestFailure
          ? normalizedRawMessage.slice(requestErrorIndex).trim()
          : normalizedRawMessage;
        const statusCode = extractCodexQuotaErrorStatusCode(rawMessage);
        const errorCode = extractCodexQuotaErrorCode(rawMessage, quotaError.code);
        const authFailureText =
          formatCodexAuthFailureMessage(normalizedRawMessage);
        const isVerbose = isVerboseCodexQuotaErrorMessage(normalizedRawMessage);
        let displayText =
          authFailureText !== normalizedRawMessage
            ? authFailureText
            : errorCode ||
              (isRefreshRequestFailure
                ? t("codex.quotaError.requestFailedManualRetry", {
                    error: summarizeCodexQuotaErrorMessage(requestErrorMessage),
                  })
                : "");
        if (!displayText) {
          if (statusCode) {
            displayText = t("codex.quotaError.httpStatusSummary", {
              status: statusCode,
              defaultValue: "API 返回错误 {{status}}",
            });
          } else if (isVerbose) {
            displayText = t(
              "codex.quotaError.generic",
              "配额刷新失败，请稍后重试",
            );
          } else {
            displayText = summarizeCodexQuotaErrorMessage(normalizedRawMessage);
          }
        } else if (isVerboseCodexQuotaErrorMessage(displayText)) {
          // Never keep HTML/body dumps in the card summary.
          displayText = statusCode
            ? t("codex.quotaError.httpStatusSummary", {
                status: statusCode,
                defaultValue: "API 返回错误 {{status}}",
              })
            : summarizeCodexQuotaErrorMessage(displayText);
        }
        return {
          statusCode,
          errorCode,
          displayText,
          rawMessage,
          isRefreshRequestFailure,
          isVerbose:
            isVerbose ||
            normalizedRawMessage.length > displayText.length + 12 ||
            normalizedRawMessage !== displayText,
        };
      },
      [formatCodexAuthFailureMessage, t],
    );
  
    const openQuotaErrorDetail = useCallback(
      (
        accountName: string,
        message: string,
        summary?: string,
        reauthorizeAccountId?: string,
        title?: string,
        clearClientAuthObservationAccountId?: string,
      ) => {
        const text = message.trim();
        if (!text) return;
        const normalizedSummary = summary?.trim();
        setQuotaErrorDetail({
          accountName: accountName.trim() || t("common.unknown", "未知"),
          title: title?.trim() || undefined,
          summary:
            normalizedSummary && normalizedSummary !== text
              ? normalizedSummary
              : undefined,
          reauthorizeAccountId: reauthorizeAccountId?.trim() || undefined,
          clearClientAuthObservationAccountId:
            clearClientAuthObservationAccountId?.trim() || undefined,
          message: text,
        });
        setClearClientAuthError(null);
      },
      [t],
    );
  
    const renderQuotaErrorInline = useCallback(
      (options: {
        accountName: string;
        displayText: string;
        rawMessage: string;
        isVerbose: boolean;
        detailSummary?: string;
        detailReauthorizeAccountId?: string;
        detailTitle?: string;
        clearClientAuthObservationAccountId?: string;
        isRefreshNotice?: boolean;
        showReauthorize?: boolean;
        onReauthorize?: () => void;
        table?: boolean;
      }) => {
        const {
          accountName,
          displayText,
          rawMessage,
          isVerbose,
          detailSummary,
          detailReauthorizeAccountId,
          detailTitle,
          clearClientAuthObservationAccountId,
          isRefreshNotice = false,
          showReauthorize = false,
          onReauthorize,
          table = false,
        } = options;
        const showDetailAction =
          Boolean(detailSummary?.trim()) ||
          isVerbose ||
          rawMessage.trim().length > displayText.trim().length + 12 ||
          rawMessage.trim() !== displayText.trim();
        return (
          <div
            className={`quota-error-inline${table ? " table" : ""}${
              isRefreshNotice ? " quota-refresh-notice" : ""
            }`}
          >
            {isRefreshNotice ? (
              <Info size={table ? 12 : 14} />
            ) : (
              <CircleAlert size={table ? 12 : 14} />
            )}
            <span className="quota-error-inline-text" title={displayText}>
              {displayText}
            </span>
            {showDetailAction && (
              <button
                type="button"
                className="btn btn-sm btn-outline quota-error-action"
                onClick={() =>
                  openQuotaErrorDetail(
                    accountName,
                    rawMessage,
                    detailSummary,
                    detailReauthorizeAccountId,
                    detailTitle,
                    clearClientAuthObservationAccountId,
                  )
                }
                title={t("codex.quotaError.viewDetails", "查看详情")}
              >
                {t("codex.quotaError.viewDetails", "查看详情")}
              </button>
            )}
            {showReauthorize && onReauthorize && (
              <button
                type="button"
                className="btn btn-sm btn-outline quota-error-action"
                onClick={onReauthorize}
                title={t("common.shared.addModal.oauth", "OAuth 授权")}
              >
                {t("common.shared.addModal.oauth", "OAuth 授权")}
              </button>
            )}
          </div>
        );
      },
      [openQuotaErrorDetail, t],
    );
  
    const renderQuotaErrorDetailModal = () => {
      if (!quotaErrorDetail) return null;
      const clipboardText = quotaErrorDetail.summary
        ? `${quotaErrorDetail.summary}\n\n${quotaErrorDetail.message}`
        : quotaErrorDetail.message;
      const clearClientAuth = async () => {
        const accountId = quotaErrorDetail.clearClientAuthObservationAccountId;
        if (!accountId || clearClientAuthBusy) return;
        setClearClientAuthBusy(true);
        setClearClientAuthError(null);
        try {
          await codexService.clearClientAuthObservation(accountId);
          setQuotaErrorDetail(null);
          await fetchAccounts({ allowEmpty: true });
        } catch (error) {
          setClearClientAuthError(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setClearClientAuthBusy(false);
        }
      };
      return createPortal(
        <div className="modal-overlay">
          <div
            className="modal-content codex-quota-error-detail-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h3>
                {quotaErrorDetail.title ||
                  t("codex.quotaError.detailTitle", "错误详情")}
              </h3>
              <button
                type="button"
                className="modal-close"
                onClick={() => setQuotaErrorDetail(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X size={16} />
              </button>
            </div>
            <div className="modal-body codex-quota-error-detail-body">
              <div className="codex-quota-error-detail-account">
                {quotaErrorDetail.accountName}
              </div>
              {quotaErrorDetail.summary && (
                <div className="codex-quota-error-detail-summary">
                  <Info size={16} />
                  <span>{quotaErrorDetail.summary}</span>
                </div>
              )}
              <pre className="codex-quota-error-detail-text">
                {quotaErrorDetail.message}
              </pre>
              {clearClientAuthError && (
                <div className="codex-switch-progress-error" role="alert">
                  {clearClientAuthError}
                </div>
              )}
            </div>
            <div className="modal-footer">
              {quotaErrorDetail.clearClientAuthObservationAccountId && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => void clearClientAuth()}
                  disabled={clearClientAuthBusy}
                >
                  {clearClientAuthBusy
                    ? t("common.loading", "加载中...")
                    : t("codex.switchAuth.clearClientAuth", "清除异常标识")}
                </button>
              )}
              {quotaErrorDetail.reauthorizeAccountId && (
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => {
                    const account = codexAccountsRef.current.find(
                      (item) => item.id === quotaErrorDetail.reauthorizeAccountId,
                    );
                    setQuotaErrorDetail(null);
                    if (account) {
                      openCodexAddModal("oauth", account);
                    }
                  }}
                >
                  {t("common.reauthorize", "重新授权")}
                </button>
              )}
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => {
                  void navigator.clipboard
                    ?.writeText(clipboardText)
                    .catch(() => undefined);
                }}
              >
                {t("common.copy", "复制")}
              </button>
              <button
                type="button"
                className={`btn ${
                  quotaErrorDetail.reauthorizeAccountId
                    ? "btn-secondary"
                    : "btn-primary"
                }`}
                onClick={() => setQuotaErrorDetail(null)}
              >
                {t("common.close", "关闭")}
              </button>
            </div>
          </div>
        </div>,
        document.body,
      );
    };
  
    const shouldOfferReauthorizeAction = useCallback(
      (quotaErrorMeta: {
        statusCode: string;
        errorCode: string;
        rawMessage: string;
      }) => {
        const statusCode = quotaErrorMeta.statusCode.trim();
        const errorCode = quotaErrorMeta.errorCode.trim().toLowerCase();
        const rawMessage = quotaErrorMeta.rawMessage.trim().toLowerCase();
        if (!statusCode && !errorCode && !rawMessage) return false;
        if (
          errorCode === "unsupported_country_region_territory" ||
          rawMessage.includes("unsupported_country_region_territory") ||
          rawMessage.includes("当前网络地区不支持刷新 codex 授权")
        ) {
          return false;
        }
  
        return (
          statusCode === "401" ||
          errorCode === "refresh_token_expired" ||
          errorCode === "refresh_token_invalidated" ||
          errorCode === "token_invalidated" ||
          errorCode === "invalid_grant" ||
          errorCode === "invalid_token" ||
          rawMessage.includes("refresh_token_expired") ||
          rawMessage.includes("refresh_token_invalidated") ||
          rawMessage.includes("token_invalidated") ||
          rawMessage.includes("your authentication token has been invalidated") ||
          rawMessage.includes("401 unauthorized") ||
          rawMessage.includes("invalid_grant") ||
          rawMessage.includes("token 已过期且无 refresh_token") ||
          rawMessage.includes("缺少 refresh_token") ||
          rawMessage.includes("token 已过期且刷新失败") ||
          rawMessage.includes("刷新 token 失败") ||
          isCodexIdTokenReauthReason(quotaErrorMeta.rawMessage)
        );
      },
      [],
    );
  
    const [planBadgeStyle, setPlanBadgeStyle] = useState<CodexPlanBadgeStyle>(
      getCodexPlanBadgeStyle,
    );
  
    useEffect(() => {
      const syncPlanBadgeStyle = () => {
        setPlanBadgeStyle(getCodexPlanBadgeStyle());
      };
      window.addEventListener(
        CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT,
        syncPlanBadgeStyle as EventListener,
      );
      return () => {
        window.removeEventListener(
          CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT,
          syncPlanBadgeStyle as EventListener,
        );
      };
    }, []);
  
    const accountPresentations = useMemo(() => {
      const map = new Map<
        string,
        ReturnType<typeof buildCodexAccountPresentation>
      >();
      // planBadgeStyle forces rebuild when quick-settings style changes (event-driven).
      void planBadgeStyle;
      accounts.forEach((a) => map.set(a.id, buildCodexAccountPresentation(a, t)));
      return map;
    }, [accounts, t, planBadgeStyle]);
  
    const resolvePresentation = useCallback(
      (account: CodexAccount) =>
        accountPresentations.get(account.id) ??
        buildCodexAccountPresentation(account, t),
      [accountPresentations, t],
    );
  
    const resolveSubscriptionPresentation = useCallback(
      (account: CodexAccount) =>
        getCodexSubscriptionPresentationForAccount(account, t),
      [t],
    );
  
    const resolveSingleExportBaseName = useCallback(
      (account: CodexAccount) => {
        const display = (
          resolvePresentation(account).displayName || account.id
        ).trim();
        const atIndex = display.indexOf("@");
        return atIndex > 0 ? display.slice(0, atIndex) : display;
      },
      [resolvePresentation],
    );
  
    const resolvePlanKey = useCallback(
      (account: CodexAccount) => getCodexPlanFilterKey(account),
      [],
    );
  
    const accountIdLabel = t("kiro.account.userId", "User ID");
  
    const accountMetaMap = useMemo(() => {
      const map = new Map<
        string,
        {
          chatgptAccountId: string;
          signedInWithText: string;
          userId: string;
          accountContextText: string;
        }
      >();
      const noneText = t("common.none", "暂无");
  
      accounts.forEach((account) => {
        if (isCodexApiKeyAccount(account)) {
          map.set(account.id, {
            chatgptAccountId: t("common.none", "暂无"),
            signedInWithText: "",
            userId: "",
            accountContextText: "",
          });
          return;
        }
  
        const metadata = getCodexAuthMetadata(account);
        const organizationId = (account.organization_id || "").trim();
        const matchedWorkspace = organizationId
          ? metadata.workspaces.find(
              (workspace) => (workspace.id || "").trim() === organizationId,
            )
          : null;
        const defaultWorkspace = metadata.workspaces.find(
          (workspace) => workspace.is_default,
        );
        const fallbackWorkspace =
          matchedWorkspace || defaultWorkspace || metadata.workspaces[0] || null;
        const workspaceTitle = fallbackWorkspace?.title?.trim() || "";
        const accountName = (account.account_name || "").trim();
        const structure = (account.account_structure || "").trim().toLowerCase();
        const isTeamLikePlan = isCodexTeamLikePlan(account.plan_type);
        const isPersonalStructure = structure.includes("personal");
        const accountContextText = isPersonalStructure
          ? t("codex.account.personal", "个人账户")
          : !structure && !isTeamLikePlan
            ? t("codex.account.personal", "个人账户")
            : accountName || workspaceTitle || "";
        const loginProvider =
          formatCodexLoginProvider(metadata.authProvider) ||
          t("kiro.account.providerUnknown", "Unknown");
        const userId =
          (metadata.userId || account.user_id || "").trim() || noneText;
        const signedInWithText = t("kiro.account.signedInWith", {
          provider: loginProvider,
          defaultValue: "Signed in with {{provider}}",
        });
        map.set(account.id, {
          chatgptAccountId:
            (metadata.chatgptAccountId || account.account_id || "").trim() ||
            noneText,
          signedInWithText,
          userId,
          accountContextText,
        });
      });
  
      return map;
    }, [accounts, t]);
  
    const resolveAccountMeta = useCallback(
      (account: CodexAccount) =>
        accountMetaMap.get(account.id) ?? {
          chatgptAccountId: t("common.none", "暂无"),
          signedInWithText: t("kiro.account.signedInWith", {
            provider: t("kiro.account.providerUnknown", "Unknown"),
            defaultValue: "Signed in with {{provider}}",
          }),
          userId: t("common.none", "暂无"),
          accountContextText: "",
        },
      [accountMetaMap, t],
    );
  
    const isAbnormalAccount = useCallback(
      (account: CodexAccount) => isCodexOverviewAccountAbnormal(account),
      [],
    );
  
    const localAccessAccountIds = useMemo(() => {
      const ids = [...(localAccessCollection?.accountIds ?? [])];
      for (const apiKey of localAccessCollection?.apiKeys ?? []) {
        ids.push(...(apiKey.accountIds ?? []));
      }
      return Array.from(new Set(ids));
    }, [localAccessCollection?.accountIds, localAccessCollection?.apiKeys]);
    const localAccessAccountIdSet = useMemo(
      () => new Set(localAccessAccountIds),
      [localAccessAccountIds],
    );
    const localAccessAccounts = useMemo(
      () =>
        localAccessAccountIds
          .map((accountId) =>
            accounts.find((account) => account.id === accountId),
          )
          .filter((account): account is CodexAccount => Boolean(account)),
      [accounts, localAccessAccountIds],
    );
    const localAccessQuotaPoolSummary = useMemo(
      () => summarizeCodexQuotaPool(localAccessAccounts),
      [localAccessAccounts],
    );
    const localAccessAccountPoolHealthSummary =
      useMemo<LocalAccessAccountPoolHealthSummary>(() => {
        const accountById = new Map(
          accounts.map((account) => [account.id, account]),
        );
        const healthById = new Map(
          (localAccessState?.accountHealth ?? []).map((health) => [
            health.accountId,
            health,
          ]),
        );
        const poolUnavailableAccountIds = new Set<string>();
        (localAccessState?.accountPoolHealth ?? []).forEach((pool) => {
          const statuses = (pool.accountStatuses ?? []).filter((member) =>
            member.accountId.trim(),
          );
          if (statuses.length === 0) {
            (localAccessCollection?.accountIds ?? []).forEach((accountId) =>
              poolUnavailableAccountIds.add(accountId),
            );
            return;
          }
          statuses
            .filter((member) => !member.available)
            .forEach((member) =>
              poolUnavailableAccountIds.add(member.accountId.trim()),
            );
        });
        const summary: LocalAccessAccountPoolHealthSummary = {
          total: localAccessAccountIds.length,
          available: 0,
          abnormal: 0,
          cooldown: 0,
          missing: 0,
          authError: 0,
          quotaLimited: 0,
        };
  
        localAccessAccountIds.forEach((accountId) => {
          const account = accountById.get(accountId);
          const health = healthById.get(accountId);
          if (!account) {
            summary.missing += 1;
            return;
          }
          if (health?.cooldowns?.length) {
            summary.cooldown += 1;
            return;
          }
          if (isBlockingCodexAccountQuotaError(account)) {
            summary.quotaLimited += 1;
            return;
          }
          if (isAbnormalLocalAccessAccountFailure(health)) {
            summary.authError += 1;
            summary.abnormal += 1;
            return;
          }
          if (health && !health.available) {
            return;
          }
          if (poolUnavailableAccountIds.has(accountId)) {
            return;
          }
          summary.available += 1;
        });
  
        return summary;
      }, [
        accounts,
        localAccessAccountIds,
        localAccessState?.accountHealth,
        localAccessState?.accountPoolHealth,
      ]);
    const localAccessAccountPoolHealthHasIssue =
      localAccessAccountPoolHealthSummary.available <
      localAccessAccountPoolHealthSummary.total ||
      localAccessAccountPoolHealthSummary.abnormal > 0 ||
      localAccessAccountPoolHealthSummary.cooldown > 0;
    const localAccessQuotaPoolLabels = useMemo(
      () => ({
        weekly: t("codex.localAccess.quotaPool.weeklyShort", "周"),
        title: t("codex.localAccess.quotaPool.title", "额度池"),
      }),
      [t],
    );
    const localAccessQuotaPreviewItems = useMemo(
      () => localAccessQuotaPoolSummary.visiblePlans.slice(0, 3),
      [localAccessQuotaPoolSummary.visiblePlans],
    );
    const localAccessQuotaHiddenCount = Math.max(
      0,
      localAccessQuotaPoolSummary.visiblePlans.length -
        localAccessQuotaPreviewItems.length,
    );
    const overviewAccounts = accounts;
    const localAccessScope = localAccessCollection?.accessScope ?? "localhost";
    const localAccessScopeLabel =
      localAccessScope === "lan"
        ? t("codex.localAccess.accessScopeLanShort", "本机+局域网")
        : t("codex.localAccess.accessScopeLocalhostShort", "仅本机");
    const localAccessBusy =
      localAccessSaving ||
      localAccessStarting ||
      localAccessRefreshing ||
      localAccessPortKilling ||
      localAccessSidecarRestarting;
    const selectedLocalAccessAddressKind: CodexLocalAccessAddressKind =
      localAccessAddressKind === "lan" && localAccessState?.lanBaseUrl
        ? "lan"
        : "local";
    const localAccessAddressOptions = useMemo(
      () => [
        {
          value: "local",
          label: t("codex.localAccess.addressLocal", "本机"),
        },
        ...(localAccessState?.lanBaseUrl
          ? [
              {
                value: "lan",
                label: t("codex.localAccess.addressLan", "局域网"),
              },
            ]
          : []),
      ],
      [localAccessState?.lanBaseUrl, t],
    );
    const handleLocalAccessAddressKindChange = useCallback((value: string) => {
      const next = normalizeLocalAccessAddressKind(value);
      setLocalAccessAddressKind(next);
      persistLocalAccessAddressKind(next);
    }, []);
  
    const resolveLocalAccessBaseUrl = useCallback(() => {
      if (
        selectedLocalAccessAddressKind === "lan" &&
        localAccessState?.lanBaseUrl
      ) {
        return localAccessState.lanBaseUrl;
      }
      if (!localAccessCollection)
        return localAccessState?.baseUrl || CODEX_LOCAL_ACCESS_FALLBACK_BASE_URL;
      return (
        localAccessState?.baseUrl ||
        `http://127.0.0.1:${localAccessCollection.port}/v1`
      );
    }, [
      localAccessCollection,
      localAccessState?.baseUrl,
      localAccessState?.lanBaseUrl,
      selectedLocalAccessAddressKind,
    ]);
  
    const handleCopyLocalAccessValue = useCallback(
      async (field: "baseUrl" | "apiKey", value: string) => {
        try {
          await navigator.clipboard.writeText(value);
          setLocalAccessCopiedField(field);
          window.setTimeout(() => {
            setLocalAccessCopiedField((current) =>
              current === field ? null : current,
            );
          }, 1200);
        } catch (error) {
          console.error("Failed to copy local access value:", error);
          setMessage({
            text: t("common.shared.export.copyFailed", "复制失败，请手动复制"),
            tone: "error",
          });
        }
      },
      [setMessage, t],
    );
  
    const openLocalAccessPanel = useCallback(() => {
      setLocalAccessModalMode("panel");
      setShowLocalAccessModal(true);
    }, []);
  
    const openCodexApiServicePage = useCallback(() => {
      setShowLocalAccessModal(false);
      window.dispatchEvent(
        new CustomEvent("app-request-navigate", {
          detail: "codex-api-service",
        }),
      );
    }, []);
  
    const openLocalAccessMemberPicker = useCallback(() => {
      setLocalAccessModalMode("members");
      setShowLocalAccessModal(true);
    }, []);
  
    const handleHideLocalAccessEntry = useCallback(() => {
      setShowLocalAccessHideConfirm(true);
    }, []);
  
    const handleRecoverLocalAccessAccounts = useCallback(
      async (accountIds: string[]) => {
        if (localAccessHealthActionBusy || accountIds.length === 0) return;
        setLocalAccessHealthActionBusy(true);
        try {
          const nextState =
            await codexLocalAccessService.recoverCodexLocalAccessAccounts(
              accountIds,
            );
          setLocalAccessState(nextState);
          setMessage({
            text: t("codex.localAccess.accountPoolHealth.recoverSuccess", {
              count: accountIds.length,
              defaultValue: "已提交 {{count}} 个账号的恢复操作",
            }),
          });
        } catch (error) {
          console.error("Failed to recover local access accounts:", error);
          const message = String(error).replace(/^Error:\s*/, "");
          setMessage({
            text: t("messages.actionFailed", {
              action: t(
                "codex.localAccess.accountPoolHealth.recover",
                "恢复账号状态",
              ),
              error: message,
            }),
            tone: "error",
          });
          throw new Error(message);
        } finally {
          setLocalAccessHealthActionBusy(false);
        }
      },
      [localAccessHealthActionBusy, setMessage, t],
    );
  
    const confirmHideLocalAccessEntry = useCallback(async () => {
      if (localAccessHideSubmitting) return;
      setLocalAccessHideSubmitting(true);
      try {
        if (localAccessCollection?.enabled) {
          const nextState =
            await codexLocalAccessService.setCodexLocalAccessEnabled(false);
          setLocalAccessState(nextState);
        }
        await invoke("set_codex_local_access_entry_visible", { enabled: false });
        setLocalAccessEntryVisible(false);
        setShowLocalAccessHideConfirm(false);
        window.dispatchEvent(new Event("codex-local-access-state-updated"));
        window.dispatchEvent(new Event("config-updated"));
      } catch (error) {
        console.error("Failed to hide codex local access entry:", error);
        setMessage({
          text: t("messages.actionFailed", {
            action: t("codex.localAccess.hideEntryAction", "关闭 API 服务入口"),
            error: String(error).replace(/^Error:\s*/, ""),
          }),
          tone: "error",
        });
      } finally {
        setLocalAccessHideSubmitting(false);
      }
    }, [
      localAccessCollection?.enabled,
      localAccessHideSubmitting,
      setMessage,
      t,
    ]);
  
    useEffect(() => {
      void reloadLocalAccessState();
    }, [accounts, reloadLocalAccessState]);
  
    const localAccessModalSelectedIds = useMemo(
      () => [...(localAccessCollection?.accountIds ?? [])],
      [localAccessCollection?.accountIds],
    );
  
    const canDirectlyAddLocalAccessAccount = useCallback(
      (account: CodexAccount) =>
        localAccessState !== null &&
        canAddCodexAccountToLocalAccess(
          account,
          localAccessAccountIdSet,
          localAccessCollection?.restrictFreeAccounts ?? true,
        ),
      [
        localAccessAccountIdSet,
        localAccessCollection?.restrictFreeAccounts,
        localAccessState,
      ],
    );
  
    const handleAddLocalAccessAccount = useCallback(
      async (accountId: string) => {
        if (addingLocalAccessAccountId) return;
        setAddingLocalAccessAccountId(accountId);
        try {
          const result =
            await codexLocalAccessService.appendCodexLocalAccessAccounts([
              accountId,
            ]);
          setLocalAccessState(result.state);
          const accountAdded = Boolean(
            result.state.collection?.accountIds.includes(accountId),
          );
          if (!accountAdded) {
            throw new Error(
              t(
                "codex.localAccess.noEligibleAccountsSelected",
                "所选账号不在当前环境中，或不符合 API 服务条件。请先在当前环境导入可用 Codex 账号后再添加。",
              ),
            );
          }
          await ensureLocalAccessEntryVisible();
          window.dispatchEvent(new Event("codex-local-access-state-updated"));
          setMessage({
            text: t("codex.localAccess.saveSuccess", "API 服务集合已更新"),
          });
        } catch (error) {
          console.error("Failed to add account to API service:", error);
          setMessage({
            text: t("messages.actionFailed", {
              action: t("codex.localAccess.entryAction", "添加至 API 服务"),
              error: String(error).replace(/^Error:\s*/, ""),
            }),
            tone: "error",
          });
        } finally {
          setAddingLocalAccessAccountId(null);
        }
      },
      [addingLocalAccessAccountId, ensureLocalAccessEntryVisible, setMessage, t],
    );
  
    const handleRemoveLocalAccessAccount = useCallback(
      async (accountId: string) => {
        if (addingLocalAccessAccountId) return;
        setAddingLocalAccessAccountId(accountId);
        try {
          const nextState =
            await codexLocalAccessService.removeCodexLocalAccessAccount(
              accountId,
            );
          setLocalAccessState(nextState);
          window.dispatchEvent(new Event("codex-local-access-state-updated"));
          setMessage({
            text: t("codex.localAccess.removeSuccess", "已从 API 服务移除该账号"),
          });
        } catch (error) {
          console.error("Failed to remove account from API service:", error);
          setMessage({
            text: t("messages.actionFailed", {
              action: t("codex.localAccess.removeAction", "移除 API 服务"),
              error: String(error).replace(/^Error:\s*/, ""),
            }),
            tone: "error",
          });
        } finally {
          setAddingLocalAccessAccountId(null);
        }
      },
      [addingLocalAccessAccountId, setMessage, t],
    );
  
    const renderAddLocalAccessAccountButton = (
      account: CodexAccount,
      className: string,
      iconSize = 14,
    ) => {
      if (!canDirectlyAddLocalAccessAccount(account)) return null;
      const label = t("codex.localAccess.entryAction", "添加至 API 服务");
      return (
        <button
          type="button"
          className={className}
          onClick={() => void handleAddLocalAccessAccount(account.id)}
          disabled={addingLocalAccessAccountId !== null}
          title={label}
          aria-label={label}
        >
          {addingLocalAccessAccountId === account.id ? (
            <RefreshCw size={iconSize} className="loading-spinner" />
          ) : (
            <Link2 size={iconSize} />
          )}
        </button>
      );
    };
  
    const handleSaveLocalAccessAccounts = useCallback(
      async (
        accountIds: string[],
        options?: {
          restrictFreeAccounts?: boolean;
          backupAccountIds?: string[];
          preferredAccountIds?: string[];
          sessionAffinity?: boolean;
          sessionAffinityTtlMs?: number;
          imageGenerationAccountPolicies?: Record<string, CodexLocalAccessImageGenerationPolicy>;
        },
      ) => {
        setLocalAccessSaving(true);
        try {
          const restrictFreeAccounts = options?.restrictFreeAccounts ?? true;
          const filteredAccountIds =
            accountIds.length === 0
              ? []
              : filterCodexLocalAccessAccountIds(
                  accountIds,
                  accounts,
                  restrictFreeAccounts,
                );
          if (accountIds.length > 0 && filteredAccountIds.length === 0) {
            throw new Error(
              t(
                "codex.localAccess.noEligibleAccountsSelected",
                "所选账号不在当前环境中，或不符合 API 服务条件。请先在当前环境导入可用 Codex 账号后再添加。",
              ),
            );
          }
          const filteredAccountIdSet = new Set(filteredAccountIds);
          const backupAccountIds = (options?.backupAccountIds ?? []).filter(
            (id) => filteredAccountIdSet.has(id),
          );
          const preferredAccountIds = (options?.preferredAccountIds ?? []).filter(
            (id) => filteredAccountIdSet.has(id),
          );
          const nextState =
            await codexLocalAccessService.saveCodexLocalAccessAccounts(
              filteredAccountIds,
              restrictFreeAccounts,
              backupAccountIds,
              preferredAccountIds,
              options?.sessionAffinity,
              options?.sessionAffinityTtlMs,
              options?.imageGenerationAccountPolicies,
            );
          setLocalAccessState(nextState);
          setMessage({
            text: t("codex.localAccess.saveSuccess", "API 服务集合已更新"),
          });
          return nextState;
        } catch (error) {
          console.error("Failed to save local access accounts:", error);
          throw error;
        } finally {
          setLocalAccessSaving(false);
        }
      },
      [accounts, setMessage, t],
    );
  
    const tierCounts = useMemo(() => {
      const counts = createCodexPlanFilterCounts(overviewAccounts.length);
      overviewAccounts.forEach((a) => {
        if (!isAbnormalAccount(a)) {
          counts.VALID += 1;
        }
        const tier = resolvePlanKey(a);
        incrementCodexPlanFilterCount(counts, tier);
        if (isAbnormalAccount(a)) counts.ERROR += 1;
        if (isCodexOverviewAccountZeroQuota(a)) counts.ZERO_QUOTA += 1;
        if (isCodexOverviewAccountSubscriptionExpired(a)) counts.EXPIRED += 1;
      });
      return counts;
    }, [isAbnormalAccount, overviewAccounts, resolvePlanKey]);
  
    const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
      () =>
        buildCodexPlanFilterOptions(tierCounts, {
          includeValid: true,
          includeZeroQuota: true,
          includeExpired: true,
          pendingLabel: t("codex.pendingAuth.badge", "待授权"),
          errorLabel: t("codex.filters.authError", "授权失败"),
          zeroQuotaLabel: t("codex.filters.zeroQuota", "0% 额度"),
          expiredLabel: t("codex.filters.expired", "已过期"),
          validOption: buildValidAccountsFilterOption(t, tierCounts.VALID),
        }),
      [t, tierCounts],
    );
  
    const codexOverviewGroupFilterOptions = useMemo<MultiSelectFilterOption[]>(
      () => buildCodexOverviewGroupFilterOptions(codexGroups),
      [codexGroups],
    );
  
    const codexAccountSortOptions = useMemo<SingleSelectFilterOption[]>(
      () => buildCodexOverviewSortOptions(t),
      [t],
    );
  
    const oauthBindingCompareAccountsBySort = useMemo(
      () =>
        createCodexOverviewAccountComparator({
          sortBy,
          sortDirection,
          customSortOrder,
          currentAccountId: localAccessLaunchCurrent
            ? null
            : (currentAccount?.id ?? null),
          resolveSubscriptionTimestamp: (account) =>
            resolveSubscriptionPresentation(account).timestampMs,
        }),
      [
        currentAccount?.id,
        customSortOrder,
        localAccessLaunchCurrent,
        resolveSubscriptionPresentation,
        sortBy,
        sortDirection,
      ],
    );
  
    const oauthBindingTierCounts = useMemo(() => {
      const counts = createCodexPlanFilterCounts(oauthAccounts.length);
      oauthAccounts.forEach((account) => {
        if (!isAbnormalAccount(account)) {
          counts.VALID += 1;
        }
        const tier = resolvePlanKey(account);
        incrementCodexPlanFilterCount(counts, tier);
        if (isAbnormalAccount(account)) counts.ERROR += 1;
        if (isCodexOverviewAccountZeroQuota(account)) counts.ZERO_QUOTA += 1;
        if (isCodexOverviewAccountSubscriptionExpired(account)) {
          counts.EXPIRED += 1;
        }
      });
      return counts;
    }, [isAbnormalAccount, oauthAccounts, resolvePlanKey]);
  
    const oauthBindingTierFilterOptions = useMemo<MultiSelectFilterOption[]>(
      () =>
        buildCodexPlanFilterOptions(oauthBindingTierCounts, {
          includeValid: true,
          includeZeroQuota: true,
          includeExpired: true,
          pendingLabel: t("codex.pendingAuth.badge", "待授权"),
          errorLabel: t("codex.filters.authError", "授权失败"),
          zeroQuotaLabel: t("codex.filters.zeroQuota", "0% 额度"),
          expiredLabel: t("codex.filters.expired", "已过期"),
          validOption: buildValidAccountsFilterOption(
            t,
            oauthBindingTierCounts.VALID,
          ),
        }),
      [oauthBindingTierCounts, t],
    );
  
    const oauthBindingAvailableTags = useMemo(() => {
      const tagSet = new Set<string>();
      oauthAccounts.forEach((account) => {
        (account.tags || []).forEach((tag) => {
          const normalized = normalizeTag(tag);
          if (normalized) {
            tagSet.add(normalized);
          }
        });
      });
      return Array.from(tagSet).sort((a, b) => a.localeCompare(b));
    }, [normalizeTag, oauthAccounts]);
  
    const oauthBindingFilteredAccounts = useMemo(
      () =>
        filterAndSortCodexOverviewAccounts({
          accounts: oauthAccounts,
          groups: codexGroups,
          searchQuery,
          filterTypes,
          tagFilter,
          groupFilter,
          activeGroupId,
          resolveDisplayName: (account) =>
            resolvePresentation(account).displayName,
          compareAccounts: oauthBindingCompareAccountsBySort,
          isAbnormalAccount,
        }),
      [
        activeGroupId,
        codexGroups,
        filterTypes,
        groupFilter,
        isAbnormalAccount,
        oauthAccounts,
        oauthBindingCompareAccountsBySort,
        resolvePresentation,
        searchQuery,
        tagFilter,
      ],
    );
  
    const oauthBindingPagination = usePagination({
      items: oauthBindingFilteredAccounts,
      storageKey: buildPaginationPageSizeStorageKey("CodexOAuthBinding"),
      pageSizeOptions: OAUTH_BINDING_PAGE_SIZE_OPTIONS,
      defaultPageSize: OAUTH_BINDING_PAGE_SIZE_OPTIONS[0],
    });
  
    useEffect(() => {
      if (!oauthBindingTargetActive) return;
      oauthBindingPagination.setCurrentPage(1);
    }, [
      activeGroupId,
      filterTypes,
      groupFilter,
      oauthBindingAccountId,
      oauthBindingPagination.setCurrentPage,
      oauthBindingTargetActive,
      searchQuery,
      sortBy,
      sortDirection,
      tagFilter,
    ]);
  
    const activeGroup = useMemo(() => {
      if (!activeGroupId) return null;
      return codexGroups.find((group) => group.id === activeGroupId) ?? null;
    }, [activeGroupId, codexGroups]);
  
    const groupQuickAddGroup = useMemo(() => {
      if (!groupQuickAddGroupId) return null;
      return (
        codexGroups.find((group) => group.id === groupQuickAddGroupId) ?? null
      );
    }, [codexGroups, groupQuickAddGroupId]);
  
    useEffect(() => {
      if (
        activeGroupId &&
        !codexGroups.some((group) => group.id === activeGroupId)
      ) {
        setActiveGroupId(null);
      }
    }, [activeGroupId, codexGroups]);
  
    useEffect(() => {
      if (
        groupQuickAddGroupId &&
        !codexGroups.some((group) => group.id === groupQuickAddGroupId)
      ) {
        setGroupQuickAddGroupId(null);
      }
    }, [codexGroups, groupQuickAddGroupId]);
  
    useEffect(() => {
      if (
        codexAddTargetGroupId &&
        !codexGroups.some((group) => group.id === codexAddTargetGroupId)
      ) {
        setCodexAddTargetGroupId(null);
      }
    }, [codexAddTargetGroupId, codexGroups]);
  
    useEffect(() => {
      if (
        batchImportTargetGroupId &&
        !codexGroups.some((group) => group.id === batchImportTargetGroupId)
      ) {
        setBatchImportTargetGroupId(null);
      }
    }, [batchImportTargetGroupId, codexGroups]);
  
    const handleEnterGroup = useCallback(
      (groupId: string) => {
        clearGroupFilter();
        setSelected(new Set());
        setActiveGroupId(groupId);
      },
      [clearGroupFilter, setSelected],
    );
  
    const handleLeaveGroup = useCallback(() => {
      setSelected(new Set());
      setActiveGroupId(null);
    }, [setSelected]);
  
    const handleRemoveFromGroup = useCallback(async () => {
      if (!activeGroupId || selected.size === 0) return;
      try {
        await removeAccountsFromCodexGroup(activeGroupId, Array.from(selected));
        setSelected(new Set());
        await reloadCodexGroups();
      } catch (error) {
        console.error(
          "Failed to remove selected codex accounts from group:",
          error,
        );
        setMessage({
          text: t("messages.actionFailed", {
            action: t("accounts.groups.removeFromGroup"),
            error: String(error),
          }),
          tone: "error",
        });
      }
    }, [activeGroupId, reloadCodexGroups, selected, setMessage, setSelected, t]);
  
    const handleRemoveSingleFromGroup = useCallback(
      async (groupId: string, accountId: string) => {
        setRemovingGroupAccountIds((prev) => {
          const next = new Set(prev);
          next.add(accountId);
          return next;
        });
  
        try {
          await removeAccountsFromCodexGroup(groupId, [accountId]);
          if (selected.has(accountId)) {
            const nextSelected = new Set(selected);
            nextSelected.delete(accountId);
            setSelected(nextSelected);
          }
          await reloadCodexGroups();
        } catch (error) {
          console.error("Failed to remove codex account from group:", error);
          setMessage({
            text: t("messages.actionFailed", {
              action: t("accounts.groups.removeFromGroup"),
              error: String(error),
            }),
            tone: "error",
          });
        } finally {
          setRemovingGroupAccountIds((prev) => {
            const next = new Set(prev);
            next.delete(accountId);
            return next;
          });
        }
      },
      [reloadCodexGroups, selected, setMessage, setSelected, t],
    );
  
    const requestDeleteGroup = useCallback(
      (groupId: string, groupName: string) => {
        setGroupDeleteError(null);
        setGroupDeleteConfirm({
          id: groupId,
          name: groupName,
        });
      },
      [setGroupDeleteError],
    );
  
    const handleQuickAddAccountsToGroup = useCallback(
      async (groupId: string, accountIds: string[]) => {
        if (accountIds.length === 0) return;
        await assignAccountsToCodexGroup(groupId, accountIds);
        await reloadCodexGroups();
      },
      [reloadCodexGroups],
    );
  
    const confirmDeleteGroup = useCallback(async () => {
      if (!groupDeleteConfirm || deletingGroup) return;
  
      setDeletingGroup(true);
      setGroupDeleteError(null);
      try {
        await deleteCodexGroup(groupDeleteConfirm.id);
        await reloadCodexGroups();
        setGroupDeleteConfirm(null);
        setGroupDeleteError(null);
      } catch (error) {
        console.error("Failed to delete codex group:", error);
        setGroupDeleteError(
          t("accounts.groups.error.deleteFailed", {
            error: String(error),
          }),
        );
      } finally {
        setDeletingGroup(false);
      }
    }, [
      deletingGroup,
      groupDeleteConfirm,
      reloadCodexGroups,
      setGroupDeleteError,
      t,
    ]);
  
    const handleRotateLocalAccessApiKey = useCallback(async () => {
      setLocalAccessSaving(true);
      try {
        const nextState =
          await codexLocalAccessService.rotateCodexLocalAccessApiKey();
        setLocalAccessState(nextState);
        setMessage({
          text: t("codex.localAccess.rotateSuccess", "API 服务密钥已重置"),
        });
        return nextState;
      } catch (error) {
        console.error("Failed to rotate local access api key:", error);
        throw new Error(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setLocalAccessSaving(false);
      }
    }, [setMessage, t]);
  
    const handleClearLocalAccessStats = useCallback(async () => {
      setLocalAccessSaving(true);
      try {
        const nextState =
          await codexLocalAccessService.clearCodexLocalAccessStats();
        setLocalAccessState(nextState);
        setMessage({
          text: t("codex.localAccess.clearStatsSuccess", "API 服务统计已清空"),
        });
        return nextState;
      } catch (error) {
        console.error("Failed to clear local access stats:", error);
        throw new Error(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setLocalAccessSaving(false);
      }
    }, [setMessage, t]);
  
    const handleKillLocalAccessPort = useCallback(async () => {
      if (!localAccessCollection) return;
      const confirmed = await confirmDialog(
        t("codex.localAccess.killPortConfirmMessage", {
          port: localAccessCollection.port,
          defaultValue:
            "将强制结束占用本机 {{port}} 端口的其他进程，然后重新启动 API 服务。确认继续吗？",
        }),
        {
          title: t("codex.localAccess.killPortTitle", "清理 API 服务端口"),
          kind: "warning",
          okLabel: t("codex.localAccess.killPortAction", "清理端口"),
          cancelLabel: t("common.cancel", "取消"),
        },
      );
      if (!confirmed) return;
  
      setLocalAccessPortKilling(true);
      try {
        const result = await codexLocalAccessService.killCodexLocalAccessPort();
        setLocalAccessState(result.state);
        setMessage({
          text:
            result.killedCount > 0
              ? t("codex.localAccess.killPortSuccess", {
                  count: result.killedCount,
                  defaultValue: "端口已清理（结束 {{count}} 个进程）",
                })
              : t(
                  "codex.localAccess.killPortSuccessNone",
                  "端口已检查，未发现外部占用进程",
                ),
        });
        return result.state;
      } catch (error) {
        console.error("Failed to kill local access port:", error);
        throw new Error(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setLocalAccessPortKilling(false);
      }
    }, [localAccessCollection, setMessage, t]);
  
    const handleRestartLocalAccessSidecar = useCallback(async () => {
      setLocalAccessSidecarRestarting(true);
      try {
        const nextState =
          await codexLocalAccessService.restartCodexLocalAccessSidecar();
        setLocalAccessState(nextState);
        setMessage({
          text: t("codex.localAccess.restartSuccess", "API 服务 Sidecar 已重启"),
        });
        return nextState;
      } catch (error) {
        console.error("Failed to restart local access sidecar:", error);
        throw new Error(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setLocalAccessSidecarRestarting(false);
      }
    }, [setMessage, t]);
  
    const handleUpdateLocalAccessPort = useCallback(
      async (port: number) => {
        setLocalAccessSaving(true);
        try {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessPort(port);
          setLocalAccessState(nextState);
          setMessage({
            text: t("codex.localAccess.portSaveSuccess", "API 服务端口已更新"),
          });
          return nextState;
        } catch (error) {
          console.error("Failed to update local access port:", error);
          throw new Error(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setLocalAccessSaving(false);
        }
      },
      [setMessage, t],
    );
  
    const handleUpdateLocalAccessRoutingStrategy = useCallback(
      async (strategy: CodexLocalAccessRoutingStrategy) => {
        setLocalAccessSaving(true);
        try {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessRoutingStrategy(
              strategy,
            );
          setLocalAccessState(nextState);
          setMessage({
            text: t(
              "codex.localAccess.routingSaveSuccess",
              "API 服务调度策略已更新",
            ),
          });
          return nextState;
        } catch (error) {
          console.error("Failed to update local access routing strategy:", error);
          throw new Error(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setLocalAccessSaving(false);
        }
      },
      [setMessage, t],
    );
  
    const handleUpdateLocalAccessCustomRouting = useCallback(
      async (rules: CodexLocalAccessCustomRoutingRule[]) => {
        setLocalAccessSaving(true);
        try {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessCustomRouting(
              rules,
            );
          setLocalAccessState(nextState);
          setMessage({
            text: t(
              "codex.localAccess.customRoutingSaveSuccess",
              "API 服务自定义调度已更新",
            ),
          });
          return nextState;
        } catch (error) {
          console.error("Failed to update local access custom routing:", error);
          throw new Error(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setLocalAccessSaving(false);
        }
      },
      [setMessage, t],
    );
  
    const handleUpdateLocalAccessUpstreamProxyConfig = useCallback(
      async (upstreamProxyUrl: string | null) => {
        setLocalAccessSaving(true);
        try {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessUpstreamProxyConfig(
              upstreamProxyUrl,
            );
          setLocalAccessState(nextState);
          setMessage({
            text: t(
              "codex.localAccess.upstreamProxySaveSuccess",
              "API 代理地址已更新",
            ),
          });
          return nextState;
        } catch (error) {
          console.error(
            "Failed to update local access upstream proxy config:",
            error,
          );
          throw new Error(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setLocalAccessSaving(false);
        }
      },
      [setMessage, t],
    );
  
    const handleUpdateLocalAccessAccessScope = useCallback(
      async (accessScope: CodexLocalAccessScope) => {
        setLocalAccessSaving(true);
        try {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessAccessScope(
              accessScope,
            );
          setLocalAccessState(nextState);
          setMessage({
            text: t(
              "codex.localAccess.accessScopeSaveSuccess",
              "API 服务访问范围已更新",
            ),
          });
          return nextState;
        } catch (error) {
          console.error("Failed to update local access scope:", error);
          throw new Error(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setLocalAccessSaving(false);
        }
      },
      [setMessage, t],
    );
  
    const handleToggleLocalAccessEnabled = useCallback(async () => {
      if (!localAccessCollection) return;
      if (!localAccessCollection.enabled) {
        const confirmed = await requestLocalAccessRiskNotice("service");
        if (!confirmed) return;
      }
      setLocalAccessSaving(true);
      try {
        const nextState =
          await codexLocalAccessService.setCodexLocalAccessEnabled(
            !localAccessCollection.enabled,
          );
        setLocalAccessState(nextState);
        setMessage({
          text: nextState.collection?.enabled
            ? t("codex.localAccess.enabledSuccess", "API 服务已启用")
            : t("codex.localAccess.disabledSuccess", "API 服务已停用"),
        });
        return nextState;
      } catch (error) {
        console.error("Failed to toggle local access service:", error);
        throw new Error(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setLocalAccessSaving(false);
      }
    }, [localAccessCollection, requestLocalAccessRiskNotice, setMessage, t]);
  
    const handleActivateLocalAccess = useCallback(
      async (options?: {
        showSuccessMessage?: boolean;
        instanceId?: string | null;
      }) => {
        if (!localAccessCollection) {
          throw new Error(
            t("codex.localAccess.testUnavailable", "当前 API 服务地址不可用"),
          );
        }
        if (!localAccessCollection.enabled) {
          const confirmedEnableAndSwitch = await confirmDialog(
            t(
              "codex.localAccess.enableBeforeActivateMessage",
              "API 服务当前未启用，需要先启用服务。是否启用并切号？",
            ),
            {
              title: t(
                "codex.localAccess.enableBeforeActivateTitle",
                "服务未启用",
              ),
              kind: "warning",
              okLabel: t(
                "codex.localAccess.enableAndActivateAction",
                "启用并切号",
              ),
              cancelLabel: t("common.cancel", "取消"),
            },
          );
          if (!confirmedEnableAndSwitch) return;
        }
        const confirmed = await requestLocalAccessRiskNotice("service");
        if (!confirmed) return;
        const flowStartedAt = performance.now();
        console.info("[Codex API Service Switch][UI] button loading started");
        setLocalAccessStarting(true);
        try {
          const nextState =
            await codexLocalAccessService.activateCodexLocalAccess(
              options?.instanceId,
            );
          setLocalAccessState(nextState);
          await fetchCurrentAccount();
          setLocalAccessLaunchCurrent(true);
          if (options?.showSuccessMessage ?? true) {
            setMessage({
              text: t("codex.localAccess.activateSuccess", "已切换到 API 服务"),
            });
          }
          return nextState;
        } catch (error) {
          throw new Error(String(error).replace(/^Error:\s*/, ""));
        } finally {
          setLocalAccessStarting(false);
          console.info("[Codex API Service Switch][UI] button loading finished", {
            elapsedMs: Math.round(performance.now() - flowStartedAt),
          });
        }
      },
      [
        fetchCurrentAccount,
        localAccessCollection,
        requestLocalAccessRiskNotice,
        setMessage,
        t,
      ],
    );
  
    const handleQuickToggleLocalAccessEnabled = useCallback(async () => {
      try {
        await handleToggleLocalAccessEnabled();
      } catch (error) {
        if (
          presentWindowsOperationError({
            error,
            operation: "start_sidecar",
            summary: t("codex.localAccess.toggleService", "切换 API 服务"),
            retry: async () => {
              await handleToggleLocalAccessEnabled();
            },
          })
        ) {
          return;
        }
        setMessage({
          text: t("messages.actionFailed", {
            action: t("codex.localAccess.toggleService", "切换 API 服务"),
            error: String(error).replace(/^Error:\s*/, ""),
          }),
          tone: "error",
        });
      }
    }, [handleToggleLocalAccessEnabled, setMessage, t]);
  
    const handleExecuteLocalAccessLaunchPreview =
      useCallback(async (): Promise<boolean> => {
        const activateSelectedTarget = async () => {
          if (launchPreviewInstanceId !== DEFAULT_CODEX_INSTANCE_ID) {
            await codexInstanceStore.updateInstance({
              instanceId: launchPreviewInstanceId,
              bindAccountId: CODEX_API_SERVICE_BIND_ID,
              deferBindAccountApplication: true,
            });
          }
          return await handleActivateLocalAccess({
            instanceId: launchPreviewInstanceId,
          });
        };
        try {
          const state = await activateSelectedTarget();
          if (!state) {
            return false;
          }
          setLocalAccessLaunchPreviewOpen(false);
          return true;
        } catch (error) {
          if (
            presentWindowsOperationError({
              error,
              operation: "start_sidecar",
              summary: t("codex.localAccess.activateAction", "启动 API 服务"),
              retry: async () => {
                await activateSelectedTarget();
              },
            })
          ) {
            setLocalAccessLaunchPreviewOpen(false);
            return true;
          }
          throw error;
        }
      }, [
        codexInstanceStore,
        handleActivateLocalAccess,
        launchPreviewInstanceId,
        t,
      ]);
  
    const handleQuickRefreshLocalAccessQuota = useCallback(async () => {
      if (!localAccessCollection) return;
      // 与分组/全量刷新一致：OAuth + New API 可刷，普通 API Key 跳过
      const targetIds = localAccessCollection.accountIds.filter((accountId) => {
        const account = accounts.find((item) => item.id === accountId);
        return Boolean(
          account &&
          (!isCodexApiKeyAccount(account) || isCodexNewApiAccount(account)),
        );
      });
  
      if (targetIds.length === 0) {
        setMessage({
          text: t("codex.refreshFailed", {
            error: t("common.shared.quota.noData", "暂无配额数据"),
          }),
          tone: "error",
        });
        return;
      }
  
      setLocalAccessRefreshing(true);
      try {
        // 后端限流并发（MAX=5），避免 N 路全开 + 每号 fetchAccounts thrash
        const successCount =
          await codexService.refreshCodexQuotasBatch(targetIds);
  
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
        setLocalAccessRefreshing(false);
      }
    }, [
      accounts,
      fetchAccounts,
      fetchCurrentAccount,
      localAccessCollection,
      setMessage,
      t,
    ]);
  return {
    accountIdLabel,
    activeGroup,
    canDirectlyAddLocalAccessAccount,
    codexAccountSortOptions,
    codexOverviewGroupFilterOptions,
    confirmDeleteGroup,
    confirmHideLocalAccessEntry,
    groupQuickAddGroup,
    handleAddLocalAccessAccount,
    handleClearLocalAccessStats,
    handleCopyLocalAccessValue,
    handleEnterGroup,
    handleExecuteLocalAccessLaunchPreview,
    handleHideLocalAccessEntry,
    handleKillLocalAccessPort,
    handleLeaveGroup,
    handleLocalAccessAddressKindChange,
    handleQuickAddAccountsToGroup,
    handleQuickRefreshLocalAccessQuota,
    handleQuickToggleLocalAccessEnabled,
    handleRecoverLocalAccessAccounts,
    handleRemoveFromGroup,
    handleRemoveLocalAccessAccount,
    handleRemoveSingleFromGroup,
    handleRestartLocalAccessSidecar,
    handleRotateLocalAccessApiKey,
    handleSaveLocalAccessAccounts,
    handleToggleLocalAccessEnabled,
    handleUpdateLocalAccessAccessScope,
    handleUpdateLocalAccessCustomRouting,
    handleUpdateLocalAccessPort,
    handleUpdateLocalAccessRoutingStrategy,
    handleUpdateLocalAccessUpstreamProxyConfig,
    isAbnormalAccount,
    localAccessAccountIdSet,
    localAccessAccountPoolHealthHasIssue,
    localAccessAccountPoolHealthSummary,
    localAccessAddressOptions,
    localAccessBusy,
    localAccessModalSelectedIds,
    localAccessQuotaHiddenCount,
    localAccessQuotaPoolLabels,
    localAccessQuotaPoolSummary,
    localAccessQuotaPreviewItems,
    localAccessScopeLabel,
    oauthBindingAvailableTags,
    oauthBindingFilteredAccounts,
    oauthBindingPagination,
    oauthBindingTierCounts,
    oauthBindingTierFilterOptions,
    openCodexApiServicePage,
    openLocalAccessMemberPicker,
    openLocalAccessPanel,
    openQuotaErrorDetail,
    overviewAccounts,
    renderAddLocalAccessAccountButton,
    renderQuotaErrorDetailModal,
    renderQuotaErrorInline,
    requestDeleteGroup,
    resolveAccountMeta,
    resolveLocalAccessBaseUrl,
    resolvePresentation,
    resolveQuotaErrorMeta,
    resolveSingleExportBaseName,
    resolveSubscriptionPresentation,
    selectedLocalAccessAddressKind,
    shouldOfferReauthorizeAction,
    tierCounts,
    tierFilterOptions,
  };
}
