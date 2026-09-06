import { useState, useEffect, useMemo, useCallback, type ReactElement } from "react";
import { RefreshCw, Database, CircleAlert, Eye, EyeOff, Link2 } from "lucide-react";
import * as codexService from "../services/codexService";
import * as codexInstanceService from "../services/codexInstanceService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { presentWindowsOperationError } from "../utils/windowsOperationDialog";
import { isCodexApiKeyAccount, isCodexAgentIdentityAccount, isCodexWebSessionAccount, isCodexChatCompletionsApiKeyAccount, isCodexNewApiAccount } from "../types/codex";
import { isCodexOAuthBindingEligibleAccount, resolveImportedCodexAccountIdsForLocalAccess } from "../utils/codexLocalAccessAccounts";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { recoverCodexBatchImportStartFromPreview } from "../utils/codexBatchImportQueue";
import { CodexSwitchAccountError } from "../utils/codexSwitchAuthFailure";
import { requestCodexOpenAddAccount } from "../utils/codexAddAccountRequest";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { DEFAULT_CODEX_INSTANCE_ID } from "../components/codex/CodexLaunchPreviewModal";
import { isDeepSeekAccount, isCodexTokenPlanAccount, resolveDeepSeekBindAccountId } from "../utils/codexDeepSeekAccess";
import { contextWindowDraftsFromRecord, parseContextWindowDrafts } from "../utils/codexModelContextWindows";
import type { CodexAccount } from "../types/codex";
import { CODEX_API_SERVICE_BIND_ID, type InstanceProfile } from "../types/instance";
import { findCodexWebSessionImports, splitCodexImportPayloads } from "../utils/codexJsonImportProgress";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import { resolveCodexModelProviderAccountName } from "../utils/codexModelProviderAccountName";
import { CODEX_API_PROVIDER_CUSTOM_ID, COCKPIT_API_PROVIDER_ID, findCodexApiProviderPresetById, resolveCodexApiProviderPresetId } from "../utils/codexProviderPresets";
import { APIKEY_FUN_PROVIDER_BASE_URL } from "../utils/apikeyFunLinks";
import { APIKEY_FUN_PREFILL_EVENT, consumeApiKeyFunPrefill, type ApiKeyFunPrefillPayload } from "../utils/apiKeyFunPrefill";
import { findCodexModelProviderById, findCodexModelProviderByBaseUrl, queryCodexModelProviderUsage, saveCodexModelProviderDetectedIntegrationType, type CodexModelProvider, type CodexModelProviderUsageSummary, upsertCodexModelProviderFromCredential } from "../services/codexModelProviderService";
import { CODEX_API_KEY_USAGE_REFRESHED_EVENT, readCodexApiKeyUsageCache, writeCodexApiKeyUsageCache, type CodexApiKeyUsageState } from "../services/codexApiKeyUsageRefreshService";
import { isModelProviderUsageUnavailableError, formatCockpitToolsApiKeyBalance, formatModelProviderUsageMoney, listModelProviderModels, resolveNewApiQuotaSnapshot, syncModelProviderUsageToCodexAccount } from "../services/modelProviderUsageService";
import { upsertSavedMfaRecord } from "../utils/mfaVault";
import md5 from "blueimp-md5";
import { CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY, DEFAULT_CODEX_API_BASE_URL, DEFAULT_CODEX_API_PROVIDER_ID, formatCockpitApiInteger, formatCockpitApiTokenCount, getCockpitApiStatsRecord, getCockpitApiUsageRecord, inferCodexAccountProviderMode, isRelayApiProviderTemplateId, isSameHttpBaseUrl, joinFilePath, maskCodexApiKey, normalizeHttpBaseUrl, normalizePathForCompare, OPENAI_OFFICIAL_PRESET_ID, parseApiModelCatalogText, parseOAuthQuotaReservePercent, persistLastCodexCliWorkingDir, readCockpitApiOptionalNumber, readCockpitApiString, readLastCodexCliWorkingDir, resolveApiKeyUsageMode, sanitizeCodexCliInstanceName, toCockpitApiRecord, type CockpitApiJsonRecord, type CodexCliInstanceDraft, type CodexCliLaunchModalState, type OAuthBindingQuotaReserveFieldErrors } from "./codexAccountsControllerModel";
import type { CodexAccountsAccessControllerContext } from "./codexAccountsAccessControllerContract";

/** 封装 useCodexAccountsPageController 的 useCodexAccountsAccessController 业务域状态与动作。 */
export function useCodexAccountsAccessController(context: CodexAccountsAccessControllerContext) {
  const {
    accounts,
    addTab,
    apiBaseUrlInput,
    apiKeyFunPrefillModelCatalogRef,
    apiKeyInput,
    apiKeyUsageInFlightRef,
    apiKeyUsageMap,
    apiModelCatalogDraft,
    apiModelContextWindowsInput,
    apiProviderPresetExplicitlySelectedRef,
    apiProviderPresetId,
    apiSyncModelCatalogToCodex,
    assignCodexAccountsToTargetGroup,
    batchImportBusy,
    batchImportCheckQuota,
    batchImportFilePaths,
    batchImportPreview,
    batchImportResult,
    batchImportSelectableIdSet,
    batchImportSelectedIds,
    batchImportSessionId,
    batchImportSessionIdRef,
    batchImportTagsInput,
    batchImportTargetGroupId,
    batchImportUnlistenersRef,
    boundLocalAccessOAuthAccount,
    buildApiProviderPayload,
    cleanupBatchImportListeners,
    cliLaunchingAccountId,
    cliLaunchModal,
    closeAddModal,
    codexAccountsRef,
    codexAddTargetGroupId,
    codexCliInstanceDefaultsRef,
    codexInstanceStore,
    deepSeekStart,
    deepSeekUsageRetryIdsRef,
    defaultApiProviderPresetId,
    editingApiBaseUrlCredentialsValue,
    editingApiKeyCredentialsId,
    editingApiKeyCredentialsValue,
    editingApiKeyNameId,
    editingApiKeyNameValue,
    editingApiModelCatalogDraft,
    editingApiModelContextWindowsInput,
    editingApiProviderPresetId,
    editingApiSyncModelCatalogToCodex,
    editingManagedProviderId,
    editingNewManagedProviderNameInput,
    ensureLocalAccessEntryVisible,
    fetchAccounts,
    fetchCurrentAccount,
    formatDate,
    inlineRenameDiscardRef,
    localAccessCollection,
    managedProviderId,
    managedProviders,
    maskAccountText,
    newManagedProviderNameInput,
    oauthAccounts,
    oauthBindingAccount,
    oauthBindingAutoSwitch,
    oauthBindingHourlyReserveDraft,
    oauthBindingHourlyReserveInputRef,
    oauthBindingQuotaReserve,
    oauthBindingSaving,
    oauthBindingTargetKind,
    oauthBindingWeeklyReserveDraft,
    oauthBindingWeeklyReserveInputRef,
    openCodexAddModal,
    page,
    pendingApiKeyFunCodexPrefillRef,
    quickSwitchAccount,
    quickSwitchSubmitting,
    reloadManagedProviders,
    resetBatchImportState,
    resolveManagedProviderIdForAccount,
    resolveValidCodexGroupId,
    savingApiKeyCredentials,
    selectedEditingManagedProvider,
    selectedEditingManagedProviderApiKey,
    selectedManagedProvider,
    selectedManagedProviderApiKey,
    selectedOAuthBindingAccount,
    selectedQuickSwitchApiKey,
    selectedQuickSwitchProvider,
    selectedTerminal,
    setAddMessage,
    setAddStatus,
    setApiBaseUrlInput,
    setApiKeyInput,
    setApiKeyInputVisible,
    setApiKeyUsageMap,
    setApiModelCatalogError,
    setApiModelCatalogFetching,
    setApiModelCatalogInput,
    setApiModelContextWindowsInput,
    setApiProviderPresetId,
    setApiSyncModelCatalogToCodex,
    setBatchImportBusy,
    setBatchImportCheckQuota,
    setBatchImportError,
    setBatchImportFilePaths,
    setBatchImportFilter,
    setBatchImportOpen,
    setBatchImportPreview,
    setBatchImportProgress,
    setBatchImportResult,
    setBatchImportSelectedIds,
    setBatchImportSessionId,
    setBatchImportTargetGroupId,
    setCliLaunchingAccountId,
    setCliLaunchModal,
    setEditingApiBaseUrlCredentialsValue,
    setEditingApiKeyCredentialsId,
    setEditingApiKeyCredentialsValue,
    setEditingApiKeyCredentialsVisible,
    setEditingApiKeyNameId,
    setEditingApiKeyNameValue,
    setEditingApiModelCatalogError,
    setEditingApiModelCatalogFetching,
    setEditingApiModelCatalogInput,
    setEditingApiModelContextWindowsInput,
    setEditingApiProviderPresetId,
    setEditingApiSyncModelCatalogToCodex,
    setEditingManagedProviderApiKeyId,
    setEditingManagedProviderId,
    setEditingNewManagedProviderNameInput,
    setImportApiServiceGuideCount,
    setImporting,
    setLocalAccessLaunchCurrent,
    setLocalAccessSaving,
    setLocalAccessState,
    setManagedProviderApiKeyId,
    setManagedProviderId,
    setMessage,
    setReauthRetryOAuthBinding,
    setNewManagedProviderNameInput,
    setOauthBindingAccountId,
    setOauthBindingAutoSwitch,
    setOauthBindingError,
    setOauthBindingHourlyReserveDraft,
    setOauthBindingQuotaReserve,
    setOauthBindingQuotaReserveEditorOpen,
    setOauthBindingQuotaReserveFieldErrors,
    setOauthBindingSaving,
    setOauthBindingSelectedAccountId,
    setOauthBindingTargetKind,
    setOauthBindingWeeklyReserveDraft,
    setPendingWebSessionImport,
    setQuickSwitchAccountId,
    setQuickSwitchApiKeyId,
    setQuickSwitchError,
    setQuickSwitchProviderId,
    setQuickSwitchSubmitting,
    setSavedMfaRecords,
    setSavingApiKeyCredentials,
    setSavingApiKeyNameId,
    setSwitching,
    setTokenImportProgress,
    setVisibleApiKeyAccountIds,
    showAddModal,
    skipManagedProviderApiKeyAutofillRef,
    sponsorApiProviderTemplates,
    store,
    switchAccount,
    syncImportedAccountsToApiService,
    syncImportedToApiService,
    t,
    tokenInput,
    updateAccountInstanceAccess,
    updateAccountName,
    updateApiKeyBoundOAuthAccount,
    updateApiKeyCredentials,
    validateApiKeyCredentialInputs,
    visibleApiKeyAccountIds,
  } = context;

  useEffect(() => {
    const clearCancelledSwitchingState = (event: Event) => {
      const detail = (
        event as CustomEvent<{
          type?: string;
          accountId?: string;
          cancelled?: boolean;
        }>
      ).detail;
      if (
        !detail ||
        (detail.type !== "cancelled" && detail.cancelled !== true) ||
        !detail.accountId
      ) {
        return;
      }
      // 后端切换事务仍会在安全检查点继续收尾，但用户已经取消后，账号卡片不应
      // 等待原 Promise 最终返回才停止旋转。只清理当前对应账号，避免影响其它操作。
      setSwitching((currentAccountId) =>
        currentAccountId === detail.accountId ? null : currentAccountId,
      );
    };

    window.addEventListener(
      "codex-switch-progress",
      clearCancelledSwitchingState,
    );
    return () => {
      window.removeEventListener(
        "codex-switch-progress",
        clearCancelledSwitchingState,
      );
    };
  }, [setSwitching]);

  const resolveBoundOAuthAccount = useCallback(
      (account: CodexAccount) => {
        const boundId = (account.bound_oauth_account_id || "").trim();
        if (!boundId) return null;
        return oauthAccounts.find((item) => item.id === boundId) ?? null;
      },
      [oauthAccounts],
    );
  
    const resetOAuthBindingModal = useCallback(() => {
      setOauthBindingTargetKind(null);
      setOauthBindingAccountId(null);
      setOauthBindingSelectedAccountId("");
      setOauthBindingAutoSwitch(false);
      setOauthBindingQuotaReserve(null);
      setOauthBindingQuotaReserveEditorOpen(false);
      setOauthBindingHourlyReserveDraft("");
      setOauthBindingWeeklyReserveDraft("");
      setOauthBindingQuotaReserveFieldErrors({});
      setOauthBindingError(null);
    }, [setOauthBindingError]);
  
    const closeOAuthBindingModal = useCallback(() => {
      if (oauthBindingSaving) return;
      resetOAuthBindingModal();
    }, [oauthBindingSaving, resetOAuthBindingModal]);
  
    const openOAuthBindingModal = useCallback(
      (account: CodexAccount, options?: { autoSwitch?: boolean }) => {
        if (!isCodexApiKeyAccount(account)) return;
        const boundAccount = resolveBoundOAuthAccount(account);
        setOauthBindingTargetKind("api_key_account");
        setOauthBindingAccountId(account.id);
        setOauthBindingSelectedAccountId(
          boundAccount && isCodexOAuthBindingEligibleAccount(boundAccount)
            ? boundAccount.id
            : "",
        );
        setOauthBindingAutoSwitch(options?.autoSwitch ?? false);
        setOauthBindingQuotaReserve(null);
        setOauthBindingQuotaReserveEditorOpen(false);
        setOauthBindingHourlyReserveDraft("");
        setOauthBindingWeeklyReserveDraft("");
        setOauthBindingQuotaReserveFieldErrors({});
        setOauthBindingError(null);
      },
      [resolveBoundOAuthAccount, setOauthBindingError],
    );
  
    const openLocalAccessOAuthBindingModal = useCallback(
      (options?: { autoSwitch?: boolean }) => {
        const persistedQuotaReserve =
          localAccessCollection?.boundOauthQuotaReserve ?? null;
        const hourlyPercent = persistedQuotaReserve
          ? parseOAuthQuotaReservePercent(
              String(persistedQuotaReserve.hourlyPercent),
            )
          : null;
        const weeklyPercent = persistedQuotaReserve
          ? parseOAuthQuotaReservePercent(
              String(persistedQuotaReserve.weeklyPercent),
            )
          : null;
        const quotaReserve =
          hourlyPercent !== null && weeklyPercent !== null
            ? { hourlyPercent, weeklyPercent }
            : null;
        setOauthBindingTargetKind("local_access");
        setOauthBindingAccountId(null);
        setOauthBindingSelectedAccountId(
          boundLocalAccessOAuthAccount &&
            isCodexOAuthBindingEligibleAccount(boundLocalAccessOAuthAccount)
            ? boundLocalAccessOAuthAccount.id
            : "",
        );
        setOauthBindingAutoSwitch(options?.autoSwitch ?? false);
        setOauthBindingQuotaReserve(quotaReserve);
        setOauthBindingQuotaReserveEditorOpen(false);
        setOauthBindingHourlyReserveDraft("");
        setOauthBindingWeeklyReserveDraft("");
        setOauthBindingQuotaReserveFieldErrors({});
        setOauthBindingError(null);
      },
      [
        boundLocalAccessOAuthAccount,
        localAccessCollection?.boundOauthQuotaReserve,
        setOauthBindingError,
      ],
    );
  
    const openOAuthBindingQuotaReserveEditor = useCallback(() => {
      setOauthBindingHourlyReserveDraft(
        oauthBindingQuotaReserve
          ? String(oauthBindingQuotaReserve.hourlyPercent)
          : "",
      );
      setOauthBindingWeeklyReserveDraft(
        oauthBindingQuotaReserve
          ? String(oauthBindingQuotaReserve.weeklyPercent)
          : "",
      );
      setOauthBindingQuotaReserveFieldErrors({});
      setOauthBindingQuotaReserveEditorOpen(true);
      window.requestAnimationFrame(() => {
        oauthBindingHourlyReserveInputRef.current?.focus();
      });
    }, [oauthBindingQuotaReserve]);
  
    const closeOAuthBindingQuotaReserveEditor = useCallback(() => {
      setOauthBindingQuotaReserveEditorOpen(false);
      setOauthBindingQuotaReserveFieldErrors({});
    }, []);
  
    const handleOAuthBindingQuotaReserveToggle = useCallback(
      (checked: boolean) => {
        setOauthBindingError(null);
        if (!checked) {
          setOauthBindingQuotaReserve(null);
          setOauthBindingQuotaReserveEditorOpen(false);
          setOauthBindingQuotaReserveFieldErrors({});
          return;
        }
        openOAuthBindingQuotaReserveEditor();
      },
      [openOAuthBindingQuotaReserveEditor, setOauthBindingError],
    );
  
    const validateOAuthBindingQuotaReserveField = useCallback(
      (field: keyof OAuthBindingQuotaReserveFieldErrors, rawValue: string) => {
        const valid = parseOAuthQuotaReservePercent(rawValue) !== null;
        setOauthBindingQuotaReserveFieldErrors((prev) => ({
          ...prev,
          [field]: valid
            ? undefined
            : t(
                "codex.localAccess.oauthBinding.quotaReserveInvalid",
                "请输入 1 到 100 的整数",
              ),
        }));
      },
      [t],
    );
  
    const confirmOAuthBindingQuotaReserveEditor = useCallback(() => {
      const hourlyPercent = parseOAuthQuotaReservePercent(
        oauthBindingHourlyReserveDraft,
      );
      const weeklyPercent = parseOAuthQuotaReservePercent(
        oauthBindingWeeklyReserveDraft,
      );
      const invalidMessage = t(
        "codex.localAccess.oauthBinding.quotaReserveInvalid",
        "请输入 1 到 100 的整数",
      );
      const fieldErrors: OAuthBindingQuotaReserveFieldErrors = {};
      if (hourlyPercent === null) {
        fieldErrors.hourlyPercent = invalidMessage;
      }
      if (weeklyPercent === null) {
        fieldErrors.weeklyPercent = invalidMessage;
      }
      if (hourlyPercent === null || weeklyPercent === null) {
        setOauthBindingQuotaReserveFieldErrors(fieldErrors);
        window.requestAnimationFrame(() => {
          const target = fieldErrors.hourlyPercent
            ? oauthBindingHourlyReserveInputRef.current
            : oauthBindingWeeklyReserveInputRef.current;
          target?.scrollIntoView({ behavior: "smooth", block: "center" });
          target?.focus();
        });
        return;
      }
      setOauthBindingQuotaReserve({ hourlyPercent, weeklyPercent });
      setOauthBindingQuotaReserveEditorOpen(false);
      setOauthBindingQuotaReserveFieldErrors({});
    }, [oauthBindingHourlyReserveDraft, oauthBindingWeeklyReserveDraft, t]);
  
    const formatCodexAuthFailureMessage = useCallback(
      (rawError: unknown) => {
        const raw = String(rawError)
          .replace(/^Error:\s*/, "")
          .trim();
        const lower = raw.toLowerCase();
        if (raw === "CODEX_STALE_ACCOUNT") {
          return t(
            "codex.authError.staleAccount",
            "该账号已不在本地账号库中，账号列表已刷新。请重新导入或重新登录该 Codex 账号。",
          );
        }
        if (
          lower.includes("unsupported_country_region_territory") ||
          raw.includes("当前网络地区不支持刷新 Codex 授权")
        ) {
          return t(
            "codex.authError.unsupportedCountryRegion",
            "当前网络地区不支持刷新 Codex 授权。OpenAI 授权服务拒绝了当前网络出口的刷新请求，请切换到支持的网络地区后重试。",
          );
        }
        if (
          lower.includes("refresh_token_expired") ||
          raw.includes("Codex 登录授权已过期")
        ) {
          return t(
            "codex.authError.refreshTokenExpired",
            "Codex 登录授权已过期，无法自动刷新。请重新登录 Codex 账号。",
          );
        }
        if (
          lower.includes("refresh_token_invalidated") ||
          lower.includes("token_invalidated") ||
          raw.includes("Codex 登录授权已被服务端撤销")
        ) {
          return t(
            "codex.authError.refreshTokenInvalidated",
            "Codex 登录授权已被服务端撤销，无法自动刷新。请重新登录 Codex 账号。",
          );
        }
        if (
          lower.includes("invalid_grant") ||
          lower.includes("invalid refresh token") ||
          raw.includes("缺少 refresh_token") ||
          raw.includes("无 refresh_token")
        ) {
          return t(
            "codex.authError.invalidGrant",
            "Codex 登录授权无效，无法自动刷新。请重新登录 Codex 账号。",
          );
        }
        return raw;
      },
      [t],
    );
  
    const executeCodexAccountSwitch = useCallback(
      async (
        accountId: string,
        options?: {
          showSuccessMessage?: boolean;
          launchAfterSwitch?: boolean;
        },
      ) => {
        const flowStartedAt = performance.now();
        console.info("[Codex Switch][UI] button loading started", {
          accountId,
        });
        const showSuccessMessage = options?.showSuccessMessage ?? true;
        setMessage(null);
        setSwitching(accountId);
        try {
          const account = await switchAccount(accountId, {
            launchAfterSwitch: options?.launchAfterSwitch,
          });
          setLocalAccessLaunchCurrent(false);
          if (showSuccessMessage) {
            setMessage({
              text: t("codex.switched", {
                email: maskAccountText(account.email),
              }),
            });
          }
          return account;
        } finally {
          setSwitching(null);
          console.info("[Codex Switch][UI] button loading finished", {
            accountId,
            elapsedMs: Math.round(performance.now() - flowStartedAt),
          });
        }
      },
      [maskAccountText, setMessage, switchAccount, t],
    );
  
    const getCodexSwitchOrLaunchBlockedReason = useCallback(
      (account?: CodexAccount | null): string | null => {
        if (isCodexAgentIdentityAccount(account)) {
          return t(
            "codex.agentIdentityRegistration.apiOnlyActionError",
            "Agent Identity 账号仅支持 API 服务，无法作为普通账号切换或启动。",
          );
        }
        if (isCodexWebSessionAccount(account)) {
          return t(
            "codex.webSessionImport.actionBlocked",
            "Web Session 账号仅支持查看额度，无法切换或启动。",
          );
        }
        return null;
      },
      [t],
    );
  
    const [launchPreviewAccount, setLaunchPreviewAccount] =
      useState<CodexAccount | null>(null);
    const [launchPreviewInstanceId, setLaunchPreviewInstanceId] = useState(
      DEFAULT_CODEX_INSTANCE_ID,
    );
    const [localAccessLaunchPreviewOpen, setLocalAccessLaunchPreviewOpen] =
      useState(false);
    const activeLaunchPreviewAccount = useMemo(() => {
      if (!launchPreviewAccount) return null;
      return (
        accounts.find((account) => account.id === launchPreviewAccount.id) ??
        launchPreviewAccount
      );
    }, [accounts, launchPreviewAccount]);
  
    const launchPreviewInstanceOptions = useMemo(() => {
      const values = codexInstanceStore.instances
        .map((instance) => ({
          value: instance.id,
          label: instance.isDefault
            ? t("instances.defaultName", "默认实例")
            : instance.name || instance.id,
          isDefault: Boolean(instance.isDefault),
        }))
        .sort((left, right) => {
          if (left.isDefault !== right.isDefault) {
            return left.isDefault ? -1 : 1;
          }
          return left.label.localeCompare(right.label);
        })
        .map(({ value, label }) => ({ value, label }));
      if (!values.some((item) => item.value === DEFAULT_CODEX_INSTANCE_ID)) {
        values.unshift({
          value: DEFAULT_CODEX_INSTANCE_ID,
          label: t("instances.defaultName", "默认实例"),
        });
      }
      return values;
    }, [codexInstanceStore.instances, t]);
  
    const launchPreviewInstanceLabel = useMemo(
      () =>
        launchPreviewInstanceOptions.find(
          (item) => item.value === launchPreviewInstanceId,
        )?.label || t("instances.defaultName", "默认实例"),
      [launchPreviewInstanceId, launchPreviewInstanceOptions, t],
    );
  
    useEffect(() => {
      if (!launchPreviewAccount && !localAccessLaunchPreviewOpen) return;
      void codexInstanceStore.refreshInstances();
    }, [
      codexInstanceStore.refreshInstances,
      launchPreviewAccount,
      localAccessLaunchPreviewOpen,
    ]);
  
    const handleSwitch = async (accountId: string) => {
      const account = codexAccountsRef.current.find(
        (item) => item.id === accountId,
      );
      const blockedReason = getCodexSwitchOrLaunchBlockedReason(account);
      if (blockedReason) {
        setMessage({
          text: blockedReason,
          tone: "error",
        });
        return;
      }
      if (isCodexWebSessionAccount(account)) {
        setMessage({
          text: t(
            "codex.webSessionImport.actionBlocked",
            "Web Session 账号仅支持查看额度，无法切换或启动。",
          ),
          tone: "error",
        });
        return;
      }
      setLaunchPreviewInstanceId(DEFAULT_CODEX_INSTANCE_ID);
      setLaunchPreviewAccount(account ?? null);
    };
  
    const handleExecuteLaunchPreview = useCallback(
      async (launchAfterSwitch: boolean): Promise<boolean> => {
        const account = activeLaunchPreviewAccount;
        if (!account) return false;
        let launchAccount = account;
        if (isDeepSeekAccount(account)) {
          const presentation = buildCodexAccountPresentation(account, t);
          const prepared = await deepSeekStart.confirmStart(
            account,
            updateAccountInstanceAccess,
            launchPreviewInstanceLabel ||
              presentation.displayName ||
              account.email ||
              account.id,
          );
          if (!prepared) return false;
          launchAccount = prepared;
        }
        if (launchPreviewInstanceId !== DEFAULT_CODEX_INSTANCE_ID) {
          const bindAccountId = isDeepSeekAccount(launchAccount)
            ? resolveDeepSeekBindAccountId(launchAccount)
            : launchAccount.id;
          await codexInstanceStore.updateInstance({
            instanceId: launchPreviewInstanceId,
            bindAccountId,
            deferBindAccountApplication: true,
          });
          if (launchAfterSwitch) {
            await codexInstanceStore.startInstance(launchPreviewInstanceId);
          }
          setLaunchPreviewAccount(null);
          return true;
        }
        setLaunchPreviewAccount(null);
        try {
          await executeCodexAccountSwitch(account.id, { launchAfterSwitch });
        } catch (error) {
          if (error instanceof CodexSwitchAccountError && error.authFailure) {
            return true;
          }
          const retrySwitch = async () => {
            try {
              await executeCodexAccountSwitch(account.id, { launchAfterSwitch });
            } catch (retryError) {
              if (
                retryError instanceof CodexSwitchAccountError &&
                retryError.authFailure
              ) {
                return;
              }
              throw retryError;
            }
          };
          if (
            presentWindowsOperationError({
              error,
              operation: "unknown",
              summary: t("codex.switch", "切换账号"),
              retry: retrySwitch,
              manualContinue: retrySwitch,
            })
          ) {
            return true;
          }
          // 切号失败由进度弹框展示当前步骤、原始原因和重试操作，
          // 避免在弹框后方重复渲染页面级红色错误。
        }
        return true;
      },
      [
        deepSeekStart,
        codexInstanceStore,
        executeCodexAccountSwitch,
        activeLaunchPreviewAccount,
        launchPreviewInstanceId,
        launchPreviewInstanceLabel,
        setMessage,
        t,
        updateAccountInstanceAccess,
      ],
    );
  
    const handleSubmitOAuthBinding = useCallback(async () => {
      if (oauthBindingTargetKind === "api_key_account" && !oauthBindingAccount) {
        return;
      }
      if (!oauthBindingTargetKind) return;
      setOauthBindingError(null);
      setOauthBindingQuotaReserveFieldErrors({});
      if (!selectedOAuthBindingAccount) {
        setOauthBindingError(
          t("codex.api.oauthBinding.validationRequired", "请选择 OAuth 账号"),
        );
        return;
      }
      if (isCodexAgentIdentityAccount(selectedOAuthBindingAccount)) {
        setOauthBindingError(
          t(
            "codex.agentIdentityRegistration.oauthBindingUnsupported",
            "Agent Identity 账号仅用于 API 服务，不能作为 OAuth 绑定账号。",
          ),
        );
        return;
      }
      if (isCodexWebSessionAccount(selectedOAuthBindingAccount)) {
        setOauthBindingError(
          t(
            "codex.webSessionImport.oauthBindingUnsupported",
            "Web Session 账号仅支持查看额度，不能作为 OAuth 绑定账号。",
          ),
        );
        return;
      }
      if (!isCodexOAuthBindingEligibleAccount(selectedOAuthBindingAccount)) {
        setOauthBindingError(
          t(
            "codex.api.oauthBinding.validationSubscriptionRequired",
            "只能绑定带 refresh_token 的 OAuth 账号",
          ),
        );
        return;
      }
  
      const quotaReserve =
        oauthBindingTargetKind === "local_access"
          ? oauthBindingQuotaReserve
          : null;
  
      setOauthBindingSaving(true);
      try {
        if (oauthBindingTargetKind === "local_access") {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessBoundOAuthAccount(
              selectedOAuthBindingAccount.id,
              quotaReserve,
            );
          setLocalAccessState(nextState);
        } else if (oauthBindingAccount) {
          await updateApiKeyBoundOAuthAccount(
            oauthBindingAccount.id,
            selectedOAuthBindingAccount.id,
          );
        }
        setMessage({
          text: t("codex.api.oauthBinding.saveSuccess", "OAuth 绑定已更新"),
        });
        const shouldSwitch =
          oauthBindingTargetKind === "api_key_account" && oauthBindingAutoSwitch;
        const accountId = oauthBindingAccount?.id ?? "";
        resetOAuthBindingModal();
        if (shouldSwitch) {
          await executeCodexAccountSwitch(accountId);
        }
      } catch (err) {
        setOauthBindingError(
          t("codex.api.oauthBinding.saveFailed", {
            defaultValue: "OAuth 绑定失败：{{error}}",
            error: String(err).replace(/^Error:\s*/, ""),
          }),
        );
      } finally {
        setOauthBindingSaving(false);
      }
    }, [
      executeCodexAccountSwitch,
      oauthBindingAccount,
      oauthBindingAutoSwitch,
      oauthBindingQuotaReserve,
      oauthBindingTargetKind,
      selectedOAuthBindingAccount,
      setMessage,
      setOauthBindingError,
      t,
      updateApiKeyBoundOAuthAccount,
      resetOAuthBindingModal,
    ]);
  
    const handleClearOAuthBinding = useCallback(async () => {
      if (!oauthBindingTargetKind) return;
      if (oauthBindingTargetKind === "api_key_account" && !oauthBindingAccount) {
        return;
      }
  
      setOauthBindingSaving(true);
      setOauthBindingError(null);
      try {
        if (oauthBindingTargetKind === "local_access") {
          const nextState =
            await codexLocalAccessService.updateCodexLocalAccessBoundOAuthAccount(
              null,
            );
          setLocalAccessState(nextState);
        } else if (oauthBindingAccount) {
          await updateApiKeyBoundOAuthAccount(oauthBindingAccount.id, null);
        }
        setMessage({
          text: t("codex.api.oauthBinding.clearSuccess", "OAuth 绑定已解除"),
        });
        resetOAuthBindingModal();
      } catch (err) {
        setOauthBindingError(
          t("codex.api.oauthBinding.clearFailed", {
            defaultValue: "解除 OAuth 绑定失败：{{error}}",
            error: String(err).replace(/^Error:\s*/, ""),
          }),
        );
      } finally {
        setOauthBindingSaving(false);
      }
    }, [
      oauthBindingAccount,
      oauthBindingTargetKind,
      resetOAuthBindingModal,
      setMessage,
      setOauthBindingError,
      t,
      updateApiKeyBoundOAuthAccount,
    ]);

    const handleReauthorizeOAuthBinding = useCallback(() => {
      const targetAccountId = selectedOAuthBindingAccount?.id?.trim();
      if (!targetAccountId) return;
      const retryOAuthBinding = oauthBindingTargetKind
        ? {
            targetKind: oauthBindingTargetKind,
            targetAccountId: oauthBindingAccount?.id?.trim() || undefined,
            quotaReserve:
              oauthBindingTargetKind === "local_access"
                ? oauthBindingQuotaReserve
                : null,
          }
        : undefined;
      setReauthRetryOAuthBinding(retryOAuthBinding ?? null);
      resetOAuthBindingModal();
      requestCodexOpenAddAccount({
        tab: "oauth",
        targetAccountId,
        retryOAuthBinding,
      });
    }, [
      oauthBindingAccount,
      oauthBindingQuotaReserve,
      oauthBindingTargetKind,
      resetOAuthBindingModal,
      selectedOAuthBindingAccount,
      setReauthRetryOAuthBinding,
    ]);
  
    const findCachedCodexCliInstance = (
      bindAccountId: string,
      workingDir: string,
    ): InstanceProfile | null => {
      const normalizedWorkingDir = normalizePathForCompare(workingDir);
      return (
        codexInstanceStore.instances.find(
          (instance) =>
            !instance.isDefault &&
            (instance.launchMode ?? "app") === "cli" &&
            instance.bindAccountId === bindAccountId &&
            normalizePathForCompare(instance.workingDir) === normalizedWorkingDir,
        ) ?? null
      );
    };
  
    const buildCodexCliInstanceDraft = async (
      modal: CodexCliLaunchModalState,
      workingDir: string,
    ): Promise<{
      instanceId: string | null;
      draft: CodexCliInstanceDraft;
    }> => {
      const normalizedWorkingDir = normalizePathForCompare(workingDir);
      const bindAccountId =
        modal.target === "apiService"
          ? CODEX_API_SERVICE_BIND_ID
          : modal.bindAccountId || modal.accountId;
      const cached = findCachedCodexCliInstance(
        bindAccountId,
        normalizedWorkingDir,
      );
      if (cached) {
        return {
          instanceId: cached.id,
          draft: {
            name: cached.name,
            userDataDir: cached.userDataDir,
            workingDir:
              normalizePathForCompare(cached.workingDir) || normalizedWorkingDir,
            extraArgs: cached.extraArgs || "",
            bindAccountId,
          },
        };
      }
  
      let defaults = codexCliInstanceDefaultsRef.current;
      if (!defaults) {
        defaults = await codexInstanceService.getInstanceDefaults();
        codexCliInstanceDefaultsRef.current = defaults;
      }
      const instanceHash = md5(
        `${bindAccountId}|${normalizedWorkingDir}`,
      ).substring(0, 12);
      const instanceName = sanitizeCodexCliInstanceName(
        `${modal.accountLabel} CLI ${instanceHash.substring(0, 6)}`,
      );
      const directoryName =
        modal.target === "apiService"
          ? `cli-api-service-${instanceHash}`
          : `cli-${instanceHash}`;
      return {
        instanceId: null,
        draft: {
          name: instanceName,
          userDataDir: joinFilePath(defaults.rootDir, directoryName),
          workingDir: normalizedWorkingDir,
          extraArgs: "",
          bindAccountId,
        },
      };
    };
  
    const resolveCodexCliInstance = async (
      draft: CodexCliInstanceDraft,
    ): Promise<InstanceProfile> => {
      const cached = findCachedCodexCliInstance(
        draft.bindAccountId,
        draft.workingDir,
      );
      if (cached) return cached;
  
      const instances = await codexInstanceService.listInstances();
      const normalizedWorkingDir = normalizePathForCompare(draft.workingDir);
      const existing = instances.find(
        (instance) =>
          !instance.isDefault &&
          (instance.launchMode ?? "app") === "cli" &&
          instance.bindAccountId === draft.bindAccountId &&
          normalizePathForCompare(instance.workingDir) === normalizedWorkingDir,
      );
      if (existing) return existing;
  
      return await codexInstanceService.createInstance({
        name: draft.name,
        userDataDir: draft.userDataDir,
        workingDir: draft.workingDir,
        extraArgs: draft.extraArgs,
        bindAccountId: draft.bindAccountId,
        launchMode: "cli",
        copySourceInstanceId: "__default__",
        initMode: "copy",
      });
    };
  
    const prepareCodexCliLaunch = async (
      modal: CodexCliLaunchModalState,
      workingDirOverride?: string,
    ): Promise<CodexCliLaunchModalState | null> => {
      const workingDir = (workingDirOverride ?? modal.workingDir).trim();
      if (!workingDir) {
        setCliLaunchModal((prev) =>
          prev
            ? {
                ...prev,
                workingDirError: t(
                  "instances.form.pathRequired",
                  "请选择工作目录",
                ),
                executeError: null,
              }
            : prev,
        );
        return null;
      }
  
      setCliLaunchingAccountId(modal.accountId);
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              workingDir,
              workingDirError: null,
              preparing: true,
              copied: false,
              executeMessage: null,
              executeError: null,
            }
          : prev,
      );
      try {
        if (
          modal.target === "account" &&
          !accounts.some((item) => item.id === modal.accountId)
        ) {
          throw new Error(t("instances.quota.accountMissing", "账号不存在"));
        }
        const { instanceId, draft } = await buildCodexCliInstanceDraft(
          modal,
          workingDir,
        );
        const launchInfo =
          await codexInstanceService.previewCodexInstanceLaunchCommand({
            userDataDir: draft.userDataDir,
            workingDir: draft.workingDir,
            extraArgs: draft.extraArgs,
            terminal: selectedTerminal,
          });
        persistLastCodexCliWorkingDir(workingDir);
        const next: CodexCliLaunchModalState = {
          ...modal,
          instanceId,
          instanceDraft: draft,
          instanceName: draft.name,
          workingDir,
          workingDirError: null,
          launchCommand: launchInfo.launchCommand,
          terminalCommand: launchInfo.terminalCommand,
          runtimePrepared: false,
          preparing: false,
          copied: false,
          executing: false,
          executeMessage: null,
          executeError: null,
        };
        setCliLaunchModal((prev) =>
          prev && prev.accountId === modal.accountId ? next : prev,
        );
        return next;
      } catch (error) {
        setCliLaunchModal((prev) =>
          prev && prev.accountId === modal.accountId
            ? {
                ...prev,
                preparing: false,
                executing: false,
                executeMessage: null,
                executeError: String(error).replace(/^Error:\s*/, ""),
              }
            : prev,
        );
        return null;
      } finally {
        setCliLaunchingAccountId(null);
      }
    };
  
    const openCodexCliLaunchModal = (
      target: "account" | "apiService",
      accountId: string,
      accountLabel: string,
      bindAccountId?: string,
    ) => {
      if (cliLaunchModal || cliLaunchingAccountId) return;
      setMessage(null);
      const modal: CodexCliLaunchModalState = {
        target,
        accountId,
        bindAccountId: bindAccountId || accountId,
        accountLabel,
        instanceId: null,
        instanceDraft: null,
        instanceName: t("common.loading", "加载中..."),
        workingDir: readLastCodexCliWorkingDir(),
        workingDirError: null,
        launchCommand: "",
        terminalCommand: "",
        runtimePrepared: false,
        preparing: false,
        copied: false,
        executing: false,
        executeMessage: null,
        executeError: null,
      };
      setCliLaunchModal(modal);
      if (modal.workingDir) {
        void prepareCodexCliLaunch(modal);
      }
    };
  
    const handleLaunchCodexCli = async (account: CodexAccount) => {
      const blockedReason = getCodexSwitchOrLaunchBlockedReason(account);
      if (blockedReason) {
        setMessage({
          text: blockedReason,
          tone: "error",
        });
        return;
      }
      if (isCodexWebSessionAccount(account)) {
        setMessage({
          text: t(
            "codex.webSessionImport.actionBlocked",
            "Web Session 账号仅支持查看额度，无法切换或启动。",
          ),
          tone: "error",
        });
        return;
      }
      let launchAccount = account;
      if (isDeepSeekAccount(account)) {
        const presentation = buildCodexAccountPresentation(account, t);
        try {
          const prepared = await deepSeekStart.confirmStart(
            account,
            updateAccountInstanceAccess,
            presentation.displayName || account.email || account.id,
          );
          if (!prepared) return;
          launchAccount = prepared;
        } catch (error) {
          setMessage({
            text: `${t("common.failed", "失败")}: ${String(error).replace(/^Error:\s*/, "")}`,
            tone: "error",
          });
          return;
        }
      }
      const presentation = buildCodexAccountPresentation(launchAccount, t);
      openCodexCliLaunchModal(
        "account",
        launchAccount.id,
        presentation.displayName || launchAccount.email || launchAccount.id,
        isDeepSeekAccount(launchAccount)
          ? resolveDeepSeekBindAccountId(launchAccount)
          : launchAccount.id,
      );
    };
  
    const handleLaunchLocalAccessCli = () => {
      if (cliLaunchModal || cliLaunchingAccountId) return;
      if (!localAccessCollection) {
        setMessage({
          text: t("codex.localAccess.testUnavailable", "当前 API 服务地址不可用"),
          tone: "error",
        });
        return;
      }
      openCodexCliLaunchModal(
        "apiService",
        CODEX_API_SERVICE_BIND_ID,
        t("codex.localAccess.title", "API 服务"),
      );
    };
  
    const updateCodexCliWorkingDir = (workingDir: string) => {
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              workingDir,
              workingDirError: null,
              instanceId: null,
              instanceDraft: null,
              instanceName: t("common.loading", "加载中..."),
              launchCommand: "",
              terminalCommand: "",
              runtimePrepared: false,
              copied: false,
              executeMessage: null,
              executeError: null,
            }
          : prev,
      );
    };
  
    const handleChooseCodexCliWorkingDir = async () => {
      if (!cliLaunchModal || cliLaunchModal.preparing || cliLaunchModal.executing)
        return;
      const selected = await openFileDialog({
        directory: true,
        multiple: false,
        title: t("codex.cli.selectWorkingDir", "选择 Codex CLI 工作目录"),
      });
      if (!selected || typeof selected !== "string") return;
      const next = { ...cliLaunchModal, workingDir: selected };
      updateCodexCliWorkingDir(selected);
      await prepareCodexCliLaunch(next, selected);
    };
  
    const handleCopyCodexCliCommand = async () => {
      if (!cliLaunchModal || cliLaunchModal.preparing) return;
      const prepared = cliLaunchModal.terminalCommand
        ? cliLaunchModal
        : await prepareCodexCliLaunch(cliLaunchModal);
      if (!prepared) return;
      const runtime = prepared.runtimePrepared
        ? prepared
        : await prepareCodexCliRuntime(prepared);
      if (!runtime) return;
      try {
        await navigator.clipboard.writeText(
          runtime.launchCommand || runtime.terminalCommand,
        );
        setCliLaunchModal((prev) =>
          prev ? { ...prev, copied: true, executeError: null } : prev,
        );
        window.setTimeout(() => {
          setCliLaunchModal((prev) => (prev ? { ...prev, copied: false } : prev));
        }, 1200);
      } catch {
        setCliLaunchModal((prev) =>
          prev
            ? {
                ...prev,
                executeError: t(
                  "common.shared.export.copyFailed",
                  "复制失败，请手动复制",
                ),
              }
            : prev,
        );
      }
    };
  
    async function prepareCodexCliRuntime(
      modal: CodexCliLaunchModalState,
    ): Promise<CodexCliLaunchModalState | null> {
      if (modal.runtimePrepared) return modal;
      setCliLaunchingAccountId(modal.accountId);
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              preparing: true,
              copied: false,
              executeMessage: null,
              executeError: null,
            }
          : prev,
      );
      try {
        if (
          modal.target === "account" &&
          !accounts.some((item) => item.id === modal.accountId)
        ) {
          throw new Error(t("instances.quota.accountMissing", "账号不存在"));
        }
        const draft =
          modal.instanceDraft ??
          (await buildCodexCliInstanceDraft(modal, modal.workingDir)).draft;
        let instanceId = modal.instanceId;
        if (!instanceId) {
          const instance = await resolveCodexCliInstance(draft);
          instanceId = instance.id;
        }
        const started = await codexInstanceService.startInstance(instanceId);
        const launchInfo =
          await codexInstanceService.getCodexInstanceLaunchCommand(
            started.id,
            selectedTerminal,
          );
        const next: CodexCliLaunchModalState = {
          ...modal,
          instanceId: started.id,
          instanceDraft: {
            name: started.name,
            userDataDir: started.userDataDir,
            workingDir:
              normalizePathForCompare(started.workingDir) || draft.workingDir,
            extraArgs: started.extraArgs || "",
            bindAccountId: started.bindAccountId || draft.bindAccountId,
          },
          instanceName: started.name,
          launchCommand: launchInfo.launchCommand,
          terminalCommand: launchInfo.terminalCommand,
          runtimePrepared: true,
          preparing: false,
          copied: false,
          executing: false,
          executeMessage: null,
          executeError: null,
        };
        setCliLaunchModal((prev) =>
          prev && prev.accountId === modal.accountId ? next : prev,
        );
        return next;
      } catch (error) {
        setCliLaunchModal((prev) =>
          prev && prev.accountId === modal.accountId
            ? {
                ...prev,
                preparing: false,
                executing: false,
                executeMessage: null,
                executeError: String(error).replace(/^Error:\s*/, ""),
              }
            : prev,
        );
        return null;
      } finally {
        setCliLaunchingAccountId(null);
      }
    }
  
    const handleExecuteCodexCli = async () => {
      if (!cliLaunchModal || cliLaunchModal.preparing || cliLaunchModal.executing)
        return;
      const prepared = cliLaunchModal.terminalCommand
        ? cliLaunchModal
        : await prepareCodexCliLaunch(cliLaunchModal);
      if (!prepared) return;
      const runtime = prepared.runtimePrepared
        ? prepared
        : await prepareCodexCliRuntime(prepared);
      if (!runtime?.instanceId) return;
      setCliLaunchingAccountId(runtime.accountId);
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: true,
              executeMessage: null,
              executeError: null,
            }
          : prev,
      );
      try {
        const result =
          await codexInstanceService.executeCodexInstanceLaunchCommand(
            runtime.instanceId,
            selectedTerminal,
          );
        await codexInstanceStore.refreshInstances();
        setCliLaunchModal((prev) =>
          prev
            ? {
                ...prev,
                executing: false,
                executeMessage:
                  result || t("codex.cli.launchSuccess", "已启动 Codex CLI"),
              }
            : prev,
        );
      } catch (error) {
        setCliLaunchModal((prev) =>
          prev
            ? {
                ...prev,
                executing: false,
                executeError: String(error).replace(/^Error:\s*/, ""),
              }
            : prev,
        );
      } finally {
        setCliLaunchingAccountId(null);
      }
    };
  
    useEffect(() => {
      const draft = cliLaunchModal?.instanceDraft;
      if (!draft || cliLaunchModal.preparing || cliLaunchModal.executing) return;
      let disposed = false;
      void codexInstanceService
        .previewCodexInstanceLaunchCommand({
          userDataDir: draft.userDataDir,
          workingDir: draft.workingDir,
          extraArgs: draft.extraArgs,
          terminal: selectedTerminal,
          launchCommand: cliLaunchModal.runtimePrepared
            ? cliLaunchModal.launchCommand
            : null,
        })
        .then((launchInfo) => {
          if (disposed) return;
          setCliLaunchModal((prev) =>
            prev && prev.instanceDraft?.userDataDir === draft.userDataDir
              ? {
                  ...prev,
                  launchCommand: launchInfo.launchCommand,
                  terminalCommand: launchInfo.terminalCommand,
                  copied: false,
                  executeMessage: null,
                  executeError: null,
                }
              : prev,
          );
        })
        .catch((error) => {
          if (disposed) return;
          setCliLaunchModal((prev) =>
            prev && prev.instanceDraft?.userDataDir === draft.userDataDir
              ? {
                  ...prev,
                  executeError: String(error).replace(/^Error:\s*/, ""),
                }
              : prev,
          );
        });
      return () => {
        disposed = true;
      };
    }, [selectedTerminal]);
  
    const handleImportFromLocal = async () => {
      page.setAddStatus("loading");
      page.setAddMessage(t("codex.import.importing", "正在导入本地账号..."));
      try {
        const account = await codexService.importCodexFromLocal();
        await fetchAccounts();
        await new Promise((resolve) => setTimeout(resolve, 180));
        await fetchAccounts();
        await assignCodexAccountsToTargetGroup([account]);
        await emitAccountsChanged({
          platformId: "codex",
          reason: "import",
        });
        try {
          await syncImportedAccountsToApiService([account.id]);
        } catch (error) {
          page.setAddStatus("error");
          page.setAddMessage(
            t(
              "codex.importApiService.syncFailed",
              "账号已导入，但加入 API 服务失败：{{error}}",
            ).replace("{{error}}", String(error).replace(/^Error:\s*/, "")),
          );
          return;
        }
        page.setAddStatus("success");
        page.setAddMessage(
          t("codex.import.successMsg", "导入成功: {{email}}").replace(
            "{{email}}",
            maskAccountText(account.email),
          ),
        );
        setTimeout(() => {
          closeAddModal();
        }, 1200);
      } catch (e) {
        page.setAddStatus("error");
        page.setAddMessage(
          t("common.shared.import.failedMsg", "导入失败: {{error}}").replace(
            "{{error}}",
            String(e).replace(/^Error:\s*/, ""),
          ),
        );
      }
    };
  
    const startBatchImportFromPaths = async (
      paths: string[],
      checkQuota: boolean,
    ) => {
      cleanupBatchImportListeners();
      setBatchImportOpen(true);
      setBatchImportSessionId(null);
      setBatchImportProgress(null);
      setBatchImportPreview(null);
      setBatchImportSelectedIds([]);
      setBatchImportFilter("all");
      setBatchImportResult(null);
      setBatchImportError(null);
      setBatchImportFilePaths(paths);
      setBatchImportCheckQuota(checkQuota);
      setBatchImportBusy(true);
      batchImportSessionIdRef.current = "__pending__";
  
      try {
        const progressUnlisten =
          await listen<codexService.CodexBatchImportProgress>(
            "codex:batch-import-progress",
            (event) => {
              if (event.payload.sessionId !== batchImportSessionIdRef.current) {
                return;
              }
              setBatchImportProgress(event.payload);
              setBatchImportCheckQuota(event.payload.checkQuota);
            },
          );
        const completedUnlisten =
          await listen<codexService.CodexBatchImportPreview>(
            "codex:batch-import-completed",
            (event) => {
              if (event.payload.sessionId !== batchImportSessionIdRef.current) {
                return;
              }
              setBatchImportPreview(event.payload);
              setBatchImportCheckQuota(event.payload.checkQuota);
              setBatchImportProgress((current) =>
                current
                  ? {
                      ...current,
                      phase: event.payload.status,
                      checkQuota: event.payload.checkQuota,
                      current: event.payload.items.length,
                      total: event.payload.total,
                    }
                  : current,
              );
              setBatchImportSelectedIds((prev) => {
                const next = new Set(prev);
                for (const item of event.payload.items) {
                  if (
                    item.defaultSelected &&
                    item.selectable &&
                    (item.status === "ready" || item.status === "existing")
                  ) {
                    next.add(item.itemId);
                  }
                }
                return Array.from(next);
              });
              setBatchImportBusy(false);
            },
          );
        const previewUnlisten =
          await listen<codexService.CodexBatchImportPreview>(
            "codex:batch-import-preview",
            (event) => {
              if (event.payload.sessionId !== batchImportSessionIdRef.current) {
                return;
              }
              setBatchImportPreview(event.payload);
              setBatchImportCheckQuota(event.payload.checkQuota);
              setBatchImportSelectedIds((prev) => {
                const next = new Set(prev);
                for (const item of event.payload.items) {
                  if (
                    item.defaultSelected &&
                    item.selectable &&
                    (item.status === "ready" || item.status === "existing")
                  ) {
                    next.add(item.itemId);
                  }
                }
                return Array.from(next);
              });
            },
          );
        batchImportUnlistenersRef.current = [
          progressUnlisten,
          previewUnlisten,
          completedUnlisten,
        ];
  
        const started = await codexService.startCodexBatchImportFromFiles(
          paths,
          checkQuota,
        );
        batchImportSessionIdRef.current = started.sessionId;
        setBatchImportSessionId(started.sessionId);
        try {
          localStorage.setItem(
            CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY,
            started.sessionId,
          );
        } catch {
          // ignore storage failures
        }
        // The backend starts scanning before this invoke resolves. A fast scan can
        // therefore emit its terminal event while the listener still filters on
        // __pending__. Re-read the now-addressable session to recover that event.
        try {
          const recoveredPreview = await codexService.getCodexBatchImportPreview(
            started.sessionId,
          );
          const recovery = recoverCodexBatchImportStartFromPreview(
            batchImportSessionIdRef.current,
            started.sessionId,
            recoveredPreview,
            batchImportSelectedIds,
          );
          if (recovery) {
            setBatchImportPreview(recovery.preview);
            setBatchImportCheckQuota(recovery.preview.checkQuota);
            setBatchImportSelectedIds(recovery.selectedIds);
            setBatchImportBusy(false);
          }
        } catch {
          // If this read fails, only later listener events follow their normal path;
          // it cannot recover terminal or error events that were already missed.
        }
      } catch (e) {
        cleanupBatchImportListeners();
        batchImportSessionIdRef.current = null;
        setBatchImportBusy(false);
        setBatchImportError(String(e).replace(/^Error:\s*/, ""));
      }
    };
  
    const handleImportFromFiles = async () => {
      try {
        const selected = await openFileDialog({
          multiple: true,
          filters: [{ name: "JSON", extensions: ["json"] }],
        });
        if (!selected || (Array.isArray(selected) && selected.length === 0)) {
          return;
        }
        const paths = Array.isArray(selected) ? selected : [selected];
        setBatchImportTargetGroupId(
          resolveValidCodexGroupId(codexAddTargetGroupId),
        );
        closeAddModal();
        // 1.3.0 交互：打开批量导入弹框，默认不检测；可选开启导入前检测。
        await startBatchImportFromPaths(paths, false);
      } catch (e) {
        setBatchImportBusy(false);
        setBatchImportError(String(e).replace(/^Error:\s*/, ""));
      }
    };
  
    const handleBatchImportCheckQuotaChange = async (checkQuota: boolean) => {
      if (
        batchImportBusy ||
        batchImportResult ||
        checkQuota === batchImportCheckQuota
      ) {
        return;
      }
      setBatchImportCheckQuota(checkQuota);
      if (batchImportFilePaths.length === 0) {
        return;
      }
      // 切换检测开关后按新模式重新解析，避免残留旧检测结果。
      await startBatchImportFromPaths(batchImportFilePaths, checkQuota);
    };
  
    const handleCancelBatchImport = async () => {
      if (!batchImportSessionId) {
        return;
      }
      if (batchImportBusy) {
        try {
          await codexService.cancelCodexBatchImport(batchImportSessionId);
          setBatchImportProgress((current) =>
            current ? { ...current, phase: "cancelling" } : current,
          );
        } catch (e) {
          setBatchImportError(String(e).replace(/^Error:\s*/, ""));
        }
      }
    };
  
    const handleCloseBatchImport = async () => {
      if (batchImportResult) {
        resetBatchImportState();
        return;
      }
      // 扫描/解析中：最小化弹框，后台继续。
      if (batchImportBusy) {
        setBatchImportOpen(false);
        return;
      }
      // 无可选账号时直接丢弃，避免任务条长期挂起。
      const selectableCount = (batchImportPreview?.items ?? []).filter(
        (item) => item.selectable && item.status !== "invalid",
      ).length;
      if (!batchImportPreview || selectableCount === 0) {
        resetBatchImportState();
        return;
      }
      setBatchImportOpen(false);
    };
  
    const handleDismissBatchImportTask = () => {
      // 进行中：先取消会话再清理；空闲/失败：直接丢弃，避免任务条无法关闭（#1445）
      if (batchImportBusy && batchImportSessionId) {
        void (async () => {
          try {
            await codexService.cancelCodexBatchImport(batchImportSessionId);
          } catch {
            // ignore cancel failures and still clear UI
          }
          resetBatchImportState();
        })();
        return;
      }
      resetBatchImportState();
    };
  
    const toggleBatchImportItem = (itemId: string) => {
      if (!batchImportSelectableIdSet.has(itemId)) return;
      setBatchImportSelectedIds((prev) =>
        prev.includes(itemId)
          ? prev.filter((id) => id !== itemId)
          : [...prev, itemId],
      );
    };
  
    const selectAllBatchImportAccounts = () => {
      const items = batchImportPreview?.items ?? [];
      const ids = items
        .filter((item) => item.selectable && item.status !== "invalid")
        .map((item) => item.itemId);
      setBatchImportFilter("all");
      setBatchImportSelectedIds(ids);
    };
  
    const selectReadyBatchImportAccounts = () => {
      const items = batchImportPreview?.items ?? [];
      const ids = items
        .filter(
          (item) =>
            item.selectable &&
            (item.status === "ready" || item.status === "existing"),
        )
        .map((item) => item.itemId);
      setBatchImportFilter("ready");
      setBatchImportSelectedIds(ids);
    };
  
    const clearBatchImportSelection = () => {
      setBatchImportFilter("all");
      setBatchImportSelectedIds([]);
    };
  
    const handleConfirmBatchImport = async (
      options: { addToApiService?: boolean } = {},
    ) => {
      const selectedSelectableIds = batchImportSelectedIds.filter((id) =>
        batchImportSelectableIdSet.has(id),
      );
      if (!batchImportSessionId || selectedSelectableIds.length === 0) {
        setBatchImportError(
          t("codex.batchImport.noSelection", "请先选择要导入的账号"),
        );
        return;
      }
      setBatchImportBusy(true);
      setBatchImportError(null);
      setBatchImportProgress({
        sessionId: batchImportSessionId,
        phase: "importing",
        checkQuota: batchImportCheckQuota,
        current: 0,
        total: selectedSelectableIds.length,
        success: 0,
        failed: 0,
        quotaFailed: 0,
        existing: 0,
        currentLabel: null,
      });
      try {
        const result = await codexService.confirmCodexBatchImport(
          batchImportSessionId,
          selectedSelectableIds,
        );
        setBatchImportResult(result);
        let apiServiceError: string | null = null;
        await fetchAccounts();
        await assignCodexAccountsToTargetGroup(
          result.imported,
          batchImportTargetGroupId,
        );
        // Optional bulk tags for this import batch (#1166)
        const batchTags = Array.from(
          new Set(
            batchImportTagsInput
              .split(/[,，\s]+/)
              .map((tag) => tag.trim().toLowerCase())
              .filter(Boolean),
          ),
        ).slice(0, 10);
        if (batchTags.length > 0 && result.imported.length > 0) {
          await Promise.allSettled(
            result.imported.map(async (account) => {
              const existing = (account.tags || [])
                .map((tag) => tag.trim().toLowerCase())
                .filter(Boolean);
              const merged = Array.from(new Set([...existing, ...batchTags]));
              await store.updateAccountTags(account.id, merged);
            }),
          );
          await fetchAccounts();
        }
        if (result.imported.length > 0) {
          await emitAccountsChanged({
            platformId: "codex",
            reason: "import",
          });
        }
  
        if (options.addToApiService) {
          const importedIds = result.imported
            .map((account) => account.id)
            .filter(Boolean);
          const nextLocalAccessAccountIds = Array.from(
            new Set([
              ...(localAccessCollection?.accountIds ?? []),
              ...importedIds,
            ]),
          );
          setLocalAccessSaving(true);
          try {
            const nextState =
              await codexLocalAccessService.saveCodexLocalAccessAccounts(
                nextLocalAccessAccountIds,
                localAccessCollection?.restrictFreeAccounts ?? true,
              );
            setLocalAccessState(nextState);
            if (importedIds.length > 0) {
              await ensureLocalAccessEntryVisible();
              setImportApiServiceGuideCount(importedIds.length);
            }
            window.dispatchEvent(new Event("codex-local-access-state-updated"));
          } catch (apiError) {
            apiServiceError = t(
              "codex.batchImport.addToApiServiceFailed",
              "账号已导入，但添加到 API 服务失败: {{error}}",
            ).replace("{{error}}", String(apiError).replace(/^Error:\s*/, ""));
          } finally {
            setLocalAccessSaving(false);
          }
        } else if (result.imported.length > 0) {
          try {
            await syncImportedAccountsToApiService(
              result.imported.map((account) => account.id),
            );
          } catch (error) {
            apiServiceError = t(
              "codex.importApiService.syncFailed",
              "账号已导入，但加入 API 服务失败：{{error}}",
            ).replace("{{error}}", String(error).replace(/^Error:\s*/, ""));
          }
        }
  
        if (apiServiceError) {
          setBatchImportError(apiServiceError);
        }
        cleanupBatchImportListeners();
        try {
          localStorage.removeItem(CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY);
        } catch {
          // ignore storage failures
        }
      } catch (e) {
        setBatchImportError(String(e).replace(/^Error:\s*/, ""));
      } finally {
        setBatchImportBusy(false);
      }
    };
  
    const handleResumeBatchImport = async () => {
      if (!batchImportSessionId || batchImportBusy) return;
      setBatchImportBusy(true);
      setBatchImportError(null);
      setBatchImportResult(null);
      try {
        await codexService.resumeCodexBatchImport(batchImportSessionId);
        setBatchImportProgress((current) =>
          current ? { ...current, phase: "scanning" } : current,
        );
        setBatchImportPreview((current) =>
          current ? { ...current, status: "scanning" } : current,
        );
      } catch (e) {
        setBatchImportBusy(false);
        setBatchImportError(String(e).replace(/^Error:\s*/, ""));
      }
    };
  
    const handleSelectApiProviderPreset = useCallback(
      (providerId: string) => {
        apiProviderPresetExplicitlySelectedRef.current = true;
        setApiProviderPresetId(providerId);
        setManagedProviderId("");
        setManagedProviderApiKeyId("");
        setApiModelCatalogError(null);
        if (selectedManagedProviderApiKey) {
          setApiKeyInput("");
        }
        if (providerId === CODEX_API_PROVIDER_CUSTOM_ID) {
          setApiBaseUrlInput("");
          setNewManagedProviderNameInput("");
          setApiModelCatalogInput("");
          setApiModelContextWindowsInput({});
          return;
        }
        const sponsorTemplate = sponsorApiProviderTemplates.find(
          (template) => template.id === providerId,
        );
        if (sponsorTemplate) {
          setApiBaseUrlInput(sponsorTemplate.baseUrl);
          setNewManagedProviderNameInput(sponsorTemplate.name);
          setApiModelCatalogInput(sponsorTemplate.modelCatalog.join("\n"));
          setApiModelContextWindowsInput({});
          return;
        }
        const preset = findCodexApiProviderPresetById(providerId);
        if (!preset || preset.baseUrls.length === 0) return;
        setApiBaseUrlInput(preset.baseUrls[0]);
        setNewManagedProviderNameInput("");
        setApiModelCatalogInput((preset.modelCatalog ?? []).join("\n"));
        setApiModelContextWindowsInput({});
        if (providerId === OPENAI_OFFICIAL_PRESET_ID) {
          setApiSyncModelCatalogToCodex(false);
        }
      },
      [selectedManagedProviderApiKey, sponsorApiProviderTemplates],
    );
  
    const handleSelectManagedProvider = useCallback(
      (providerId: string) => {
        apiProviderPresetExplicitlySelectedRef.current = true;
        setApiProviderPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
        setManagedProviderId(providerId);
        const provider = managedProviders.find((item) => item.id === providerId);
        if (!provider) return;
        setApiBaseUrlInput(provider.baseUrl);
        setApiModelCatalogInput((provider.modelCatalog ?? []).join("\n"));
        setApiModelContextWindowsInput(
          contextWindowDraftsFromRecord(
            provider.modelContextWindows,
            provider.modelCatalog ?? [],
          ),
        );
        setApiModelCatalogError(null);
        const firstKey = provider.apiKeys[0];
        if (firstKey) {
          setManagedProviderApiKeyId(firstKey.id);
          setApiKeyInput(firstKey.apiKey);
          setApiKeyInputVisible(false);
        } else {
          setManagedProviderApiKeyId("");
        }
        setNewManagedProviderNameInput(provider.name);
      },
      [managedProviders],
    );
  
    const handleSelectManagedProviderApiKey = useCallback(
      (apiKeyId: string) => {
        setManagedProviderApiKeyId(apiKeyId);
        if (!apiKeyId.trim()) {
          // Manual entry: clear prefilled secret so user can paste a new key.
          setApiKeyInput("");
          setApiKeyInputVisible(true);
          return;
        }
        const key = selectedManagedProvider?.apiKeys.find(
          (item) => item.id === apiKeyId,
        );
        if (key) {
          setApiKeyInput(key.apiKey);
          setApiKeyInputVisible(false);
        }
      },
      [selectedManagedProvider],
    );
  
    const handleApiKeyInputChange = useCallback(
      (value: string) => {
        setApiKeyInput(value);
        setApiModelCatalogError(null);
        if (
          selectedManagedProviderApiKey &&
          value.trim() !== selectedManagedProviderApiKey.apiKey.trim()
        ) {
          setManagedProviderApiKeyId("");
        }
      },
      [selectedManagedProviderApiKey],
    );
  
    const handleApiBaseUrlInputChange = useCallback(
      (value: string) => {
        setApiBaseUrlInput(value);
        setApiModelCatalogError(null);
        if (
          selectedManagedProvider &&
          !isSameHttpBaseUrl(selectedManagedProvider.baseUrl, value)
        ) {
          setManagedProviderId("");
          setManagedProviderApiKeyId("");
        }
      },
      [selectedManagedProvider],
    );
  
    const handleFetchApiModelCatalog = useCallback(async () => {
      const apiKey = apiKeyInput.trim();
      const baseUrl = apiBaseUrlInput.trim() || DEFAULT_CODEX_API_BASE_URL;
      if (!apiKey || !baseUrl) {
        setApiModelCatalogError(
          t(
            "codex.api.modelCatalog.fetchCredentialsRequired",
            "请先填写 API Key 和 Base URL。",
          ),
        );
        return;
      }
      setApiModelCatalogFetching(true);
      setApiModelCatalogError(null);
      try {
        const result = await listModelProviderModels({ baseUrl, apiKey });
        const models = parseApiModelCatalogText(
          result.models.map((model) => model.id).join("\n"),
        );
        if (models.length === 0) {
          setApiModelCatalogError(
            t(
              "codex.api.modelCatalog.fetchEmpty",
              "上游未返回可用模型，已保留当前列表。",
            ),
          );
          return;
        }
        setApiModelCatalogInput(models.join("\n"));
      } catch (error) {
        setApiModelCatalogError(
          t("codex.api.modelCatalog.fetchFailed", {
            defaultValue: "获取上游模型失败：{{error}}",
            error: String(error).replace(/^Error:\s*/, ""),
          }),
        );
      } finally {
        setApiModelCatalogFetching(false);
      }
    }, [apiBaseUrlInput, apiKeyInput, t]);
  
    const applyApiKeyFunPrefill = useCallback(
      (request: ApiKeyFunPrefillPayload) => {
        if (request.target !== "codex") return;
        const apiKey = request.apiKey.trim();
        if (!apiKey) return;
  
        pendingApiKeyFunCodexPrefillRef.current = request;
        openCodexAddModal("apikey");
      },
      [openCodexAddModal],
    );
  
    useEffect(() => {
      if (!showAddModal || addTab !== "apikey") return;
      const request = pendingApiKeyFunCodexPrefillRef.current;
      if (!request) return;
      pendingApiKeyFunCodexPrefillRef.current = null;
  
      const apiKey = request.apiKey.trim();
      if (!apiKey) return;
  
      const requestBaseUrl =
        request.baseUrl?.trim() || APIKEY_FUN_PROVIDER_BASE_URL;
      const normalizedRequestBaseUrl =
        normalizeHttpBaseUrl(requestBaseUrl)?.toLowerCase() ?? "";
      const sponsorTemplate =
        sponsorApiProviderTemplates.find((template) => {
          const normalizedTemplateBaseUrl =
            normalizeHttpBaseUrl(template.baseUrl)?.toLowerCase() ?? "";
          const searchable = [
            template.name,
            template.website,
            template.apiKeyUrl,
            template.baseUrl,
          ]
            .join(" ")
            .toLowerCase();
          return (
            normalizedTemplateBaseUrl === normalizedRequestBaseUrl ||
            searchable.includes("apikey.fun") ||
            searchable.includes("api.apikey.fun")
          );
        }) ?? null;
  
      skipManagedProviderApiKeyAutofillRef.current = true;
      apiProviderPresetExplicitlySelectedRef.current = true;
      apiKeyFunPrefillModelCatalogRef.current = request.modelCatalog ?? null;
      setApiKeyInput(apiKey);
      setApiKeyInputVisible(false);
      setApiBaseUrlInput(sponsorTemplate?.baseUrl ?? requestBaseUrl);
      setManagedProviderId("");
      setManagedProviderApiKeyId("");
      setApiProviderPresetId(sponsorTemplate?.id ?? CODEX_API_PROVIDER_CUSTOM_ID);
      setNewManagedProviderNameInput(
        sponsorTemplate?.name ?? request.providerName?.trim() ?? "APIKEY.FUN",
      );
      setApiModelCatalogInput((request.modelCatalog ?? []).join("\n"));
      setApiModelContextWindowsInput({});
      setApiModelCatalogError(null);
      setAddStatus("idle");
      setAddMessage(
        t(
          "apiKeyFun.prefill.codexReady",
          "已带入 APIKEY.FUN 配置，请确认后添加到 Codex。",
        ),
      );
    }, [
      addTab,
      setAddMessage,
      setAddStatus,
      showAddModal,
      sponsorApiProviderTemplates,
      t,
    ]);
  
    useEffect(() => {
      const consumePrefill = () => {
        const request = consumeApiKeyFunPrefill("codex");
        if (request) {
          applyApiKeyFunPrefill(request);
        }
      };
      consumePrefill();
      window.addEventListener(APIKEY_FUN_PREFILL_EVENT, consumePrefill);
      return () => {
        window.removeEventListener(APIKEY_FUN_PREFILL_EVENT, consumePrefill);
      };
    }, [applyApiKeyFunPrefill]);
  
    const handleSelectEditingApiProviderPreset = useCallback(
      (providerId: string) => {
        setEditingApiProviderPresetId(providerId);
        setEditingManagedProviderId("");
        setEditingManagedProviderApiKeyId("");
        setEditingNewManagedProviderNameInput("");
        setEditingApiModelCatalogError(null);
        if (providerId === CODEX_API_PROVIDER_CUSTOM_ID) {
          setEditingApiModelCatalogInput("");
          setEditingApiModelContextWindowsInput({});
        }
        const preset = findCodexApiProviderPresetById(providerId);
        if (!preset || preset.baseUrls.length === 0) return;
        setEditingApiBaseUrlCredentialsValue(preset.baseUrls[0]);
        setEditingApiModelCatalogInput((preset.modelCatalog ?? []).join("\n"));
        setEditingApiModelContextWindowsInput({});
        if (providerId === OPENAI_OFFICIAL_PRESET_ID) {
          setEditingApiSyncModelCatalogToCodex(false);
        }
      },
      [],
    );
  
    const handleSelectEditingManagedProvider = useCallback(
      (providerId: string) => {
        setEditingApiProviderPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
        setEditingManagedProviderId(providerId);
        const provider = managedProviders.find((item) => item.id === providerId);
        if (!provider) return;
        setEditingApiBaseUrlCredentialsValue(provider.baseUrl);
        setEditingApiModelCatalogInput((provider.modelCatalog ?? []).join("\n"));
        setEditingApiModelContextWindowsInput(
          contextWindowDraftsFromRecord(
            provider.modelContextWindows,
            provider.modelCatalog ?? [],
          ),
        );
        setEditingApiModelCatalogError(null);
        const firstKey = provider.apiKeys[0];
        if (firstKey) {
          setEditingManagedProviderApiKeyId(firstKey.id);
          setEditingApiKeyCredentialsValue(firstKey.apiKey);
          setEditingApiKeyCredentialsVisible(false);
        } else {
          setEditingManagedProviderApiKeyId("");
        }
        setEditingNewManagedProviderNameInput(provider.name);
      },
      [managedProviders],
    );
  
    const handleSelectEditingManagedProviderApiKey = useCallback(
      (apiKeyId: string) => {
        setEditingManagedProviderApiKeyId(apiKeyId);
        if (!apiKeyId.trim()) {
          setEditingApiKeyCredentialsValue("");
          setEditingApiKeyCredentialsVisible(true);
          return;
        }
        const key = selectedEditingManagedProvider?.apiKeys.find(
          (item) => item.id === apiKeyId,
        );
        if (key) {
          setEditingApiKeyCredentialsValue(key.apiKey);
          setEditingApiKeyCredentialsVisible(false);
        }
      },
      [selectedEditingManagedProvider],
    );
  
    const handleEditingApiKeyCredentialsChange = useCallback(
      (value: string) => {
        setEditingApiKeyCredentialsValue(value);
        setEditingApiModelCatalogError(null);
        if (
          selectedEditingManagedProviderApiKey &&
          value.trim() !== selectedEditingManagedProviderApiKey.apiKey.trim()
        ) {
          setEditingManagedProviderApiKeyId("");
        }
      },
      [selectedEditingManagedProviderApiKey],
    );
  
    const handleEditingApiBaseUrlCredentialsChange = useCallback(
      (value: string) => {
        setEditingApiBaseUrlCredentialsValue(value);
        setEditingApiModelCatalogError(null);
        if (
          selectedEditingManagedProvider &&
          !isSameHttpBaseUrl(selectedEditingManagedProvider.baseUrl, value)
        ) {
          setEditingManagedProviderId("");
          setEditingManagedProviderApiKeyId("");
        }
      },
      [selectedEditingManagedProvider],
    );
  
    const handleFetchEditingApiModelCatalog = useCallback(async () => {
      const apiKey = editingApiKeyCredentialsValue.trim();
      const baseUrl =
        editingApiBaseUrlCredentialsValue.trim() || DEFAULT_CODEX_API_BASE_URL;
      if (!apiKey || !baseUrl) {
        setEditingApiModelCatalogError(
          t(
            "codex.api.modelCatalog.fetchCredentialsRequired",
            "请先填写 API Key 和 Base URL。",
          ),
        );
        return;
      }
      setEditingApiModelCatalogFetching(true);
      setEditingApiModelCatalogError(null);
      try {
        const result = await listModelProviderModels({ baseUrl, apiKey });
        const models = parseApiModelCatalogText(
          result.models.map((model) => model.id).join("\n"),
        );
        if (models.length === 0) {
          setEditingApiModelCatalogError(
            t(
              "codex.api.modelCatalog.fetchEmpty",
              "上游未返回可用模型，已保留当前列表。",
            ),
          );
          return;
        }
        setEditingApiModelCatalogInput(models.join("\n"));
      } catch (error) {
        setEditingApiModelCatalogError(
          t("codex.api.modelCatalog.fetchFailed", {
            defaultValue: "获取上游模型失败：{{error}}",
            error: String(error).replace(/^Error:\s*/, ""),
          }),
        );
      } finally {
        setEditingApiModelCatalogFetching(false);
      }
    }, [editingApiBaseUrlCredentialsValue, editingApiKeyCredentialsValue, t]);
  
    const closeQuickSwitchModal = useCallback(() => {
      if (quickSwitchSubmitting) return;
      setQuickSwitchAccountId(null);
      setQuickSwitchProviderId("");
      setQuickSwitchApiKeyId("");
      setQuickSwitchError(null);
    }, [quickSwitchSubmitting]);
  
    const openQuickSwitchProviderModal = useCallback(
      (account: CodexAccount) => {
        if (!isCodexApiKeyAccount(account)) return;
        const baseUrl = (account.api_base_url || "").trim();
        const apiKey = (account.openai_api_key || "").trim();
        const matchedProvider =
          findCodexModelProviderById(managedProviders, account.api_provider_id) ??
          findCodexModelProviderByBaseUrl(managedProviders, baseUrl);
        const fallbackProvider = matchedProvider ?? managedProviders[0] ?? null;
        const matchedApiKey = matchedProvider?.apiKeys.find(
          (item) => item.apiKey.trim() === apiKey,
        );
        const fallbackApiKey =
          matchedApiKey ?? fallbackProvider?.apiKeys[0] ?? null;
  
        setQuickSwitchAccountId(account.id);
        setQuickSwitchProviderId(fallbackProvider?.id ?? "");
        setQuickSwitchApiKeyId(fallbackApiKey?.id ?? "");
        setQuickSwitchError(null);
      },
      [managedProviders],
    );
  
    const handleSelectQuickSwitchProvider = useCallback(
      (providerId: string) => {
        setQuickSwitchProviderId(providerId);
        const provider = managedProviders.find((item) => item.id === providerId);
        setQuickSwitchApiKeyId(provider?.apiKeys[0]?.id ?? "");
        setQuickSwitchError(null);
      },
      [managedProviders],
    );
  
    const handleSelectQuickSwitchApiKey = useCallback((apiKeyId: string) => {
      setQuickSwitchApiKeyId(apiKeyId);
      setQuickSwitchError(null);
    }, []);
  
    const handleSubmitQuickSwitch = useCallback(async () => {
      if (!quickSwitchAccount) return;
      if (!selectedQuickSwitchProvider) {
        setQuickSwitchError(
          t("codex.quickSwitch.validation.providerRequired", "请选择供应商"),
        );
        return;
      }
      if (!selectedQuickSwitchApiKey) {
        setQuickSwitchError(
          t("codex.quickSwitch.validation.apiKeyRequired", "请选择 API Key"),
        );
        return;
      }
  
      setQuickSwitchSubmitting(true);
      setQuickSwitchError(null);
      try {
        await updateApiKeyCredentials(
          quickSwitchAccount.id,
          selectedQuickSwitchApiKey.apiKey,
          selectedQuickSwitchProvider.baseUrl,
          "custom",
          selectedQuickSwitchProvider.id,
          selectedQuickSwitchProvider.name,
          selectedQuickSwitchProvider.modelCatalog,
          selectedQuickSwitchProvider.supportsVision,
          Object.fromEntries(
            Object.entries(
              selectedQuickSwitchProvider.modelCapabilities ?? {},
            ).map(([model, capability]) => [
              model,
              capability.supportsVision === true,
            ]),
          ),
          selectedQuickSwitchProvider.visionRoutingModel,
          selectedQuickSwitchProvider.wireApi ?? undefined,
          selectedQuickSwitchProvider.supportsWebsockets,
          quickSwitchAccount.api_sync_model_catalog_to_codex === true,
          resolveCodexModelProviderAccountName(
            selectedQuickSwitchProvider.name,
            selectedQuickSwitchApiKey.name,
          ),
          selectedQuickSwitchProvider.modelContextWindows,
        );
        setMessage({
          text: t("codex.quickSwitch.success", {
            defaultValue: "已切换到供应商：{{provider}}",
            provider: selectedQuickSwitchProvider.name,
          }),
        });
        setApiKeyUsageMap((previous) => {
          const next = { ...previous };
          delete next[quickSwitchAccount.id];
          return next;
        });
        setQuickSwitchAccountId(null);
        setQuickSwitchProviderId("");
        setQuickSwitchApiKeyId("");
        setQuickSwitchError(null);
      } catch (err) {
        setQuickSwitchError(
          t("codex.quickSwitch.failed", {
            defaultValue: "切换供应商失败：{{error}}",
            error: String(err).replace(/^Error:\s*/, ""),
          }),
        );
      } finally {
        setQuickSwitchSubmitting(false);
      }
    }, [
      quickSwitchAccount,
      selectedQuickSwitchApiKey,
      selectedQuickSwitchProvider,
      setMessage,
      t,
      updateApiKeyCredentials,
    ]);
  
    const handleOpenProviderLink = useCallback(async (url: string) => {
      try {
        await openUrl(url);
      } catch {
        await navigator.clipboard.writeText(url).catch(() => {});
      }
    }, []);
  
    const handleApiKeyLogin = async () => {
      const validation = validateApiKeyCredentialInputs(
        apiKeyInput,
        apiBaseUrlInput,
      );
      if (!validation.ok) {
        page.setAddStatus("error");
        page.setAddMessage(validation.message);
        return;
      }
      if (apiSyncModelCatalogToCodex && apiModelCatalogDraft.length === 0) {
        setApiModelCatalogError(
          t(
            "codex.api.modelCatalog.syncRequiresModels",
            "同步到 Codex 前请先获取或填写模型列表。",
          ),
        );
        return;
      }
      const parsedWindows = parseContextWindowDrafts(
        apiModelContextWindowsInput,
        apiModelCatalogDraft,
      );
      if (!parsedWindows.ok) {
        setApiModelCatalogError(
          t(
            "codex.api.modelCatalog.contextWindowInvalid",
            "上下文窗口必须是大于 0 的整数",
          ),
        );
        return;
      }
      setApiModelCatalogError(null);
      const existingApiKeyAccount = accounts.find(
        (account) =>
          isCodexApiKeyAccount(account) &&
          account.openai_api_key?.trim() === validation.apiKey,
      );
      const providerPayload = {
        ...buildApiProviderPayload(
          apiBaseUrlInput,
          apiProviderPresetId,
          managedProviderId,
          newManagedProviderNameInput,
          selectedManagedProviderApiKey?.name,
        ),
        apiModelCatalog: apiModelCatalogDraft,
        apiModelContextWindows: parsedWindows.windows,
      };
  
      page.setAddStatus("loading");
      page.setAddMessage(t("common.shared.token.importing", "正在导入..."));
      try {
        let finalProviderPayload = providerPayload;
        if (
          validation.apiBaseUrl &&
          providerPayload.apiProviderMode === "custom" &&
          providerPayload.apiProviderId !== COCKPIT_API_PROVIDER_ID
        ) {
          try {
            const savedProvider = await upsertCodexModelProviderFromCredential({
              providerId: isRelayApiProviderTemplateId(
                providerPayload.apiProviderId,
              )
                ? null
                : (providerPayload.apiProviderId ?? null),
              previousProviderId: resolveManagedProviderIdForAccount(
                existingApiKeyAccount,
              ),
              providerName: providerPayload.apiProviderName ?? null,
              apiBaseUrl: validation.apiBaseUrl,
              apiKey: validation.apiKey,
              apiKeyName: providerPayload.accountName,
              sourceTag: providerPayload.sponsorTemplate?.id ?? null,
              modelCatalog: providerPayload.apiModelCatalog,
              modelContextWindows: providerPayload.apiModelContextWindows,
              supportsVision: providerPayload.sponsorTemplate?.supportsVision,
              website: providerPayload.sponsorTemplate?.website,
              apiKeyUrl: providerPayload.sponsorTemplate?.apiKeyUrl,
              wireApi: providerPayload.sponsorTemplate?.wireApi,
              integrationType:
                providerPayload.apiIntegrationType ??
                providerPayload.sponsorTemplate?.integrationType,
            });
            finalProviderPayload = {
              ...providerPayload,
              apiProviderId: savedProvider.id,
              apiProviderName: savedProvider.name,
              apiModelCatalog:
                savedProvider.modelCatalog ?? providerPayload.apiModelCatalog,
              apiSupportsVision: savedProvider.supportsVision,
              apiWireApi: savedProvider.wireApi ?? undefined,
              apiSupportsWebsockets: savedProvider.supportsWebsockets,
              accountName: providerPayload.accountName || savedProvider.name,
            };
            try {
              const usageSummary = await queryCodexModelProviderUsage({
                baseUrl: savedProvider.baseUrl,
                apiKey: validation.apiKey,
                integrationType: savedProvider.integrationType ?? null,
              });
              if (
                (usageSummary.mode === "sub2api" ||
                  usageSummary.mode === "new_api") &&
                usageSummary.mode !== savedProvider.integrationType
              ) {
                await saveCodexModelProviderDetectedIntegrationType(
                  savedProvider.id,
                  usageSummary.mode,
                );
              }
            } catch (usageErr) {
              console.warn("[CodexModelProviders] 额度类型探测失败", usageErr);
            }
            await reloadManagedProviders();
          } catch (providerErr) {
            console.warn(
              "[CodexModelProviders] 添加账号前写入供应商失败",
              providerErr,
            );
            throw providerErr;
          }
        }
        const account = await codexService.addCodexAccountWithApiKey(
          validation.apiKey,
          validation.apiBaseUrl,
          finalProviderPayload.apiProviderMode,
          finalProviderPayload.apiProviderId,
          finalProviderPayload.apiProviderName,
          finalProviderPayload.apiModelCatalog,
          finalProviderPayload.apiSupportsVision,
          finalProviderPayload.apiModelVisionSupport,
          finalProviderPayload.apiVisionRoutingModel,
          finalProviderPayload.accountName,
          finalProviderPayload.apiWireApi,
          finalProviderPayload.apiSupportsWebsockets,
          apiSyncModelCatalogToCodex,
          parsedWindows.windows,
        );
        await fetchAccounts();
        await fetchCurrentAccount();
        await assignCodexAccountsToTargetGroup([account]);
        await emitAccountsChanged({
          platformId: "codex",
          reason: "import",
        });
        try {
          await syncImportedAccountsToApiService([account.id]);
        } catch (error) {
          page.setAddStatus("error");
          page.setAddMessage(
            t(
              "codex.importApiService.syncFailed",
              "账号已导入，但加入 API 服务失败：{{error}}",
            ).replace("{{error}}", String(error).replace(/^Error:\s*/, "")),
          );
          return;
        }
        page.setAddStatus("success");
        page.setAddMessage(
          `${t("codex.import.successMsg", "导入成功: {{email}}").replace(
            "{{email}}",
            maskAccountText(account.email),
          )}${
            Object.keys(parsedWindows.windows).length > 0 ||
            apiSyncModelCatalogToCodex
              ? ` ${t(
                  "codex.api.modelCatalog.restartHint",
                  "模型目录已更新。若 Codex 正在运行，请重启后生效。",
                )}`
              : ""
          }`,
        );
        setApiKeyInput("");
        setApiBaseUrlInput(DEFAULT_CODEX_API_BASE_URL);
        setApiProviderPresetId(defaultApiProviderPresetId);
        setManagedProviderId("");
        setManagedProviderApiKeyId("");
        setNewManagedProviderNameInput("");
        setTimeout(() => {
          closeAddModal();
        }, 1200);
      } catch (e) {
        page.setAddStatus("error");
        page.setAddMessage(
          t("common.shared.token.importFailedMsg", "导入失败: {{error}}").replace(
            "{{error}}",
            String(e).replace(/^Error:\s*/, ""),
          ),
        );
      }
    };
  
    const performTokenImport = async (rawContent: string) => {
      const trimmed = rawContent.trim();
      if (!trimmed) {
        page.setAddStatus("error");
        page.setAddMessage(
          t("common.shared.token.empty", "请输入 Token 或 JSON"),
        );
        return;
      }
      const payloads = splitCodexImportPayloads(trimmed);
      if (payloads.length === 0) {
        page.setAddStatus("error");
        page.setAddMessage(
          t("common.shared.token.empty", "请输入 Token 或 JSON"),
        );
        return;
      }
  
      setImporting(true);
      setTokenImportProgress({ current: 0, total: payloads.length });
      page.setAddStatus("loading");
      try {
        const imported: CodexAccount[] = [];
        const failures: string[] = [];
        for (let index = 0; index < payloads.length; index += 1) {
          const current = index + 1;
          setTokenImportProgress({ current, total: payloads.length });
          page.setAddMessage(
            t("common.shared.externalImport.statusImporting", {
              current,
              total: payloads.length,
              defaultValue: "正在导入第 {{current}} / {{total}} 个账号",
            }),
          );
          try {
            imported.push(
              ...(await codexService.importCodexFromJson(payloads[index])),
            );
          } catch (error) {
            failures.push(
              `${current}: ${String(error).replace(/^Error:\s*/, "")}`,
            );
          }
        }
        if (imported.length === 0) {
          throw new Error(failures.join("; ") || "无法解析导入内容");
        }
        // 待授权账号若带 2FA 秘钥，同步写入本地 MFA 速查
        for (const account of imported) {
          const secret = account.two_factor_secret?.trim();
          if (!secret) continue;
          setSavedMfaRecords(
            upsertSavedMfaRecord({
              secret,
              accountName: account.email,
              remark: account.account_note,
            }),
          );
        }
        await fetchAccounts();
        await assignCodexAccountsToTargetGroup(imported);
        if (imported.length > 0) {
          await emitAccountsChanged({
            platformId: "codex",
            reason: "import",
          });
        }
        try {
          const accountIdsToSync = resolveImportedCodexAccountIdsForLocalAccess(
            imported,
            syncImportedToApiService,
            false,
          );
          const syncResult = await syncImportedAccountsToApiService(
            accountIdsToSync,
            false,
          );
          if (failures.length > 0) {
            page.setAddStatus("error");
            page.setAddMessage(
              t("codex.token.importPartial", {
                success: imported.length,
                failed: failures.length,
                errors: failures.slice(0, 3).join("; "),
                defaultValue:
                  "导入完成：成功 {{success}} 个，失败 {{failed}} 个。{{errors}}",
              }),
            );
            return;
          }
          page.setAddStatus("success");
          page.setAddMessage(
            t(
              "common.shared.token.importSuccessMsg",
              "成功导入 {{count}} 个账号",
            ).replace("{{count}}", String(imported.length)),
          );
          if (syncResult && syncResult.syncedAccountIds.length > 0) {
            closeAddModal();
          } else {
            setTimeout(() => {
              closeAddModal();
            }, 1200);
          }
        } catch (error) {
          page.setAddStatus("error");
          page.setAddMessage(
            t(
              "codex.importApiService.syncFailed",
              "账号已导入，但加入 API 服务失败：{{error}}",
            ).replace("{{error}}", String(error).replace(/^Error:\s*/, "")),
          );
        }
      } catch (e) {
        page.setAddStatus("error");
        page.setAddMessage(
          t("common.shared.token.importFailedMsg", "导入失败: {{error}}").replace(
            "{{error}}",
            String(e).replace(/^Error:\s*/, ""),
          ),
        );
      } finally {
        setImporting(false);
        setTokenImportProgress(null);
      }
    };
  
    const handleTokenImport = async () => {
      const trimmed = tokenInput.trim();
      if (!trimmed) {
        page.setAddStatus("error");
        page.setAddMessage(
          t("common.shared.token.empty", "请输入 Token 或 JSON"),
        );
        return;
      }
      const webSessions = findCodexWebSessionImports(trimmed);
      if (webSessions.length > 0) {
        page.setAddStatus("idle");
        page.setAddMessage("");
        setPendingWebSessionImport({
          content: trimmed,
          accountLabels: webSessions.map((item) => item.label),
        });
        return;
      }
      await performTokenImport(trimmed);
    };
  
    const clearInlineRename = useCallback(() => {
      setEditingApiKeyNameId(null);
      setEditingApiKeyNameValue("");
    }, []);
  
    const handleAccountNameDoubleClick = useCallback((account: CodexAccount) => {
      if (!isCodexApiKeyAccount(account)) return;
      inlineRenameDiscardRef.current = false;
      setEditingApiKeyNameId(account.id);
      setEditingApiKeyNameValue(
        (account.account_name || account.email || "").trim(),
      );
    }, []);
  
    const handleSubmitInlineRename = useCallback(
      async (account: CodexAccount) => {
        if (inlineRenameDiscardRef.current) {
          inlineRenameDiscardRef.current = false;
          return;
        }
        if (!isCodexApiKeyAccount(account)) return;
        if (editingApiKeyNameId !== account.id) return;
  
        const nextName = editingApiKeyNameValue.trim();
        const currentName = (account.account_name || "").trim();
        const fallbackName = (account.email || "").trim();
        const unchanged =
          nextName === currentName || (!currentName && nextName === fallbackName);
        if (unchanged) {
          clearInlineRename();
          return;
        }
  
        setSavingApiKeyNameId(account.id);
        try {
          await updateAccountName(account.id, nextName);
          setMessage({ text: t("codex.apiKey.renameSuccess", "已重命名") });
        } catch (e) {
          setMessage({
            text: `${t("codex.apiKey.renameFailed", "重命名失败")}: ${String(e)}`,
            tone: "error",
          });
        } finally {
          setSavingApiKeyNameId(null);
          clearInlineRename();
        }
      },
      [
        clearInlineRename,
        editingApiKeyNameId,
        editingApiKeyNameValue,
        setMessage,
        t,
        updateAccountName,
      ],
    );
  
    const toggleAccountApiKeyVisible = useCallback((accountId: string) => {
      setVisibleApiKeyAccountIds((prev) => {
        const next = new Set(prev);
        if (next.has(accountId)) {
          next.delete(accountId);
        } else {
          next.add(accountId);
        }
        return next;
      });
    }, []);
  
    const resolveApiKeyDisplayText = useCallback(
      (account: CodexAccount, visible: boolean) => {
        const apiKey = (account.openai_api_key || "").trim();
        if (!apiKey) return t("common.none", "暂无");
        return visible ? apiKey : maskCodexApiKey(apiKey);
      },
      [t],
    );
  
    const renderApiKeyRevealLine = useCallback(
      (account: CodexAccount): ReactElement => {
        const visible = visibleApiKeyAccountIds.has(account.id);
        const label = t("codex.addModal.token", "API Key");
        const value = resolveApiKeyDisplayText(account, visible);
        const line = `${label}：${value}`;
        const actionLabel = visible
          ? t("codex.api.hideApiKey", "隐藏 API Key")
          : t("codex.api.showApiKey", "显示 API Key");
        return (
          <button
            type="button"
            className="codex-api-key-reveal-line"
            onClick={() => toggleAccountApiKeyVisible(account.id)}
            title={
              visible
                ? line
                : t("codex.api.apiKeyHiddenHint", "API Key 已隐藏，点击显示")
            }
            aria-label={actionLabel}
          >
            <span className="codex-login-subline">{line}</span>
            {visible ? <EyeOff size={12} /> : <Eye size={12} />}
          </button>
        );
      },
      [
        resolveApiKeyDisplayText,
        t,
        toggleAccountApiKeyVisible,
        visibleApiKeyAccountIds,
      ],
    );
  
    const renderOAuthBindingLine = useCallback(
      (account: CodexAccount): ReactElement => {
        const boundAccount = resolveBoundOAuthAccount(account);
        const label = t("codex.api.oauthBinding.label", "OAuth 绑定");
        const value = boundAccount
          ? maskAccountText(
              boundAccount.account_name || boundAccount.email || boundAccount.id,
            )
          : t("codex.api.oauthBinding.unbound", "未绑定");
        const line = `${label}：${value}`;
        const boundOAuthNeedsReauth = Boolean(boundAccount?.requires_reauth);
        const boundOAuthIssueText =
          boundAccount?.reauth_reason?.trim() ||
          t(
            "codex.switchAuth.reauthorizeDescription",
            "当前登录凭据无法自动更新，请重新授权后继续使用。",
          );
        return (
          <div className="account-sub-line codex-provider-inline-line codex-oauth-binding-line">
            <span
              className="codex-login-subline codex-provider-inline-text"
              title={line}
            >
              {line}
            </span>
            {boundOAuthNeedsReauth && (
              <span
                className="codex-status-pill quota-error"
                title={boundOAuthIssueText}
              >
                <CircleAlert size={12} />
                {t("codex.authError.badge", "授权异常")}
              </span>
            )}
            {boundOAuthNeedsReauth && boundAccount && (
              <button
                type="button"
                className="codex-provider-inline-switch codex-oauth-binding-action"
                onClick={() => openCodexAddModal("oauth", boundAccount)}
                title={t("common.reauthorize", "重新授权")}
              >
                <RefreshCw size={11} />
                {t("common.reauthorize", "重新授权")}
              </button>
            )}
            <button
              type="button"
              className="codex-provider-inline-switch codex-oauth-binding-action"
              onClick={() => openOAuthBindingModal(account)}
              title={t("codex.api.oauthBinding.action", "绑定 OAuth")}
            >
              <Link2 size={11} />
              {t("codex.api.oauthBinding.actionShort", "绑定")}
            </button>
          </div>
        );
      },
      [
        maskAccountText,
        openCodexAddModal,
        openOAuthBindingModal,
        resolveBoundOAuthAccount,
        t,
      ],
    );
  
    const resolveApiProviderDisplayName = useCallback(
      (account: CodexAccount): string => {
        const providerMode = inferCodexAccountProviderMode(account);
        if (providerMode === "openai_builtin") {
          const fallback = findCodexApiProviderPresetById(
            OPENAI_OFFICIAL_PRESET_ID,
          );
          return fallback
            ? t(`codex.api.providers.${fallback.id}.name`, fallback.name)
            : t("common.none", "暂无");
        }
        if (account.api_provider_name?.trim()) {
          return account.api_provider_name.trim();
        }
        const baseUrl = (account.api_base_url || "").trim();
        const matchedProvider = findCodexModelProviderByBaseUrl(
          managedProviders,
          baseUrl,
        );
        if (matchedProvider) return matchedProvider.name;
        const preset = findCodexApiProviderPresetById(
          resolveCodexApiProviderPresetId(baseUrl),
        );
        if (preset)
          return t(`codex.api.providers.${preset.id}.name`, preset.name);
        return t("codex.api.provider.custom", "自定义");
      },
      [managedProviders, t],
    );
  
    const resolveUsageProviderForApiKeyAccount = useCallback(
      (account: CodexAccount): CodexModelProvider | null => {
        if (!isCodexApiKeyAccount(account) || isCodexNewApiAccount(account)) {
          return null;
        }
        const provider =
          findCodexModelProviderById(managedProviders, account.api_provider_id) ??
          findCodexModelProviderByBaseUrl(
            managedProviders,
            (account.api_base_url || "").trim(),
          );
        return provider ?? null;
      },
      [managedProviders],
    );
  
    const refreshApiKeyUsage = useCallback(
      async (account: CodexAccount, provider?: CodexModelProvider | null) => {
        if (
          isCodexChatCompletionsApiKeyAccount(account) &&
          !isDeepSeekAccount(account) &&
          !isCodexTokenPlanAccount(account)
        ) {
          return;
        }
        const targetProvider =
          provider ?? resolveUsageProviderForApiKeyAccount(account);
        const apiKey = (account.openai_api_key || "").trim();
        const baseUrl =
          targetProvider?.baseUrl.trim() || (account.api_base_url || "").trim();
        if (!baseUrl || !apiKey) return;
        if (apiKeyUsageInFlightRef.current.has(account.id)) {
          return;
        }
        apiKeyUsageInFlightRef.current.add(account.id);
        setApiKeyUsageMap((previous) => ({
          ...previous,
          [account.id]: {
            ...previous[account.id],
            loading: true,
            error: undefined,
            unavailable: false,
          },
        }));
        try {
          const summary = await queryCodexModelProviderUsage({
            baseUrl,
            apiKey,
            integrationType: targetProvider?.integrationType ?? null,
          });
          const updatedAt = Date.now();
          if (
            targetProvider &&
            (summary.mode === "sub2api" || summary.mode === "new_api") &&
            summary.mode !== targetProvider.integrationType
          ) {
            await saveCodexModelProviderDetectedIntegrationType(
              targetProvider.id,
              summary.mode,
            );
            await reloadManagedProviders();
          }
          setApiKeyUsageMap((previous) => ({
            ...previous,
            [account.id]: { loading: false, summary, updatedAt },
          }));
        } catch (error) {
          const updatedAt = Date.now();
          setApiKeyUsageMap((previous) => ({
            ...previous,
            [account.id]: {
              loading: false,
              summary: previous[account.id]?.summary,
              error: isModelProviderUsageUnavailableError(error)
                ? undefined
                : String(error).replace(/^Error:\s*/, ""),
              unavailable: isModelProviderUsageUnavailableError(error),
              updatedAt,
            },
          }));
        } finally {
          apiKeyUsageInFlightRef.current.delete(account.id);
        }
      },
      [reloadManagedProviders, resolveUsageProviderForApiKeyAccount],
    );
  
    const canRefreshApiKeyUsage = useCallback(
      (account: CodexAccount, provider?: CodexModelProvider | null): boolean => {
        if (
          !isCodexApiKeyAccount(account) ||
          isCodexNewApiAccount(account) ||
          (isCodexChatCompletionsApiKeyAccount(account) &&
            !isDeepSeekAccount(account) &&
            !isCodexTokenPlanAccount(account))
        ) {
          return false;
        }
        const targetProvider =
          provider ?? resolveUsageProviderForApiKeyAccount(account);
        const apiKey = (account.openai_api_key || "").trim();
        const baseUrl =
          targetProvider?.baseUrl.trim() || (account.api_base_url || "").trim();
        return Boolean(apiKey && baseUrl);
      },
      [resolveUsageProviderForApiKeyAccount],
    );
  
    const shouldAutoRefreshApiKeyUsage = useCallback(
      (account: CodexAccount, provider?: CodexModelProvider | null): boolean => {
        if (!canRefreshApiKeyUsage(account, provider)) {
          return false;
        }
        const state = apiKeyUsageMap[account.id];
        if (state?.loading || apiKeyUsageInFlightRef.current.has(account.id)) {
          return false;
        }
        if (state?.unavailable) {
          return (
            isDeepSeekAccount(account) &&
            !state.summary &&
            !deepSeekUsageRetryIdsRef.current.has(account.id)
          );
        }
        return !state?.updatedAt;
      },
      [apiKeyUsageMap, canRefreshApiKeyUsage],
    );
  
    const refreshApiKeyUsageByAccountId = useCallback(
      async (accountId: string, options?: { force?: boolean }) => {
        const account = accounts.find((item) => item.id === accountId);
        if (!account) return;
        const provider = resolveUsageProviderForApiKeyAccount(account);
        if (
          options?.force === false &&
          !shouldAutoRefreshApiKeyUsage(account, provider)
        ) {
          return;
        }
        await refreshApiKeyUsage(account, provider);
      },
      [
        accounts,
        refreshApiKeyUsage,
        resolveUsageProviderForApiKeyAccount,
        shouldAutoRefreshApiKeyUsage,
      ],
    );
  
    useEffect(() => {
      writeCodexApiKeyUsageCache(apiKeyUsageMap);
      for (const [accountId, state] of Object.entries(apiKeyUsageMap)) {
        if (
          state.loading ||
          state.error ||
          state.unavailable ||
          !state.summary ||
          !state.updatedAt
        ) {
          continue;
        }
        void syncModelProviderUsageToCodexAccount({
          accountId,
          summary: state.summary,
          updatedAtMs: state.updatedAt,
        }).catch((error) => {
          console.warn("[CodexApiKeyUsage] 同步 API 服务余额快照失败", error);
        });
      }
    }, [apiKeyUsageMap]);
  
    useEffect(() => {
      for (const account of accounts) {
        const provider = resolveUsageProviderForApiKeyAccount(account);
        if (!shouldAutoRefreshApiKeyUsage(account, provider)) continue;
        if (isDeepSeekAccount(account)) {
          deepSeekUsageRetryIdsRef.current.add(account.id);
        }
        void refreshApiKeyUsage(account, provider);
      }
    }, [
      accounts,
      refreshApiKeyUsage,
      resolveUsageProviderForApiKeyAccount,
      shouldAutoRefreshApiKeyUsage,
    ]);
  
    useEffect(() => {
      const syncUsageCache = () => setApiKeyUsageMap(readCodexApiKeyUsageCache());
      window.addEventListener(
        CODEX_API_KEY_USAGE_REFRESHED_EVENT,
        syncUsageCache,
      );
      return () =>
        window.removeEventListener(
          CODEX_API_KEY_USAGE_REFRESHED_EVENT,
          syncUsageCache,
        );
    }, []);
  
    useEffect(() => {
      const accountIds = new Set(accounts.map((account) => account.id));
      const chatCompletionsAccountIds = new Set(
        accounts
          .filter(
            (account) =>
              isCodexChatCompletionsApiKeyAccount(account) &&
              !isDeepSeekAccount(account) &&
              !isCodexTokenPlanAccount(account),
          )
          .map((account) => account.id),
      );
      setApiKeyUsageMap((previous) => {
        let changed = false;
        const next: Record<string, CodexApiKeyUsageState> = {};
        for (const [accountId, state] of Object.entries(previous)) {
          if (
            accountIds.has(accountId) &&
            !chatCompletionsAccountIds.has(accountId)
          ) {
            next[accountId] = state;
          } else {
            changed = true;
          }
        }
        return changed ? next : previous;
      });
    }, [accounts]);
  
    useEffect(() => {
      let unlistenAccountsChanged: UnlistenFn | null = null;
      let unlistenCurrentChanged: UnlistenFn | null = null;
  
      void listen("accounts:changed", async (event) => {
        const payload = event.payload as {
          platformId?: string;
          accountId?: string | null;
          reason?: string;
        } | null;
        if (payload?.platformId !== "codex") return;
        if (payload.reason === "delete") return;
        if (
          payload.reason === "client-auth-observation" ||
          payload.reason === "client-auth-launch"
        ) {
          // CDP 已完成连续确认并落盘，立即回读权威账号快照，避免卡片等待轮询。
          await Promise.all([fetchAccounts(), fetchCurrentAccount()]);
          return;
        }
        if (payload.accountId) {
          await refreshApiKeyUsageByAccountId(payload.accountId, {
            force: false,
          });
          return;
        }
      }).then((fn) => {
        unlistenAccountsChanged = fn;
      });
  
      void listen("accounts:current-changed", async (event) => {
        const payload = event.payload as {
          platformId?: string;
          accountId?: string | null;
          reason?: string;
        } | null;
        if (payload?.platformId !== "codex") return;
        if (payload.reason === "delete") return;
        if (payload.accountId) {
          await refreshApiKeyUsageByAccountId(payload.accountId, {
            force: false,
          });
        }
      }).then((fn) => {
        unlistenCurrentChanged = fn;
      });
  
      return () => {
        unlistenAccountsChanged?.();
        unlistenCurrentChanged?.();
      };
    }, [fetchAccounts, fetchCurrentAccount, refreshApiKeyUsageByAccountId]);
  
    const formatApiKeyUsageMoney = useCallback(
      (value?: number | null, unit?: string | null): string =>
        formatModelProviderUsageMoney(value ?? undefined, unit ?? undefined),
      [],
    );
  
    const formatApiKeyUsageQuotaValue = useCallback(
      (
        summary: CodexModelProviderUsageSummary | undefined,
        value?: number | null,
      ): string => {
        if (summary?.quotaUnlimited === true) {
          return t("codex.modelProviders.usage.unlimitedQuota", "无限额度");
        }
        return formatApiKeyUsageMoney(value, summary?.unit);
      },
      [formatApiKeyUsageMoney, t],
    );
  
    const resolveCockpitApiAccountBalanceText = useCallback(
      (account: CodexAccount): string | null => {
        const usage = getCockpitApiUsageRecord(account);
        const stats = getCockpitApiStatsRecord(account);
        const total = toCockpitApiRecord(stats?.total);
        const profile = toCockpitApiRecord(
          toCockpitApiRecord(account.quota?.raw_data)?.profile,
        );
        const records = [usage, total, profile].filter(
          (record): record is CockpitApiJsonRecord => Boolean(record),
        );
        const displayKeys = [
          "balance_display",
          "account_balance_display",
          "wallet_balance_display",
        ];
        for (const record of records) {
          for (const key of displayKeys) {
            const value = readCockpitApiString(record, key);
            if (value) return value;
          }
        }
        const numberKeys = ["balance", "account_balance", "wallet_balance"];
        for (const record of records) {
          for (const key of numberKeys) {
            const value = readCockpitApiOptionalNumber(record, key);
            if (value != null) return formatApiKeyUsageMoney(value, "USD");
          }
        }
        return null;
      },
      [formatApiKeyUsageMoney],
    );
  
    const formatApiKeyUsagePercent = useCallback(
      (summary?: CodexModelProviderUsageSummary): number => {
        if (summary?.mode === "new_api") {
          const { granted, available } = resolveNewApiQuotaSnapshot(summary);
          if (granted != null && available != null && granted > 0) {
            return Math.max(
              0,
              Math.min(100, Math.round(((granted - available) / granted) * 100)),
            );
          }
        }
        const used = summary?.quotaUsed ?? summary?.totalCost;
        const limit = summary?.quotaLimit;
        if (
          typeof used !== "number" ||
          typeof limit !== "number" ||
          !Number.isFinite(used) ||
          !Number.isFinite(limit) ||
          limit <= 0
        ) {
          return 0;
        }
        return Math.max(0, Math.min(100, Math.round((used / limit) * 100)));
      },
      [],
    );
  
    const formatApiKeyUsageDetailLabel = useCallback(
      (key: string, fallback: string): string => {
        const labels: Record<string, string> = {
          modelName: t("codex.modelProviders.usage.fields.modelName", "Model"),
          intervalRemaining: t(
            "codex.modelProviders.usage.fields.intervalRemaining",
            "Interval Remaining",
          ),
          intervalLimit: t(
            "codex.modelProviders.usage.fields.intervalLimit",
            "Interval Limit",
          ),
          intervalRemainingPercent: t(
            "codex.modelProviders.usage.fields.intervalRemainingPercent",
            "Interval Remaining %",
          ),
          intervalExpiresAt: t(
            "codex.modelProviders.usage.fields.intervalExpiresAt",
            "Interval Reset",
          ),
          weeklyRemaining: t(
            "codex.modelProviders.usage.fields.weeklyRemaining",
            "Weekly Remaining",
          ),
          weeklyLimit: t(
            "codex.modelProviders.usage.fields.weeklyLimit",
            "Weekly Limit",
          ),
          weeklyRemainingPercent: t(
            "codex.modelProviders.usage.fields.weeklyRemainingPercent",
            "Weekly Remaining %",
          ),
          weeklyExpiresAt: t(
            "codex.modelProviders.usage.fields.weeklyExpiresAt",
            "Weekly Reset",
          ),
          status: t("codex.modelProviders.usage.fields.status", "状态"),
          planName: t("codex.modelProviders.usage.fields.planName", "订阅"),
          remaining: t("codex.modelProviders.usage.fields.remaining", "剩余额度"),
          balance: t("codex.modelProviders.usage.fields.balance", "余额"),
          quotaUnlimited: t(
            "codex.modelProviders.usage.fields.quotaUnlimited",
            "无限额度",
          ),
          todayRequests: t(
            "codex.modelProviders.usage.fields.todayRequests",
            "今日请求",
          ),
          todayTokens: t(
            "codex.modelProviders.usage.fields.todayTokens",
            "今日 Token",
          ),
          todayCost: t("codex.modelProviders.usage.fields.todayCost", "今日消耗"),
          totalRequests: t(
            "codex.modelProviders.usage.fields.totalRequests",
            "累计请求",
          ),
          totalTokens: t(
            "codex.modelProviders.usage.fields.totalTokens",
            "累计 Token",
          ),
          totalCost: t("codex.modelProviders.usage.fields.totalCost", "累计消耗"),
          hardLimitUsd: t(
            "codex.modelProviders.usage.fields.hardLimitUsd",
            "硬额度",
          ),
          softLimitUsd: t(
            "codex.modelProviders.usage.fields.softLimitUsd",
            "软额度",
          ),
          systemHardLimitUsd: t(
            "codex.modelProviders.usage.fields.systemHardLimitUsd",
            "系统额度",
          ),
          accessUntil: t(
            "codex.modelProviders.usage.fields.accessUntil",
            "可用至",
          ),
          expiresAt: t("codex.modelProviders.usage.fields.expiresAt", "过期时间"),
          totalGranted: t(
            "codex.modelProviders.usage.fields.totalGranted",
            "授予额度",
          ),
          totalAvailable: t(
            "codex.modelProviders.usage.fields.totalAvailable",
            "可用额度",
          ),
          modelLimitsEnabled: t(
            "codex.modelProviders.usage.fields.modelLimitsEnabled",
            "模型限制",
          ),
          totalUsage: t(
            "codex.modelProviders.usage.fields.totalUsage",
            "累计消耗",
          ),
          isAvailable: t(
            "codex.modelProviders.usage.fields.isAvailable",
            "余额可用",
          ),
          currency: t("codex.modelProviders.usage.fields.currency", "币种"),
          totalBalance: t(
            "codex.modelProviders.usage.fields.totalBalance",
            "总余额",
          ),
          apiKeyBalance: t(
            "codex.modelProviders.usage.fields.apiKeyBalance",
            "API_KEY 余额",
          ),
          grantedBalance: t(
            "codex.modelProviders.usage.fields.grantedBalance",
            "赠金余额",
          ),
          toppedUpBalance: t(
            "codex.modelProviders.usage.fields.toppedUpBalance",
            "充值余额",
          ),
          fiveHourRemainingPercent: t(
            "codex.modelProviders.usage.fields.fiveHourRemainingPercent",
            "5h 剩余",
          ),
          accountCount: t(
            "codex.modelProviders.usage.fields.accountCount",
            "账号池",
          ),
          availableAccountCount: t(
            "codex.modelProviders.usage.fields.availableAccountCount",
            "可用账号",
          ),
          abnormalAccountCount: t(
            "codex.modelProviders.usage.fields.abnormalAccountCount",
            "异常账号",
          ),
          cooldownAccountCount: t(
            "codex.modelProviders.usage.fields.cooldownAccountCount",
            "冷却账号",
          ),
        };
        return labels[key] ?? fallback;
      },
      [t],
    );
  
    const formatApiKeyUsageDetailValue = useCallback(
      (item: { key: string; value: string }, unit?: string | null): string => {
        const raw = item.value.trim();
        const numeric = Number(raw);
        if (
          Number.isFinite(numeric) &&
          (item.key.includes("Tokens") ||
            item.key === "todayTokens" ||
            item.key === "totalTokens")
        ) {
          return formatCockpitApiTokenCount(numeric);
        }
        if (Number.isFinite(numeric) && item.key === "accessUntil") {
          return numeric > 0 ? formatDate(numeric * 1000) : "-";
        }
        if (Number.isFinite(numeric) && item.key === "expiresAt") {
          return numeric > 0 ? formatDate(numeric * 1000) : "-";
        }
        if (
          Number.isFinite(numeric) &&
          (item.key === "intervalExpiresAt" || item.key === "weeklyExpiresAt")
        ) {
          return numeric > 0 ? formatDate(numeric * 1000) : "-";
        }
        if (
          item.key === "quotaUnlimited" ||
          item.key === "modelLimitsEnabled" ||
          item.key === "isAvailable"
        ) {
          if (raw === "true")
            return t("codex.modelProviders.usage.booleanTrue", "是");
          if (raw === "false")
            return t("codex.modelProviders.usage.booleanFalse", "否");
        }
        if (Number.isFinite(numeric) && item.key === "apiKeyBalance") {
          return formatCockpitToolsApiKeyBalance(numeric);
        }
        if (
          Number.isFinite(numeric) &&
          [
            "remaining",
            "balance",
            "todayCost",
            "totalCost",
            "hardLimitUsd",
            "softLimitUsd",
            "systemHardLimitUsd",
            "totalBalance",
            "grantedBalance",
            "toppedUpBalance",
          ].includes(item.key)
        ) {
          return formatApiKeyUsageMoney(numeric, unit);
        }
        if (
          Number.isFinite(numeric) &&
          ["totalGranted", "totalAvailable"].includes(item.key)
        ) {
          return formatCockpitApiInteger(numeric);
        }
        if (Number.isFinite(numeric) && item.key === "totalUsage") {
          return formatApiKeyUsageMoney(numeric / 100, unit);
        }
        if (
          Number.isFinite(numeric) &&
          (item.key.includes("Requests") ||
            item.key === "todayRequests" ||
            item.key === "totalRequests")
        ) {
          return formatCockpitApiInteger(numeric);
        }
        return raw || "-";
      },
      [formatApiKeyUsageMoney, t],
    );
  
    const findApiKeyUsageDetail = useCallback(
      (summary: CodexModelProviderUsageSummary | undefined, key: string) =>
        summary?.details?.find((item) => item.key === key),
      [],
    );
  
    const formatApiKeyUsageDetailByKey = useCallback(
      (
        summary: CodexModelProviderUsageSummary | undefined,
        key: string,
      ): string => {
        const detail = findApiKeyUsageDetail(summary, key);
        if (!detail) return "-";
        return formatApiKeyUsageDetailValue(detail, summary?.unit);
      },
      [findApiKeyUsageDetail, formatApiKeyUsageDetailValue],
    );

    const formatCockpitToolsPercent = useCallback(
      (
        summary: CodexModelProviderUsageSummary | undefined,
        key: string,
      ): string => {
        const value = formatApiKeyUsageDetailByKey(summary, key);
        return value === "-" ? value : `${value}%`;
      },
      [formatApiKeyUsageDetailByKey],
    );
  
    const renderApiKeyUsagePanel = useCallback(
      (
        account: CodexAccount,
        provider: CodexModelProvider | null,
        variant: "card" | "table" = "card",
      ): ReactElement => {
        if (
          isCodexChatCompletionsApiKeyAccount(account) &&
          !isDeepSeekAccount(account) &&
          !isCodexTokenPlanAccount(account)
        ) {
          return <></>;
        }
        const usageState = apiKeyUsageMap[account.id];
        const summary = usageState?.summary;
        const loading = usageState?.loading === true;
        const apiKey = (account.openai_api_key || "").trim();
        const baseUrl =
          provider?.baseUrl.trim() || (account.api_base_url || "").trim();
        const canRefresh = Boolean(apiKey && baseUrl);
        const usageMode = resolveApiKeyUsageMode(summary);
        const isDeepSeekUsage =
          isDeepSeekAccount(account) || usageMode === "deepseek";
        const isNewApiUsage = usageMode === "new_api";
        const isSub2ApiUsage = usageMode === "sub2api";
        const isTokenPlanUsage = usageMode === "token_plan";
        const usedPercent = formatApiKeyUsagePercent(summary);
        if (isDeepSeekUsage) {
          return (
            <div className={`codex-api-key-usage-panel ${variant} sub2api`}>
              <div className="codex-api-key-usage-grid">
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.totalBalance",
                      "总余额",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageMoney(summary?.balance, summary?.unit)}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.grantedBalance",
                      "赠金余额",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(summary, "grantedBalance")}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.toppedUpBalance",
                      "充值余额",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(summary, "toppedUpBalance")}
                  </strong>
                </div>
              </div>
              {!summary && usageState?.error ? (
                <div className="codex-api-key-usage-empty">
                  {t("common.shared.quota.queryFailed", "配额查询失败")}
                </div>
              ) : null}
            </div>
          );
        }
        if (summary && usageMode === "cockpit_tools") {
          return (
            <div className={`codex-api-key-usage-panel ${variant} cockpit-tools`}>
              <div className="codex-api-key-usage-grid">
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.fiveHourRemainingPercent",
                      "5h 剩余",
                    )}
                  </span>
                  <strong>
                    {formatCockpitToolsPercent(
                      summary,
                      "fiveHourRemainingPercent",
                    )}
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
                    {formatCockpitToolsPercent(
                      summary,
                      "weeklyRemainingPercent",
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
                    {formatApiKeyUsageDetailByKey(summary, "apiKeyBalance")}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.accountCount",
                      "账号池",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(summary, "accountCount")}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.availableAccountCount",
                      "可用账号",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(
                      summary,
                      "availableAccountCount",
                    )}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.abnormalAccountCount",
                      "异常 / 冷却",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(
                      summary,
                      "abnormalAccountCount",
                    )}{" "}
                    /{" "}
                    {formatApiKeyUsageDetailByKey(
                      summary,
                      "cooldownAccountCount",
                    )}
                  </strong>
                </div>
              </div>
              <span className="quota-reset">
                {summary.planName || "Cockpit Tools"}
                {summary.status === "stale"
                  ? ` · ${t("codex.modelProviders.usage.fields.stale", "数据较旧")}`
                  : ""}
              </span>
            </div>
          );
        }
        if (variant === "card" && summary && isNewApiUsage) {
          const quota = resolveNewApiQuotaSnapshot(summary);
          const grantedText = formatApiKeyUsageMoney(quota.granted, summary.unit);
          const availableText = formatApiKeyUsageMoney(
            quota.available,
            summary.unit,
          );
          const expiresText =
            quota.expiresAt != null
              ? formatApiKeyUsageDetailValue({
                  key: "expiresAt",
                  value: String(quota.expiresAt),
                })
              : "-";
          const unlimitedText = t("codex.newApi.quota.unlimited", "不限量");
          const quotaValueText =
            summary.quotaUnlimited === true
              ? unlimitedText
              : `${availableText} / ${grantedText}`;
          const quotaBarWidth =
            summary.quotaUnlimited === true ? 100 : usedPercent;
          return (
            <div
              className="quota-item codex-api-key-quota-item new-api"
              title={`${t("codex.cockpitApi.balance", "额度")}：${quotaValueText}`}
            >
              <div className="quota-header">
                <Database size={14} />
                <span className="quota-label">
                  {t("codex.cockpitApi.balance", "额度")}
                </span>
                <span className="quota-pct high">{quotaValueText}</span>
              </div>
              <div className="quota-bar-track">
                <div
                  className="quota-bar high"
                  style={{ width: `${quotaBarWidth}%` }}
                />
              </div>
              {expiresText !== "-" && (
                <span className="quota-reset">
                  {t("codex.modelProviders.usage.fields.expiresAt", "过期时间")}：
                  {expiresText}
                </span>
              )}
            </div>
          );
        }
        if (variant === "card" && summary && isTokenPlanUsage) {
          const resetDetail =
            findApiKeyUsageDetail(summary, "intervalExpiresAt") ??
            findApiKeyUsageDetail(summary, "weeklyExpiresAt") ??
            findApiKeyUsageDetail(summary, "expiresAt");
          return (
            <div
              className="quota-item codex-api-key-quota-item token-plan"
              title={`${t(
                "codex.modelProviders.usage.fields.remaining",
                "Remaining",
              )}: ${formatApiKeyUsageQuotaValue(
                summary,
                summary.quotaRemaining ?? summary.remaining,
              )}`}
            >
              <div className="quota-header">
                <Database size={14} />
                <span className="quota-label">
                  {t("codex.modelProviders.usage.fields.planName", "Token Plan")}
                </span>
                <span className="quota-pct high">
                  {formatApiKeyUsageQuotaValue(
                    summary,
                    summary.quotaRemaining ?? summary.remaining,
                  )}
                </span>
              </div>
              <div className="quota-bar-track">
                <div
                  className="quota-bar high"
                  style={{ width: `${Math.max(0, Math.min(100, usedPercent))}%` }}
                />
              </div>
              {(summary.planName || resetDetail) && (
                <span className="quota-reset">
                  {summary.planName || "Token Plan"}
                  {resetDetail
                    ? ` · ${formatApiKeyUsageDetailValue(resetDetail, summary.unit)}`
                    : ""}
                </span>
              )}
            </div>
          );
        }
        if (variant === "card" && summary && isSub2ApiUsage) {
          return (
            <div className="codex-api-key-usage-panel sub2api">
              <div className="codex-api-key-usage-grid">
                <div>
                  <span>
                    {t("codex.modelProviders.usage.accountBalance", "账户余额")}
                  </span>
                  <strong>
                    {formatApiKeyUsageQuotaValue(
                      summary,
                      summary.remaining ??
                        summary.balance ??
                        summary.quotaRemaining,
                    )}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.todayRequests",
                      "今日请求",
                    )}
                  </span>
                  <strong>
                    {formatCockpitApiInteger(summary.todayRequests ?? 0)}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.todayTokens",
                      "今日 Token",
                    )}
                  </span>
                  <strong>
                    {formatCockpitApiTokenCount(summary.todayTotalTokens ?? 0)}
                  </strong>
                </div>
              </div>
            </div>
          );
        }
        if (summary && !usageMode) {
          return <></>;
        }
        return (
          <div
            className={`codex-api-key-usage-panel ${variant} ${summary ? "" : "empty"}`}
          >
            {summary ? (
              <>
                <div className="codex-api-key-usage-grid">
                  {isDeepSeekUsage ? (
                    <>
                      {[
                        ["totalBalance", "总余额"],
                        ["grantedBalance", "赠金余额"],
                        ["toppedUpBalance", "充值余额"],
                      ].map(([key, fallback]) => (
                        <div key={key}>
                          <span>
                            {formatApiKeyUsageDetailLabel(key, fallback)}
                          </span>
                          <strong>
                            {formatApiKeyUsageDetailByKey(summary, key)}
                          </strong>
                        </div>
                      ))}
                    </>
                  ) : isNewApiUsage ? (
                    <>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.totalGranted",
                            "授予额度",
                          )}
                        </span>
                        <strong>
                          {(() => {
                            const raw = Number(
                              findApiKeyUsageDetail(summary, "totalGranted")
                                ?.value ?? NaN,
                            );
                            return Number.isFinite(raw)
                              ? formatApiKeyUsageMoney(raw, summary.unit)
                              : formatApiKeyUsageDetailByKey(
                                  summary,
                                  "totalGranted",
                                );
                          })()}
                        </strong>
                      </div>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.totalAvailable",
                            "可用额度",
                          )}
                        </span>
                        <strong>
                          {(() => {
                            const raw = Number(
                              findApiKeyUsageDetail(summary, "totalAvailable")
                                ?.value ?? NaN,
                            );
                            return Number.isFinite(raw)
                              ? formatApiKeyUsageMoney(raw, summary.unit)
                              : formatApiKeyUsageDetailByKey(
                                  summary,
                                  "totalAvailable",
                                );
                          })()}
                        </strong>
                      </div>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.expiresAt",
                            "过期时间",
                          )}
                        </span>
                        <strong>
                          {formatApiKeyUsageDetailByKey(summary, "expiresAt")}
                        </strong>
                      </div>
                    </>
                  ) : isTokenPlanUsage ? (
                    <>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.remaining",
                            "Remaining",
                          )}
                        </span>
                        <strong>
                          {formatApiKeyUsageQuotaValue(
                            summary,
                            summary.quotaRemaining ?? summary.remaining,
                          )}
                        </strong>
                      </div>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.planName",
                            "Plan",
                          )}
                        </span>
                        <strong>{summary.planName || "-"}</strong>
                      </div>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.expiresAt",
                            "Next Reset",
                          )}
                        </span>
                        <strong>
                          {formatApiKeyUsageDetailByKey(
                            summary,
                            findApiKeyUsageDetail(summary, "intervalExpiresAt")
                              ? "intervalExpiresAt"
                              : findApiKeyUsageDetail(summary, "weeklyExpiresAt")
                                ? "weeklyExpiresAt"
                                : "expiresAt",
                          )}
                        </strong>
                      </div>
                    </>
                  ) : isSub2ApiUsage ? (
                    <>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.accountBalance",
                            "账户余额",
                          )}
                        </span>
                        <strong>
                          {formatApiKeyUsageQuotaValue(
                            summary,
                            summary.remaining ??
                              summary.balance ??
                              summary.quotaRemaining,
                          )}
                        </strong>
                      </div>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.todayRequests",
                            "今日请求",
                          )}
                        </span>
                        <strong>
                          {formatCockpitApiInteger(summary.todayRequests ?? 0)}
                        </strong>
                      </div>
                      <div>
                        <span>
                          {t(
                            "codex.modelProviders.usage.fields.todayTokens",
                            "今日 Token",
                          )}
                        </span>
                        <strong>
                          {formatCockpitApiTokenCount(
                            summary.todayTotalTokens ?? 0,
                          )}
                        </strong>
                      </div>
                    </>
                  ) : null}
                </div>
                {isNewApiUsage || isTokenPlanUsage ? (
                  <div className="codex-api-key-usage-progress">
                    <div className="cockpit-api-progress-track">
                      <div
                        className="cockpit-api-progress-bar"
                        style={{ width: `${usedPercent}%` }}
                      />
                    </div>
                    <span>{usedPercent}%</span>
                  </div>
                ) : null}
              </>
            ) : (
              <div className="codex-api-key-usage-empty">
                {loading
                  ? t("codex.modelProviders.usage.loading", "正在查询额度...")
                  : usageState?.error
                    ? null
                    : canRefresh
                      ? t("codex.modelProviders.usage.pending", "等待查询额度")
                      : t("codex.modelProviders.usage.noKey", "暂无可查询额度")}
              </div>
            )}
          </div>
        );
      },
      [
        apiKeyUsageMap,
        formatApiKeyUsagePercent,
        formatApiKeyUsageMoney,
        formatApiKeyUsageQuotaValue,
        formatApiKeyUsageDetailLabel,
        formatApiKeyUsageDetailValue,
        formatApiKeyUsageDetailByKey,
        formatCockpitToolsPercent,
        t,
      ],
    );
  
    const closeApiKeyCredentialsModal = useCallback(() => {
      if (savingApiKeyCredentials) return;
      setEditingApiKeyCredentialsId(null);
      setEditingApiKeyCredentialsValue("");
      setEditingApiKeyCredentialsVisible(false);
      setEditingApiBaseUrlCredentialsValue(DEFAULT_CODEX_API_BASE_URL);
      setEditingApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
      setEditingManagedProviderId("");
      setEditingManagedProviderApiKeyId("");
      setEditingNewManagedProviderNameInput("");
      setEditingApiModelCatalogInput("");
      setEditingApiModelContextWindowsInput({});
      setEditingApiSyncModelCatalogToCodex(false);
      setEditingApiModelCatalogFetching(false);
      setEditingApiModelCatalogError(null);
    }, [savingApiKeyCredentials]);
  
    const openApiKeyCredentialsModal = useCallback(
      (account: CodexAccount) => {
        if (!isCodexApiKeyAccount(account)) return;
        const initialBaseUrl = (account.api_base_url || "").trim();
        const initialApiKey = (account.openai_api_key || "").trim();
        const providerMode = inferCodexAccountProviderMode(account);
        const matchedProvider =
          findCodexModelProviderById(managedProviders, account.api_provider_id) ??
          findCodexModelProviderByBaseUrl(managedProviders, initialBaseUrl);
        const matchedProviderKey = matchedProvider?.apiKeys.find(
          (item) => item.apiKey.trim() === initialApiKey,
        );
  
        setEditingApiKeyCredentialsId(account.id);
        setEditingApiKeyCredentialsValue(initialApiKey);
        setEditingApiKeyCredentialsVisible(false);
        setEditingApiBaseUrlCredentialsValue(initialBaseUrl);
        setEditingApiProviderPresetId(
          providerMode === "openai_builtin"
            ? OPENAI_OFFICIAL_PRESET_ID
            : resolveCodexApiProviderPresetId(initialBaseUrl),
        );
        setEditingManagedProviderId(matchedProvider?.id ?? "");
        setEditingManagedProviderApiKeyId(matchedProviderKey?.id ?? "");
        setEditingNewManagedProviderNameInput(
          matchedProvider?.name ?? account.api_provider_name ?? "",
        );
        setEditingApiModelCatalogInput(
          (account.api_model_catalog ?? matchedProvider?.modelCatalog ?? []).join(
            "\n",
          ),
        );
        setEditingApiModelContextWindowsInput(
          contextWindowDraftsFromRecord(
            account.api_model_context_windows ??
              matchedProvider?.modelContextWindows,
            account.api_model_catalog ?? matchedProvider?.modelCatalog ?? [],
          ),
        );
        setEditingApiSyncModelCatalogToCodex(
          account.api_sync_model_catalog_to_codex === true,
        );
        setEditingApiModelCatalogFetching(false);
        setEditingApiModelCatalogError(null);
      },
      [managedProviders],
    );
  
    const handleSubmitApiKeyCredentials = useCallback(async () => {
      const accountId = editingApiKeyCredentialsId;
      if (!accountId) return;
  
      const validation = validateApiKeyCredentialInputs(
        editingApiKeyCredentialsValue,
        editingApiBaseUrlCredentialsValue,
      );
      if (!validation.ok) {
        setMessage({
          text: validation.message,
          tone: "error",
        });
        return;
      }
      if (
        editingApiSyncModelCatalogToCodex &&
        editingApiModelCatalogDraft.length === 0
      ) {
        setEditingApiModelCatalogError(
          t(
            "codex.api.modelCatalog.syncRequiresModels",
            "同步到 Codex 前请先获取或填写模型列表。",
          ),
        );
        return;
      }
      const parsedWindows = parseContextWindowDrafts(
        editingApiModelContextWindowsInput,
        editingApiModelCatalogDraft,
      );
      if (!parsedWindows.ok) {
        setEditingApiModelCatalogError(
          t(
            "codex.api.modelCatalog.contextWindowInvalid",
            "上下文窗口必须是大于 0 的整数",
          ),
        );
        return;
      }
      setEditingApiModelCatalogError(null);
      const editingAccount = accounts.find((account) => account.id === accountId);
      const providerPayload = {
        ...buildApiProviderPayload(
          editingApiBaseUrlCredentialsValue,
          editingApiProviderPresetId,
          editingManagedProviderId,
          editingNewManagedProviderNameInput,
          selectedEditingManagedProviderApiKey?.name,
        ),
        apiModelCatalog: editingApiModelCatalogDraft,
        apiModelContextWindows: parsedWindows.windows,
      };
  
      setSavingApiKeyCredentials(true);
      try {
        await updateApiKeyCredentials(
          accountId,
          validation.apiKey,
          validation.apiBaseUrl,
          providerPayload.apiProviderMode,
          providerPayload.apiProviderId,
          providerPayload.apiProviderName,
          providerPayload.apiModelCatalog,
          providerPayload.apiSupportsVision,
          providerPayload.apiModelVisionSupport,
          providerPayload.apiVisionRoutingModel,
          providerPayload.apiWireApi,
          providerPayload.apiSupportsWebsockets,
          editingApiSyncModelCatalogToCodex,
          providerPayload.accountName,
          parsedWindows.windows,
        );
        if (
          validation.apiBaseUrl &&
          providerPayload.apiProviderMode === "custom" &&
          providerPayload.apiProviderId !== COCKPIT_API_PROVIDER_ID
        ) {
          try {
            const savedProvider = await upsertCodexModelProviderFromCredential({
              providerId: isRelayApiProviderTemplateId(
                providerPayload.apiProviderId,
              )
                ? null
                : (providerPayload.apiProviderId ?? null),
              previousProviderId:
                resolveManagedProviderIdForAccount(editingAccount),
              providerName: providerPayload.apiProviderName ?? null,
              apiBaseUrl: validation.apiBaseUrl,
              apiKey: validation.apiKey,
              apiKeyName: providerPayload.accountName,
              sourceTag: providerPayload.sponsorTemplate?.id ?? null,
              modelCatalog: providerPayload.apiModelCatalog,
              modelContextWindows: providerPayload.apiModelContextWindows,
              supportsVision: providerPayload.sponsorTemplate?.supportsVision,
              website: providerPayload.sponsorTemplate?.website,
              apiKeyUrl: providerPayload.sponsorTemplate?.apiKeyUrl,
              wireApi: providerPayload.apiWireApi,
              supportsWebsockets: providerPayload.apiSupportsWebsockets,
              integrationType:
                providerPayload.apiIntegrationType ??
                providerPayload.sponsorTemplate?.integrationType,
            });
            try {
              const usageSummary = await queryCodexModelProviderUsage({
                baseUrl: savedProvider.baseUrl,
                apiKey: validation.apiKey,
                integrationType: savedProvider.integrationType ?? null,
              });
              if (
                (usageSummary.mode === "sub2api" ||
                  usageSummary.mode === "new_api") &&
                usageSummary.mode !== savedProvider.integrationType
              ) {
                await saveCodexModelProviderDetectedIntegrationType(
                  savedProvider.id,
                  usageSummary.mode,
                );
              }
            } catch (usageErr) {
              console.warn("[CodexModelProviders] 额度类型探测失败", usageErr);
            }
            await reloadManagedProviders();
          } catch (providerErr) {
            console.warn(
              "[CodexModelProviders] 更新凭据后写入供应商失败",
              providerErr,
            );
          }
        }
        setMessage({
          text:
            Object.keys(parsedWindows.windows).length > 0 ||
            editingApiSyncModelCatalogToCodex
              ? `${t("instances.messages.updated", "实例已更新")} ${t(
                  "codex.api.modelCatalog.restartHint",
                  "模型目录已更新。若 Codex 正在运行，请重启后生效。",
                )}`
              : t("instances.messages.updated", "实例已更新"),
        });
        setApiKeyUsageMap((previous) => {
          const next = { ...previous };
          delete next[accountId];
          return next;
        });
        setEditingApiKeyCredentialsId(null);
        setEditingApiKeyCredentialsValue("");
        setEditingApiKeyCredentialsVisible(false);
        setEditingApiBaseUrlCredentialsValue(DEFAULT_CODEX_API_BASE_URL);
        setEditingApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
        setEditingManagedProviderId("");
        setEditingManagedProviderApiKeyId("");
        setEditingNewManagedProviderNameInput("");
        setEditingApiModelCatalogInput("");
        setEditingApiSyncModelCatalogToCodex(false);
        setEditingApiModelCatalogError(null);
      } catch (e) {
        setMessage({
          text: `${t("common.failed", "失败")}: ${String(e)}`,
          tone: "error",
        });
      } finally {
        setSavingApiKeyCredentials(false);
      }
    }, [
      buildApiProviderPayload,
      editingApiBaseUrlCredentialsValue,
      editingApiKeyCredentialsId,
      editingApiKeyCredentialsValue,
      editingApiModelCatalogDraft,
      editingApiModelContextWindowsInput,
      editingApiProviderPresetId,
      editingApiSyncModelCatalogToCodex,
      editingManagedProviderId,
      editingNewManagedProviderNameInput,
      reloadManagedProviders,
      resolveManagedProviderIdForAccount,
      setMessage,
      t,
      upsertCodexModelProviderFromCredential,
      updateApiKeyCredentials,
      validateApiKeyCredentialInputs,
    ]);
  return {
    activeLaunchPreviewAccount,
    canRefreshApiKeyUsage,
    clearBatchImportSelection,
    clearInlineRename,
    closeApiKeyCredentialsModal,
    closeOAuthBindingModal,
    closeOAuthBindingQuotaReserveEditor,
    closeQuickSwitchModal,
    confirmOAuthBindingQuotaReserveEditor,
    findApiKeyUsageDetail,
    formatApiKeyUsageDetailByKey,
    formatApiKeyUsageDetailLabel,
    formatApiKeyUsageDetailValue,
    formatApiKeyUsageMoney,
    formatApiKeyUsagePercent,
    formatApiKeyUsageQuotaValue,
    formatCodexAuthFailureMessage,
    getCodexSwitchOrLaunchBlockedReason,
    handleAccountNameDoubleClick,
    handleApiBaseUrlInputChange,
    handleApiKeyInputChange,
    handleApiKeyLogin,
    handleBatchImportCheckQuotaChange,
    handleCancelBatchImport,
    handleChooseCodexCliWorkingDir,
    handleClearOAuthBinding,
    handleCloseBatchImport,
    handleConfirmBatchImport,
    handleCopyCodexCliCommand,
    handleDismissBatchImportTask,
    handleEditingApiBaseUrlCredentialsChange,
    handleEditingApiKeyCredentialsChange,
    handleExecuteCodexCli,
    handleExecuteLaunchPreview,
    handleFetchApiModelCatalog,
    handleFetchEditingApiModelCatalog,
    handleImportFromFiles,
    handleImportFromLocal,
    handleLaunchCodexCli,
    handleLaunchLocalAccessCli,
    handleOAuthBindingQuotaReserveToggle,
    handleOpenProviderLink,
    handleReauthorizeOAuthBinding,
    handleResumeBatchImport,
    handleSelectApiProviderPreset,
    handleSelectEditingApiProviderPreset,
    handleSelectEditingManagedProvider,
    handleSelectEditingManagedProviderApiKey,
    handleSelectManagedProvider,
    handleSelectManagedProviderApiKey,
    handleSelectQuickSwitchApiKey,
    handleSelectQuickSwitchProvider,
    handleSubmitApiKeyCredentials,
    handleSubmitInlineRename,
    handleSubmitOAuthBinding,
    handleSubmitQuickSwitch,
    handleSwitch,
    handleTokenImport,
    launchPreviewInstanceId,
    launchPreviewInstanceLabel,
    launchPreviewInstanceOptions,
    localAccessLaunchPreviewOpen,
    openApiKeyCredentialsModal,
    openLocalAccessOAuthBindingModal,
    openOAuthBindingModal,
    openOAuthBindingQuotaReserveEditor,
    openQuickSwitchProviderModal,
    performTokenImport,
    prepareCodexCliLaunch,
    refreshApiKeyUsage,
    refreshApiKeyUsageByAccountId,
    renderApiKeyRevealLine,
    renderApiKeyUsagePanel,
    renderOAuthBindingLine,
    resolveApiKeyDisplayText,
    resolveApiProviderDisplayName,
    resolveBoundOAuthAccount,
    resolveCockpitApiAccountBalanceText,
    resolveUsageProviderForApiKeyAccount,
    selectAllBatchImportAccounts,
    selectReadyBatchImportAccounts,
    setLaunchPreviewAccount,
    setLaunchPreviewInstanceId,
    setLocalAccessLaunchPreviewOpen,
    toggleAccountApiKeyVisible,
    toggleBatchImportItem,
    updateCodexCliWorkingDir,
    validateOAuthBindingQuotaReserveField,
  };
}
