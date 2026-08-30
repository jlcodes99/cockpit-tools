import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { Download, Copy, Check, Eye, EyeOff, FileText, FolderOpen } from "lucide-react";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import { useCodexInstanceStore } from "../stores/useCodexInstanceStore";
import * as codexService from "../services/codexService";
import * as codexInstanceService from "../services/codexInstanceService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { maskJsonPreviewContent } from "../components/ExportJsonModal";
import { useModalErrorState } from "../components/ModalErrorMessage";
import { type CodexAccountGroup, assignAccountsToCodexGroup, getCodexAccountGroups } from "../services/codexAccountGroupService";
import { formatCodexResetTime, formatCodexResetTimeAbsolute, isCodexOpaqueAccessTokenOnlyAccount, type CodexBatchDeleteJobStatus, type CodexResetCredit, type CodexResetCreditsSnapshot } from "../types/codex";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { type CodexWindowStats } from "../utils/codexWindowStats";
import { readCodexImportSyncApiService, writeCodexImportSyncApiService } from "../utils/codexImportPreferences";
import { CODEX_OPEN_ADD_ACCOUNT_EVENT, takePendingCodexOpenAddAccountRequest, type CodexOAuthBindingRetryDetail, type CodexOpenAddAccountDetail } from "../utils/codexAddAccountRequest";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import type { CodexTab } from "../components/CodexOverviewTabsHeader";
import { useDeepSeekDirectModelPrompt } from "../components/codex/DeepSeekDirectModelModal";
import { type CodexWakeupTestOpenRequest } from "../components/codex/CodexWakeupContent";
import { CodexSpeedSelect } from "../components/codex/CodexSpeedSelect";
import { useProviderAccountsPage } from "../hooks/useProviderAccountsPage";
import { usePlatformRuntimeSupport } from "../hooks/usePlatformRuntimeSupport";
import { useEscClose } from "../hooks/useEscClose";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import type { SingleSelectFilterOption } from "../components/SingleSelectFilterDropdown";
import type { CodexAccount, CodexAppSpeed } from "../types/codex";
import type { CodexLocalAccessAddressKind, CodexLocalAccessState } from "../types/codexLocalAccess";
import { CODEX_API_SERVICE_BIND_ID, type InstanceDefaults } from "../types/instance";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import { readCodexCustomSortActive } from "../utils/codexAccountOverview";
import { useSponsorStore } from "../stores/useSponsorStore";
import { buildCodexExportContent, buildCodexExportFileNameBase, hasCodexExportAgentIdentity, hasCodexExportSensitiveNotes, type CodexExportFormat } from "../utils/codexExportFormats";
import { readAccountsOverviewFilterField, readAccountsOverviewFilterPersistenceEnabled, readAccountsOverviewFilterStringArray, removeAccountsOverviewFilterField, writeAccountsOverviewFilterField } from "../utils/accountsOverviewFilterPersistence";
import { isCodexLocalAccessRiskNoticeDismissed, setCodexLocalAccessRiskNoticeDismissed, type CodexLocalAccessRiskNoticeAction } from "../utils/codexLocalAccessRiskNotice";
import { getMfaOtpToken, getMfaTimeRemaining, loadSavedMfaRecords, parseMfaCredentialInput, upsertSavedMfaRecord, type MfaRecord } from "../utils/mfaVault";
import { findFirstMailVerificationCode } from "../utils/mailVerificationCode";
import { ACTIVE_GROUP_ID_FIELD, buildCodexAccountNoteForm, buildExportFileName, CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY, CODEX_FILTER_PERSISTENCE_SCOPE, CODEX_HIDE_RELAY_QUOTA_LEGACY_KEY, CODEX_LOCAL_ACCESS_EXPANDED_KEY, CODEX_OVERVIEW_LAYOUT_MODE_KEY, EMPTY_CODEX_ACCOUNT_NOTE_FORM, EXPIRY_FILTER_FIELD, FILTER_TYPES_FIELD, getCodexAccountNoteTitle, getDirectoryPath, GROUP_FILTER_FIELD, hasCodexAccountNoteDetails, hasCodexAccountNoteFormDetails, isHttpLikeUrl, joinFilePath, normalizeCodexOverviewLayoutMode, normalizeHttpBaseUrl, readStoredLocalAccessAddressKind, SEARCH_QUERY_FIELD, shouldAutoHideBatchDeleteJob, type CodexAccountNoteFieldErrors, type CodexAccountNoteFormState, type CodexAccountNoteMailPreviewSnapshot, type CodexAccountNoteMailPreviewState, type CodexBatchImportFilter, type CodexCliLaunchModalState, type CodexOverviewGeneralConfig, type CodexOverviewLayoutMode } from "./codexAccountsControllerModel";

/** 封装 useCodexAccountsPageController 的 useCodexAccountsBaseController 业务域状态与动作。 */
export function useCodexAccountsBaseController() {
  const isMacOS = usePlatformRuntimeSupport("macos-only");
    const isWindows = usePlatformRuntimeSupport("windows-only");
    const isCliLaunchSupported = isMacOS || isWindows;
    const sponsorModule = useSponsorStore((state) => state.state.sponsorModule);
    const fetchSponsorState = useSponsorStore((state) => state.fetchState);
    const [activeTab, setActiveTab] = useState<CodexTab>("overview");
    const [sessionWindowStats, setSessionWindowStats] = useState<{
      ready: boolean;
      byAccountId: Record<
        string,
        { primary?: CodexWindowStats; secondary?: CodexWindowStats }
      >;
    }>({ ready: false, byAccountId: {} });
    const [wakeupPresetManagerSignal, setWakeupPresetManagerSignal] = useState(0);
    const [fullQuotaWakeupOpenRequest, setFullQuotaWakeupOpenRequest] =
      useState<CodexWakeupTestOpenRequest | null>(null);
    const fullQuotaWakeupOpenSignalRef = useRef(0);
    const untaggedKey = "__untagged__";
    const [filterTypes, setFilterTypes] = useState<string[]>(() =>
      readAccountsOverviewFilterPersistenceEnabled(CODEX_FILTER_PERSISTENCE_SCOPE)
        ? readAccountsOverviewFilterStringArray(
            CODEX_FILTER_PERSISTENCE_SCOPE,
            FILTER_TYPES_FIELD,
          )
        : [],
    );
    const [exportFormat, setExportFormat] =
      useState<CodexExportFormat>("cockpit_tools");
    const [includeExportSensitiveNotes, setIncludeExportSensitiveNotes] =
      useState(false);
    const [exportFileNameBase, setExportFileNameBase] =
      useState("codex_accounts");
    const [formattedExportJsonCopied, setFormattedExportJsonCopied] =
      useState(false);
    const [formattedSavingExportJson, setFormattedSavingExportJson] =
      useState(false);
    const [formattedExportSavedPath, setFormattedExportSavedPath] = useState<
      string | null
    >(null);
    const [
      formattedExportSavedPathIsDirectory,
      setFormattedExportSavedPathIsDirectory,
    ] = useState(false);
    const [formattedExportPathCopied, setFormattedExportPathCopied] =
      useState(false);
    const [formattedBatchSavingExportJson, setFormattedBatchSavingExportJson] =
      useState(false);
    const [formattedSavingExportDocumentId, setFormattedSavingExportDocumentId] =
      useState<string | null>(null);
    const {
      message: exportModalError,
      scrollKey: exportModalErrorScrollKey,
      report: reportExportModalError,
      clear: clearExportModalError,
    } = useModalErrorState();
  
    // ─── Codex 账号分组 ────────────────────────────────────────────
    const [codexGroups, setCodexGroups] = useState<CodexAccountGroup[]>([]);
    const [groupFilter, setGroupFilter] = useState<string[]>(() =>
      readAccountsOverviewFilterPersistenceEnabled(CODEX_FILTER_PERSISTENCE_SCOPE)
        ? readAccountsOverviewFilterStringArray(
            CODEX_FILTER_PERSISTENCE_SCOPE,
            GROUP_FILTER_FIELD,
          )
        : [],
    );
    const [activeGroupId, setActiveGroupId] = useState<string | null>(() => {
      if (
        !readAccountsOverviewFilterPersistenceEnabled(
          CODEX_FILTER_PERSISTENCE_SCOPE,
        )
      ) {
        return null;
      }
      const saved = readAccountsOverviewFilterField<string | null>(
        CODEX_FILTER_PERSISTENCE_SCOPE,
        ACTIVE_GROUP_ID_FIELD,
        null,
      );
      return typeof saved === "string" && saved.trim() ? saved : null;
    });
    const [showCodexGroupModal, setShowCodexGroupModal] = useState(false);
    const [showAddToCodexGroupModal, setShowAddToCodexGroupModal] =
      useState(false);
    const [groupQuickAddGroupId, setGroupQuickAddGroupId] = useState<
      string | null
    >(null);
    const [codexAddTargetGroupId, setCodexAddTargetGroupId] = useState<
      string | null
    >(null);
    const [batchImportTargetGroupId, setBatchImportTargetGroupId] = useState<
      string | null
    >(null);
    const [groupDeleteConfirm, setGroupDeleteConfirm] = useState<{
      id: string;
      name: string;
    } | null>(null);
    const {
      message: groupDeleteError,
      scrollKey: groupDeleteErrorScrollKey,
      set: setGroupDeleteError,
    } = useModalErrorState();
    const [deletingGroup, setDeletingGroup] = useState(false);
    const [refreshingGroupId, setRefreshingGroupId] = useState<string | null>(
      null,
    );
    const [refreshingSubscriptionAccountId, setRefreshingSubscriptionAccountId] =
      useState<string | null>(null);
    const [resettingResetCreditAccountId, setResettingResetCreditAccountId] =
      useState<string | null>(null);
    const [resetCreditConfirmAccountId, setResetCreditConfirmAccountId] =
      useState<string | null>(null);
    const [resetCreditConfirmSnapshot, setResetCreditConfirmSnapshot] =
      useState<CodexResetCreditsSnapshot | null>(null);
    const [resetCreditConfirmLoading, setResetCreditConfirmLoading] =
      useState(false);
    const resetCreditConfirmRequestSeqRef = useRef(0);
    const [resetCreditConfirmActionLocked, setResetCreditConfirmActionLocked] =
      useState(false);
    const {
      message: resetCreditConfirmError,
      scrollKey: resetCreditConfirmErrorScrollKey,
      set: setResetCreditConfirmError,
    } = useModalErrorState();
    const [removingGroupAccountIds, setRemovingGroupAccountIds] = useState<
      Set<string>
    >(new Set());
    const [localAccessState, setLocalAccessState] =
      useState<CodexLocalAccessState | null>(null);
    const localAccessStateRequestSeqRef = useRef(0);
    const [showLocalAccessModal, setShowLocalAccessModal] = useState(false);
    const [showLocalAccessHealthModal, setShowLocalAccessHealthModal] =
      useState(false);
    const [localAccessHealthActionBusy, setLocalAccessHealthActionBusy] =
      useState(false);
    const [localAccessModalMode, setLocalAccessModalMode] = useState<
      "panel" | "members"
    >("panel");
    const [localAccessSaving, setLocalAccessSaving] = useState(false);
    const [addingLocalAccessAccountId, setAddingLocalAccessAccountId] = useState<
      string | null
    >(null);
    const [localAccessStarting, setLocalAccessStarting] = useState(false);
    const [syncImportedToApiService, setSyncImportedToApiService] = useState(
      readCodexImportSyncApiService,
    );
    const [importApiServiceGuideCount, setImportApiServiceGuideCount] = useState<
      number | null
    >(null);
    const [externalImportSyncError, setExternalImportSyncError] = useState<
      string | null
    >(null);
    const [localAccessRefreshing, setLocalAccessRefreshing] = useState(false);
    const [localAccessPortKilling, setLocalAccessPortKilling] = useState(false);
    const [localAccessSidecarRestarting, setLocalAccessSidecarRestarting] =
      useState(false);
    const [showLocalAccessHideConfirm, setShowLocalAccessHideConfirm] =
      useState(false);
    const [localAccessHideSubmitting, setLocalAccessHideSubmitting] =
      useState(false);
    const [localAccessRiskNoticeAction, setLocalAccessRiskNoticeAction] =
      useState<CodexLocalAccessRiskNoticeAction | null>(null);
    const [localAccessRiskNoticeRemember, setLocalAccessRiskNoticeRemember] =
      useState(false);
    const [localAccessCopiedField, setLocalAccessCopiedField] = useState<
      "baseUrl" | "apiKey" | null
    >(null);
    const [localAccessKeyVisible, setLocalAccessKeyVisible] = useState(false);
    const [localAccessAddressKind, setLocalAccessAddressKind] =
      useState<CodexLocalAccessAddressKind>(() =>
        readStoredLocalAccessAddressKind(),
      );
    const [localAccessEntryVisible, setLocalAccessEntryVisible] = useState(true);
    const [localAccessLaunchCurrent, setLocalAccessLaunchCurrent] =
      useState(false);
    const [showLocalAccessQuotaStatsModal, setShowLocalAccessQuotaStatsModal] =
      useState(false);
    const localAccessRiskNoticeResolverRef = useRef<
      ((accepted: boolean) => void) | null
    >(null);
    const [localAccessDetailsExpanded, setLocalAccessDetailsExpanded] =
      useState<boolean>(() => {
        try {
          return localStorage.getItem(CODEX_LOCAL_ACCESS_EXPANDED_KEY) === "1";
        } catch {
          return false;
        }
      });
    const ensureLocalAccessEntryVisible = useCallback(async () => {
      if (localAccessEntryVisible) return;
      await invoke("set_codex_local_access_entry_visible", { enabled: true });
      setLocalAccessEntryVisible(true);
      window.dispatchEvent(new Event("codex-local-access-state-updated"));
      window.dispatchEvent(new Event("config-updated"));
    }, [localAccessEntryVisible]);
    const handleExternalImportedAccounts = useCallback(
      async (accountIds: string[]) => {
        setExternalImportSyncError(null);
        if (!readCodexImportSyncApiService()) return;
        try {
          const result =
            await codexLocalAccessService.appendCodexLocalAccessAccounts(
              accountIds,
            );
          setLocalAccessState(result.state);
          if (result.syncedAccountIds.length > 0) {
            await ensureLocalAccessEntryVisible();
            setImportApiServiceGuideCount(result.syncedAccountIds.length);
          }
        } catch (error) {
          setExternalImportSyncError(String(error).replace(/^Error:\s*/, ""));
        }
      },
      [ensureLocalAccessEntryVisible],
    );
  
    const [codexGroupsReady, setCodexGroupsReady] = useState(false);
    const reloadCodexGroups = useCallback(async () => {
      setCodexGroups(await getCodexAccountGroups());
      setCodexGroupsReady(true);
    }, []);
  
    const codexAddTargetGroup = useMemo(() => {
      if (!codexAddTargetGroupId) return null;
      return (
        codexGroups.find((group) => group.id === codexAddTargetGroupId) ?? null
      );
    }, [codexAddTargetGroupId, codexGroups]);
  
    const resolveValidCodexGroupId = useCallback(
      (groupId?: string | null) => {
        const normalized = groupId?.trim();
        if (!normalized) return null;
        return codexGroups.some((group) => group.id === normalized)
          ? normalized
          : null;
      },
      [codexGroups],
    );
  
    const assignCodexAccountsToTargetGroup = useCallback(
      async (
        targetAccounts: Array<CodexAccount | null | undefined>,
        targetGroupId = codexAddTargetGroupId,
      ) => {
        const resolvedGroupId = resolveValidCodexGroupId(targetGroupId);
        if (!resolvedGroupId) return;
  
        const accountIds = Array.from(
          new Set(
            targetAccounts
              .map((account) => account?.id?.trim())
              .filter((id): id is string => Boolean(id)),
          ),
        );
        if (accountIds.length === 0) return;
  
        await assignAccountsToCodexGroup(resolvedGroupId, accountIds);
        await reloadCodexGroups();
      },
      [codexAddTargetGroupId, reloadCodexGroups, resolveValidCodexGroupId],
    );
  
    useEffect(() => {
      reloadCodexGroups();
    }, [reloadCodexGroups]);
  
    useEffect(
      () => () => {
        if (localAccessRiskNoticeResolverRef.current) {
          localAccessRiskNoticeResolverRef.current(false);
          localAccessRiskNoticeResolverRef.current = null;
        }
      },
      [],
    );
  
    const closeLocalAccessRiskNotice = useCallback(
      (accepted: boolean) => {
        if (accepted && localAccessRiskNoticeRemember) {
          setCodexLocalAccessRiskNoticeDismissed(true);
        }
        const resolver = localAccessRiskNoticeResolverRef.current;
        localAccessRiskNoticeResolverRef.current = null;
        setLocalAccessRiskNoticeAction(null);
        setLocalAccessRiskNoticeRemember(false);
        resolver?.(accepted);
      },
      [localAccessRiskNoticeRemember],
    );
  
    const requestLocalAccessRiskNotice = useCallback(
      (action: CodexLocalAccessRiskNoticeAction): Promise<boolean> => {
        if (isCodexLocalAccessRiskNoticeDismissed()) {
          return Promise.resolve(true);
        }
        setLocalAccessRiskNoticeRemember(false);
        setLocalAccessRiskNoticeAction(action);
        return new Promise<boolean>((resolve) => {
          localAccessRiskNoticeResolverRef.current = resolve;
        });
      },
      [],
    );
  
    const toggleGroupFilterValue = useCallback((groupId: string) => {
      setGroupFilter((prev) => {
        if (prev.includes(groupId)) return prev.filter((id) => id !== groupId);
        return [...prev, groupId];
      });
    }, []);
  
    const clearGroupFilter = useCallback(() => {
      setGroupFilter([]);
    }, []);
  
    /** Drop stale group filter IDs after groups are loaded (not on empty initial state). */
    useEffect(() => {
      if (!codexGroupsReady) return;
      const validIds = new Set(codexGroups.map((group) => group.id));
      setGroupFilter((prev) => {
        if (prev.length === 0) return prev;
        const next = prev.filter((id) => validIds.has(id));
        return next.length === prev.length ? prev : next;
      });
    }, [codexGroups, codexGroupsReady]);
  
    const [overviewLayoutMode, setOverviewLayoutMode] =
      useState<CodexOverviewLayoutMode>(() => {
        try {
          const saved = normalizeCodexOverviewLayoutMode(
            localStorage.getItem(CODEX_OVERVIEW_LAYOUT_MODE_KEY),
          );
          if (saved) return saved;
          const legacy = normalizeCodexOverviewLayoutMode(
            localStorage.getItem("agtools.codex.accounts_view_mode"),
          );
          if (legacy === "list" || legacy === "grid") return legacy;
        } catch {
          // ignore persistence failures
        }
        return "grid";
      });
    const [hideRelayQuota, setHideRelayQuota] = useState(false);
    const store = useCodexAccountStore();
    const codexInstanceStore = useCodexInstanceStore();
    const [cliLaunchingAccountId, setCliLaunchingAccountId] = useState<
      string | null
    >(null);
    const [cliLaunchModal, setCliLaunchModal] =
      useState<CodexCliLaunchModalState | null>(null);
    const deepSeekStart = useDeepSeekDirectModelPrompt();
    const codexCliInstanceDefaultsRef = useRef<InstanceDefaults | null>(null);
    const { terminalOptions, selectedTerminal, setSelectedTerminal } =
      useLaunchTerminalOptions(isCliLaunchSupported);
    const closeCliLaunchModal = useCallback(() => {
      setCliLaunchModal(null);
      setCliLaunchingAccountId(null);
    }, []);
    useEscClose(Boolean(cliLaunchModal), closeCliLaunchModal);
    const [cockpitApiPanelAccountId, setCockpitApiPanelAccountId] = useState<
      string | null
    >(null);
    const [apiKeyUsageDetailAccountId, setApiKeyUsageDetailAccountId] = useState<
      string | null
    >(null);
    const [quotaErrorDetail, setQuotaErrorDetail] = useState<{
      accountName: string;
      title?: string;
      summary?: string;
      reauthorizeAccountId?: string;
      clearClientAuthObservationAccountId?: string;
      message: string;
    } | null>(null);
    const [editingAccountNoteId, setEditingAccountNoteId] = useState<
      string | null
    >(null);
    const [editingAccountNoteForm, setEditingAccountNoteForm] =
      useState<CodexAccountNoteFormState>(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
    const [accountNoteFieldErrors, setAccountNoteFieldErrors] =
      useState<CodexAccountNoteFieldErrors>({});
    const [accountNoteSecretVisible, setAccountNoteSecretVisible] =
      useState(true);
    const [accountNotePasswordVisible, setAccountNotePasswordVisible] =
      useState(true);
    const [accountNoteCopiedKey, setAccountNoteCopiedKey] = useState<
      string | null
    >(null);
    const [pendingOAuthEmailInput, setPendingOAuthEmailInput] = useState("");
    const [pendingOAuthNoteForm, setPendingOAuthNoteForm] =
      useState<CodexAccountNoteFormState>(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
    const [pendingOAuthFieldErrors, setPendingOAuthFieldErrors] = useState<
      CodexAccountNoteFieldErrors & { email?: string }
    >({});
    const [pendingOAuthNoteModalOpen, setPendingOAuthNoteModalOpen] =
      useState(false);
    const [savingPendingOAuthAccount, setSavingPendingOAuthAccount] =
      useState(false);
    const [savedMfaRecords, setSavedMfaRecords] = useState<MfaRecord[]>([]);
    const [accountNoteMfaPickerOpen, setAccountNoteMfaPickerOpen] =
      useState(false);
    const [accountNoteMailPreview, setAccountNoteMailPreview] =
      useState<CodexAccountNoteMailPreviewState | null>(null);
    const [accountNoteMailPreviewLoading, setAccountNoteMailPreviewLoading] =
      useState(false);
    const [accountNoteMailPreviewError, setAccountNoteMailPreviewError] =
      useState<string | null>(null);
    const accountNoteMailPreviewSeqRef = useRef(0);
    const accountNoteMailPreviewSnapshotRef =
      useRef<CodexAccountNoteMailPreviewSnapshot | null>(null);
    const [mfaTimeRemaining, setMfaTimeRemaining] = useState(getMfaTimeRemaining);
    const [savingAccountNote, setSavingAccountNote] = useState(false);
    const [savingAppSpeedId, setSavingAppSpeedId] = useState<string | null>(null);
    const [apiServiceAppSpeed, setApiServiceAppSpeed] =
      useState<CodexAppSpeed>("standard");
    const [reauthTargetAccount, setReauthTargetAccount] =
      useState<CodexAccount | null>(null);
    const [reauthRetrySwitchAccountId, setReauthRetrySwitchAccountId] = useState<
      string | null
    >(null);
    const [reauthRetryLaunchAfterSwitch, setReauthRetryLaunchAfterSwitch] =
      useState<boolean | undefined>(undefined);
    const [reauthRetryInstanceId, setReauthRetryInstanceId] = useState<
      string | null
    >(null);
    const [reauthRetryOAuthBinding, setReauthRetryOAuthBinding] =
      useState<CodexOAuthBindingRetryDetail | null>(null);
    const [reauthEmailCopied, setReauthEmailCopied] = useState(false);
    const {
      message: accountNoteError,
      scrollKey: accountNoteErrorScrollKey,
      set: setAccountNoteError,
    } = useModalErrorState();
  
    useEffect(() => {
      const timer = window.setInterval(() => {
        setMfaTimeRemaining(getMfaTimeRemaining());
      }, 1000);
      return () => window.clearInterval(timer);
    }, []);
  
    // Use the common hook WITHOUT oauthService since Codex uses Tauri event-based OAuth
    // Codex batch-delete confirm is wired after confirmCodexDelete is defined.
    // Built-in Enter confirm is disabled here so it cannot call generic confirmDelete.
    const page = useProviderAccountsPage<CodexAccount>({
      platformKey: "Codex",
      oauthLogPrefix: "CodexOAuth",
      exportFilePrefix: "codex_accounts",
      store: {
        accounts: store.accounts,
        loading: store.loading,
        error: store.error,
        fetchAccounts: store.fetchAccounts,
        switchAccount: store.switchAccount,
        deleteAccounts: store.deleteAccounts,
        refreshToken: (id) => store.refreshQuota(id).then(() => {}),
        refreshAllTokens: () => store.refreshAllQuotas().then(() => {}),
        updateAccountTags: store.updateAccountTags,
      },
      dataService: {
        importFromJson: codexService.importCodexFromJson,
        exportAccounts: codexService.exportCodexAccounts,
      },
      getDisplayEmail: (account) => account.email ?? account.id,
      initialSearchQuery: readAccountsOverviewFilterPersistenceEnabled(
        CODEX_FILTER_PERSISTENCE_SCOPE,
      )
        ? readAccountsOverviewFilterField(
            CODEX_FILTER_PERSISTENCE_SCOPE,
            SEARCH_QUERY_FIELD,
            "",
          )
        : "",
      // Prefer custom sort whenever the dedicated flag is set (#1123).
      defaultSortBy: readCodexCustomSortActive() ? "custom" : undefined,
      onExternalImportCompleted: handleExternalImportedAccounts,
      disableEnterConfirmDelete: true,
    });
  
    const {
      t,
      maskAccountText,
      privacyModeEnabled,
      togglePrivacyMode,
      viewMode,
      setViewMode,
      searchQuery,
      setSearchQuery,
      filterPersistenceEnabled,
      filterPersistenceScope,
      sortBy,
      setSortBy,
      sortDirection,
      setSortDirection,
      selected,
      setSelected,
      toggleSelect,
      toggleSelectAll,
      tagFilter,
      groupByTag,
      setGroupByTag,
      showTagFilter,
      setShowTagFilter,
      showTagModal,
      setShowTagModal,
      tagFilterRef,
      availableTags,
      toggleTagFilterValue,
      clearTagFilter,
      tagDeleteConfirm,
      tagDeleteConfirmError,
      tagDeleteConfirmErrorScrollKey,
      setTagDeleteConfirm,
      deletingTag,
      requestDeleteTag,
      confirmDeleteTag,
      openTagModal,
      handleSaveTags,
      refreshing,
      refreshingAll,
      handleRefresh,
      handleRefreshAll,
      handleDelete,
      deleteConfirm,
      deleteConfirmError,
      deleteConfirmErrorScrollKey,
      setDeleteConfirm,
      message,
      setMessage,
      exporting,
      handleExport: handleBaseExport,
      handleExportByIds: handleBaseExportByIds,
      getScopedSelectedCount,
      showExportModal,
      closeExportModal,
      exportJsonContent,
      exportJsonHidden,
      toggleExportJsonHidden,
      showAddModal,
      addTab,
      addStatus,
      addMessage,
      tokenInput,
      setTokenInput,
      importing,
      setImporting,
      openAddModal,
      closeAddModal,
      externalImportProgress,
      closeExternalImportProgressModal,
      formatDate,
      normalizeTag,
      saveJsonFile,
    } = page;
    const [isAllFilteredSelected, setIsAllFilteredSelected] = useState(false);
  
    /** Clear every overview filter so the table matches the full account total. */
    const clearAllOverviewFilters = useCallback(() => {
      setSearchQuery("");
      setFilterTypes([]);
      clearTagFilter();
      setGroupFilter([]);
      setActiveGroupId(null);
      setSelected(new Set());
    }, [clearTagFilter, setSearchQuery, setSelected]);
  
    const handleSyncImportedToApiServiceChange = useCallback(
      (enabled: boolean) => {
        setSyncImportedToApiService(enabled);
        writeCodexImportSyncApiService(enabled);
      },
      [],
    );
  
    const syncImportedAccountsToApiService = useCallback(
      async (accountIds: string[], force = false) => {
        if ((!syncImportedToApiService && !force) || accountIds.length === 0)
          return null;
        const result =
          await codexLocalAccessService.appendCodexLocalAccessAccounts(
            accountIds,
          );
        setLocalAccessState(result.state);
        if (result.syncedAccountIds.length > 0) {
          await ensureLocalAccessEntryVisible();
          setImportApiServiceGuideCount(result.syncedAccountIds.length);
        }
        return result;
      },
      [ensureLocalAccessEntryVisible, syncImportedToApiService],
    );
  
    const reauthTargetAccountId = reauthTargetAccount?.id?.trim() ?? "";
    const reauthTargetEmail = reauthTargetAccount?.email?.trim() ?? "";
    const shouldShowPendingOAuthDraftForm =
      addTab === "oauth" && !reauthTargetAccount;
    const pendingOAuthHasNoteDetails =
      hasCodexAccountNoteFormDetails(pendingOAuthNoteForm);
    const [batchImportOpen, setBatchImportOpen] = useState(false);
    const [batchImportSessionId, setBatchImportSessionId] = useState<
      string | null
    >(null);
    const [batchImportProgress, setBatchImportProgress] =
      useState<codexService.CodexBatchImportProgress | null>(null);
    const [batchImportPreview, setBatchImportPreview] =
      useState<codexService.CodexBatchImportPreview | null>(null);
    const [batchImportSelectedIds, setBatchImportSelectedIds] = useState<
      string[]
    >([]);
    const [batchImportFilter, setBatchImportFilter] =
      useState<CodexBatchImportFilter>("all");
    const [batchImportBusy, setBatchImportBusy] = useState(false);
    const [batchImportError, setBatchImportError] = useState<string | null>(null);
    const [batchImportResult, setBatchImportResult] =
      useState<codexService.CodexBatchImportConfirmResult | null>(null);
    const [batchImportFilePaths, setBatchImportFilePaths] = useState<string[]>(
      [],
    );
    const [batchImportCheckQuota, setBatchImportCheckQuota] = useState(false);
    const [batchImportTagsInput, setBatchImportTagsInput] = useState("");
    const [tokenImportProgress, setTokenImportProgress] = useState<{
      current: number;
      total: number;
    } | null>(null);
    const [pendingWebSessionImport, setPendingWebSessionImport] = useState<{
      content: string;
      accountLabels: string[];
    } | null>(null);
    const [batchDeleteJob, setBatchDeleteJob] =
      useState<CodexBatchDeleteJobStatus | null>(null);
    const [batchDeleteBusy, setBatchDeleteBusy] = useState(false);
    const [batchDeleteModalError, setBatchDeleteModalError] = useState<
      string | null
    >(null);
    const batchImportUnlistenersRef = useRef<UnlistenFn[]>([]);
    const batchImportSessionIdRef = useRef<string | null>(null);
    const batchDeleteRemoveIdsRef = useRef<Set<string>>(new Set());
    const batchDeleteRefreshedCompletedRef = useRef(0);
    const codexAccountsRef = useRef<CodexAccount[]>(store.accounts);
    const codexCurrentAccountRef = useRef<CodexAccount | null>(
      store.currentAccount,
    );
    const fetchCodexAccounts = store.fetchAccounts;
    const fetchCodexCurrentAccount = store.fetchCurrentAccount;
  
    useEffect(() => {
      codexAccountsRef.current = store.accounts;
      codexCurrentAccountRef.current = store.currentAccount;
    }, [store.accounts, store.currentAccount]);
  
    const getBatchDeleteRefreshOptions = useCallback(() => {
      const removeIds = batchDeleteRemoveIdsRef.current;
      const accounts = codexAccountsRef.current;
      const currentAccount = codexCurrentAccountRef.current;
      return {
        allowEmptyAccounts:
          accounts.length > 0 &&
          accounts.every((account) => removeIds.has(account.id)),
        allowEmptyCurrent: !!currentAccount && removeIds.has(currentAccount.id),
      };
    }, []);
  
    const refreshAccountsAfterBatchDelete = useCallback(async () => {
      const { allowEmptyAccounts, allowEmptyCurrent } =
        getBatchDeleteRefreshOptions();
      await fetchCodexAccounts({ allowEmpty: allowEmptyAccounts });
      await fetchCodexCurrentAccount({ allowEmpty: allowEmptyCurrent });
      await reloadCodexGroups();
      await emitAccountsChanged({
        platformId: "codex",
        reason: "delete",
      });
    }, [
      fetchCodexAccounts,
      fetchCodexCurrentAccount,
      getBatchDeleteRefreshOptions,
      reloadCodexGroups,
    ]);
  
    const refreshAccountsDuringBatchDelete = useCallback(async () => {
      const { allowEmptyAccounts, allowEmptyCurrent } =
        getBatchDeleteRefreshOptions();
      await fetchCodexAccounts({ allowEmpty: allowEmptyAccounts });
      await fetchCodexCurrentAccount({ allowEmpty: allowEmptyCurrent });
    }, [
      fetchCodexAccounts,
      fetchCodexCurrentAccount,
      getBatchDeleteRefreshOptions,
    ]);
  
    useEffect(() => {
      const jobId = batchDeleteJob?.jobId;
      if (!jobId || batchDeleteJob.status !== "running") return;
  
      let disposed = false;
      let timer: number | null = null;
      const pollJob = async () => {
        let shouldContinue = true;
        try {
          const nextJob = await codexService.getCodexBatchDelete(jobId);
          if (disposed) return;
          setBatchDeleteJob((current) =>
            current?.jobId === jobId ? nextJob : current,
          );
          if (nextJob.completed > batchDeleteRefreshedCompletedRef.current) {
            await refreshAccountsDuringBatchDelete();
            batchDeleteRefreshedCompletedRef.current = nextJob.completed;
          }
          shouldContinue = nextJob.status === "running";
        } catch (error) {
          console.warn("[Codex Batch Delete] 查询任务进度失败:", error);
        }
        if (!disposed && shouldContinue) {
          timer = window.setTimeout(pollJob, 250);
        }
      };
  
      timer = window.setTimeout(pollJob, 100);
      return () => {
        disposed = true;
        if (timer !== null) window.clearTimeout(timer);
      };
    }, [
      batchDeleteJob?.jobId,
      batchDeleteJob?.status,
      refreshAccountsDuringBatchDelete,
    ]);
  
    useEffect(() => {
      if (!shouldAutoHideBatchDeleteJob(batchDeleteJob)) return;
      let disposed = false;
      const clearCompletedJob = async () => {
        try {
          await codexService.clearCodexBatchDelete(batchDeleteJob.jobId);
        } catch {
          // ignore cleanup failures
        }
        if (disposed) return;
        await refreshAccountsAfterBatchDelete();
        if (disposed) return;
        batchDeleteRemoveIdsRef.current = new Set();
        batchDeleteRefreshedCompletedRef.current = 0;
        setBatchDeleteJob(null);
      };
      void clearCompletedJob();
      return () => {
        disposed = true;
      };
    }, [batchDeleteJob, refreshAccountsAfterBatchDelete]);
  
    const cleanupBatchImportListeners = useCallback(() => {
      for (const unlisten of batchImportUnlistenersRef.current) {
        try {
          unlisten();
        } catch {
          // ignore listener cleanup failures
        }
      }
      batchImportUnlistenersRef.current = [];
    }, []);
  
    useEffect(() => cleanupBatchImportListeners, [cleanupBatchImportListeners]);
  
    const resetBatchImportState = useCallback(() => {
      cleanupBatchImportListeners();
      batchImportSessionIdRef.current = null;
      setBatchImportOpen(false);
      setBatchImportSessionId(null);
      setBatchImportProgress(null);
      setBatchImportPreview(null);
      setBatchImportSelectedIds([]);
      setBatchImportFilter("all");
      setBatchImportBusy(false);
      setBatchImportError(null);
      setBatchImportResult(null);
      setBatchImportFilePaths([]);
      setBatchImportCheckQuota(false);
      setBatchImportTagsInput("");
      setBatchImportTargetGroupId(null);
      try {
        localStorage.removeItem(CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY);
      } catch {
        // ignore storage failures
      }
    }, [cleanupBatchImportListeners]);
  
    useEffect(() => {
      let disposed = false;
      const restoreBatchImportSession = async () => {
        let savedSessionId: string | null = null;
        try {
          savedSessionId = localStorage.getItem(
            CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY,
          );
        } catch {
          savedSessionId = null;
        }
        if (!savedSessionId || batchImportSessionIdRef.current) {
          return;
        }
        try {
          const preview =
            await codexService.getCodexBatchImportPreview(savedSessionId);
          if (disposed) return;
          // 无可导入项时丢弃残留任务，避免黄色任务条一直挂着（#1445）
          const selectableCount = (preview.items ?? []).filter(
            (item) => item.selectable && item.status !== "invalid",
          ).length;
          if (selectableCount === 0) {
            batchImportSessionIdRef.current = null;
            try {
              localStorage.removeItem(CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY);
            } catch {
              // ignore storage failures
            }
            try {
              await codexService.cancelCodexBatchImport(savedSessionId);
            } catch {
              // session may already be gone
            }
            return;
          }
          batchImportSessionIdRef.current = savedSessionId;
          setBatchImportSessionId(savedSessionId);
          setBatchImportPreview(preview);
          setBatchImportCheckQuota(preview.checkQuota);
          setBatchImportBusy(false);
          setBatchImportSelectedIds(
            preview.items
              .filter(
                (item) =>
                  item.defaultSelected &&
                  item.selectable &&
                  (item.status === "ready" || item.status === "existing"),
              )
              .map((item) => item.itemId),
          );
        } catch {
          try {
            localStorage.removeItem(CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY);
          } catch {
            // ignore storage failures
          }
        }
      };
      void restoreBatchImportSession();
      return () => {
        disposed = true;
      };
    }, []);
  
    const batchImportCounts = useMemo(() => {
      const items = batchImportPreview?.items ?? [];
      return {
        ready: items.filter((item) => item.status === "ready").length,
        quotaFailed: items.filter((item) => item.status === "quota_failed")
          .length,
        existing: items.filter((item) => item.status === "existing").length,
        invalid: items.filter((item) => item.status === "invalid").length,
      };
    }, [batchImportPreview]);
  
    const batchImportVisibleItems = useMemo(() => {
      const items = batchImportPreview?.items ?? [];
      return batchImportFilter === "ready"
        ? items.filter(
            (item) => item.status === "ready" || item.status === "existing",
          )
        : items;
    }, [batchImportFilter, batchImportPreview]);
    const batchImportSelectableIds = useMemo(
      () =>
        (batchImportPreview?.items ?? [])
          .filter((item) => item.selectable && item.status !== "invalid")
          .map((item) => item.itemId),
      [batchImportPreview],
    );
    const batchImportSelectableIdSet = useMemo(
      () => new Set(batchImportSelectableIds),
      [batchImportSelectableIds],
    );
    const batchImportSelectedSelectableCount = batchImportSelectedIds.filter(
      (id) => batchImportSelectableIdSet.has(id),
    ).length;
    const batchImportSelectedCountLabel = t(
      "codex.batchImport.selectedCount",
      "已选 {{count}}/{{total}}",
    )
      .replace("{{count}}", String(batchImportSelectedSelectableCount))
      .replace("{{total}}", String(batchImportSelectableIds.length));
    const activeBatchImportCheckQuota =
      batchImportProgress?.checkQuota ??
      batchImportPreview?.checkQuota ??
      batchImportCheckQuota;
    const batchImportProgressCurrent =
      batchImportProgress?.current ?? batchImportPreview?.items.length ?? 0;
    const batchImportProgressTotal =
      batchImportProgress?.total ?? batchImportPreview?.total ?? 0;
    const batchImportProgressPercent = batchImportProgressTotal
      ? Math.min(
          100,
          Math.round(
            (batchImportProgressCurrent / batchImportProgressTotal) * 100,
          ),
        )
      : 0;
  
    const openCodexAddModal = useCallback(
      (
        tab: string,
        targetAccount?: CodexAccount | null,
        options?: {
          retrySwitchAfterOAuth?: boolean;
          retrySwitchLaunchAfterSwitch?: boolean;
          retryInstanceLaunchAfterOAuth?: boolean;
          retryInstanceId?: string;
          retryOAuthBinding?: CodexOAuthBindingRetryDetail;
        },
      ) => {
        setReauthTargetAccount(targetAccount ?? null);
        setReauthRetrySwitchAccountId(
          targetAccount && options?.retrySwitchAfterOAuth
            ? targetAccount.id
            : null,
        );
        setReauthRetryLaunchAfterSwitch(
          targetAccount && options?.retrySwitchAfterOAuth
            ? options.retrySwitchLaunchAfterSwitch
            : undefined,
        );
        setReauthRetryInstanceId(
          targetAccount && options?.retryInstanceLaunchAfterOAuth
            ? options.retryInstanceId?.trim() || null
            : null,
        );
        setReauthRetryOAuthBinding(options?.retryOAuthBinding ?? null);
        setCodexAddTargetGroupId(
          targetAccount ? null : resolveValidCodexGroupId(activeGroupId),
        );
        setReauthEmailCopied(false);
        if (!targetAccount) {
          setPendingOAuthEmailInput("");
          setPendingOAuthNoteForm(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
        }
        setPendingOAuthFieldErrors({});
        setPendingOAuthNoteModalOpen(false);
        setPendingWebSessionImport(null);
        openAddModal(tab);
      },
      [activeGroupId, openAddModal, resolveValidCodexGroupId],
    );
  
    const closeCodexAddModal = useCallback(() => {
      if (importing) return;
      setReauthTargetAccount(null);
      setReauthRetrySwitchAccountId(null);
      setReauthRetryLaunchAfterSwitch(undefined);
      setReauthRetryInstanceId(null);
      setReauthRetryOAuthBinding(null);
      setCodexAddTargetGroupId(null);
      setReauthEmailCopied(false);
      setPendingOAuthEmailInput("");
      setPendingOAuthNoteForm(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
      setPendingOAuthFieldErrors({});
      setPendingOAuthNoteModalOpen(false);
      setPendingWebSessionImport(null);
      closeAddModal();
    }, [closeAddModal, importing]);
  
    // Keep the shared modal independent from the currently visible Codex page.
    useEffect(() => {
      const openFromRequest = async (detail?: CodexOpenAddAccountDetail) => {
        if (detail?.autoJoinApiService) {
          setSyncImportedToApiService(true);
          writeCodexImportSyncApiService(true);
        }
        let targetAccount = detail?.targetAccountId
          ? (codexAccountsRef.current.find(
              (account) => account.id === detail.targetAccountId,
            ) ?? null)
          : null;
        if (detail?.targetAccountId && !targetAccount) {
          await fetchCodexAccounts();
          targetAccount =
            useCodexAccountStore
              .getState()
              .accounts.find(
                (account) => account.id === detail.targetAccountId,
              ) ?? null;
        }
        openCodexAddModal(detail?.tab ?? "oauth", targetAccount, {
          retrySwitchAfterOAuth: detail?.retrySwitchAfterOAuth,
          retrySwitchLaunchAfterSwitch: detail?.retrySwitchLaunchAfterSwitch,
          retryInstanceLaunchAfterOAuth: detail?.retryInstanceLaunchAfterOAuth,
          retryInstanceId: detail?.retryInstanceId,
          retryOAuthBinding: detail?.retryOAuthBinding,
        });
      };
      const handleOpenAddAccount = (event: Event) => {
        const eventDetail = (event as CustomEvent<CodexOpenAddAccountDetail>)
          .detail;
        const detail = takePendingCodexOpenAddAccountRequest() ?? eventDetail;
        void openFromRequest(detail);
      };
      window.addEventListener(CODEX_OPEN_ADD_ACCOUNT_EVENT, handleOpenAddAccount);
      const pendingRequest = takePendingCodexOpenAddAccountRequest();
      if (pendingRequest) {
        void openFromRequest(pendingRequest);
      }
      return () => {
        window.removeEventListener(
          CODEX_OPEN_ADD_ACCOUNT_EVENT,
          handleOpenAddAccount,
        );
      };
    }, [fetchCodexAccounts, openCodexAddModal]);
  
    const handleCopyReauthEmail = useCallback(async () => {
      if (!reauthTargetEmail) return;
      try {
        await navigator.clipboard.writeText(reauthTargetEmail);
        setReauthEmailCopied(true);
        window.setTimeout(() => setReauthEmailCopied(false), 1200);
      } catch {}
    }, [reauthTargetEmail]);
  
    useEffect(() => {
      if (showAddModal) return;
      setReauthTargetAccount(null);
      setReauthRetrySwitchAccountId(null);
      setReauthRetryLaunchAfterSwitch(undefined);
      setReauthRetryInstanceId(null);
      setReauthRetryOAuthBinding(null);
      setCodexAddTargetGroupId(null);
      setReauthEmailCopied(false);
      setPendingOAuthEmailInput("");
      setPendingOAuthNoteForm(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
      setPendingOAuthFieldErrors({});
      setPendingOAuthNoteModalOpen(false);
    }, [showAddModal]);
  
    useEffect(() => {
      if (!filterPersistenceEnabled) {
        removeAccountsOverviewFilterField(
          filterPersistenceScope,
          SEARCH_QUERY_FIELD,
        );
        return;
      }
      writeAccountsOverviewFilterField(
        filterPersistenceScope,
        SEARCH_QUERY_FIELD,
        searchQuery,
      );
    }, [filterPersistenceEnabled, filterPersistenceScope, searchQuery]);
  
    useEffect(() => {
      if (!filterPersistenceEnabled) {
        removeAccountsOverviewFilterField(
          filterPersistenceScope,
          FILTER_TYPES_FIELD,
        );
        return;
      }
      writeAccountsOverviewFilterField(
        filterPersistenceScope,
        FILTER_TYPES_FIELD,
        filterTypes,
      );
    }, [filterPersistenceEnabled, filterPersistenceScope, filterTypes]);
  
    useEffect(() => {
      removeAccountsOverviewFilterField(
        filterPersistenceScope,
        EXPIRY_FILTER_FIELD,
      );
    }, [filterPersistenceScope]);
  
    useEffect(() => {
      if (!filterPersistenceEnabled) {
        removeAccountsOverviewFilterField(
          filterPersistenceScope,
          GROUP_FILTER_FIELD,
        );
        return;
      }
      writeAccountsOverviewFilterField(
        filterPersistenceScope,
        GROUP_FILTER_FIELD,
        groupFilter,
      );
    }, [filterPersistenceEnabled, filterPersistenceScope, groupFilter]);
  
    useEffect(() => {
      if (!filterPersistenceEnabled) {
        removeAccountsOverviewFilterField(
          filterPersistenceScope,
          ACTIVE_GROUP_ID_FIELD,
        );
        return;
      }
      writeAccountsOverviewFilterField(
        filterPersistenceScope,
        ACTIVE_GROUP_ID_FIELD,
        activeGroupId,
      );
    }, [activeGroupId, filterPersistenceEnabled, filterPersistenceScope]);
  
    const reloadLocalAccessState = useCallback(async () => {
      const requestSeq = ++localAccessStateRequestSeqRef.current;
      try {
        const nextState =
          await codexLocalAccessService.getCodexLocalAccessState();
        if (requestSeq !== localAccessStateRequestSeqRef.current) return;
        setLocalAccessState(nextState);
      } catch (error) {
        if (requestSeq !== localAccessStateRequestSeqRef.current) return;
        console.error("Failed to load codex local access state:", error);
        setMessage({
          text: t("messages.actionFailed", {
            action: t("codex.localAccess.title", "API 服务"),
            error: String(error),
          }),
          tone: "error",
        });
      }
    }, [setMessage, t]);
  
    const reloadLocalAccessEntryVisibility = useCallback(async () => {
      try {
        const config =
          await invoke<CodexOverviewGeneralConfig>("get_general_config");
        setLocalAccessEntryVisible(
          config.codex_local_access_entry_visible ?? true,
        );
      } catch (error) {
        console.error(
          "Failed to load codex local access entry visibility:",
          error,
        );
      }
    }, []);
  
    const reloadHideRelayQuota = useCallback(async () => {
      try {
        const config =
          await invoke<CodexOverviewGeneralConfig>("get_general_config");
        let hide = config.codex_hide_relay_quota ?? false;
        // One-time migrate toolbar preference from localStorage into user config.
        try {
          const legacy = localStorage.getItem(CODEX_HIDE_RELAY_QUOTA_LEGACY_KEY);
          if (legacy === "1" && !hide) {
            hide = true;
            await invoke("patch_general_config", {
              updates: { codex_hide_relay_quota: true },
            });
            window.dispatchEvent(new Event("config-updated"));
          }
          if (legacy !== null) {
            localStorage.removeItem(CODEX_HIDE_RELAY_QUOTA_LEGACY_KEY);
          }
        } catch {
          // ignore migration failures
        }
        setHideRelayQuota(hide);
      } catch (error) {
        console.error("Failed to load codex hide-relay-quota preference:", error);
      }
    }, []);
  
    const reloadLocalAccessLaunchCurrent = useCallback(async () => {
      try {
        const instances = await codexInstanceService.listInstances();
        const defaultInstance = instances.find((instance) => instance.isDefault);
        setLocalAccessLaunchCurrent(
          defaultInstance?.bindAccountId === CODEX_API_SERVICE_BIND_ID,
        );
      } catch (error) {
        console.warn(
          "Failed to resolve Codex API service current marker:",
          error,
        );
      }
    }, []);
  
    const exportFormatOptions = useMemo<SingleSelectFilterOption[]>(
      () => [
        {
          value: "cockpit_tools",
          label: t("codex.exportFormat.cockpitTools", "Cockpit Tools"),
        },
        {
          value: "auth_json",
          label: t("codex.exportFormat.authJson", "auth.json"),
        },
        {
          value: "sub2api",
          label: t("codex.exportFormat.sub2api", "sub2api"),
        },
        {
          value: "cpa",
          label: t("codex.exportFormat.cpa", "cpa"),
        },
      ],
      [t],
    );
  
    useEffect(() => {
      void reloadLocalAccessState();
    }, [reloadLocalAccessState]);
  
    useEffect(() => {
      if (
        !localAccessState?.running ||
        !localAccessState.collection?.boundOauthQuotaReserve
      ) {
        return;
      }
      const timer = window.setInterval(() => {
        void reloadLocalAccessState();
      }, 60_000);
      return () => window.clearInterval(timer);
    }, [
      localAccessState?.collection?.boundOauthQuotaReserve,
      localAccessState?.running,
      reloadLocalAccessState,
    ]);
  
    useEffect(() => {
      void reloadLocalAccessEntryVisibility();
    }, [reloadLocalAccessEntryVisibility]);
  
    useEffect(() => {
      void reloadHideRelayQuota();
    }, [reloadHideRelayQuota]);
  
    useEffect(() => {
      void reloadLocalAccessLaunchCurrent();
    }, [reloadLocalAccessLaunchCurrent]);
  
    useEffect(() => {
      try {
        localStorage.setItem(
          CODEX_LOCAL_ACCESS_EXPANDED_KEY,
          localAccessDetailsExpanded ? "1" : "0",
        );
      } catch {
        // ignore persistence failures
      }
    }, [localAccessDetailsExpanded]);
  
    useEffect(() => {
      const handleConfigUpdated = () => {
        void reloadLocalAccessEntryVisibility();
        void reloadHideRelayQuota();
        void reloadLocalAccessLaunchCurrent();
      };
      window.addEventListener("config-updated", handleConfigUpdated);
      return () => {
        window.removeEventListener("config-updated", handleConfigUpdated);
      };
    }, [
      reloadLocalAccessEntryVisibility,
      reloadHideRelayQuota,
      reloadLocalAccessLaunchCurrent,
    ]);
  
    useEffect(() => {
      const handleLocalAccessUpdated = () => {
        void reloadLocalAccessState();
        void reloadLocalAccessLaunchCurrent();
      };
      window.addEventListener(
        "codex-local-access-state-updated",
        handleLocalAccessUpdated,
      );
      return () => {
        window.removeEventListener(
          "codex-local-access-state-updated",
          handleLocalAccessUpdated,
        );
      };
    }, [reloadLocalAccessLaunchCurrent, reloadLocalAccessState]);

    useEffect(() => {
      let disposed = false;
      let unlisten: (() => void) | null = null;
      void listen("codex-local-access-state-updated", () => {
        void reloadLocalAccessState();
        void reloadLocalAccessLaunchCurrent();
      }).then((dispose) => {
        if (disposed) {
          dispose();
          return;
        }
        unlisten = dispose;
      });
      return () => {
        disposed = true;
        unlisten?.();
      };
    }, [reloadLocalAccessLaunchCurrent, reloadLocalAccessState]);
  
    useEffect(() => {
      if (!localAccessEntryVisible) {
        setShowLocalAccessModal(false);
      }
    }, [localAccessEntryVisible]);
  
    useEffect(() => {
      if (!showExportModal) {
        return;
      }
      setExportFormat("cockpit_tools");
      setFormattedExportJsonCopied(false);
      setFormattedSavingExportJson(false);
      setFormattedExportSavedPath(null);
      setFormattedExportSavedPathIsDirectory(false);
      setFormattedExportPathCopied(false);
      setFormattedBatchSavingExportJson(false);
      setFormattedSavingExportDocumentId(null);
      setIncludeExportSensitiveNotes(false);
      clearExportModalError();
    }, [clearExportModalError, exportJsonContent, showExportModal]);
  
    useEffect(() => {
      if (!showExportModal) {
        return;
      }
      setFormattedExportJsonCopied(false);
      setFormattedExportSavedPath(null);
      setFormattedExportSavedPathIsDirectory(false);
      setFormattedExportPathCopied(false);
      setFormattedBatchSavingExportJson(false);
      setFormattedSavingExportDocumentId(null);
      clearExportModalError();
    }, [clearExportModalError, exportFormat, showExportModal]);
  
    const exportHasAgentIdentity = useMemo(() => {
      return hasCodexExportAgentIdentity(exportJsonContent);
    }, [exportJsonContent]);
  
    const formattedExportResult = useMemo(() => {
      const exportFormatSupportsSensitiveNotes =
        exportFormat !== "sub2api" && exportFormat !== "auth_json";
      const exportOptions = {
        includeSensitiveNotes:
          includeExportSensitiveNotes && exportFormatSupportsSensitiveNotes,
      };
      if (!exportJsonContent) {
        return {
          content: {
            type: "single" as const,
            fileNameBase: buildCodexExportFileNameBase(
              exportFileNameBase,
              exportFormat,
            ),
            jsonContent: "",
          },
          failed: false,
        };
      }
      try {
        return {
          content: buildCodexExportContent(
            exportJsonContent,
            exportFormat,
            exportFileNameBase,
            exportOptions,
          ),
          failed: false,
        };
      } catch (error) {
        console.error("[CodexExport] transform failed:", error);
        return {
          content: {
            type: "single" as const,
            fileNameBase: buildCodexExportFileNameBase(
              exportFileNameBase,
              exportFormat,
            ),
            jsonContent: "",
          },
          failed: true,
        };
      }
    }, [
      exportFileNameBase,
      exportFormat,
      exportJsonContent,
      includeExportSensitiveNotes,
    ]);
  
    const formattedExportContent = formattedExportResult.content;
  
    useEffect(() => {
      if (!showExportModal || !formattedExportResult.failed) {
        return;
      }
      reportExportModalError(
        t(
          "codex.exportFormat.buildFailed",
          "无法生成导出内容：所选账号包含不支持的类型或认证信息不完整。",
        ),
      );
    }, [
      formattedExportResult.failed,
      reportExportModalError,
      showExportModal,
      t,
    ]);
  
    const exportHasSensitiveNotes = useMemo(() => {
      return hasCodexExportSensitiveNotes(exportJsonContent);
    }, [exportJsonContent]);
    const exportCanIncludeSensitiveNotes =
      exportHasSensitiveNotes &&
      exportFormat !== "sub2api" &&
      exportFormat !== "auth_json";
  
    const formattedExportJsonContent = useMemo(() => {
      return formattedExportContent.type === "single"
        ? formattedExportContent.jsonContent
        : "";
    }, [formattedExportContent]);
  
    const formattedExportDocuments = useMemo(() => {
      if (formattedExportContent.type !== "multiple") {
        return [];
      }
      return formattedExportContent.documents;
    }, [formattedExportContent]);
  
    const handleExportByIds = useCallback(
      async (ids: string[], fileNameBase?: string) => {
        setExportFileNameBase(fileNameBase || "codex_accounts");
        await handleBaseExportByIds(ids, fileNameBase);
      },
      [handleBaseExportByIds],
    );
  
    const handleExport = useCallback(
      async (scopeIds?: string[]) => {
        setExportFileNameBase("codex_accounts");
        await handleBaseExport(scopeIds);
      },
      [handleBaseExport],
    );
  
    const handleCloseExportModal = useCallback(() => {
      closeExportModal();
      setExportFormat("cockpit_tools");
      setIncludeExportSensitiveNotes(false);
      setFormattedExportJsonCopied(false);
      setFormattedSavingExportJson(false);
      setFormattedExportSavedPath(null);
      setFormattedExportSavedPathIsDirectory(false);
      setFormattedExportPathCopied(false);
      setFormattedBatchSavingExportJson(false);
      setFormattedSavingExportDocumentId(null);
      clearExportModalError();
    }, [clearExportModalError, closeExportModal]);
  
    const handleToggleExportJsonHidden = useCallback(() => {
      clearExportModalError();
      toggleExportJsonHidden();
    }, [clearExportModalError, toggleExportJsonHidden]);
  
    const copyFormattedExportJson = useCallback(async () => {
      if (!formattedExportJsonContent || formattedExportDocuments.length > 0)
        return;
      try {
        clearExportModalError();
        await navigator.clipboard.writeText(formattedExportJsonContent);
        setFormattedExportJsonCopied(true);
        window.setTimeout(() => setFormattedExportJsonCopied(false), 1200);
      } catch (error) {
        console.error("[CodexExport] copy failed:", error);
        reportExportModalError(
          t("messages.exportFailed", { error: String(error) }),
        );
      }
    }, [
      clearExportModalError,
      formattedExportDocuments.length,
      formattedExportJsonContent,
      reportExportModalError,
      t,
    ]);
  
    const saveFormattedExportJson = useCallback(async () => {
      if (
        !formattedExportJsonContent ||
        formattedSavingExportJson ||
        formattedExportDocuments.length > 0
      )
        return;
      setFormattedSavingExportJson(true);
      try {
        clearExportModalError();
        const fileName = buildExportFileName(
          buildCodexExportFileNameBase(exportFileNameBase, exportFormat),
        );
        const savedPath = await saveJsonFile(
          formattedExportJsonContent,
          fileName,
        );
        if (savedPath) {
          setFormattedExportSavedPath(savedPath);
          setFormattedExportSavedPathIsDirectory(false);
          setFormattedExportPathCopied(false);
        }
      } catch (error) {
        console.error("[CodexExport] save failed:", error);
        reportExportModalError(
          t("messages.exportFailed", { error: String(error) }),
        );
      } finally {
        setFormattedSavingExportJson(false);
      }
    }, [
      clearExportModalError,
      exportFileNameBase,
      exportFormat,
      formattedExportDocuments.length,
      formattedExportJsonContent,
      formattedSavingExportJson,
      reportExportModalError,
      saveJsonFile,
      t,
    ]);
  
    const saveFormattedExportDocument = useCallback(
      async (documentId: string, jsonContent: string, fileNameBase: string) => {
        if (!jsonContent || formattedSavingExportDocumentId) return;
        setFormattedSavingExportDocumentId(documentId);
        try {
          clearExportModalError();
          const savedPath = await saveJsonFile(
            jsonContent,
            buildExportFileName(fileNameBase),
          );
          if (savedPath) {
            setFormattedExportSavedPath(savedPath);
            setFormattedExportSavedPathIsDirectory(false);
            setFormattedExportPathCopied(false);
          }
        } catch (error) {
          console.error("[CodexExport] save single CPA document failed:", error);
          reportExportModalError(
            t("messages.exportFailed", { error: String(error) }),
          );
        } finally {
          setFormattedSavingExportDocumentId(null);
        }
      },
      [
        clearExportModalError,
        formattedSavingExportDocumentId,
        reportExportModalError,
        saveJsonFile,
        t,
      ],
    );
  
    const saveAllFormattedExportDocuments = useCallback(async () => {
      if (!formattedExportDocuments.length || formattedBatchSavingExportJson)
        return;
      setFormattedBatchSavingExportJson(true);
      try {
        clearExportModalError();
        let defaultPath: string | undefined;
        try {
          defaultPath = await invoke<string>("get_downloads_dir");
        } catch (error) {
          console.warn("[CodexExport] get downloads dir failed:", error);
        }
  
        const selected = await openFileDialog({
          directory: true,
          multiple: false,
          defaultPath,
        });
        if (!selected || Array.isArray(selected)) {
          return;
        }
  
        for (const document of formattedExportDocuments) {
          const targetPath = joinFilePath(
            selected,
            buildExportFileName(document.fileNameBase),
          );
          await invoke("save_text_file", {
            path: targetPath,
            content: document.jsonContent,
          });
        }
  
        setFormattedExportSavedPath(selected);
        setFormattedExportSavedPathIsDirectory(true);
        setFormattedExportPathCopied(false);
      } catch (error) {
        console.error("[CodexExport] save CPA documents failed:", error);
        reportExportModalError(
          t("messages.exportFailed", { error: String(error) }),
        );
      } finally {
        setFormattedBatchSavingExportJson(false);
      }
    }, [
      clearExportModalError,
      formattedBatchSavingExportJson,
      formattedExportDocuments,
      reportExportModalError,
      t,
    ]);
  
    const canOpenFormattedExportSavedDirectory = useMemo(
      () => Boolean(formattedExportSavedPath),
      [formattedExportSavedPath],
    );
  
    const openFormattedExportSavedDirectory = useCallback(async () => {
      if (!formattedExportSavedPath) return;
      try {
        clearExportModalError();
        await openPath(
          formattedExportSavedPathIsDirectory
            ? formattedExportSavedPath
            : getDirectoryPath(formattedExportSavedPath),
        );
      } catch (error) {
        console.error("[CodexExport] open directory failed:", error);
        reportExportModalError(
          t("messages.exportFailed", { error: String(error) }),
        );
      }
    }, [
      clearExportModalError,
      formattedExportSavedPath,
      formattedExportSavedPathIsDirectory,
      reportExportModalError,
      t,
    ]);
  
    const copyFormattedExportSavedPath = useCallback(async () => {
      if (!formattedExportSavedPath) return;
      try {
        clearExportModalError();
        await navigator.clipboard.writeText(formattedExportSavedPath);
        setFormattedExportPathCopied(true);
        window.setTimeout(() => setFormattedExportPathCopied(false), 1200);
      } catch (error) {
        console.error("[CodexExport] copy path failed:", error);
        reportExportModalError(
          t("messages.exportFailed", { error: String(error) }),
        );
      }
    }, [
      clearExportModalError,
      formattedExportSavedPath,
      reportExportModalError,
      t,
    ]);
  
    const formattedExportModalCustomContent = useMemo(() => {
      if (!formattedExportDocuments.length) {
        return undefined;
      }
  
      return (
        <>
          <div className="export-json-actions">
            <button
              className="btn btn-secondary btn-sm"
              onClick={handleToggleExportJsonHidden}
            >
              {exportJsonHidden ? <Eye size={14} /> : <EyeOff size={14} />}
              {exportJsonHidden
                ? t("common.preview", "预览")
                : t("common.close", "关闭")}
            </button>
            <button
              className="btn btn-primary btn-sm"
              onClick={() => void saveAllFormattedExportDocuments()}
              disabled={formattedBatchSavingExportJson}
            >
              <Download size={14} />
              {formattedBatchSavingExportJson
                ? t("common.loading", "加载中...")
                : t("codex.exportFormat.downloadAll", "一键下载全部")}
            </button>
          </div>
  
          <div className="export-json-card-list">
            {formattedExportDocuments.map((document, index) => (
              <div key={document.id} className="export-json-card">
                <div className="export-json-card-header">
                  <div className="export-json-card-heading">
                    <div className="export-json-card-title">
                      {t("codex.exportFormat.cpaCardTitle", "账号 {{index}}", {
                        index: index + 1,
                      })}
                    </div>
                    {!exportJsonHidden ? (
                      <div className="export-json-card-subtitle">
                        {document.label}
                      </div>
                    ) : null}
                  </div>
                  <div className="export-json-card-actions">
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={() =>
                        void saveFormattedExportDocument(
                          document.id,
                          document.jsonContent,
                          document.fileNameBase,
                        )
                      }
                      disabled={
                        Boolean(formattedSavingExportDocumentId) ||
                        formattedBatchSavingExportJson
                      }
                    >
                      <Download size={14} />
                      {formattedSavingExportDocumentId === document.id
                        ? t("common.loading", "加载中...")
                        : t("settings.about.download", "Download")}
                    </button>
                  </div>
                </div>
  
                <textarea
                  className="export-json-textarea export-json-card-textarea"
                  readOnly
                  spellCheck={false}
                  value={
                    exportJsonHidden
                      ? maskJsonPreviewContent(document.jsonContent)
                      : document.jsonContent
                  }
                />
              </div>
            ))}
          </div>
  
          {formattedExportSavedPath ? (
            <div className="export-json-path-box">
              <div className="export-json-path-title">
                {formattedExportSavedPathIsDirectory
                  ? t("codex.exportFormat.savedFolder", "保存目录")
                  : t("codex.exportFormat.savedPath", "保存路径")}
              </div>
              <div className="export-json-path-value">
                {formattedExportSavedPath}
              </div>
              <div className="export-json-path-actions">
                <button
                  className="btn btn-secondary btn-sm"
                  onClick={() => void openFormattedExportSavedDirectory()}
                  disabled={!canOpenFormattedExportSavedDirectory}
                >
                  <FolderOpen size={14} />
                  {t("instances.actions.openFolder", "打开文件夹")}
                </button>
                <button
                  className="btn btn-secondary btn-sm"
                  onClick={() => void copyFormattedExportSavedPath()}
                >
                  {formattedExportPathCopied ? (
                    <Check size={14} />
                  ) : (
                    <Copy size={14} />
                  )}
                  {formattedExportPathCopied
                    ? t("common.success", "成功")
                    : t("common.copy", "复制")}
                </button>
              </div>
            </div>
          ) : null}
        </>
      );
    }, [
      canOpenFormattedExportSavedDirectory,
      copyFormattedExportSavedPath,
      exportJsonHidden,
      formattedBatchSavingExportJson,
      formattedExportDocuments,
      formattedExportPathCopied,
      formattedExportSavedPath,
      formattedExportSavedPathIsDirectory,
      formattedSavingExportDocumentId,
      openFormattedExportSavedDirectory,
      saveAllFormattedExportDocuments,
      saveFormattedExportDocument,
      t,
      handleToggleExportJsonHidden,
    ]);
  
    useEffect(() => {
      try {
        localStorage.setItem(CODEX_OVERVIEW_LAYOUT_MODE_KEY, overviewLayoutMode);
      } catch {
        // ignore persistence failures
      }
    }, [overviewLayoutMode]);
  
    const handleChangeOverviewLayoutMode = useCallback(
      (mode: CodexOverviewLayoutMode) => {
        setOverviewLayoutMode(mode);
        if (mode === "list" || mode === "grid") {
          setViewMode(mode);
        }
      },
      [setViewMode],
    );
  
    useEffect(() => {
      if (overviewLayoutMode !== "compact" && viewMode !== overviewLayoutMode) {
        setViewMode(overviewLayoutMode);
      }
    }, [overviewLayoutMode, setViewMode, viewMode]);
  
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
  
    const validateApiKeyCredentialInputs = useCallback(
      (
        apiKeyRaw: string,
        apiBaseUrlRaw: string,
      ):
        | { ok: true; apiKey: string; apiBaseUrl?: string }
        | { ok: false; message: string } => {
        const apiKey = apiKeyRaw.trim();
        if (!apiKey) {
          return {
            ok: false,
            message: t("common.shared.token.empty", "请输入 Token 或 JSON"),
          };
        }
        if (isHttpLikeUrl(apiKey)) {
          return {
            ok: false,
            message: t(
              "codex.api.validation.apiKeyCannotBeUrl",
              "API Key 不能是 URL，请检查是否填反",
            ),
          };
        }
  
        const rawBaseUrl = apiBaseUrlRaw.trim();
        if (!rawBaseUrl) {
          return { ok: true, apiKey };
        }
        const normalizedBaseUrl = normalizeHttpBaseUrl(rawBaseUrl);
        if (!normalizedBaseUrl) {
          return {
            ok: false,
            message: t(
              "codex.api.validation.baseUrlInvalid",
              "Base URL 格式无效，请输入完整的 http:// 或 https:// 地址",
            ),
          };
        }
        if (normalizedBaseUrl === apiKey) {
          return {
            ok: false,
            message: t(
              "codex.api.validation.apiKeyEqualsBaseUrl",
              "API Key 不能与 Base URL 相同",
            ),
          };
        }
        return {
          ok: true,
          apiKey,
          apiBaseUrl: normalizedBaseUrl,
        };
      },
      [t],
    );
  
    const {
      accounts,
      loading,
      currentAccount,
      fetchAccounts,
      fetchCurrentAccount,
      applyAccountSnapshot,
      switchAccount,
      refreshQuota,
      refreshSubscriptionInfo,
      hydrateAccountProfilesIfNeeded,
      updateAccountName,
      updateApiKeyCredentials,
      updateApiKeyBoundOAuthAccount,
      updateAccountAppSpeed,
      updateAccountInstanceAccess,
    } = store;
    const localAccessCollection = localAccessState?.collection ?? null;
  
    const getResetCreditsAvailable = useCallback((account: CodexAccount) => {
      const value = account.quota?.reset_credits_available;
      return typeof value === "number" && Number.isFinite(value) ? value : null;
    }, []);
  
    const isAvailableResetCredit = useCallback((credit: CodexResetCredit) => {
      const normalizedStatus = (credit.status || credit.raw_status || "available")
        .trim()
        .toLowerCase();
      if (
        normalizedStatus === "redeemed" ||
        normalizedStatus === "used" ||
        normalizedStatus === "consumed" ||
        normalizedStatus === "expired"
      ) {
        return false;
      }
      return !(
        typeof credit.expires_at === "number" &&
        Number.isFinite(credit.expires_at) &&
        credit.expires_at <= Math.floor(Date.now() / 1000)
      );
    }, []);
  
    const getResetCreditDetails = useCallback((account: CodexAccount) => {
      return Array.isArray(account.quota?.reset_credits)
        ? account.quota.reset_credits
        : [];
    }, []);
  
    const getResetCreditNextExpiresAt = useCallback(
      (account: CodexAccount) => {
        const explicit = account.quota?.reset_credits_next_expires_at;
        if (typeof explicit === "number" && Number.isFinite(explicit)) {
          return explicit;
        }
  
        const next = getResetCreditDetails(account)
          .filter(isAvailableResetCredit)
          .map((credit) => credit.expires_at)
          .filter(
            (value): value is number =>
              typeof value === "number" && Number.isFinite(value),
          )
          .sort((a, b) => a - b)[0];
        return next ?? null;
      },
      [getResetCreditDetails, isAvailableResetCredit],
    );
  
    const formatResetCreditTime = useCallback(
      (timestamp: number | null | undefined) => {
        return timestamp
          ? formatCodexResetTime(timestamp, t)
          : t("codex.quota.resetCreditTimeUnknown", "时间未知");
      },
      [t],
    );
  
    const formatResetCreditAbsoluteTime = useCallback(
      (timestamp: number | null | undefined) => {
        return timestamp
          ? formatCodexResetTimeAbsolute(timestamp)
          : t("codex.quota.resetCreditTimeUnknown", "时间未知");
      },
      [t],
    );
  
    const getResetCreditStatusLabel = useCallback(
      (credit: CodexResetCredit) => {
        const normalizedStatus = (credit.status || credit.raw_status || "")
          .trim()
          .toLowerCase();
        if (
          normalizedStatus === "redeemed" ||
          normalizedStatus === "used" ||
          normalizedStatus === "consumed"
        ) {
          return t("codex.quota.resetCreditStatusRedeemed", "已使用");
        }
        if (normalizedStatus === "available") {
          return isAvailableResetCredit(credit)
            ? t("codex.quota.resetCreditStatusAvailable", "可用")
            : t("codex.quota.resetCreditStatusExpired", "已过期");
        }
        if (normalizedStatus === "expired") {
          return t("codex.quota.resetCreditStatusExpired", "已过期");
        }
        if (!isAvailableResetCredit(credit)) {
          return t("codex.quota.resetCreditStatusExpired", "已过期");
        }
        return (
          credit.raw_status ||
          credit.status ||
          t("codex.quota.resetCreditStatusUnknown", "未知")
        );
      },
      [isAvailableResetCredit, t],
    );
  
    const getResetCreditStatusTone = useCallback(
      (credit: CodexResetCredit) => {
        const normalizedStatus = (credit.status || credit.raw_status || "")
          .trim()
          .toLowerCase();
        if (normalizedStatus === "available" && isAvailableResetCredit(credit)) {
          return "is-available";
        }
        if (
          normalizedStatus === "redeemed" ||
          normalizedStatus === "used" ||
          normalizedStatus === "consumed"
        ) {
          return "is-redeemed";
        }
        if (normalizedStatus === "expired" || !isAvailableResetCredit(credit)) {
          return "is-expired";
        }
        return "is-unknown";
      },
      [isAvailableResetCredit],
    );
  
    const buildResetCreditsTitle = useCallback(
      (account: CodexAccount, availableCount: number) => {
        if (availableCount <= 0) {
          return t("codex.quota.resetCreditNoCredits", "没有可用的主动重置次数");
        }
  
        const nextExpiresAt = getResetCreditNextExpiresAt(account);
        if (nextExpiresAt) {
          return t("codex.quota.resetCreditsTitleWithExpiry", {
            count: availableCount,
            time: formatResetCreditTime(nextExpiresAt),
            defaultValue:
              "可用于重置当前 5 小时窗口的剩余次数：{{count}}，最近到期：{{time}}",
          });
        }
  
        return t("codex.quota.resetCreditsTitle", {
          count: availableCount,
        });
      },
      [formatResetCreditTime, getResetCreditNextExpiresAt, t],
    );
  
    const resetCreditConfirmAccount = useMemo(
      () =>
        resetCreditConfirmAccountId
          ? (accounts.find(
              (account) => account.id === resetCreditConfirmAccountId,
            ) ?? null)
          : null,
      [accounts, resetCreditConfirmAccountId],
    );
  
    const resetCreditConfirmAvailableCount =
      resetCreditConfirmSnapshot?.available_count ??
      (resetCreditConfirmAccount
        ? getResetCreditsAvailable(resetCreditConfirmAccount)
        : null);
    const resetCreditConfirmCredits = resetCreditConfirmSnapshot?.credits ?? [];
    const resetCreditConfirmNextExpiresAt =
      resetCreditConfirmSnapshot?.next_expires_at ?? null;
    const isResetCreditConfirmSubmitting = resetCreditConfirmAccount
      ? resettingResetCreditAccountId === resetCreditConfirmAccount.id
      : false;
  
    const loadResetCreditConfirmSnapshot = useCallback(
      async (accountId: string) => {
        const requestSeq = resetCreditConfirmRequestSeqRef.current + 1;
        resetCreditConfirmRequestSeqRef.current = requestSeq;
        setResetCreditConfirmLoading(true);
        setResetCreditConfirmSnapshot(null);
  
        try {
          const snapshot = await codexService.getCodexResetCredits(accountId);
          if (resetCreditConfirmRequestSeqRef.current !== requestSeq) return;
          setResetCreditConfirmSnapshot({
            available_count: snapshot.available_count,
            credits: Array.isArray(snapshot.credits) ? snapshot.credits : [],
            next_expires_at: snapshot.next_expires_at,
          });
        } catch (error) {
          if (resetCreditConfirmRequestSeqRef.current !== requestSeq) return;
          setResetCreditConfirmError(
            t("codex.quota.resetCreditRecordsLoadFailed", {
              error: String(error).replace(/^Error:\s*/, ""),
            }),
          );
        } finally {
          if (resetCreditConfirmRequestSeqRef.current === requestSeq) {
            setResetCreditConfirmLoading(false);
          }
        }
      },
      [setResetCreditConfirmError, t],
    );
  
    const openResetCreditConfirmModal = useCallback(
      (account: CodexAccount) => {
        setResetCreditConfirmError(null);
        setResetCreditConfirmActionLocked(false);
        setResetCreditConfirmSnapshot(null);
        setResetCreditConfirmAccountId(account.id);
        void loadResetCreditConfirmSnapshot(account.id);
      },
      [loadResetCreditConfirmSnapshot, setResetCreditConfirmError],
    );
  
    const closeResetCreditConfirmModal = useCallback(() => {
      if (resettingResetCreditAccountId) return;
      resetCreditConfirmRequestSeqRef.current += 1;
      setResetCreditConfirmAccountId(null);
      setResetCreditConfirmSnapshot(null);
      setResetCreditConfirmLoading(false);
      setResetCreditConfirmActionLocked(false);
      setResetCreditConfirmError(null);
    }, [resettingResetCreditAccountId, setResetCreditConfirmError]);
  
    const handleConfirmConsumeResetCredit = useCallback(async () => {
      const account = resetCreditConfirmAccount;
      if (!account) return;
  
      const availableCount = resetCreditConfirmAvailableCount;
      if (availableCount == null || availableCount <= 0) {
        setResetCreditConfirmError(
          t("codex.quota.resetCreditNoCredits", "没有可用的主动重置次数"),
        );
        return;
      }
  
      setResetCreditConfirmError(null);
      setResetCreditConfirmActionLocked(false);
      setResettingResetCreditAccountId(account.id);
  
      try {
        await codexService.consumeCodexResetCredit(account.id);
        try {
          await refreshQuota(account.id);
          setMessage({
            text: t("codex.quota.resetCreditConsumed", "已重置 5 小时额度"),
          });
          setResetCreditConfirmAccountId(null);
        } catch (error) {
          setResetCreditConfirmActionLocked(true);
          setResetCreditConfirmError(
            t("codex.quota.resetCreditRefreshAfterConsumeFailed", {
              error: String(error).replace(/^Error:\s*/, ""),
            }),
          );
        }
      } catch (error) {
        setResetCreditConfirmError(
          t("codex.quota.resetCreditFailed", {
            error: String(error).replace(/^Error:\s*/, ""),
          }),
        );
        return;
      } finally {
        setResettingResetCreditAccountId(null);
      }
    }, [
      refreshQuota,
      resetCreditConfirmAccount,
      resetCreditConfirmAvailableCount,
      setMessage,
      setResetCreditConfirmError,
      t,
    ]);
  
    const handleRefreshSubscriptionInfo = useCallback(
      async (accountId: string) => {
        setRefreshingSubscriptionAccountId(accountId);
        try {
          await refreshSubscriptionInfo(accountId);
        } catch (error) {
          console.error(error);
        } finally {
          setRefreshingSubscriptionAccountId(null);
        }
      },
      [refreshSubscriptionInfo],
    );
  
    const editingAccountNoteAccount = useMemo(
      () =>
        accounts.find((account) => account.id === editingAccountNoteId) || null,
      [accounts, editingAccountNoteId],
    );
    const activeAccountNoteMode = editingAccountNoteAccount
      ? "account"
      : pendingOAuthNoteModalOpen
        ? "pendingOAuth"
        : null;
    const activeAccountNoteForm =
      activeAccountNoteMode === "pendingOAuth"
        ? pendingOAuthNoteForm
        : editingAccountNoteForm;
    const activeAccountNoteSaving =
      savingAccountNote ||
      (activeAccountNoteMode === "pendingOAuth" && savingPendingOAuthAccount);
    const activeAccountNoteDisplayName =
      activeAccountNoteMode === "pendingOAuth"
        ? pendingOAuthEmailInput.trim() ||
          t("codex.pendingAuth.emailLabel", "待授权账号")
        : editingAccountNoteAccount
          ? buildCodexAccountPresentation(editingAccountNoteAccount, t)
              .displayName
          : "";
    const activeAccountNoteEmail =
      activeAccountNoteMode === "pendingOAuth"
        ? pendingOAuthEmailInput.trim()
        : editingAccountNoteAccount?.email?.trim() || "";
    const activeAccountUsesPersonalAccessToken = Boolean(
      editingAccountNoteAccount &&
      isCodexOpaqueAccessTokenOnlyAccount(editingAccountNoteAccount),
    );
  
    const refreshSavedMfaRecords = useCallback(() => {
      setSavedMfaRecords(loadSavedMfaRecords());
    }, []);
  
    const resetAccountNoteMailPreview = useCallback(() => {
      accountNoteMailPreviewSeqRef.current += 1;
      accountNoteMailPreviewSnapshotRef.current = null;
      setAccountNoteMailPreview(null);
      setAccountNoteMailPreviewError(null);
      setAccountNoteMailPreviewLoading(false);
    }, []);
  
    const fetchAccountNoteMailPreviewForUrl = useCallback(
      async (rawUrl: string) => {
        const mailUrl = rawUrl.trim();
        accountNoteMailPreviewSeqRef.current += 1;
        const requestSeq = accountNoteMailPreviewSeqRef.current;
        setAccountNoteMailPreview(null);
        setAccountNoteMailPreviewError(null);
        if (!mailUrl) {
          accountNoteMailPreviewSnapshotRef.current = null;
          setAccountNoteMailPreviewLoading(false);
          return;
        }
  
        setAccountNoteMailPreviewLoading(true);
        try {
          const response =
            await codexService.fetchCodexAccountNoteMailUrl(mailUrl);
          if (accountNoteMailPreviewSeqRef.current !== requestSeq) return;
          const preview = findFirstMailVerificationCode(response.body);
          if (!preview) {
            setAccountNoteMailPreviewError(
              t("codex.accountNote.mailPreviewNoCode", "未匹配到连续 6 位验证码"),
            );
            return;
          }
          const previousPreview = accountNoteMailPreviewSnapshotRef.current;
          const status =
            previousPreview?.mailUrl === mailUrl
              ? previousPreview.code === preview.code
                ? "unchanged"
                : "changed"
              : "initial";
          accountNoteMailPreviewSnapshotRef.current = {
            mailUrl,
            code: preview.code,
          };
          setAccountNoteMailPreview({
            ...preview,
            fetchedAt: Date.now(),
            truncated: response.truncated,
            status,
          });
        } catch (error) {
          if (accountNoteMailPreviewSeqRef.current !== requestSeq) return;
          const rawError = String(error).replace(/^Error:\s*/, "");
          const httpError = rawError.match(/^MAIL_PREVIEW_HTTP_FAILED:(\d+)$/);
          const errorDetail =
            rawError === "MAIL_URL_EMPTY"
              ? t("codex.accountNote.mailPreviewUrlRequired", "请输入邮件地址")
              : rawError === "MAIL_URL_INVALID"
                ? t(
                    "codex.accountNote.mailPreviewUrlInvalid",
                    "邮件地址格式无效，请输入完整的 http:// 或 https:// 地址",
                  )
                : rawError === "MAIL_URL_UNSUPPORTED_SCHEME"
                  ? t(
                      "codex.accountNote.mailPreviewUnsupportedProtocol",
                      "邮件地址仅支持 http 或 https 协议",
                    )
                  : httpError
                    ? t("codex.accountNote.mailPreviewHttpFailed", {
                        defaultValue: "邮件地址请求失败：HTTP {{status}}",
                        status: httpError[1],
                      })
                    : rawError
                        .replace(/^MAIL_PREVIEW_CLIENT_FAILED:\s*/, "")
                        .replace(/^MAIL_PREVIEW_REQUEST_FAILED:\s*/, "")
                        .replace(/^MAIL_PREVIEW_READ_FAILED:\s*/, "");
          setAccountNoteMailPreviewError(
            t("codex.accountNote.mailPreviewFetchFailed", {
              error: errorDetail,
              defaultValue: "读取邮件失败：{{error}}",
            }),
          );
        } finally {
          if (accountNoteMailPreviewSeqRef.current === requestSeq) {
            setAccountNoteMailPreviewLoading(false);
          }
        }
      },
      [t],
    );
  
    const updateActiveAccountNoteForm = useCallback(
      (update: Partial<CodexAccountNoteFormState>) => {
        if (activeAccountNoteMode === "pendingOAuth") {
          setPendingOAuthNoteForm((prev) => ({ ...prev, ...update }));
          setPendingOAuthFieldErrors((prev) => ({
            ...prev,
            twoFactorSecret: undefined,
          }));
        } else {
          setEditingAccountNoteForm((prev) => ({ ...prev, ...update }));
        }
        setAccountNoteFieldErrors((prev) => ({
          ...prev,
          twoFactorSecret: undefined,
        }));
        if (Object.prototype.hasOwnProperty.call(update, "mailUrl")) {
          resetAccountNoteMailPreview();
        }
        setAccountNoteError(null);
      },
      [activeAccountNoteMode, resetAccountNoteMailPreview, setAccountNoteError],
    );
  
    const openAccountNoteModal = useCallback(
      (account: CodexAccount) => {
        setEditingAccountNoteId(account.id);
        setEditingAccountNoteForm(buildCodexAccountNoteForm(account));
        setPendingOAuthNoteModalOpen(false);
        setAccountNoteFieldErrors({});
        setAccountNoteSecretVisible(true);
        setAccountNotePasswordVisible(true);
        setAccountNoteCopiedKey(null);
        setAccountNoteMfaPickerOpen(false);
        resetAccountNoteMailPreview();
        refreshSavedMfaRecords();
        setAccountNoteError(null);
        void fetchAccountNoteMailPreviewForUrl(account.mail_url ?? "");
      },
      [
        fetchAccountNoteMailPreviewForUrl,
        refreshSavedMfaRecords,
        resetAccountNoteMailPreview,
        setAccountNoteError,
      ],
    );
  
    const openPendingOAuthNoteModal = useCallback(() => {
      setPendingOAuthNoteModalOpen(true);
      setEditingAccountNoteId(null);
      setAccountNoteFieldErrors({});
      setAccountNoteSecretVisible(true);
      setAccountNotePasswordVisible(true);
      setAccountNoteCopiedKey(null);
      setAccountNoteMfaPickerOpen(false);
      resetAccountNoteMailPreview();
      refreshSavedMfaRecords();
      setAccountNoteError(null);
      void fetchAccountNoteMailPreviewForUrl(pendingOAuthNoteForm.mailUrl);
    }, [
      fetchAccountNoteMailPreviewForUrl,
      pendingOAuthNoteForm.mailUrl,
      refreshSavedMfaRecords,
      resetAccountNoteMailPreview,
      setAccountNoteError,
    ]);
  
    const closeAccountNoteModal = useCallback(() => {
      if (savingAccountNote || savingPendingOAuthAccount) return;
      setEditingAccountNoteId(null);
      setEditingAccountNoteForm(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
      setPendingOAuthNoteModalOpen(false);
      setAccountNoteFieldErrors({});
      setAccountNoteSecretVisible(true);
      setAccountNotePasswordVisible(true);
      setAccountNoteCopiedKey(null);
      setAccountNoteMfaPickerOpen(false);
      resetAccountNoteMailPreview();
      setAccountNoteError(null);
    }, [
      resetAccountNoteMailPreview,
      savingAccountNote,
      savingPendingOAuthAccount,
      setAccountNoteError,
    ]);
  
    const loadApiServiceAppSpeed = useCallback(async () => {
      try {
        const config = await codexService.getCodexApiServiceAppSpeedConfig();
        setApiServiceAppSpeed(config.speed);
      } catch (error) {
        console.warn("加载 Codex API 服务速度失败:", error);
      }
    }, []);
  
    useEffect(() => {
      void loadApiServiceAppSpeed();
    }, [loadApiServiceAppSpeed]);
  
    const handleAccountAppSpeedChange = useCallback(
      async (account: CodexAccount, speed: CodexAppSpeed) => {
        if (savingAppSpeedId) return;
        setSavingAppSpeedId(account.id);
        try {
          await updateAccountAppSpeed(account.id, speed);
          setMessage({
            text: t("codex.speed.saveSuccess", "速度已更新"),
          });
        } catch (error) {
          setMessage({
            text: t("codex.speed.saveFailed", {
              defaultValue: "保存速度失败：{{error}}",
              error: String(error),
            }),
            tone: "error",
          });
        } finally {
          setSavingAppSpeedId(null);
        }
      },
      [savingAppSpeedId, setMessage, t, updateAccountAppSpeed],
    );
  
    const handleApiServiceAppSpeedChange = useCallback(
      async (speed: CodexAppSpeed) => {
        if (savingAppSpeedId) return;
        const previousSpeed = apiServiceAppSpeed;
        setApiServiceAppSpeed(speed);
        setSavingAppSpeedId(CODEX_API_SERVICE_BIND_ID);
        try {
          const saved = await codexService.saveCodexApiServiceAppSpeed(speed);
          setApiServiceAppSpeed(saved.speed);
          setMessage({
            text: t("codex.speed.saveSuccess", "速度已更新"),
          });
        } catch (error) {
          setApiServiceAppSpeed(previousSpeed);
          setMessage({
            text: t("codex.speed.saveFailed", {
              defaultValue: "保存速度失败：{{error}}",
              error: String(error),
            }),
            tone: "error",
          });
        } finally {
          setSavingAppSpeedId(null);
        }
      },
      [apiServiceAppSpeed, savingAppSpeedId, setMessage, t],
    );
  
    const renderAccountSpeedSelect = useCallback(
      (account: CodexAccount, compact = false) => (
        <CodexSpeedSelect
          value={account.app_speed ?? "standard"}
          onChange={(speed) => handleAccountAppSpeedChange(account, speed)}
          busy={savingAppSpeedId === account.id}
          compact={compact}
          preferredPlacement="top"
          ariaLabel={t("codex.speed.title", "速度")}
        />
      ),
      [handleAccountAppSpeedChange, savingAppSpeedId, t],
    );
  
    const handleSubmitAccountNote = useCallback(async () => {
      if (!activeAccountNoteMode || activeAccountNoteSaving) return;
      setSavingAccountNote(true);
      setAccountNoteError(null);
      setAccountNoteFieldErrors({});
      try {
        const rawTwoFactorSecret = activeAccountNoteForm.twoFactorSecret.trim();
        const parsedTwoFactorSecret = rawTwoFactorSecret
          ? parseMfaCredentialInput(rawTwoFactorSecret)
          : null;
        if (rawTwoFactorSecret && !parsedTwoFactorSecret) {
          setAccountNoteFieldErrors({
            twoFactorSecret: t(
              "codex.accountNote.twoFactorSecretInvalid",
              "2FA 秘钥格式无效，请输入 Base32 secret 或 otpauth:// 链接",
            ),
          });
          return;
        }
        const normalizedTwoFactorSecret =
          parsedTwoFactorSecret?.secret ?? rawTwoFactorSecret;
        const noteUpdate = {
          note: activeAccountNoteForm.note,
          twoFactorSecret: normalizedTwoFactorSecret,
          accountPassword: activeAccountNoteForm.accountPassword,
          phoneNumber: activeAccountNoteForm.phoneNumber,
          mailUrl: activeAccountNoteForm.mailUrl,
          ...(activeAccountUsesPersonalAccessToken
            ? { chatgptAccountId: activeAccountNoteForm.chatgptAccountId }
            : {}),
        };
  
        if (normalizedTwoFactorSecret) {
          setSavedMfaRecords(
            upsertSavedMfaRecord({
              secret: normalizedTwoFactorSecret,
              accountName:
                activeAccountNoteDisplayName ||
                parsedTwoFactorSecret?.accountName ||
                null,
              remark: activeAccountNoteForm.note,
            }),
          );
        }
  
        if (activeAccountNoteMode === "pendingOAuth") {
          setPendingOAuthNoteForm({
            ...activeAccountNoteForm,
            ...noteUpdate,
          });
          setPendingOAuthFieldErrors((prev) => ({
            ...prev,
            twoFactorSecret: undefined,
          }));
        } else if (editingAccountNoteId) {
          await store.updateAccountNote(editingAccountNoteId, noteUpdate);
          setEditingAccountNoteForm(EMPTY_CODEX_ACCOUNT_NOTE_FORM);
        } else {
          return;
        }
        setMessage({
          text: t("codex.accountNote.saved", "账号备注已保存"),
          tone: "success",
        });
        setEditingAccountNoteId(null);
        setPendingOAuthNoteModalOpen(false);
        setAccountNoteCopiedKey(null);
        setAccountNoteMfaPickerOpen(false);
        resetAccountNoteMailPreview();
      } catch (error) {
        setAccountNoteError(
          t("codex.accountNote.saveFailed", {
            error: String(error).replace(/^Error:\s*/, ""),
            defaultValue: "保存账号备注失败：{{error}}",
          }),
        );
      } finally {
        setSavingAccountNote(false);
      }
    }, [
      activeAccountNoteDisplayName,
      activeAccountNoteForm,
      activeAccountNoteMode,
      activeAccountNoteSaving,
      activeAccountUsesPersonalAccessToken,
      editingAccountNoteId,
      setAccountNoteError,
      setMessage,
      resetAccountNoteMailPreview,
      store,
      t,
    ]);
  
    const activeAccountNoteOtpToken = useMemo(() => {
      const secret = activeAccountNoteForm.twoFactorSecret.trim();
      return secret ? getMfaOtpToken(secret) : "";
    }, [activeAccountNoteForm.twoFactorSecret, mfaTimeRemaining]);
  
    const copyAccountNoteValue = useCallback(
      async (copyKey: string, value?: string | null) => {
        const text = value?.trim();
        if (!text) return;
        try {
          await navigator.clipboard.writeText(text);
          setAccountNoteCopiedKey(copyKey);
          window.setTimeout(() => {
            setAccountNoteCopiedKey((current) =>
              current === copyKey ? null : current,
            );
          }, 1200);
        } catch {
          setAccountNoteError(
            t("common.shared.export.copyFailed", "复制失败，请手动复制"),
          );
        }
      },
      [setAccountNoteError, t],
    );
  
    const handleRefreshAccountNoteMailPreview = useCallback(() => {
      void fetchAccountNoteMailPreviewForUrl(activeAccountNoteForm.mailUrl);
    }, [activeAccountNoteForm.mailUrl, fetchAccountNoteMailPreviewForUrl]);
  
    const handleOpenAccountNoteMailUrl = useCallback(async () => {
      const mailUrl = activeAccountNoteForm.mailUrl.trim();
      if (!mailUrl) return;
      try {
        await openUrl(mailUrl);
      } catch (error) {
        setAccountNoteError(
          t("codex.accountNote.mailOpenFailed", {
            error: String(error).replace(/^Error:\s*/, ""),
            defaultValue: "打开邮件地址失败：{{error}}",
          }),
        );
      }
    }, [activeAccountNoteForm.mailUrl, setAccountNoteError, t]);
  
    const renderAccountNoteButton = useCallback(
      (account: CodexAccount, className = "codex-account-note-chip") => {
        const hasNote = hasCodexAccountNoteDetails(account);
        return (
          <button
            type="button"
            className={`${className} ${hasNote ? "has-note" : "empty-note"}`}
            onClick={() => openAccountNoteModal(account)}
            title={
              hasNote
                ? getCodexAccountNoteTitle(
                    account,
                    t("codex.accountNote.short", "账号备注"),
                  )
                : t("codex.accountNote.emptyTitle", "填写账号备注")
            }
          >
            <FileText size={12} />
            <span>
              {hasNote
                ? t("codex.accountNote.short", "账号备注")
                : t("codex.accountNote.addShort", "加备注")}
            </span>
          </button>
        );
      },
      [openAccountNoteModal, t],
    );
  return {
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
    activeBatchImportCheckQuota,
    activeGroupId,
    activeTab,
    addingLocalAccessAccountId,
    addMessage,
    addStatus,
    addTab,
    apiKeyUsageDetailAccountId,
    apiServiceAppSpeed,
    applyAccountSnapshot,
    assignCodexAccountsToTargetGroup,
    availableTags,
    batchDeleteBusy,
    batchDeleteJob,
    batchDeleteModalError,
    batchDeleteRefreshedCompletedRef,
    batchDeleteRemoveIdsRef,
    batchImportBusy,
    batchImportCheckQuota,
    batchImportCounts,
    batchImportError,
    batchImportFilePaths,
    batchImportOpen,
    batchImportPreview,
    batchImportProgress,
    batchImportProgressCurrent,
    batchImportProgressPercent,
    batchImportProgressTotal,
    batchImportResult,
    batchImportSelectableIds,
    batchImportSelectableIdSet,
    batchImportSelectedCountLabel,
    batchImportSelectedIds,
    batchImportSelectedSelectableCount,
    batchImportSessionId,
    batchImportSessionIdRef,
    batchImportTagsInput,
    batchImportTargetGroupId,
    batchImportUnlistenersRef,
    batchImportVisibleItems,
    buildResetCreditsTitle,
    canOpenFormattedExportSavedDirectory,
    cleanupBatchImportListeners,
    clearAllOverviewFilters,
    clearFilterTypes,
    clearGroupFilter,
    clearTagFilter,
    cliLaunchingAccountId,
    cliLaunchModal,
    closeAccountNoteModal,
    closeAddModal,
    closeCliLaunchModal,
    closeCodexAddModal,
    closeExternalImportProgressModal,
    closeLocalAccessRiskNotice,
    closeResetCreditConfirmModal,
    cockpitApiPanelAccountId,
    codexAccountsRef,
    codexAddTargetGroup,
    codexAddTargetGroupId,
    codexCliInstanceDefaultsRef,
    codexGroups,
    codexInstanceStore,
    confirmDeleteTag,
    copyAccountNoteValue,
    copyFormattedExportJson,
    copyFormattedExportSavedPath,
    currentAccount,
    deepSeekStart,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    deletingGroup,
    deletingTag,
    ensureLocalAccessEntryVisible,
    exportCanIncludeSensitiveNotes,
    exportFormat,
    exportFormatOptions,
    exportHasAgentIdentity,
    exporting,
    exportJsonHidden,
    exportModalError,
    exportModalErrorScrollKey,
    externalImportProgress,
    externalImportSyncError,
    fetchAccounts,
    fetchCurrentAccount,
    fetchSponsorState,
    filterTypes,
    formatDate,
    formatResetCreditAbsoluteTime,
    formatResetCreditTime,
    formattedExportJsonContent,
    formattedExportJsonCopied,
    formattedExportModalCustomContent,
    formattedExportPathCopied,
    formattedExportSavedPath,
    formattedSavingExportJson,
    fullQuotaWakeupOpenRequest,
    fullQuotaWakeupOpenSignalRef,
    getResetCreditDetails,
    getResetCreditsAvailable,
    getResetCreditStatusLabel,
    getResetCreditStatusTone,
    getScopedSelectedCount,
    groupByTag,
    groupDeleteConfirm,
    groupDeleteError,
    groupDeleteErrorScrollKey,
    groupFilter,
    groupQuickAddGroupId,
    handleApiServiceAppSpeedChange,
    handleChangeOverviewLayoutMode,
    handleCloseExportModal,
    handleConfirmConsumeResetCredit,
    handleCopyReauthEmail,
    handleDelete,
    handleExport,
    handleExportByIds,
    handleOpenAccountNoteMailUrl,
    handleRefresh,
    handleRefreshAccountNoteMailPreview,
    handleRefreshAll,
    handleRefreshSubscriptionInfo,
    handleSaveTags,
    handleSubmitAccountNote,
    handleSyncImportedToApiServiceChange,
    handleToggleExportJsonHidden,
    hideRelayQuota,
    hydrateAccountProfilesIfNeeded,
    importApiServiceGuideCount,
    importing,
    includeExportSensitiveNotes,
    isAllFilteredSelected,
    isAvailableResetCredit,
    isMacOS,
    isResetCreditConfirmSubmitting,
    loading,
    localAccessAddressKind,
    localAccessCollection,
    localAccessCopiedField,
    localAccessDetailsExpanded,
    localAccessEntryVisible,
    localAccessHealthActionBusy,
    localAccessHideSubmitting,
    localAccessKeyVisible,
    localAccessLaunchCurrent,
    localAccessModalMode,
    localAccessPortKilling,
    localAccessRefreshing,
    localAccessRiskNoticeAction,
    localAccessRiskNoticeRemember,
    localAccessSaving,
    localAccessSidecarRestarting,
    localAccessStarting,
    localAccessState,
    maskAccountText,
    message,
    mfaTimeRemaining,
    normalizeTag,
    openAccountNoteModal,
    openCodexAddModal,
    openFormattedExportSavedDirectory,
    openPendingOAuthNoteModal,
    openResetCreditConfirmModal,
    openTagModal,
    overviewLayoutMode,
    page,
    pendingOAuthEmailInput,
    pendingOAuthFieldErrors,
    pendingOAuthHasNoteDetails,
    pendingOAuthNoteForm,
    pendingWebSessionImport,
    privacyModeEnabled,
    quotaErrorDetail,
    reauthEmailCopied,
    reauthRetryInstanceId,
    reauthRetryLaunchAfterSwitch,
    reauthRetryOAuthBinding,
    reauthRetrySwitchAccountId,
    reauthTargetAccount,
    reauthTargetAccountId,
    reauthTargetEmail,
    refreshAccountsAfterBatchDelete,
    refreshing,
    refreshingAll,
    refreshingGroupId,
    refreshingSubscriptionAccountId,
    refreshSavedMfaRecords,
    reloadCodexGroups,
    reloadLocalAccessState,
    removingGroupAccountIds,
    renderAccountNoteButton,
    renderAccountSpeedSelect,
    reportExportModalError,
    requestDeleteTag,
    requestLocalAccessRiskNotice,
    resetBatchImportState,
    resetCreditConfirmAccount,
    resetCreditConfirmActionLocked,
    resetCreditConfirmAvailableCount,
    resetCreditConfirmCredits,
    resetCreditConfirmError,
    resetCreditConfirmErrorScrollKey,
    resetCreditConfirmLoading,
    resetCreditConfirmNextExpiresAt,
    resettingResetCreditAccountId,
    resolveValidCodexGroupId,
    savedMfaRecords,
    saveFormattedExportJson,
    savingAppSpeedId,
    savingPendingOAuthAccount,
    searchQuery,
    selected,
    selectedTerminal,
    sessionWindowStats,
    setAccountNoteError,
    setAccountNoteMfaPickerOpen,
    setAccountNotePasswordVisible,
    setAccountNoteSecretVisible,
    setActiveGroupId,
    setActiveTab,
    setAddingLocalAccessAccountId,
    setApiKeyUsageDetailAccountId,
    setBatchDeleteBusy,
    setBatchDeleteJob,
    setBatchDeleteModalError,
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
    setBatchImportTagsInput,
    setBatchImportTargetGroupId,
    setCliLaunchingAccountId,
    setCliLaunchModal,
    setCockpitApiPanelAccountId,
    setCodexAddTargetGroupId,
    setDeleteConfirm,
    setDeletingGroup,
    setExportFormat,
    setExternalImportSyncError,
    setFullQuotaWakeupOpenRequest,
    setGroupByTag,
    setGroupDeleteConfirm,
    setGroupDeleteError,
    setGroupQuickAddGroupId,
    setImportApiServiceGuideCount,
    setImporting,
    setIncludeExportSensitiveNotes,
    setIsAllFilteredSelected,
    setLocalAccessAddressKind,
    setLocalAccessCopiedField,
    setLocalAccessDetailsExpanded,
    setLocalAccessEntryVisible,
    setLocalAccessHealthActionBusy,
    setLocalAccessHideSubmitting,
    setLocalAccessKeyVisible,
    setLocalAccessLaunchCurrent,
    setLocalAccessModalMode,
    setLocalAccessPortKilling,
    setLocalAccessRefreshing,
    setLocalAccessRiskNoticeRemember,
    setLocalAccessSaving,
    setLocalAccessSidecarRestarting,
    setLocalAccessStarting,
    setLocalAccessState,
    setMessage,
    setPendingOAuthEmailInput,
    setPendingOAuthFieldErrors,
    setPendingWebSessionImport,
    setQuotaErrorDetail,
    setReauthRetryOAuthBinding,
    setReauthTargetAccount,
    setRefreshingGroupId,
    setRemovingGroupAccountIds,
    setSavedMfaRecords,
    setSavingPendingOAuthAccount,
    setSearchQuery,
    setSelected,
    setSelectedTerminal,
    setSessionWindowStats,
    setShowAddToCodexGroupModal,
    setShowCodexGroupModal,
    setShowLocalAccessHealthModal,
    setShowLocalAccessHideConfirm,
    setShowLocalAccessModal,
    setShowLocalAccessQuotaStatsModal,
    setShowTagFilter,
    setShowTagModal,
    setSortBy,
    setSortDirection,
    setTagDeleteConfirm,
    setTokenImportProgress,
    setTokenInput,
    setWakeupPresetManagerSignal,
    shouldShowPendingOAuthDraftForm,
    showAddModal,
    showAddToCodexGroupModal,
    showCodexGroupModal,
    showExportModal,
    showLocalAccessHealthModal,
    showLocalAccessHideConfirm,
    showLocalAccessModal,
    showLocalAccessQuotaStatsModal,
    showTagFilter,
    showTagModal,
    sortBy,
    sortDirection,
    sponsorModule,
    store,
    switchAccount,
    syncImportedAccountsToApiService,
    syncImportedToApiService,
    t,
    tagDeleteConfirm,
    tagDeleteConfirmError,
    tagDeleteConfirmErrorScrollKey,
    tagFilter,
    tagFilterRef,
    terminalOptions,
    toggleFilterTypeValue,
    toggleGroupFilterValue,
    togglePrivacyMode,
    toggleSelect,
    toggleSelectAll,
    toggleTagFilterValue,
    tokenImportProgress,
    tokenInput,
    untaggedKey,
    updateAccountInstanceAccess,
    updateAccountName,
    updateActiveAccountNoteForm,
    updateApiKeyBoundOAuthAccount,
    updateApiKeyCredentials,
    validateApiKeyCredentialInputs,
    viewMode,
    wakeupPresetManagerSignal,
  };
}
