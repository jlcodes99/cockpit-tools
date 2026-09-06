import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import * as codexService from "../services/codexService";
import * as codexInstanceService from "../services/codexInstanceService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { useModalErrorState } from "../components/ModalErrorMessage";
import { isCodexApiKeyAccount, type CodexApiProviderMode } from "../types/codex";
import { isCodexOAuthBindingEligibleAccount } from "../utils/codexLocalAccessAccounts";
import { mergeIdListsPreferExisting, subscribeUserMemory } from "../utils/userMemory";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { CodexAccount } from "../types/codex";
import type { CodexLocalAccessOAuthQuotaReserve } from "../types/codexLocalAccess";
import { CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT, CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT, isCodexAdditionalQuotaVisibleByDefault, isCodexCodeReviewQuotaVisibleByDefault } from "../utils/codexPreferences";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import { resolveCodexModelProviderAccountName } from "../utils/codexModelProviderAccountName";
import { readCodexCustomSortOrder, writeCodexCustomSortActive, writeCodexCustomSortOrder } from "../utils/codexAccountOverview";
import { CODEX_API_PROVIDER_CUSTOM_ID, COCKPIT_API_PROVIDER_ID, COCKPIT_API_PROVIDER_NAME, COCKPIT_TOOLS_API_PROVIDER_ID, codexApiProviderPresetVisionSupport, findCodexApiProviderPresetById, isCockpitApiProviderBaseUrl } from "../utils/codexProviderPresets";
import { isApiKeyFunProviderBaseUrl } from "../utils/apikeyFunLinks";
import { type ApiKeyFunPrefillPayload } from "../utils/apiKeyFunPrefill";
import { resolveCodexProviderCapabilityProfile } from "../utils/codexProviderGateway";
import { findCodexModelProviderById, findCodexModelProviderByBaseUrl, listCodexModelProviders, type CodexModelProvider, type CodexModelProviderIntegrationType } from "../services/codexModelProviderService";
import { readCodexApiKeyUsageCache, type CodexApiKeyUsageState } from "../services/codexApiKeyUsageRefreshService";
import { parseMfaCredentialInput, upsertSavedMfaRecord } from "../utils/mfaVault";
import { DEFAULT_CODEX_API_BASE_URL, DEFAULT_CODEX_API_PROVIDER_ID, getDefaultApiProviderPresetId, isSameHttpBaseUrl, normalizeHttpBaseUrl, normalizeSponsorApiProviderTemplates, OPENAI_OFFICIAL_PRESET_ID, parseApiModelCatalogText, resolveApiProviderPresetDefaults, type OAuthBindingQuotaReserveFieldErrors, type OAuthBindingTargetKind, type SponsorApiProviderTemplate } from "./codexAccountsControllerModel";
import type { useCodexAccountsBaseController } from "./useCodexAccountsBaseController";

/** 封装 useCodexAccountsPageController 的 useCodexAccountsOAuthController 业务域状态与动作。 */
export function useCodexAccountsOAuthController(context: Pick<ReturnType<typeof useCodexAccountsBaseController>,
  | "accounts"
  | "addStatus"
  | "addTab"
  | "apiKeyUsageDetailAccountId"
  | "applyAccountSnapshot"
  | "assignCodexAccountsToTargetGroup"
  | "cockpitApiPanelAccountId"
  | "fetchAccounts"
  | "fetchCurrentAccount"
  | "fetchSponsorState"
  | "localAccessCollection"
  | "openPendingOAuthNoteModal"
  | "page"
  | "pendingOAuthEmailInput"
  | "pendingOAuthNoteForm"
  | "reauthRetryInstanceId"
  | "reauthRetryOAuthBinding"
  | "reauthRetryLaunchAfterSwitch"
  | "reauthRetrySwitchAccountId"
  | "reauthTargetAccountId"
  | "reloadLocalAccessState"
  | "savingPendingOAuthAccount"
  | "setAccountNoteError"
  | "setApiKeyUsageDetailAccountId"
  | "setCockpitApiPanelAccountId"
  | "setPendingOAuthEmailInput"
  | "setPendingOAuthFieldErrors"
  | "setReauthTargetAccount"
  | "setSavedMfaRecords"
  | "setSavingPendingOAuthAccount"
  | "showAddModal"
  | "sortBy"
  | "sponsorModule"
  | "syncImportedAccountsToApiService"
  | "updateApiKeyBoundOAuthAccount"
  | "t"
>) {
  const {
    accounts,
    addStatus,
    addTab,
    apiKeyUsageDetailAccountId,
    applyAccountSnapshot,
    assignCodexAccountsToTargetGroup,
    cockpitApiPanelAccountId,
    fetchAccounts,
    fetchCurrentAccount,
    fetchSponsorState,
    localAccessCollection,
    openPendingOAuthNoteModal,
    page,
    pendingOAuthEmailInput,
    pendingOAuthNoteForm,
    reauthRetryInstanceId,
    reauthRetryOAuthBinding,
    reauthRetryLaunchAfterSwitch,
    reauthRetrySwitchAccountId,
    reauthTargetAccountId,
    reloadLocalAccessState,
    savingPendingOAuthAccount,
    setAccountNoteError,
    setApiKeyUsageDetailAccountId,
    setCockpitApiPanelAccountId,
    setPendingOAuthEmailInput,
    setPendingOAuthFieldErrors,
    setReauthTargetAccount,
    setSavedMfaRecords,
    setSavingPendingOAuthAccount,
    showAddModal,
    sortBy,
    sponsorModule,
    syncImportedAccountsToApiService,
    updateApiKeyBoundOAuthAccount,
    t,
  } = context;
  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
    const [oauthUrlCopied, setOauthUrlCopied] = useState(false);
    const [oauthPrepareError, setOauthPrepareError] = useState<string | null>(
      null,
    );
    const [oauthPortInUse, setOauthPortInUse] = useState<number | null>(null);
    const [oauthTimeoutInfo, setOauthTimeoutInfo] = useState<{
      loginId?: string;
      callbackUrl?: string;
      timeoutSeconds?: number;
    } | null>(null);
    const [oauthCallbackInput, setOauthCallbackInput] = useState("");
    const [oauthCallbackSubmitting, setOauthCallbackSubmitting] = useState(false);
    const [oauthCallbackError, setOauthCallbackError] = useState<string | null>(
      null,
    );
    const [deviceAuthInfo, setDeviceAuthInfo] = useState<{
      loginId: string;
      userCode: string;
      verificationUrl: string;
      pollIntervalSeconds: number;
    } | null>(null);
    const [deviceAuthStarting, setDeviceAuthStarting] = useState(false);
    const [deviceAuthError, setDeviceAuthError] = useState<string | null>(null);
    const [oauthMethod, setOauthMethod] = useState<"browser" | "device">(
      "browser",
    );
    const [deviceCodeCopied, setDeviceCodeCopied] = useState(false);
    const [oauthTokenExchangeRetryVisible, setOauthTokenExchangeRetryVisible] =
      useState(false);
    const [switching, setSwitching] = useState<string | null>(null);
    const [apiKeyInput, setApiKeyInput] = useState("");
    const [apiKeyInputVisible, setApiKeyInputVisible] = useState(false);
    const [apiBaseUrlInput, setApiBaseUrlInput] = useState(
      DEFAULT_CODEX_API_BASE_URL,
    );
    const [apiModelCatalogInput, setApiModelCatalogInput] = useState("");
    const [apiModelContextWindowsInput, setApiModelContextWindowsInput] =
      useState<Record<string, string>>({});
    const [apiSyncModelCatalogToCodex, setApiSyncModelCatalogToCodex] =
      useState(false);
    const [apiModelCatalogFetching, setApiModelCatalogFetching] = useState(false);
    const [apiModelCatalogError, setApiModelCatalogError] = useState<
      string | null
    >(null);
    const [apiProviderPresetId, setApiProviderPresetId] = useState(
      DEFAULT_CODEX_API_PROVIDER_ID,
    );
    const [managedProviders, setManagedProviders] = useState<
      CodexModelProvider[]
    >([]);
    const [managedProvidersLoading, setManagedProvidersLoading] = useState(false);
    const [apiKeyUsageMap, setApiKeyUsageMap] = useState<
      Record<string, CodexApiKeyUsageState>
    >(() => readCodexApiKeyUsageCache());
    const apiKeyUsageInFlightRef = useRef<Set<string>>(new Set());
    const deepSeekUsageRetryIdsRef = useRef<Set<string>>(new Set());
    const [managedProviderId, setManagedProviderId] = useState<string>("");
    const [managedProviderApiKeyId, setManagedProviderApiKeyId] =
      useState<string>("");
    const [newManagedProviderNameInput, setNewManagedProviderNameInput] =
      useState("");
    const [editingApiKeyNameId, setEditingApiKeyNameId] = useState<string | null>(
      null,
    );
    const [editingApiKeyNameValue, setEditingApiKeyNameValue] = useState("");
    const [savingApiKeyNameId, setSavingApiKeyNameId] = useState<string | null>(
      null,
    );
    const [editingApiKeyCredentialsId, setEditingApiKeyCredentialsId] = useState<
      string | null
    >(null);
    const [editingApiKeyCredentialsValue, setEditingApiKeyCredentialsValue] =
      useState("");
    const [editingApiKeyCredentialsVisible, setEditingApiKeyCredentialsVisible] =
      useState(false);
    const [
      editingApiBaseUrlCredentialsValue,
      setEditingApiBaseUrlCredentialsValue,
    ] = useState("");
    const [editingApiProviderPresetId, setEditingApiProviderPresetId] = useState(
      DEFAULT_CODEX_API_PROVIDER_ID,
    );
    const [editingApiModelCatalogInput, setEditingApiModelCatalogInput] =
      useState("");
    const [
      editingApiModelContextWindowsInput,
      setEditingApiModelContextWindowsInput,
    ] = useState<Record<string, string>>({});
    const [
      editingApiSyncModelCatalogToCodex,
      setEditingApiSyncModelCatalogToCodex,
    ] = useState(false);
    const [editingApiModelCatalogFetching, setEditingApiModelCatalogFetching] =
      useState(false);
    const [editingApiModelCatalogError, setEditingApiModelCatalogError] =
      useState<string | null>(null);
    const [editingManagedProviderId, setEditingManagedProviderId] =
      useState<string>("");
    const [editingManagedProviderApiKeyId, setEditingManagedProviderApiKeyId] =
      useState<string>("");
    const [
      editingNewManagedProviderNameInput,
      setEditingNewManagedProviderNameInput,
    ] = useState("");
    const [savingApiKeyCredentials, setSavingApiKeyCredentials] = useState(false);
    const [quickSwitchAccountId, setQuickSwitchAccountId] = useState<
      string | null
    >(null);
    const [quickSwitchProviderId, setQuickSwitchProviderId] =
      useState<string>("");
    const [quickSwitchApiKeyId, setQuickSwitchApiKeyId] = useState<string>("");
    const [quickSwitchSubmitting, setQuickSwitchSubmitting] = useState(false);
    const [quickSwitchError, setQuickSwitchError] = useState<string | null>(null);
    const [oauthBindingTargetKind, setOauthBindingTargetKind] =
      useState<OAuthBindingTargetKind | null>(null);
    const [oauthBindingAccountId, setOauthBindingAccountId] = useState<
      string | null
    >(null);
    const [oauthBindingSelectedAccountId, setOauthBindingSelectedAccountId] =
      useState("");
    const [oauthBindingSaving, setOauthBindingSaving] = useState(false);
    const [oauthBindingAutoSwitch, setOauthBindingAutoSwitch] = useState(false);
    const [oauthBindingQuotaReserve, setOauthBindingQuotaReserve] =
      useState<CodexLocalAccessOAuthQuotaReserve | null>(null);
    const [
      oauthBindingQuotaReserveEditorOpen,
      setOauthBindingQuotaReserveEditorOpen,
    ] = useState(false);
    const [oauthBindingHourlyReserveDraft, setOauthBindingHourlyReserveDraft] =
      useState("");
    const [oauthBindingWeeklyReserveDraft, setOauthBindingWeeklyReserveDraft] =
      useState("");
    const [
      oauthBindingQuotaReserveFieldErrors,
      setOauthBindingQuotaReserveFieldErrors,
    ] = useState<OAuthBindingQuotaReserveFieldErrors>({});
    const oauthBindingHourlyReserveInputRef = useRef<HTMLInputElement | null>(
      null,
    );
    const oauthBindingWeeklyReserveInputRef = useRef<HTMLInputElement | null>(
      null,
    );
    const {
      message: oauthBindingError,
      scrollKey: oauthBindingErrorScrollKey,
      set: setOauthBindingError,
    } = useModalErrorState();
    const [visibleApiKeyAccountIds, setVisibleApiKeyAccountIds] = useState<
      Set<string>
    >(() => new Set());
    const [showCodeReviewQuota, setShowCodeReviewQuota] = useState<boolean>(
      isCodexCodeReviewQuotaVisibleByDefault,
    );
    const [showAdditionalQuota, setShowAdditionalQuota] = useState<boolean>(
      isCodexAdditionalQuotaVisibleByDefault,
    );
    const [customSortOrder, setCustomSortOrder] = useState<string[]>(
      readCodexCustomSortOrder,
    );
    const [showCustomSortModal, setShowCustomSortModal] = useState(false);
    const [draggedCustomSortAccountId, setDraggedCustomSortAccountId] = useState<
      string | null
    >(null);
    const [customSortDropTargetId, setCustomSortDropTargetId] = useState<
      string | null
    >(null);
    const showAddModalRef = useRef(showAddModal);
    const addTabRef = useRef(addTab);
    const addStatusRef = useRef(addStatus);
    const oauthActiveRef = useRef(false);
    const oauthLoginIdRef = useRef<string | null>(null);
    const oauthCompletingRef = useRef(false);
    const oauthEventSeqRef = useRef(0);
    const oauthAttemptSeqRef = useRef(0);
    const inlineRenameDiscardRef = useRef(false);
    const skipManagedProviderApiKeyAutofillRef = useRef(false);
    const apiProviderPresetExplicitlySelectedRef = useRef(false);
    const apiKeyFunPrefillModelCatalogRef = useRef<string[] | null>(null);
    const pendingApiKeyFunCodexPrefillRef =
      useRef<ApiKeyFunPrefillPayload | null>(null);
  
    const selectedApiProviderPreset = useMemo(
      () => findCodexApiProviderPresetById(apiProviderPresetId),
      [apiProviderPresetId],
    );
    const sponsorApiProviderTemplates = useMemo(
      () => normalizeSponsorApiProviderTemplates(sponsorModule?.sponsors),
      [sponsorModule?.sponsors],
    );
    const selectedSponsorApiProviderTemplate = useMemo(
      () =>
        sponsorApiProviderTemplates.find(
          (template) => template.id === apiProviderPresetId,
        ) ?? null,
      [apiProviderPresetId, sponsorApiProviderTemplates],
    );
    const defaultApiProviderPresetId = useMemo(
      () => getDefaultApiProviderPresetId(sponsorApiProviderTemplates),
      [sponsorApiProviderTemplates],
    );
    const selectedEditingApiProviderPreset = useMemo(
      () => findCodexApiProviderPresetById(editingApiProviderPresetId),
      [editingApiProviderPresetId],
    );
    const selectedManagedProvider = useMemo(
      () =>
        managedProviders.find((item) => item.id === managedProviderId) ?? null,
      [managedProviderId, managedProviders],
    );
    const selectedManagedProviderApiKey = useMemo(
      () =>
        selectedManagedProvider?.apiKeys.find(
          (item) => item.id === managedProviderApiKeyId,
        ) ?? null,
      [managedProviderApiKeyId, selectedManagedProvider],
    );
    const selectedEditingManagedProvider = useMemo(
      () =>
        managedProviders.find((item) => item.id === editingManagedProviderId) ??
        null,
      [editingManagedProviderId, managedProviders],
    );
    const selectedEditingManagedProviderApiKey = useMemo(
      () =>
        selectedEditingManagedProvider?.apiKeys.find(
          (item) => item.id === editingManagedProviderApiKeyId,
        ) ?? null,
      [editingManagedProviderApiKeyId, selectedEditingManagedProvider],
    );
    const apiModelCatalogDraft = useMemo(
      () => parseApiModelCatalogText(apiModelCatalogInput),
      [apiModelCatalogInput],
    );
    const editingApiModelCatalogDraft = useMemo(
      () => parseApiModelCatalogText(editingApiModelCatalogInput),
      [editingApiModelCatalogInput],
    );
    const apiModelCatalogSyncAvailable = useMemo(
      () =>
        apiProviderPresetId !== OPENAI_OFFICIAL_PRESET_ID &&
        resolveCodexProviderCapabilityProfile({
          presetId: apiProviderPresetId,
          baseUrl: apiBaseUrlInput,
          wireApi:
            selectedManagedProvider?.wireApi ??
            selectedSponsorApiProviderTemplate?.wireApi ??
            null,
        }).wireApi === "responses",
      [
        apiBaseUrlInput,
        apiProviderPresetId,
        selectedManagedProvider?.wireApi,
        selectedSponsorApiProviderTemplate?.wireApi,
      ],
    );
    const editingApiModelCatalogSyncAvailable = useMemo(
      () =>
        editingApiProviderPresetId !== OPENAI_OFFICIAL_PRESET_ID &&
        resolveCodexProviderCapabilityProfile({
          presetId: editingApiProviderPresetId,
          baseUrl: editingApiBaseUrlCredentialsValue,
          wireApi: selectedEditingManagedProvider?.wireApi ?? null,
        }).wireApi === "responses",
      [
        editingApiBaseUrlCredentialsValue,
        editingApiProviderPresetId,
        selectedEditingManagedProvider?.wireApi,
      ],
    );
    useEffect(() => {
      if (!apiModelCatalogSyncAvailable) {
        setApiSyncModelCatalogToCodex(false);
      }
    }, [apiModelCatalogSyncAvailable]);
    useEffect(() => {
      if (!editingApiModelCatalogSyncAvailable) {
        setEditingApiSyncModelCatalogToCodex(false);
      }
    }, [editingApiModelCatalogSyncAvailable]);
    const quickSwitchAccount = useMemo(
      () =>
        quickSwitchAccountId
          ? (accounts.find((item) => item.id === quickSwitchAccountId) ?? null)
          : null,
      [accounts, quickSwitchAccountId],
    );
    const selectedQuickSwitchProvider = useMemo(
      () =>
        managedProviders.find((item) => item.id === quickSwitchProviderId) ??
        null,
      [managedProviders, quickSwitchProviderId],
    );
    const selectedQuickSwitchApiKey = useMemo(
      () =>
        selectedQuickSwitchProvider?.apiKeys.find(
          (item) => item.id === quickSwitchApiKeyId,
        ) ?? null,
      [quickSwitchApiKeyId, selectedQuickSwitchProvider],
    );
    const oauthAccounts = useMemo(
      () => accounts.filter((account) => !isCodexApiKeyAccount(account)),
      [accounts],
    );
    const oauthBindingEligibleAccounts = useMemo(
      () => oauthAccounts.filter(isCodexOAuthBindingEligibleAccount),
      [oauthAccounts],
    );
    const oauthBindingAccount = useMemo(
      () =>
        oauthBindingAccountId
          ? (accounts.find((item) => item.id === oauthBindingAccountId) ?? null)
          : null,
      [accounts, oauthBindingAccountId],
    );
    const selectedOAuthBindingAccount = useMemo(
      () =>
        oauthBindingEligibleAccounts.find(
          (item) => item.id === oauthBindingSelectedAccountId,
        ) ?? null,
      [oauthBindingEligibleAccounts, oauthBindingSelectedAccountId],
    );
    const boundLocalAccessOAuthAccount = useMemo(
      () =>
        localAccessCollection?.boundOauthAccountId
          ? (oauthAccounts.find(
              (item) => item.id === localAccessCollection.boundOauthAccountId,
            ) ?? null)
          : null,
      [localAccessCollection?.boundOauthAccountId, oauthAccounts],
    );
    const oauthBindingHasExistingBinding = useMemo(() => {
      if (oauthBindingTargetKind === "local_access") {
        return Boolean(localAccessCollection?.boundOauthAccountId);
      }
      if (oauthBindingTargetKind === "api_key_account") {
        return Boolean(oauthBindingAccount?.bound_oauth_account_id?.trim());
      }
      return false;
    }, [
      localAccessCollection?.boundOauthAccountId,
      oauthBindingAccount?.bound_oauth_account_id,
      oauthBindingTargetKind,
    ]);
    const oauthBindingTargetActive =
      oauthBindingTargetKind === "local_access" ||
      (oauthBindingTargetKind === "api_key_account" &&
        Boolean(oauthBindingAccount));
    const isLocalAccessOAuthBinding = oauthBindingTargetKind === "local_access";
    const cockpitApiPanelAccount = useMemo(
      () =>
        cockpitApiPanelAccountId
          ? (accounts.find((item) => item.id === cockpitApiPanelAccountId) ??
            null)
          : null,
      [accounts, cockpitApiPanelAccountId],
    );
    const apiKeyUsageDetailAccount = useMemo(
      () =>
        apiKeyUsageDetailAccountId
          ? (accounts.find((item) => item.id === apiKeyUsageDetailAccountId) ??
            null)
          : null,
      [accounts, apiKeyUsageDetailAccountId],
    );
  
    useEffect(() => {
      if (cockpitApiPanelAccountId && !cockpitApiPanelAccount) {
        setCockpitApiPanelAccountId(null);
      }
    }, [cockpitApiPanelAccount, cockpitApiPanelAccountId]);
  
    useEffect(() => {
      if (apiKeyUsageDetailAccountId && !apiKeyUsageDetailAccount) {
        setApiKeyUsageDetailAccountId(null);
      }
    }, [apiKeyUsageDetailAccount, apiKeyUsageDetailAccountId]);
  
    useEffect(() => {
      if (
        oauthBindingTargetKind === "api_key_account" &&
        oauthBindingAccountId &&
        !oauthBindingAccount
      ) {
        setOauthBindingTargetKind(null);
        setOauthBindingAccountId(null);
        setOauthBindingSelectedAccountId("");
        setOauthBindingAutoSwitch(false);
        setOauthBindingError(null);
      }
      if (oauthBindingTargetKind === "local_access" && !localAccessCollection) {
        setOauthBindingTargetKind(null);
        setOauthBindingAccountId(null);
        setOauthBindingSelectedAccountId("");
        setOauthBindingAutoSwitch(false);
        setOauthBindingError(null);
      }
    }, [
      localAccessCollection,
      oauthBindingAccount,
      oauthBindingAccountId,
      oauthBindingTargetKind,
      setOauthBindingError,
    ]);
  
    const oauthLog = useCallback((...args: unknown[]) => {
      console.info("[CodexOAuth]", ...args);
    }, []);
  
    const reloadManagedProviders = useCallback(async () => {
      setManagedProvidersLoading(true);
      try {
        const items = await listCodexModelProviders();
        setManagedProviders(items);
      } catch (err) {
        console.error("[CodexModelProviders] 加载失败", err);
      } finally {
        setManagedProvidersLoading(false);
      }
    }, []);
  
    const buildApiProviderPayload = useCallback(
      (
        apiBaseUrl: string,
        providerPresetId: string,
        providerId: string,
        customProviderName: string,
        managedProviderApiKeyName?: string | null,
      ): {
        apiProviderMode: CodexApiProviderMode;
        apiProviderId?: string;
        apiProviderName?: string;
        apiIntegrationType?: CodexModelProviderIntegrationType;
        apiModelCatalog?: string[];
        apiWireApi?: "responses" | "chat_completions";
        apiSupportsWebsockets?: boolean;
        apiSupportsVision?: boolean;
        apiModelVisionSupport?: Record<string, boolean>;
        apiVisionRoutingModel?: string;
        accountName?: string;
        sponsorTemplate?: SponsorApiProviderTemplate;
      } => {
        const normalizedBaseUrl = normalizeHttpBaseUrl(apiBaseUrl);
        if (!normalizedBaseUrl) {
          return { apiProviderMode: "openai_builtin" };
        }
        if (isCockpitApiProviderBaseUrl(normalizedBaseUrl)) {
          return {
            apiProviderMode: "custom",
            apiProviderId: COCKPIT_API_PROVIDER_ID,
            apiProviderName: COCKPIT_API_PROVIDER_NAME,
          };
        }
        const selectedPreset = findCodexApiProviderPresetById(providerPresetId);
        const selectedPresetBaseUrlMatches = Boolean(
          selectedPreset?.baseUrls.some((baseUrl) =>
            isSameHttpBaseUrl(baseUrl, normalizedBaseUrl),
          ),
        );
        if (
          providerPresetId === OPENAI_OFFICIAL_PRESET_ID &&
          selectedPresetBaseUrlMatches
        ) {
          return { apiProviderMode: "openai_builtin" };
        }
  
        const sponsorTemplate = sponsorApiProviderTemplates.find(
          (template) => template.id === providerPresetId,
        );
        if (sponsorTemplate) {
          return {
            apiProviderMode: "custom",
            apiProviderId: sponsorTemplate.id,
            apiProviderName: sponsorTemplate.name,
            apiModelCatalog: sponsorTemplate.modelCatalog,
            apiWireApi: sponsorTemplate.wireApi ?? undefined,
            apiSupportsVision: sponsorTemplate.supportsVision,
            accountName: sponsorTemplate.name,
            sponsorTemplate,
          };
        }
  
        const managedProvider = findCodexModelProviderById(
          managedProviders,
          providerId,
        );
        if (
          managedProvider &&
          isSameHttpBaseUrl(managedProvider.baseUrl, normalizedBaseUrl)
        ) {
          return {
            apiProviderMode: "custom",
            apiProviderId: managedProvider.id,
            apiProviderName: managedProvider.name,
            apiModelCatalog: managedProvider.modelCatalog,
            apiWireApi: managedProvider.wireApi ?? undefined,
            apiSupportsWebsockets: managedProvider.supportsWebsockets,
            apiSupportsVision: managedProvider.supportsVision,
            apiModelVisionSupport: Object.fromEntries(
              Object.entries(managedProvider.modelCapabilities ?? {}).map(
                ([model, capability]) => [
                  model,
                  capability.supportsVision === true,
                ],
              ),
            ),
            apiVisionRoutingModel: managedProvider.visionRoutingModel,
            accountName: resolveCodexModelProviderAccountName(
              managedProvider.name,
              managedProviderApiKeyName,
            ),
          };
        }
  
        const preset = selectedPreset;
        if (
          preset &&
          providerPresetId !== CODEX_API_PROVIDER_CUSTOM_ID &&
          (providerPresetId !== OPENAI_OFFICIAL_PRESET_ID ||
            selectedPresetBaseUrlMatches)
        ) {
          return {
            apiProviderMode: "custom",
            apiProviderId: preset.id,
            apiProviderName: preset.name,
            apiIntegrationType:
              preset.id === COCKPIT_TOOLS_API_PROVIDER_ID
                ? "cockpit_tools"
                : undefined,
            apiModelCatalog: preset.modelCatalog,
            apiWireApi: resolveCodexProviderCapabilityProfile({
              presetId: preset.id,
              baseUrl: normalizedBaseUrl,
              wireApi: null,
            }).wireApi,
            apiModelVisionSupport: codexApiProviderPresetVisionSupport(preset),
            accountName: preset.name,
          };
        }
  
        const isApiKeyFunProvider = isApiKeyFunProviderBaseUrl(normalizedBaseUrl);
        const apiKeyFunModelCatalog = isApiKeyFunProvider
          ? (apiKeyFunPrefillModelCatalogRef.current ?? undefined)
          : undefined;
        const trimmedName = customProviderName.trim();
        const customProviderDisplayName =
          trimmedName || (isApiKeyFunProvider ? "APIKEY.FUN" : undefined);
        return {
          apiProviderMode: "custom",
          apiProviderName: customProviderDisplayName,
          apiModelCatalog: apiKeyFunModelCatalog,
          apiWireApi: isApiKeyFunProvider ? "responses" : undefined,
          accountName: customProviderDisplayName,
        };
      },
      [managedProviders, sponsorApiProviderTemplates],
    );
  
    const resolveManagedProviderIdForAccount = useCallback(
      (account: CodexAccount | null | undefined): string | null => {
        if (!account || !isCodexApiKeyAccount(account)) return null;
        return (
          (
            findCodexModelProviderById(
              managedProviders,
              account.api_provider_id,
            ) ??
            findCodexModelProviderByBaseUrl(
              managedProviders,
              account.api_base_url ?? "",
            )
          )?.id ?? null
        );
      },
      [managedProviders],
    );
  
    useEffect(() => {
      showAddModalRef.current = showAddModal;
      addTabRef.current = addTab;
      addStatusRef.current = addStatus;
    }, [showAddModal, addTab, addStatus]);
  
    useEffect(() => {
      fetchAccounts();
      fetchCurrentAccount();
    }, [fetchAccounts, fetchCurrentAccount]);
  
    useEffect(() => {
      const accountIds = new Set(accounts.map((account) => account.id));
      setVisibleApiKeyAccountIds((prev) => {
        let changed = false;
        const next = new Set<string>();
        prev.forEach((accountId) => {
          if (accountIds.has(accountId)) {
            next.add(accountId);
          } else {
            changed = true;
          }
        });
        return changed ? next : prev;
      });
    }, [accounts]);
  
    useEffect(() => {
      return subscribeUserMemory(() => {
        setCustomSortOrder((prev) =>
          mergeIdListsPreferExisting(readCodexCustomSortOrder(), prev),
        );
      });
    }, []);
  
    useEffect(() => {
      if (accounts.length === 0) {
        return;
      }
      const accountIds = accounts.map((account) => account.id);
      setCustomSortOrder((prev) => {
        const next = [...prev];
        const seen = new Set(next);
        for (const accountId of accountIds) {
          if (!seen.has(accountId)) {
            next.push(accountId);
            seen.add(accountId);
          }
        }
        const unchanged =
          next.length === prev.length &&
          next.every((accountId, index) => accountId === prev[index]);
        return unchanged ? prev : next;
      });
    }, [accounts]);
  
    useEffect(() => {
      writeCodexCustomSortOrder(customSortOrder);
    }, [customSortOrder]);
  
    useEffect(() => {
      writeCodexCustomSortActive(sortBy === "custom");
    }, [sortBy]);
  
    useEffect(() => {
      if (!showCustomSortModal || !draggedCustomSortAccountId) return;
      const handleMouseUp = () => {
        setDraggedCustomSortAccountId(null);
        setCustomSortDropTargetId(null);
      };
      window.addEventListener("mouseup", handleMouseUp);
      return () => window.removeEventListener("mouseup", handleMouseUp);
    }, [showCustomSortModal, draggedCustomSortAccountId]);
  
    useEffect(() => {
      if (!showCustomSortModal) {
        setDraggedCustomSortAccountId(null);
        setCustomSortDropTargetId(null);
      }
    }, [showCustomSortModal]);
  
    useEffect(() => {
      void reloadManagedProviders();
    }, [reloadManagedProviders]);
  
    useEffect(() => {
      void fetchSponsorState();
    }, [fetchSponsorState]);
  
    useEffect(() => {
      if (!showAddModal) {
        apiProviderPresetExplicitlySelectedRef.current = false;
        if (!pendingApiKeyFunCodexPrefillRef.current) {
          apiKeyFunPrefillModelCatalogRef.current = null;
        }
        const defaultProvider = resolveApiProviderPresetDefaults(
          defaultApiProviderPresetId,
          sponsorApiProviderTemplates,
        );
        setApiKeyInput("");
        setApiKeyInputVisible(false);
        setApiBaseUrlInput(defaultProvider.baseUrl);
        setApiProviderPresetId(defaultApiProviderPresetId);
        setManagedProviderId("");
        setManagedProviderApiKeyId("");
        setNewManagedProviderNameInput(defaultProvider.providerName);
        const defaultModels =
          sponsorApiProviderTemplates.find(
            (template) => template.id === defaultApiProviderPresetId,
          )?.modelCatalog ??
          findCodexApiProviderPresetById(defaultApiProviderPresetId)
            ?.modelCatalog ??
          [];
        setApiModelCatalogInput(defaultModels.join("\n"));
        setApiModelContextWindowsInput({});
        setApiSyncModelCatalogToCodex(false);
        setApiModelCatalogFetching(false);
        setApiModelCatalogError(null);
      }
    }, [defaultApiProviderPresetId, showAddModal, sponsorApiProviderTemplates]);
  
    useEffect(() => {
      if (showAddModal && addTab === "apikey") {
        setApiKeyInputVisible(false);
      }
    }, [addTab, showAddModal]);
  
    useEffect(() => {
      if (!showAddModal || addTab !== "apikey") {
        return;
      }
      if (sponsorApiProviderTemplates.length === 0) {
        return;
      }
      if (apiProviderPresetExplicitlySelectedRef.current) {
        return;
      }
      const shouldUseDefaultProvider =
        apiProviderPresetId === DEFAULT_CODEX_API_PROVIDER_ID ||
        !apiProviderPresetId.trim();
      const nextProviderPresetId = shouldUseDefaultProvider
        ? defaultApiProviderPresetId
        : apiProviderPresetId;
      const shouldSyncSponsorDefaults =
        shouldUseDefaultProvider ||
        (sponsorApiProviderTemplates.some(
          (template) => template.id === nextProviderPresetId,
        ) &&
          normalizeHttpBaseUrl(apiBaseUrlInput) ===
            normalizeHttpBaseUrl(DEFAULT_CODEX_API_BASE_URL));
      if (apiProviderPresetId !== nextProviderPresetId) {
        setApiProviderPresetId(nextProviderPresetId);
      }
      if (shouldSyncSponsorDefaults) {
        const defaultProvider = resolveApiProviderPresetDefaults(
          nextProviderPresetId,
          sponsorApiProviderTemplates,
        );
        setApiBaseUrlInput(defaultProvider.baseUrl);
        setNewManagedProviderNameInput(defaultProvider.providerName);
        const defaultModels =
          sponsorApiProviderTemplates.find(
            (template) => template.id === nextProviderPresetId,
          )?.modelCatalog ??
          findCodexApiProviderPresetById(nextProviderPresetId)?.modelCatalog ??
          [];
        setApiModelCatalogInput(defaultModels.join("\n"));
      }
    }, [
      addTab,
      apiBaseUrlInput,
      apiProviderPresetId,
      defaultApiProviderPresetId,
      showAddModal,
      sponsorApiProviderTemplates,
    ]);
  
    useEffect(() => {
      if (apiProviderPresetId === OPENAI_OFFICIAL_PRESET_ID) {
        skipManagedProviderApiKeyAutofillRef.current = false;
        setManagedProviderId("");
        setManagedProviderApiKeyId("");
        return;
      }
  
      // Prefer the explicitly selected provider when its base URL still matches;
      // otherwise auto-link by Base URL so multi-key pickers appear after preset/URL selection.
      const matchedById = managedProviderId
        ? findCodexModelProviderById(managedProviders, managedProviderId)
        : null;
      const matchedByUrl = apiBaseUrlInput.trim()
        ? findCodexModelProviderByBaseUrl(managedProviders, apiBaseUrlInput)
        : null;
      const matched =
        matchedById && isSameHttpBaseUrl(matchedById.baseUrl, apiBaseUrlInput)
          ? matchedById
          : matchedByUrl;
  
      if (matched) {
        if (managedProviderId !== matched.id) {
          setManagedProviderId(matched.id);
        }
      } else if (managedProviderId) {
        skipManagedProviderApiKeyAutofillRef.current = false;
        setManagedProviderId("");
        setManagedProviderApiKeyId("");
        return;
      } else {
        skipManagedProviderApiKeyAutofillRef.current = false;
        setManagedProviderApiKeyId("");
        return;
      }
  
      if (
        matched.apiKeys.length === 0 ||
        skipManagedProviderApiKeyAutofillRef.current
      ) {
        skipManagedProviderApiKeyAutofillRef.current = false;
        setManagedProviderApiKeyId("");
        return;
      }
      setManagedProviderApiKeyId((prev) => {
        if (matched.apiKeys.some((item) => item.id === prev)) return prev;
        return matched.apiKeys[0]?.id ?? "";
      });
    }, [
      apiBaseUrlInput,
      apiProviderPresetId,
      managedProviderId,
      managedProviders,
    ]);
  
    useEffect(() => {
      if (!selectedManagedProviderApiKey) return;
      setApiKeyInput(selectedManagedProviderApiKey.apiKey);
      setApiKeyInputVisible(false);
    }, [managedProviderApiKeyId, selectedManagedProviderApiKey]);
  
    useEffect(() => {
      if (editingApiProviderPresetId === OPENAI_OFFICIAL_PRESET_ID) {
        setEditingManagedProviderId("");
        setEditingManagedProviderApiKeyId("");
        return;
      }
  
      const matchedById = editingManagedProviderId
        ? findCodexModelProviderById(managedProviders, editingManagedProviderId)
        : null;
      const matchedByUrl = editingApiBaseUrlCredentialsValue.trim()
        ? findCodexModelProviderByBaseUrl(
            managedProviders,
            editingApiBaseUrlCredentialsValue,
          )
        : null;
      const matched =
        matchedById &&
        isSameHttpBaseUrl(matchedById.baseUrl, editingApiBaseUrlCredentialsValue)
          ? matchedById
          : matchedByUrl;
  
      if (matched) {
        if (editingManagedProviderId !== matched.id) {
          setEditingManagedProviderId(matched.id);
        }
      } else if (editingManagedProviderId) {
        setEditingManagedProviderId("");
        setEditingManagedProviderApiKeyId("");
        return;
      } else {
        setEditingManagedProviderApiKeyId("");
        return;
      }
  
      if (matched.apiKeys.length === 0) {
        setEditingManagedProviderApiKeyId("");
        return;
      }
      setEditingManagedProviderApiKeyId((prev) => {
        if (matched.apiKeys.some((item) => item.id === prev)) return prev;
        return matched.apiKeys[0]?.id ?? "";
      });
    }, [
      editingApiBaseUrlCredentialsValue,
      editingApiProviderPresetId,
      editingManagedProviderId,
      managedProviders,
    ]);
  
    useEffect(() => {
      if (!selectedEditingManagedProviderApiKey) return;
      setEditingApiKeyCredentialsValue(
        selectedEditingManagedProviderApiKey.apiKey,
      );
      setEditingApiKeyCredentialsVisible(false);
    }, [editingManagedProviderApiKeyId, selectedEditingManagedProviderApiKey]);
  
    useEffect(() => {
      if (!quickSwitchAccountId) return;
      if (accounts.some((item) => item.id === quickSwitchAccountId)) return;
      setQuickSwitchAccountId(null);
      setQuickSwitchProviderId("");
      setQuickSwitchApiKeyId("");
      setQuickSwitchError(null);
    }, [accounts, quickSwitchAccountId]);
  
    useEffect(() => {
      if (!selectedQuickSwitchProvider) {
        setQuickSwitchApiKeyId("");
        return;
      }
      setQuickSwitchApiKeyId((prev) => {
        if (
          selectedQuickSwitchProvider.apiKeys.some((item) => item.id === prev)
        ) {
          return prev;
        }
        return selectedQuickSwitchProvider.apiKeys[0]?.id ?? "";
      });
    }, [selectedQuickSwitchProvider]);
  
    useEffect(() => {
      const syncCodeReviewVisibility = () => {
        setShowCodeReviewQuota(isCodexCodeReviewQuotaVisibleByDefault());
      };
      const syncAdditionalQuotaVisibility = () => {
        setShowAdditionalQuota(isCodexAdditionalQuotaVisibleByDefault());
      };
  
      window.addEventListener(
        CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncCodeReviewVisibility as EventListener,
      );
      window.addEventListener(
        CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncAdditionalQuotaVisibility as EventListener,
      );
      return () => {
        window.removeEventListener(
          CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
          syncCodeReviewVisibility as EventListener,
        );
        window.removeEventListener(
          CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT,
          syncAdditionalQuotaVisibility as EventListener,
        );
      };
    }, []);
  
    // Hook provides setAddStatus/setAddMessage but we need refs to page's versions
    const { setAddStatus, setAddMessage, resetAddModalState, setShowAddModal } =
      page;
  
    const handlePendingOAuthEmailInputChange = useCallback(
      (value: string) => {
        setPendingOAuthEmailInput(value);
        setPendingOAuthFieldErrors((prev) => ({
          ...prev,
          email: undefined,
        }));
        setAccountNoteError(null);
        setAddStatus("idle");
        setAddMessage("");
      },
      [setAccountNoteError, setAddMessage, setAddStatus],
    );
  
    const buildPendingOAuthNoteUpdate = useCallback(() => {
      const rawTwoFactorSecret = pendingOAuthNoteForm.twoFactorSecret.trim();
      const parsedTwoFactorSecret = rawTwoFactorSecret
        ? parseMfaCredentialInput(rawTwoFactorSecret)
        : null;
      if (rawTwoFactorSecret && !parsedTwoFactorSecret) {
        setPendingOAuthFieldErrors((prev) => ({
          ...prev,
          twoFactorSecret: t(
            "codex.accountNote.twoFactorSecretInvalid",
            "2FA 秘钥格式无效，请输入 Base32 secret 或 otpauth:// 链接",
          ),
        }));
        openPendingOAuthNoteModal();
        return null;
      }
  
      return {
        note: pendingOAuthNoteForm.note,
        twoFactorSecret: parsedTwoFactorSecret?.secret ?? rawTwoFactorSecret,
        accountPassword: pendingOAuthNoteForm.accountPassword,
        phoneNumber: pendingOAuthNoteForm.phoneNumber,
        mailUrl: pendingOAuthNoteForm.mailUrl,
      };
    }, [openPendingOAuthNoteModal, pendingOAuthNoteForm, t]);
  
    const handleSavePendingOAuthAccount = useCallback(async () => {
      if (savingPendingOAuthAccount) return;
      const email = pendingOAuthEmailInput.trim();
      setPendingOAuthFieldErrors({});
      setOauthPrepareError(null);
      setAddStatus("idle");
      setAddMessage("");
  
      if (!email) {
        setPendingOAuthFieldErrors({
          email: t("codex.pendingAuth.emailRequired", "请输入账号邮箱"),
        });
        return;
      }
  
      const noteUpdate = buildPendingOAuthNoteUpdate();
      if (!noteUpdate) return;
  
      setSavingPendingOAuthAccount(true);
      setAddStatus("loading");
      setAddMessage(t("codex.pendingAuth.saving", "正在保存待授权账号..."));
      try {
        const account = await codexService.createPendingCodexOAuthAccount(
          email,
          noteUpdate,
        );
        if (noteUpdate.twoFactorSecret.trim()) {
          setSavedMfaRecords(
            upsertSavedMfaRecord({
              secret: noteUpdate.twoFactorSecret,
              accountName: email,
              remark: noteUpdate.note,
            }),
          );
        }
        await fetchAccounts();
        await assignCodexAccountsToTargetGroup([account]);
        await emitAccountsChanged({
          platformId: "codex",
          accountId: account.id,
          reason: "pending_oauth",
        });
        setAddStatus("success");
        setAddMessage(t("codex.pendingAuth.saved", "待授权账号已保存"));
        setReauthTargetAccount(account);
        window.setTimeout(() => {
          setShowAddModal(false);
          resetAddModalState();
        }, 900);
      } catch (error) {
        setAddStatus("error");
        setAddMessage(
          t("codex.pendingAuth.saveFailed", {
            defaultValue: "保存待授权账号失败：{{error}}",
            error: String(error).replace(/^Error:\s*/, ""),
          }),
        );
      } finally {
        setSavingPendingOAuthAccount(false);
      }
    }, [
      buildPendingOAuthNoteUpdate,
      assignCodexAccountsToTargetGroup,
      fetchAccounts,
      pendingOAuthEmailInput,
      resetAddModalState,
      savingPendingOAuthAccount,
      setAddMessage,
      setAddStatus,
      setShowAddModal,
      t,
    ]);
  
    const handleOauthPrepareError = useCallback(
      (e: unknown) => {
        console.error("[CodexOAuth] 准备授权链接失败", { error: String(e) });
        oauthActiveRef.current = false;
        setOauthTimeoutInfo(null);
        setOauthCallbackSubmitting(false);
        setOauthCallbackError(null);
        setOauthTokenExchangeRetryVisible(false);
        setDeviceAuthInfo(null);
        setDeviceAuthError(null);
        setOauthMethod("browser");
        setDeviceCodeCopied(false);
        const match = String(e).match(/CODEX_OAUTH_PORT_IN_USE:(\d+)/);
        if (match) {
          const port = Number(match[1]);
          setOauthPortInUse(Number.isNaN(port) ? null : port);
          setOauthPrepareError(t("codex.oauth.portInUse", { port: match[1] }));
          return;
        }
        setOauthPrepareError(
          t("common.shared.oauth.failed", "授权失败") + ": " + String(e),
        );
      },
      [t],
    );
  
    const completeOauthSuccess = useCallback(
      async (account?: CodexAccount | null) => {
        oauthLog("授权完成并保存成功", { loginId: oauthLoginIdRef.current });
        if (account) {
          // OAuth 回调已经返回后端刚保存的账号快照，先立即合并到 UI，
          // 覆盖旧的 localStorage/store 状态，再执行后台回读和额度刷新。
          applyAccountSnapshot(account);
        }
        await fetchAccounts();
        await fetchCurrentAccount();
        // API Service 的 OAuth 绑定信息也依赖账号授权状态，重新授权后后台回读，
        // 避免卡片继续展示重新授权前的运行态快照。
        void reloadLocalAccessState();
        // 绑定流程发起的重新授权需要在新凭据落盘后自动恢复原操作，
        // 保证 API Service 与 API Key 绑定 OAuth 使用同一套授权状态机。
        if (reauthRetryOAuthBinding && account?.id) {
          try {
            if (reauthRetryOAuthBinding.targetKind === "local_access") {
              await codexLocalAccessService.updateCodexLocalAccessBoundOAuthAccount(
                account.id,
                reauthRetryOAuthBinding.quotaReserve ?? null,
              );
              await reloadLocalAccessState();
            } else if (reauthRetryOAuthBinding.targetAccountId) {
              await updateApiKeyBoundOAuthAccount(
                reauthRetryOAuthBinding.targetAccountId,
                account.id,
              );
            } else {
              throw new Error(
                t("codex.api.oauthBinding.saveFailed", {
                  defaultValue: "OAuth 绑定失败：绑定目标账号缺失",
                  error: t(
                    "codex.api.oauthBinding.validationRequired",
                    "请选择 OAuth 账号",
                  ),
                }),
              );
            }
          } catch (error) {
            setAddStatus("error");
            setAddMessage(
              t("codex.api.oauthBinding.saveFailed", {
                defaultValue: "OAuth 绑定失败：{{error}}",
                error: String(error).replace(/^Error:\s*/, ""),
              }),
            );
            return;
          }
        }
        if (!reauthTargetAccountId) {
          await assignCodexAccountsToTargetGroup([account]);
        }
        await emitAccountsChanged({
          platformId: "codex",
          reason: "oauth",
        });
        if (!reauthTargetAccountId && account?.id) {
          try {
            await syncImportedAccountsToApiService([account.id]);
          } catch (error) {
            setAddStatus("error");
            setAddMessage(
              t(
                "codex.importApiService.syncFailed",
                "账号已导入，但加入 API 服务失败：{{error}}",
              ).replace("{{error}}", String(error).replace(/^Error:\s*/, "")),
            );
            oauthActiveRef.current = false;
            oauthCompletingRef.current = false;
            oauthLoginIdRef.current = null;
            return;
          }
        }
        setAddStatus("success");
        if (reauthRetrySwitchAccountId && account?.id) {
          setAddStatus("loading");
          setAddMessage(
            t(
              "codex.switchAuth.switchingAfterReauth",
              "授权成功，正在继续切换账号...",
            ),
          );
          try {
            const refreshedAccount =
              useCodexAccountStore
                .getState()
                .accounts.find((item) => item.id === account.id) || account;
            const tokenGeneration = refreshedAccount.token_generation;
            if (
              typeof tokenGeneration !== "number" ||
              !Number.isFinite(tokenGeneration)
            ) {
              throw new Error(t("codex.switchAuth.reauthGenerationMissing"));
            }
            await useCodexAccountStore.getState().switchAccount(account.id, {
              reauthTokenGeneration: tokenGeneration,
              reconcileAfterSwitch: true,
              launchAfterSwitch: reauthRetryLaunchAfterSwitch,
            });
          } catch (error) {
            setAddStatus("error");
            setAddMessage(
              t("codex.switchAuth.switchAfterReauthFailed", {
                defaultValue: "授权已更新，但继续切换失败：{{error}}",
                error: String(error).replace(/^Error:\s*/, ""),
              }),
            );
            return;
          }
        }
        if (reauthRetryInstanceId && account?.id) {
          setAddStatus("loading");
          setAddMessage(
            t(
              "codex.switchAuth.startingInstanceAfterReauth",
              "授权成功，正在重新启动实例...",
            ),
          );
          try {
            await codexInstanceService.startInstance(reauthRetryInstanceId);
          } catch (error) {
            setAddStatus("error");
            setAddMessage(
              t("codex.switchAuth.startInstanceAfterReauthFailed", {
                defaultValue: "授权已更新，但实例启动失败：{{error}}",
                error: String(error).replace(/^Error:\s*/, ""),
              }),
            );
            return;
          }
        }
        setAddStatus("success");
        setAddMessage(
          reauthRetrySwitchAccountId
            ? t("codex.switchAuth.reauthorizedAndSwitched", "重新授权并切换成功")
            : reauthRetryInstanceId
              ? t(
                  "codex.switchAuth.reauthorizedAndStartedInstance",
                  "重新授权并启动实例成功",
                )
              : t("common.shared.oauth.success", "授权成功"),
        );
        oauthActiveRef.current = false;
        oauthCompletingRef.current = false;
        oauthLoginIdRef.current = null;
        setOauthUrl("");
        setOauthUrlCopied(false);
        setOauthPrepareError(null);
        setOauthPortInUse(null);
        setOauthTimeoutInfo(null);
        setOauthCallbackInput("");
        setOauthCallbackSubmitting(false);
        setOauthCallbackError(null);
        setOauthTokenExchangeRetryVisible(false);
        setDeviceAuthInfo(null);
        setDeviceAuthError(null);
        setOauthMethod("browser");
        setDeviceCodeCopied(false);
        setTimeout(() => {
          setShowAddModal(false);
          resetAddModalState();
        }, 1200);
      },
      [
        assignCodexAccountsToTargetGroup,
        applyAccountSnapshot,
        fetchAccounts,
        fetchCurrentAccount,
        reloadLocalAccessState,
        reauthTargetAccountId,
        reauthRetrySwitchAccountId,
        reauthRetryLaunchAfterSwitch,
        reauthRetryInstanceId,
        reauthRetryOAuthBinding,
        syncImportedAccountsToApiService,
        updateApiKeyBoundOAuthAccount,
        t,
        oauthLog,
        setAddStatus,
        setAddMessage,
        setShowAddModal,
        resetAddModalState,
      ],
    );
  
    const completeOauthError = useCallback(
      (e: unknown, allowTokenExchangeRetry = false) => {
        setAddStatus("error");
        setAddMessage(
          t("common.shared.oauth.failed", "授权失败") + ": " + String(e),
        );
        setOauthTokenExchangeRetryVisible(allowTokenExchangeRetry);
      },
      [t, setAddStatus, setAddMessage],
    );
  
    const isOauthTimeoutState = useMemo(
      () => !!oauthTimeoutInfo,
      [oauthTimeoutInfo],
    );
    const isOauthTokenExchangeErrorState = useMemo(() => {
      return addStatus === "error" && oauthTokenExchangeRetryVisible;
    }, [addStatus, oauthTokenExchangeRetryVisible]);
  
    useEffect(() => {
      let unlistenExtension: UnlistenFn | undefined;
      let unlistenTimeout: UnlistenFn | undefined;
      let unlistenDeviceError: UnlistenFn | undefined;
      let disposed = false;
  
      listen<{ loginId?: string }>(
        "codex-oauth-login-completed",
        async (event) => {
          ++oauthEventSeqRef.current;
          if (
            !showAddModalRef.current ||
            addTabRef.current !== "oauth" ||
            addStatusRef.current === "loading" ||
            oauthCompletingRef.current
          )
            return;
          const loginId = event.payload?.loginId;
          if (!loginId) return;
          if (oauthLoginIdRef.current && oauthLoginIdRef.current !== loginId)
            return;
          ++oauthAttemptSeqRef.current;
          setAddStatus("loading");
          setAddMessage(t("codex.oauth.exchanging", "正在交换令牌..."));
          oauthCompletingRef.current = true;
          try {
            const account = await codexService.completeCodexOAuthLogin(
              loginId,
              reauthTargetAccountId || null,
            );
            await completeOauthSuccess(account);
          } catch (e) {
            completeOauthError(e, true);
          } finally {
            oauthCompletingRef.current = false;
          }
        },
      ).then((fn) => {
        if (disposed) fn();
        else unlistenExtension = fn;
      });
  
      listen<{ loginId?: string; callbackUrl?: string; timeoutSeconds?: number }>(
        "codex-oauth-login-timeout",
        async (event) => {
          if (!showAddModalRef.current || addTabRef.current !== "oauth") return;
          const payload = event.payload ?? {};
          const loginId = payload.loginId;
          if (
            oauthLoginIdRef.current &&
            loginId &&
            oauthLoginIdRef.current !== loginId
          )
            return;
          oauthActiveRef.current = false;
          setOauthUrlCopied(false);
          setOauthPortInUse(null);
          setOauthTimeoutInfo(payload);
          setOauthPrepareError(null);
          setOauthCallbackSubmitting(false);
          setOauthCallbackError(null);
          setOauthTokenExchangeRetryVisible(false);
          setDeviceAuthInfo(null);
          setAddStatus("idle");
          setAddMessage("");
        },
      ).then((fn) => {
        if (disposed) fn();
        else unlistenTimeout = fn;
      });
  
      listen<{ loginId?: string; error?: string }>(
        "codex-device-auth-error",
        (event) => {
          if (
            event.payload?.loginId &&
            oauthLoginIdRef.current !== event.payload.loginId
          )
            return;
          oauthActiveRef.current = false;
          setDeviceAuthError(
            event.payload?.error || t("common.shared.oauth.failed", "授权失败"),
          );
          setDeviceAuthInfo(null);
        },
      ).then((fn) => {
        if (disposed) fn();
        else unlistenDeviceError = fn;
      });
  
      return () => {
        disposed = true;
        unlistenExtension?.();
        unlistenTimeout?.();
        unlistenDeviceError?.();
      };
    }, [
      completeOauthError,
      completeOauthSuccess,
      reauthTargetAccountId,
      t,
      setAddStatus,
      setAddMessage,
    ]);
  
    const prepareOauthUrl = useCallback(() => {
      if (!showAddModalRef.current || addTabRef.current !== "oauth") return;
      if (oauthActiveRef.current) return;
      const attemptSeq = ++oauthAttemptSeqRef.current;
      oauthActiveRef.current = true;
      setOauthPrepareError(null);
      setOauthPortInUse(null);
      setOauthTimeoutInfo(null);
      setOauthCallbackInput("");
      setOauthCallbackSubmitting(false);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
  
      codexService
        .startCodexOAuthLogin()
        .then(({ loginId, authUrl }) => {
          if (attemptSeq !== oauthAttemptSeqRef.current) {
            if (loginId) {
              codexService.cancelCodexOAuthLogin(loginId).catch(() => {});
            }
            oauthLog("忽略过期 OAuth start 响应", { loginId, attemptSeq });
            return;
          }
          oauthLoginIdRef.current = loginId ?? null;
          if (
            typeof authUrl === "string" &&
            authUrl.length > 0 &&
            showAddModalRef.current &&
            addTabRef.current === "oauth"
          ) {
            setOauthUrl(authUrl);
          } else {
            oauthActiveRef.current = false;
          }
        })
        .catch((e) => {
          if (attemptSeq !== oauthAttemptSeqRef.current) {
            oauthLog("忽略过期 OAuth start 异常回调", {
              attemptSeq,
              error: String(e),
            });
            return;
          }
          handleOauthPrepareError(e);
        });
    }, [handleOauthPrepareError, oauthLog]);
  
    useEffect(() => {
      if (
        !showAddModal ||
        addTab !== "oauth" ||
        oauthUrl ||
        oauthTimeoutInfo ||
        oauthMethod !== "browser" ||
        deviceAuthInfo ||
        deviceAuthStarting ||
        deviceAuthError
      )
        return;
      prepareOauthUrl();
    }, [
      showAddModal,
      addTab,
      oauthUrl,
      oauthTimeoutInfo,
      oauthMethod,
      deviceAuthInfo,
      deviceAuthStarting,
      deviceAuthError,
      prepareOauthUrl,
    ]);
  
    useEffect(() => {
      if (showAddModal && addTab === "oauth") return;
      const loginId = oauthLoginIdRef.current ?? undefined;
      const hasOauthUiResidue =
        Boolean(oauthUrl) ||
        Boolean(oauthTimeoutInfo) ||
        oauthCallbackInput.length > 0 ||
        oauthCallbackSubmitting ||
        Boolean(oauthCallbackError) ||
        Boolean(oauthPrepareError) ||
        Boolean(deviceAuthInfo) ||
        Boolean(deviceAuthError) ||
        oauthMethod !== "browser" ||
        oauthPortInUse !== null ||
        oauthUrlCopied;
      if (
        !loginId &&
        !oauthActiveRef.current &&
        !oauthCompletingRef.current &&
        !hasOauthUiResidue
      )
        return;
      oauthAttemptSeqRef.current += 1;
      if (loginId) {
        codexService.cancelCodexOAuthLogin(loginId).catch(() => {});
      }
      oauthActiveRef.current = false;
      oauthCompletingRef.current = false;
      oauthLoginIdRef.current = null;
      setOauthUrl("");
      setOauthUrlCopied(false);
      setOauthTimeoutInfo(null);
      setOauthCallbackInput("");
      setOauthCallbackSubmitting(false);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
      setDeviceAuthInfo(null);
      setDeviceAuthError(null);
      setOauthMethod("browser");
      setDeviceCodeCopied(false);
    }, [
      showAddModal,
      addTab,
      oauthUrl,
      oauthTimeoutInfo,
      oauthCallbackInput,
      oauthCallbackSubmitting,
      oauthCallbackError,
      oauthPrepareError,
      oauthMethod,
      deviceAuthInfo,
      deviceAuthError,
      oauthPortInUse,
      oauthUrlCopied,
      oauthTokenExchangeRetryVisible,
    ]);
  
    useEffect(
      () => () => {
        oauthAttemptSeqRef.current += 1;
        const loginId = oauthLoginIdRef.current ?? undefined;
        if (loginId) {
          oauthLog("页面卸载，准备取消授权流程", { loginId });
          codexService.cancelCodexOAuthLogin(loginId).catch(() => {});
        }
        oauthActiveRef.current = false;
        oauthCompletingRef.current = false;
        oauthLoginIdRef.current = null;
      },
      [oauthLog],
    );
  
    const handleCopyOauthUrl = async () => {
      if (!oauthUrl) return;
      try {
        await navigator.clipboard.writeText(oauthUrl);
        setOauthUrlCopied(true);
        setTimeout(() => setOauthUrlCopied(false), 1200);
      } catch {}
    };
  
    const handleReleaseOauthPort = async () => {
      const port = oauthPortInUse;
      if (!port) return;
      const confirmed = await confirmDialog(
        t("codex.oauth.portInUseConfirm", { port }),
        {
          title: t("codex.oauth.portInUseTitle"),
          kind: "warning",
          okLabel: t("common.confirm"),
          cancelLabel: t("common.cancel"),
        },
      );
      if (!confirmed) return;
      setOauthPrepareError(null);
      try {
        await codexService.closeCodexOAuthPort();
      } catch (e) {
        setOauthPrepareError(
          t("codex.oauth.portCloseFailed", { error: String(e) }),
        );
        setOauthPortInUse(port);
        return;
      }
      prepareOauthUrl();
    };
  
    const handleRetryOauthAfterTimeout = () => {
      oauthActiveRef.current = false;
      oauthLoginIdRef.current = null;
      setOauthTimeoutInfo(null);
      setOauthPrepareError(null);
      setOauthPortInUse(null);
      setOauthUrl("");
      setOauthUrlCopied(false);
      setOauthCallbackInput("");
      setOauthCallbackSubmitting(false);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
      prepareOauthUrl();
    };
  
    const handleOpenOauthUrl = async () => {
      if (!oauthUrl) return;
      try {
        await openUrl(oauthUrl);
      } catch {
        await navigator.clipboard.writeText(oauthUrl).catch(() => {});
        setOauthUrlCopied(true);
        setTimeout(() => setOauthUrlCopied(false), 1200);
      }
    };
  
    const handleOpenOauthIncognitoWindow = async () => {
      if (!oauthUrl) return;
      setAddStatus("idle");
      setAddMessage("");
      try {
        await codexService.openCodexOAuthIncognitoWindow(oauthUrl);
      } catch (error) {
        setAddStatus("error");
        setAddMessage(
          t("common.shared.oauth.failed", "授权失败") +
            ": " +
            String(error).replace(/^Error:\s*/, ""),
        );
      }
    };
  
    const handleStartDeviceAuth = async () => {
      if (
        deviceAuthStarting ||
        oauthCompletingRef.current ||
        (oauthMethod === "device" && deviceAuthInfo)
      )
        return;
      setOauthMethod("device");
      setDeviceAuthStarting(true);
      setDeviceAuthError(null);
      setOauthPrepareError(null);
      const currentLoginId = oauthLoginIdRef.current;
      if (currentLoginId) {
        await codexService.cancelCodexOAuthLogin(currentLoginId).catch(() => {});
      }
      oauthAttemptSeqRef.current += 1;
      oauthActiveRef.current = false;
      oauthLoginIdRef.current = null;
      setOauthUrl(null);
      try {
        const info = await codexService.startCodexDeviceAuth();
        oauthLoginIdRef.current = info.loginId;
        oauthActiveRef.current = true;
        setDeviceAuthInfo(info);
        setDeviceCodeCopied(false);
        setAddStatus("idle");
        setAddMessage("");
      } catch (error) {
        setDeviceAuthError(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setDeviceAuthStarting(false);
      }
    };
  
    const handleSwitchBrowserOAuth = async () => {
      if (oauthMethod === "browser" || oauthCompletingRef.current) return;
      const currentLoginId = oauthLoginIdRef.current;
      oauthAttemptSeqRef.current += 1;
      oauthActiveRef.current = false;
      oauthLoginIdRef.current = null;
      if (currentLoginId) {
        await codexService.cancelCodexOAuthLogin(currentLoginId).catch(() => {});
      }
      setOauthMethod("browser");
      setDeviceAuthInfo(null);
      setDeviceAuthError(null);
      setDeviceAuthStarting(false);
      setDeviceCodeCopied(false);
      setOauthPrepareError(null);
      setOauthTimeoutInfo(null);
      setOauthUrl(null);
      prepareOauthUrl();
    };
  
    const handleCopyDeviceCode = async () => {
      if (!deviceAuthInfo?.userCode) return;
      try {
        await navigator.clipboard.writeText(deviceAuthInfo.userCode);
        setDeviceCodeCopied(true);
        window.setTimeout(() => setDeviceCodeCopied(false), 1200);
      } catch (error) {
        setDeviceAuthError(String(error).replace(/^Error:\s*/, ""));
      }
    };
  
    const handleOpenDeviceAuthUrl = async () => {
      if (!deviceAuthInfo?.verificationUrl) return;
      try {
        await openUrl(deviceAuthInfo.verificationUrl);
      } catch (error) {
        setDeviceAuthError(String(error).replace(/^Error:\s*/, ""));
      }
    };
  
    const handleOpenCodexSecuritySettings = async () => {
      try {
        await openUrl("https://chatgpt.com/#settings/Security");
      } catch (error) {
        setDeviceAuthError(String(error).replace(/^Error:\s*/, ""));
      }
    };
  
    const handleSubmitOauthCallbackUrl = async () => {
      const callbackUrl = oauthCallbackInput.trim();
      if (!callbackUrl) return;
      const loginId = oauthLoginIdRef.current;
      if (!loginId) {
        setOauthCallbackError(t("common.shared.oauth.failed", "授权失败"));
        return;
      }
  
      setOauthCallbackSubmitting(true);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
      oauthCompletingRef.current = true;
      let tokenExchangeStarted = false;
      try {
        await codexService.submitCodexOAuthCallbackUrl(loginId, callbackUrl);
        setAddStatus("loading");
        setAddMessage(t("codex.oauth.exchanging", "正在交换令牌..."));
        tokenExchangeStarted = true;
        const account = await codexService.completeCodexOAuthLogin(
          loginId,
          reauthTargetAccountId || null,
        );
        await completeOauthSuccess(account);
      } catch (e) {
        completeOauthError(e, tokenExchangeStarted);
        setOauthCallbackError(String(e).replace(/^Error:\s*/, ""));
      } finally {
        oauthCompletingRef.current = false;
        setOauthCallbackSubmitting(false);
      }
    };
  
    const handleRetryOauthTokenExchange = async () => {
      const loginId = oauthLoginIdRef.current;
      if (!loginId || oauthCompletingRef.current) return;
      setOauthCallbackSubmitting(true);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
      setAddStatus("loading");
      setAddMessage(t("codex.oauth.exchanging", "正在交换令牌..."));
      oauthCompletingRef.current = true;
      try {
        const account = await codexService.completeCodexOAuthLogin(
          loginId,
          reauthTargetAccountId || null,
        );
        await completeOauthSuccess(account);
      } catch (e) {
        completeOauthError(e, true);
        setOauthCallbackError(String(e).replace(/^Error:\s*/, ""));
      } finally {
        oauthCompletingRef.current = false;
        setOauthCallbackSubmitting(false);
      }
    };
  return {
    apiBaseUrlInput,
    apiKeyFunPrefillModelCatalogRef,
    apiKeyInput,
    apiKeyInputVisible,
    apiKeyUsageDetailAccount,
    apiKeyUsageInFlightRef,
    apiKeyUsageMap,
    apiModelCatalogDraft,
    apiModelCatalogError,
    apiModelCatalogFetching,
    apiModelCatalogInput,
    apiModelCatalogSyncAvailable,
    apiModelContextWindowsInput,
    apiProviderPresetExplicitlySelectedRef,
    apiProviderPresetId,
    apiSyncModelCatalogToCodex,
    boundLocalAccessOAuthAccount,
    buildApiProviderPayload,
    cockpitApiPanelAccount,
    customSortDropTargetId,
    customSortOrder,
    deepSeekUsageRetryIdsRef,
    defaultApiProviderPresetId,
    deviceAuthError,
    deviceAuthInfo,
    deviceAuthStarting,
    deviceCodeCopied,
    draggedCustomSortAccountId,
    editingApiBaseUrlCredentialsValue,
    editingApiKeyCredentialsId,
    editingApiKeyCredentialsValue,
    editingApiKeyCredentialsVisible,
    editingApiKeyNameId,
    editingApiKeyNameValue,
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
    handleCopyDeviceCode,
    handleCopyOauthUrl,
    handleOpenCodexSecuritySettings,
    handleOpenDeviceAuthUrl,
    handleOpenOauthIncognitoWindow,
    handleOpenOauthUrl,
    handlePendingOAuthEmailInputChange,
    handleReleaseOauthPort,
    handleRetryOauthAfterTimeout,
    handleRetryOauthTokenExchange,
    handleSavePendingOAuthAccount,
    handleStartDeviceAuth,
    handleSubmitOauthCallbackUrl,
    handleSwitchBrowserOAuth,
    inlineRenameDiscardRef,
    isLocalAccessOAuthBinding,
    isOauthTimeoutState,
    isOauthTokenExchangeErrorState,
    managedProviderApiKeyId,
    managedProviderId,
    managedProviders,
    managedProvidersLoading,
    newManagedProviderNameInput,
    oauthAccounts,
    oauthBindingAccount,
    oauthBindingAccountId,
    oauthBindingAutoSwitch,
    oauthBindingEligibleAccounts,
    oauthBindingError,
    oauthBindingErrorScrollKey,
    oauthBindingHasExistingBinding,
    oauthBindingHourlyReserveDraft,
    oauthBindingHourlyReserveInputRef,
    oauthBindingQuotaReserve,
    oauthBindingQuotaReserveEditorOpen,
    oauthBindingQuotaReserveFieldErrors,
    oauthBindingSaving,
    oauthBindingSelectedAccountId,
    oauthBindingTargetActive,
    oauthBindingTargetKind,
    oauthBindingWeeklyReserveDraft,
    oauthBindingWeeklyReserveInputRef,
    oauthCallbackError,
    oauthCallbackInput,
    oauthCallbackSubmitting,
    oauthCompletingRef,
    oauthLoginIdRef,
    oauthMethod,
    oauthPortInUse,
    oauthPrepareError,
    oauthTimeoutInfo,
    oauthUrl,
    oauthUrlCopied,
    pendingApiKeyFunCodexPrefillRef,
    quickSwitchAccount,
    quickSwitchAccountId,
    quickSwitchApiKeyId,
    quickSwitchError,
    quickSwitchProviderId,
    quickSwitchSubmitting,
    reloadManagedProviders,
    resolveManagedProviderIdForAccount,
    savingApiKeyCredentials,
    savingApiKeyNameId,
    selectedApiProviderPreset,
    selectedEditingApiProviderPreset,
    selectedEditingManagedProvider,
    selectedEditingManagedProviderApiKey,
    selectedManagedProvider,
    selectedManagedProviderApiKey,
    selectedOAuthBindingAccount,
    selectedQuickSwitchApiKey,
    selectedQuickSwitchProvider,
    selectedSponsorApiProviderTemplate,
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
    setCustomSortDropTargetId,
    setCustomSortOrder,
    setDraggedCustomSortAccountId,
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
    setManagedProviderApiKeyId,
    setManagedProviderId,
    setManagedProviders,
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
    setOauthCallbackInput,
    setQuickSwitchAccountId,
    setQuickSwitchApiKeyId,
    setQuickSwitchError,
    setQuickSwitchProviderId,
    setQuickSwitchSubmitting,
    setSavingApiKeyCredentials,
    setSavingApiKeyNameId,
    setShowCustomSortModal,
    setSwitching,
    setVisibleApiKeyAccountIds,
    showAdditionalQuota,
    showCodeReviewQuota,
    showCustomSortModal,
    skipManagedProviderApiKeyAutofillRef,
    sponsorApiProviderTemplates,
    switching,
    visibleApiKeyAccountIds,
  };
}
