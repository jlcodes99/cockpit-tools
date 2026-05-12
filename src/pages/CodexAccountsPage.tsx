import {
  useState,
  useEffect,
  useRef,
  useMemo,
  useCallback,
  Fragment,
  type ReactElement,
  type MouseEvent as ReactMouseEvent,
} from "react";
import {
  Plus,
  RefreshCw,
  Download,
  Upload,
  Trash2,
  X,
  Globe,
  KeyRound,
  Power,
  Database,
  Copy,
  Check,
  Play,
  RotateCw,
  Repeat,
  CircleAlert,
  Info,
  Rows3,
  LayoutGrid,
  List,
  Search,
  ArrowDownWideNarrow,
  ArrowUp,
  ArrowDown,
  GripVertical,
  Clock,
  Calendar,
  Tag,
  Star,
  Eye,
  EyeOff,
  BookOpen,
  FileUp,
  FileText,
  ExternalLink,
  Pencil,
  FolderOpen,
  FolderPlus,
  ChevronRight,
  LogOut,
  Server,
  Wrench,
  Terminal,
} from "lucide-react";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import { useCodexInstanceStore } from "../stores/useCodexInstanceStore";
import * as codexService from "../services/codexService";
import * as codexInstanceService from "../services/codexInstanceService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { TagEditModal } from "../components/TagEditModal";
import {
  ExportJsonModal,
  maskJsonPreviewContent,
} from "../components/ExportJsonModal";
import {
  ModalErrorMessage,
  useModalErrorState,
} from "../components/ModalErrorMessage";
import { PaginationControls } from "../components/PaginationControls";
import {
  CodexAccountGroupModal,
  CodexAddToGroupModal,
} from "../components/CodexAccountGroupModal";
import { CodexGroupAccountPickerModal } from "../components/CodexGroupAccountPickerModal";
import { CodexLocalAccessModal } from "../components/CodexLocalAccessModal";
import {
  type CodexAccountGroup,
  assignAccountsToCodexGroup,
  cleanupDeletedCodexAccounts,
  deleteCodexGroup,
  getCodexAccountGroups,
  removeAccountsFromCodexGroup,
} from "../services/codexAccountGroupService";
import {
  hasCodexAccountStructure,
  formatCodexLoginProvider,
  getCodexAuthMetadata,
  getCodexPlanFilterKey,
  getCodexSubscriptionPresentation,
  hasCodexAccountName,
  isCodexApiKeyAccount,
  isCodexExplicitFreePlanType,
  isCodexNewApiAccount,
  isCodexTeamLikePlan,
  type CodexApiProviderMode,
  type CodexQuotaErrorInfo,
} from "../types/codex";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";

import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  confirm as confirmDialog,
  open as openFileDialog,
} from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import {
  CodexOverviewTabsHeader,
  CodexTab,
} from "../components/CodexOverviewTabsHeader";
import { CodexInstancesContent } from "./CodexInstancesPage";
import { CodexSessionManager } from "../components/codex/CodexSessionManager";
import { CodexWakeupContent } from "../components/codex/CodexWakeupContent";
import { CodexModelProviderManager } from "../components/codex/CodexModelProviderManager";
import { QuickSettingsPopover } from "../components/QuickSettingsPopover";
import { useProviderAccountsPage } from "../hooks/useProviderAccountsPage";
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from "../components/MultiSelectFilterDropdown";
import {
  SingleSelectFilterDropdown,
  type SingleSelectFilterOption,
} from "../components/SingleSelectFilterDropdown";
import type { CodexAccount } from "../types/codex";
import type {
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessState,
} from "../types/codexLocalAccess";
import {
  CODEX_API_SERVICE_BIND_ID,
  type InstanceProfile,
} from "../types/instance";
import {
  CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
  isCodexCodeReviewQuotaVisibleByDefault,
} from "../utils/codexPreferences";
import { formatCodexSessionVisibilityRepairMessage } from "../utils/codexSessionVisibility";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import { compareCurrentAccountFirst } from "../utils/currentAccountSort";
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  CODEX_API_PROVIDER_PRESETS,
  COCKPIT_API_PROVIDER_ID,
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from "../utils/codexProviderPresets";
import {
  formatCodexQuotaPoolPercent,
  summarizeCodexQuotaPool,
} from "../utils/codexQuotaPool";
import {
  findCodexModelProviderById,
  findCodexModelProviderByBaseUrl,
  listCodexModelProviders,
  type CodexModelProvider,
  upsertCodexModelProviderFromCredential,
} from "../services/codexModelProviderService";
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
} from "../utils/accountValidityFilter";
import {
  buildPaginatedGroups,
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from "../hooks/usePagination";
import {
  buildCodexExportContent,
  buildCodexExportFileNameBase,
  type CodexExportFormat,
} from "../utils/codexExportFormats";
import {
  normalizeAccountsOverviewScope,
  readAccountsOverviewFilterField,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from "../utils/accountsOverviewFilterPersistence";
import {
  getCodexLocalAccessRiskNoticeConfirmLabel,
  isCodexLocalAccessRiskNoticeDismissed,
  setCodexLocalAccessRiskNoticeDismissed,
  type CodexLocalAccessRiskNoticeAction,
} from "../utils/codexLocalAccessRiskNotice";
import md5 from "blueimp-md5";

const CODEX_TOKEN_SINGLE_EXAMPLE = `{
  "tokens": {
    "id_token": "eyJ...",
    "access_token": "eyJ...",
    "refresh_token": "rt_..."
  }
}`;
const CODEX_TOKEN_REFRESH_ONLY_EXAMPLE = `{
  "refresh_token": "rt_..."
}`;
const CODEX_TOKEN_BATCH_EXAMPLE = `[
  {
    "id": "codex_demo_1",
    "email": "user@example.com",
    "tokens": {
      "id_token": "eyJ...",
      "access_token": "eyJ...",
      "refresh_token": "rt_..."
    },
    "created_at": 1730000000,
    "last_used": 1730000000
  }
]`;
const OPENAI_OFFICIAL_PRESET_ID = "openai_official";
const COCKPIT_API_BASE_URL = "https://chongcodex.cn/v1";

function normalizeCodexApiBaseUrl(rawValue?: string | null): string {
  return normalizeHttpBaseUrl(rawValue ?? "") ?? "";
}

function inferCodexAccountProviderMode(
  account: CodexAccount,
): CodexApiProviderMode {
  if (
    account.api_provider_mode === "custom" ||
    account.api_provider_mode === "openai_builtin"
  ) {
    return account.api_provider_mode;
  }
  const normalizedBaseUrl = normalizeCodexApiBaseUrl(account.api_base_url);
  if (!normalizedBaseUrl || normalizedBaseUrl === "https://api.openai.com/v1") {
    return "openai_builtin";
  }
  return "custom";
}
const CODEX_OVERVIEW_LAYOUT_MODE_KEY =
  "agtools.codex.accounts.overview_layout_mode";
const CODEX_LOCAL_ACCESS_EXPANDED_KEY =
  "agtools.codex.local_access_entry_expanded.v1";
const CODEX_CUSTOM_SORT_ORDER_KEY =
  "agtools.codex.accounts.custom_sort_order.v1";
const DEFAULT_CODEX_API_PROVIDER_ID = COCKPIT_API_PROVIDER_ID;
const DEFAULT_CODEX_API_BASE_URL = COCKPIT_API_BASE_URL;
const CODEX_LOCAL_ACCESS_FALLBACK_PORT = 54140;
const CODEX_LOCAL_ACCESS_FALLBACK_BASE_URL = `http://127.0.0.1:${CODEX_LOCAL_ACCESS_FALLBACK_PORT}/v1`;
const CODEX_LOCAL_ACCESS_FALLBACK_API_KEY_MASK = "agt_codex_••••••••••••";
const CODEX_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope("Codex");
const FILTER_TYPES_FIELD = "filter_types";
const EXPIRY_FILTER_FIELD = "expiry_filter";
const GROUP_FILTER_FIELD = "group_filter";
const ACTIVE_GROUP_ID_FIELD = "active_group_id";

type CodexOverviewLayoutMode = "compact" | "list" | "grid";
type CodexLaunchCredentialKind = "api-key" | "api-service" | "account";
type CodexLaunchCredentialType = "api" | "account";
type CodexApiSwitchNoticeContext = {
  from: CodexLaunchCredentialKind;
  to: CodexLaunchCredentialKind;
};

function getCodexLaunchCredentialKind(
  account: CodexAccount,
): CodexLaunchCredentialKind {
  return isCodexApiKeyAccount(account) ? "api-key" : "account";
}

function getCodexLaunchCredentialType(
  kind: CodexLaunchCredentialKind,
): CodexLaunchCredentialType {
  return kind === "account" ? "account" : "api";
}

type CockpitApiJsonRecord = Record<string, unknown>;

function toCockpitApiRecord(value: unknown): CockpitApiJsonRecord | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as CockpitApiJsonRecord)
    : null;
}

function readCockpitApiString(
  record: CockpitApiJsonRecord | null,
  key: string,
): string {
  const value = record?.[key];
  return typeof value === "string" ? value.trim() : "";
}

function readCockpitApiNumber(
  record: CockpitApiJsonRecord | null,
  key: string,
): number {
  const value = record?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function formatCockpitApiInteger(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(
    Math.max(0, value),
  );
}

function getCockpitApiUsageRecord(
  account: CodexAccount,
): CockpitApiJsonRecord | null {
  const raw = toCockpitApiRecord(account.quota?.raw_data);
  const profile = toCockpitApiRecord(raw?.profile);
  return toCockpitApiRecord(raw?.usage) ?? toCockpitApiRecord(profile?.usage);
}

function getCockpitApiStatsRecord(
  account: CodexAccount,
): CockpitApiJsonRecord | null {
  const usage = getCockpitApiUsageRecord(account);
  return toCockpitApiRecord(usage?.stats);
}

function readCodexCustomSortOrder(): string[] {
  try {
    const raw = localStorage.getItem(CODEX_CUSTOM_SORT_ORDER_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is string =>
        typeof item === "string" && item.trim().length > 0,
    );
  } catch {
    return [];
  }
}

function writeCodexCustomSortOrder(accountIds: string[]): void {
  try {
    localStorage.setItem(
      CODEX_CUSTOM_SORT_ORDER_KEY,
      JSON.stringify(accountIds),
    );
  } catch {
    // ignore persistence failures
  }
}

interface CodexOverviewGeneralConfig {
  codex_local_access_entry_visible?: boolean;
}

function normalizeCodexOverviewLayoutMode(
  value: string | null,
): CodexOverviewLayoutMode | null {
  if (value === "compact" || value === "list" || value === "grid") return value;
  return null;
}

function isHttpLikeUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const parsed = new URL(trimmed);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    const lower = trimmed.toLowerCase();
    return lower.startsWith("http://") || lower.startsWith("https://");
  }
}

function normalizeHttpBaseUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:")
      return null;
    return trimmed.replace(/\/+$/, "");
  } catch {
    return null;
  }
}

function buildExportFileName(baseName: string): string {
  const date = new Date().toISOString().slice(0, 10);
  return `${baseName}_${date}.json`;
}

function getDirectoryPath(filePath: string): string {
  const slashIndex = Math.max(
    filePath.lastIndexOf("/"),
    filePath.lastIndexOf("\\"),
  );
  if (slashIndex <= 0) {
    return filePath;
  }
  return filePath.slice(0, slashIndex);
}

function joinFilePath(directory: string, fileName: string): string {
  if (!directory) return fileName;
  const separator = directory.includes("\\") ? "\\" : "/";
  return directory.endsWith("/") || directory.endsWith("\\")
    ? `${directory}${fileName}`
    : `${directory}${separator}${fileName}`;
}

function normalizePathForCompare(value?: string | null): string {
  return (value || "").trim().replace(/[\\/]+$/, "");
}

function sanitizeCodexCliInstanceName(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "Codex CLI";
  return trimmed
    .replace(/[\\/:*?"<>|]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function maskCodexApiKey(value: string): string {
  const raw = value.trim();
  if (!raw) return raw;
  if (raw.startsWith("sk-")) return "sk-••••••••••••••••";
  return "••••••••••••••••";
}

export function CodexAccountsPage() {
  const [activeTab, setActiveTab] = useState<CodexTab>("overview");
  const [wakeupPresetManagerSignal, setWakeupPresetManagerSignal] = useState(0);
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
  const [removingGroupAccountIds, setRemovingGroupAccountIds] = useState<
    Set<string>
  >(new Set());
  const [localAccessState, setLocalAccessState] =
    useState<CodexLocalAccessState | null>(null);
  const [showLocalAccessModal, setShowLocalAccessModal] = useState(false);
  const [localAccessModalMode, setLocalAccessModalMode] = useState<
    "panel" | "members"
  >("panel");
  const [localAccessSaving, setLocalAccessSaving] = useState(false);
  const [localAccessTesting, setLocalAccessTesting] = useState(false);
  const [localAccessStarting, setLocalAccessStarting] = useState(false);
  const [localAccessRefreshing, setLocalAccessRefreshing] = useState(false);
  const [localAccessPortKilling, setLocalAccessPortKilling] = useState(false);
  const [showLocalAccessHideConfirm, setShowLocalAccessHideConfirm] =
    useState(false);
  const [localAccessHideSubmitting, setLocalAccessHideSubmitting] =
    useState(false);
  const [localAccessRiskNoticeAction, setLocalAccessRiskNoticeAction] =
    useState<CodexLocalAccessRiskNoticeAction | null>(null);
  const [localAccessRiskNoticeRemember, setLocalAccessRiskNoticeRemember] =
    useState(false);
  const [apiSwitchNoticeContext, setApiSwitchNoticeContext] =
    useState<CodexApiSwitchNoticeContext | null>(null);
  const [apiSwitchNoticeRepairing, setApiSwitchNoticeRepairing] =
    useState(false);
  const [apiSwitchNoticeRepairResult, setApiSwitchNoticeRepairResult] =
    useState<string | null>(null);
  const {
    message: apiSwitchNoticeError,
    scrollKey: apiSwitchNoticeErrorScrollKey,
    set: setApiSwitchNoticeError,
  } = useModalErrorState();
  const [localAccessCopiedField, setLocalAccessCopiedField] = useState<
    "baseUrl" | "apiKey" | null
  >(null);
  const [localAccessKeyVisible, setLocalAccessKeyVisible] = useState(false);
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

  const reloadCodexGroups = useCallback(async () => {
    setCodexGroups(await getCodexAccountGroups());
  }, []);

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

  const store = useCodexAccountStore();
  const codexInstanceStore = useCodexInstanceStore();
  const [cliLaunchingAccountId, setCliLaunchingAccountId] = useState<
    string | null
  >(null);
  const [cockpitApiPanelAccountId, setCockpitApiPanelAccountId] = useState<
    string | null
  >(null);
  const [editingAccountNoteId, setEditingAccountNoteId] = useState<
    string | null
  >(null);
  const [editingAccountNoteValue, setEditingAccountNoteValue] = useState("");
  const [savingAccountNote, setSavingAccountNote] = useState(false);
  const {
    message: accountNoteError,
    scrollKey: accountNoteErrorScrollKey,
    set: setAccountNoteError,
  } = useModalErrorState();

  // Use the common hook WITHOUT oauthService since Codex uses Tauri event-based OAuth
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
    handleBatchDelete,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    setDeleteConfirm,
    deleting,
    confirmDelete,
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
    openAddModal,
    closeAddModal,
    externalImportProgress,
    closeExternalImportProgressModal,
    formatDate,
    normalizeTag,
    saveJsonFile,
  } = page;

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
    try {
      const nextState =
        await codexLocalAccessService.getCodexLocalAccessState();
      setLocalAccessState(nextState);
    } catch (error) {
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
    void reloadLocalAccessEntryVisibility();
  }, [reloadLocalAccessEntryVisibility]);

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
      void reloadLocalAccessLaunchCurrent();
    };
    window.addEventListener("config-updated", handleConfigUpdated);
    return () => {
      window.removeEventListener("config-updated", handleConfigUpdated);
    };
  }, [reloadLocalAccessEntryVisibility, reloadLocalAccessLaunchCurrent]);

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

  const formattedExportContent = useMemo(() => {
    if (!exportJsonContent) {
      return {
        type: "single" as const,
        fileNameBase: buildCodexExportFileNameBase(
          exportFileNameBase,
          exportFormat,
        ),
        jsonContent: "",
      };
    }
    try {
      return buildCodexExportContent(
        exportJsonContent,
        exportFormat,
        exportFileNameBase,
      );
    } catch (error) {
      console.error("[CodexExport] transform failed:", error);
      return buildCodexExportContent(
        exportJsonContent,
        "cockpit_tools",
        exportFileNameBase,
      );
    }
  }, [exportFileNameBase, exportFormat, exportJsonContent]);

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
    switchAccount,
    refreshQuota,
    hydrateAccountProfilesIfNeeded,
    updateAccountName,
    updateApiKeyCredentials,
  } = store;

  const editingAccountNoteAccount = useMemo(
    () =>
      accounts.find((account) => account.id === editingAccountNoteId) || null,
    [accounts, editingAccountNoteId],
  );

  const openAccountNoteModal = useCallback(
    (account: CodexAccount) => {
      setEditingAccountNoteId(account.id);
      setEditingAccountNoteValue(account.account_note || "");
      setAccountNoteError(null);
    },
    [setAccountNoteError],
  );

  const closeAccountNoteModal = useCallback(() => {
    if (savingAccountNote) return;
    setEditingAccountNoteId(null);
    setEditingAccountNoteValue("");
    setAccountNoteError(null);
  }, [savingAccountNote, setAccountNoteError]);

  const handleSubmitAccountNote = useCallback(async () => {
    if (!editingAccountNoteId || savingAccountNote) return;
    setSavingAccountNote(true);
    setAccountNoteError(null);
    try {
      await store.updateAccountNote(
        editingAccountNoteId,
        editingAccountNoteValue,
      );
      setMessage({
        text: t("codex.accountNote.saved", "账号备注已保存"),
        tone: "success",
      });
      setEditingAccountNoteId(null);
      setEditingAccountNoteValue("");
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
    editingAccountNoteId,
    editingAccountNoteValue,
    savingAccountNote,
    setAccountNoteError,
    setMessage,
    store,
    t,
  ]);

  const renderAccountNoteButton = useCallback(
    (account: CodexAccount, className = "codex-account-note-chip") => {
      const hasNote = Boolean(account.account_note?.trim());
      return (
        <button
          type="button"
          className={`${className} ${hasNote ? "has-note" : "empty-note"}`}
          onClick={() => openAccountNoteModal(account)}
          title={
            hasNote
              ? account.account_note
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

  // ─── Codex-specific: OAuth via Tauri events ──────────────────────────

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
  const [oauthTokenExchangeRetryVisible, setOauthTokenExchangeRetryVisible] =
    useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [apiKeyInputVisible, setApiKeyInputVisible] = useState(false);
  const [apiBaseUrlInput, setApiBaseUrlInput] = useState(
    DEFAULT_CODEX_API_BASE_URL,
  );
  const [apiProviderPresetId, setApiProviderPresetId] = useState(
    DEFAULT_CODEX_API_PROVIDER_ID,
  );
  const [managedProviders, setManagedProviders] = useState<
    CodexModelProvider[]
  >([]);
  const [managedProvidersLoading, setManagedProvidersLoading] = useState(false);
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
  const [visibleApiKeyAccountIds, setVisibleApiKeyAccountIds] = useState<
    Set<string>
  >(() => new Set());
  const [showCodeReviewQuota, setShowCodeReviewQuota] = useState<boolean>(
    isCodexCodeReviewQuotaVisibleByDefault,
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
  const repairSessionVisibilityAcrossInstances = useCodexInstanceStore(
    (state) => state.repairSessionVisibilityAcrossInstances,
  );

  const showAddModalRef = useRef(showAddModal);
  const addTabRef = useRef(addTab);
  const addStatusRef = useRef(addStatus);
  const oauthActiveRef = useRef(false);
  const oauthLoginIdRef = useRef<string | null>(null);
  const oauthCompletingRef = useRef(false);
  const oauthEventSeqRef = useRef(0);
  const oauthAttemptSeqRef = useRef(0);
  const inlineRenameDiscardRef = useRef(false);
  const apiSwitchNoticeRepairSeqRef = useRef(0);
  const apiSwitchNoticeAutoCloseTimerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (apiSwitchNoticeAutoCloseTimerRef.current != null) {
        window.clearTimeout(apiSwitchNoticeAutoCloseTimerRef.current);
        apiSwitchNoticeAutoCloseTimerRef.current = null;
      }
    },
    [],
  );

  const selectedApiProviderPreset = useMemo(
    () => findCodexApiProviderPresetById(apiProviderPresetId),
    [apiProviderPresetId],
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
  const cockpitApiPanelAccount = useMemo(
    () =>
      cockpitApiPanelAccountId
        ? (accounts.find((item) => item.id === cockpitApiPanelAccountId) ??
          null)
        : null,
    [accounts, cockpitApiPanelAccountId],
  );

  useEffect(() => {
    if (cockpitApiPanelAccountId && !cockpitApiPanelAccount) {
      setCockpitApiPanelAccountId(null);
    }
  }, [cockpitApiPanelAccount, cockpitApiPanelAccountId]);

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
    ): {
      apiProviderMode: CodexApiProviderMode;
      apiProviderId?: string;
      apiProviderName?: string;
    } => {
      const normalizedBaseUrl = normalizeHttpBaseUrl(apiBaseUrl);
      if (
        providerPresetId === OPENAI_OFFICIAL_PRESET_ID ||
        !normalizedBaseUrl
      ) {
        return { apiProviderMode: "openai_builtin" };
      }

      const managedProvider = findCodexModelProviderById(
        managedProviders,
        providerId,
      );
      if (managedProvider) {
        return {
          apiProviderMode: "custom",
          apiProviderId: managedProvider.id,
          apiProviderName: managedProvider.name,
        };
      }

      const preset = findCodexApiProviderPresetById(providerPresetId);
      if (preset && providerPresetId !== CODEX_API_PROVIDER_CUSTOM_ID) {
        return {
          apiProviderMode: "custom",
          apiProviderId: preset.id,
          apiProviderName: preset.name,
        };
      }

      const trimmedName = customProviderName.trim();
      return {
        apiProviderMode: "custom",
        apiProviderName: trimmedName || undefined,
      };
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
    const accountIds = accounts.map((account) => account.id);
    const accountIdSet = new Set(accountIds);
    setCustomSortOrder((prev) => {
      const next = prev.filter((accountId) => accountIdSet.has(accountId));
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
    if (!showAddModal) {
      setApiKeyInput("");
      setApiKeyInputVisible(false);
      setApiBaseUrlInput(DEFAULT_CODEX_API_BASE_URL);
      setApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
      setManagedProviderId("");
      setManagedProviderApiKeyId("");
      setNewManagedProviderNameInput("");
    }
  }, [showAddModal]);

  useEffect(() => {
    if (showAddModal && addTab === "apikey") {
      setApiKeyInputVisible(false);
    }
  }, [addTab, showAddModal]);

  useEffect(() => {
    if (apiProviderPresetId === OPENAI_OFFICIAL_PRESET_ID) {
      setManagedProviderId("");
      setManagedProviderApiKeyId("");
      return;
    }
    const matched = findCodexModelProviderByBaseUrl(
      managedProviders,
      apiBaseUrlInput,
    );
    setManagedProviderId((prev) =>
      prev === (matched?.id ?? "") ? prev : (matched?.id ?? ""),
    );
    if (!matched || matched.apiKeys.length === 0) {
      setManagedProviderApiKeyId("");
      return;
    }
    setManagedProviderApiKeyId((prev) => {
      if (matched.apiKeys.some((item) => item.id === prev)) return prev;
      return matched.apiKeys[0]?.id ?? "";
    });
  }, [apiBaseUrlInput, apiProviderPresetId, managedProviders]);

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
    const matched = findCodexModelProviderByBaseUrl(
      managedProviders,
      editingApiBaseUrlCredentialsValue,
    );
    setEditingManagedProviderId((prev) =>
      prev === (matched?.id ?? "") ? prev : (matched?.id ?? ""),
    );
    if (!matched || matched.apiKeys.length === 0) {
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

    window.addEventListener(
      CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
      syncCodeReviewVisibility as EventListener,
    );
    return () => {
      window.removeEventListener(
        CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncCodeReviewVisibility as EventListener,
      );
    };
  }, []);

  // Hook provides setAddStatus/setAddMessage but we need refs to page's versions
  const { setAddStatus, setAddMessage, resetAddModalState, setShowAddModal } =
    page;

  const handleOauthPrepareError = useCallback(
    (e: unknown) => {
      console.error("[CodexOAuth] 准备授权链接失败", { error: String(e) });
      oauthActiveRef.current = false;
      setOauthTimeoutInfo(null);
      setOauthCallbackSubmitting(false);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
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

  const completeOauthSuccess = useCallback(async () => {
    oauthLog("授权完成并保存成功", { loginId: oauthLoginIdRef.current });
    await fetchAccounts();
    await fetchCurrentAccount();
    await emitAccountsChanged({
      platformId: "codex",
      reason: "oauth",
    });
    setAddStatus("success");
    setAddMessage(t("common.shared.oauth.success", "授权成功"));
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
    setTimeout(() => {
      setShowAddModal(false);
      resetAddModalState();
    }, 1200);
  }, [
    fetchAccounts,
    fetchCurrentAccount,
    t,
    oauthLog,
    setAddStatus,
    setAddMessage,
    setShowAddModal,
    resetAddModalState,
  ]);

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
          await codexService.completeCodexOAuthLogin(loginId);
          await completeOauthSuccess();
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
        setAddStatus("idle");
        setAddMessage("");
      },
    ).then((fn) => {
      if (disposed) fn();
      else unlistenTimeout = fn;
    });

    return () => {
      disposed = true;
      unlistenExtension?.();
      unlistenTimeout?.();
    };
  }, [
    completeOauthError,
    completeOauthSuccess,
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
    if (!showAddModal || addTab !== "oauth" || oauthUrl || oauthTimeoutInfo)
      return;
    prepareOauthUrl();
  }, [showAddModal, addTab, oauthUrl, oauthTimeoutInfo, prepareOauthUrl]);

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
  }, [
    showAddModal,
    addTab,
    oauthUrl,
    oauthTimeoutInfo,
    oauthCallbackInput,
    oauthCallbackSubmitting,
    oauthCallbackError,
    oauthPrepareError,
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
      await codexService.completeCodexOAuthLogin(loginId);
      await completeOauthSuccess();
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
      await codexService.completeCodexOAuthLogin(loginId);
      await completeOauthSuccess();
    } catch (e) {
      completeOauthError(e, true);
      setOauthCallbackError(String(e).replace(/^Error:\s*/, ""));
    } finally {
      oauthCompletingRef.current = false;
      setOauthCallbackSubmitting(false);
    }
  };

  // ─── Codex-specific: Switch / Import ─────────────────────────────────

  const resolveCurrentCodexLaunchCredentialKind =
    useCallback(async (): Promise<CodexLaunchCredentialKind | null> => {
      try {
        const activeAccount = await codexService.getCurrentCodexAccount();
        if (activeAccount) {
          return getCodexLaunchCredentialKind(activeAccount);
        }

        const instances = await codexInstanceService.listInstances();
        const defaultInstance = instances.find(
          (instance) => instance.isDefault,
        );
        return defaultInstance?.bindAccountId === CODEX_API_SERVICE_BIND_ID
          ? "api-service"
          : null;
      } catch (error) {
        console.warn(
          "Failed to resolve current Codex launch credential kind:",
          error,
        );
        return null;
      }
    }, []);

  const shouldShowApiSwitchVisibilityNotice = useCallback(
    (
      currentKind: CodexLaunchCredentialKind | null,
      targetKind: CodexLaunchCredentialKind | null,
    ) => {
      if (!currentKind || !targetKind) {
        return false;
      }
      return (
        getCodexLaunchCredentialType(currentKind) !==
        getCodexLaunchCredentialType(targetKind)
      );
    },
    [],
  );

  const closeApiSwitchVisibilityNotice = useCallback(() => {
    apiSwitchNoticeRepairSeqRef.current += 1;
    if (apiSwitchNoticeAutoCloseTimerRef.current != null) {
      window.clearTimeout(apiSwitchNoticeAutoCloseTimerRef.current);
      apiSwitchNoticeAutoCloseTimerRef.current = null;
    }
    setApiSwitchNoticeContext(null);
    setApiSwitchNoticeRepairing(false);
    setApiSwitchNoticeRepairResult(null);
    setApiSwitchNoticeError(null);
  }, [setApiSwitchNoticeError]);

  const runApiSwitchVisibilityRepair = useCallback(async () => {
    const repairSeq = apiSwitchNoticeRepairSeqRef.current + 1;
    apiSwitchNoticeRepairSeqRef.current = repairSeq;
    if (apiSwitchNoticeAutoCloseTimerRef.current != null) {
      window.clearTimeout(apiSwitchNoticeAutoCloseTimerRef.current);
      apiSwitchNoticeAutoCloseTimerRef.current = null;
    }
    setApiSwitchNoticeError(null);
    setApiSwitchNoticeRepairResult(null);
    setApiSwitchNoticeRepairing(true);
    try {
      const summary = await repairSessionVisibilityAcrossInstances();
      if (apiSwitchNoticeRepairSeqRef.current === repairSeq) {
        setApiSwitchNoticeRepairResult(
          formatCodexSessionVisibilityRepairMessage(summary, t),
        );
        apiSwitchNoticeAutoCloseTimerRef.current = window.setTimeout(() => {
          if (apiSwitchNoticeRepairSeqRef.current !== repairSeq) return;
          apiSwitchNoticeRepairSeqRef.current += 1;
          apiSwitchNoticeAutoCloseTimerRef.current = null;
          setApiSwitchNoticeContext(null);
          setApiSwitchNoticeRepairing(false);
          setApiSwitchNoticeRepairResult(null);
          setApiSwitchNoticeError(null);
        }, 1200);
      }
    } catch {
      if (apiSwitchNoticeRepairSeqRef.current === repairSeq) {
        setApiSwitchNoticeError(
          t(
            "codex.apiSwitchNotice.repairFailed",
            "自动修复失败。你仍可稍后在「会话管理」中使用「修复可见性」重试。",
          ),
        );
      }
    } finally {
      if (apiSwitchNoticeRepairSeqRef.current === repairSeq) {
        setApiSwitchNoticeRepairing(false);
      }
    }
  }, [repairSessionVisibilityAcrossInstances, setApiSwitchNoticeError, t]);

  const openApiSwitchVisibilityNotice = useCallback(
    (context: CodexApiSwitchNoticeContext) => {
      setApiSwitchNoticeContext(context);
      setApiSwitchNoticeRepairResult(null);
      setApiSwitchNoticeError(null);
      void runApiSwitchVisibilityRepair();
    },
    [runApiSwitchVisibilityRepair, setApiSwitchNoticeError],
  );

  const formatCodexLaunchCredentialKindLabel = useCallback(
    (kind: CodexLaunchCredentialKind) => {
      if (kind === "api-service") {
        return t("codex.apiSwitchNotice.type.apiService", "API 服务");
      }
      if (kind === "api-key") {
        return t("codex.apiSwitchNotice.type.apiKey", "API Key");
      }
      return t("codex.apiSwitchNotice.type.account", "账号");
    },
    [t],
  );

  const formatCodexAuthFailureMessage = useCallback(
    (rawError: unknown) => {
      const raw = String(rawError)
        .replace(/^Error:\s*/, "")
        .trim();
      const lower = raw.toLowerCase();
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
        lower.includes("refresh_token_reused") ||
        raw.includes("refresh_token 已被其它客户端或实例使用过")
      ) {
        return t(
          "codex.authError.refreshTokenReused",
          "Codex 授权已失效：refresh_token 已被其它客户端或实例使用过。请重新登录，并避免官方 Codex、其它实例或外部工具同时刷新同一账号。",
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
    async (accountId: string, options?: { showSuccessMessage?: boolean }) => {
      const showSuccessMessage = options?.showSuccessMessage ?? true;
      setMessage(null);
      setSwitching(accountId);
      try {
        const account = await switchAccount(accountId);
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
      }
    },
    [maskAccountText, setMessage, switchAccount, t],
  );

  const handleSwitch = async (accountId: string) => {
    const targetAccount = accounts.find((account) => account.id === accountId);

    try {
      const currentKind = await resolveCurrentCodexLaunchCredentialKind();
      const targetKind = targetAccount
        ? getCodexLaunchCredentialKind(targetAccount)
        : null;
      const shouldShowVisibilityNotice = shouldShowApiSwitchVisibilityNotice(
        currentKind,
        targetKind,
      );
      const switchedAccount = await executeCodexAccountSwitch(accountId);
      if (shouldShowVisibilityNotice && currentKind && targetKind) {
        openApiSwitchVisibilityNotice({
          from: currentKind,
          to: getCodexLaunchCredentialKind(switchedAccount),
        });
      }
    } catch (e) {
      setMessage({
        text: t("codex.switchFailed", {
          error: formatCodexAuthFailureMessage(e),
        }),
        tone: "error",
      });
    }
  };

  const resolveCodexCliInstanceForAccount = async (
    account: CodexAccount,
    workingDir: string,
  ): Promise<InstanceProfile> => {
    const normalizedWorkingDir = normalizePathForCompare(workingDir);
    const instances = await codexInstanceService.listInstances();
    const existing = instances.find(
      (instance) =>
        !instance.isDefault &&
        (instance.launchMode ?? "app") === "cli" &&
        instance.bindAccountId === account.id &&
        normalizePathForCompare(instance.workingDir) === normalizedWorkingDir,
    );
    if (existing) {
      return existing;
    }

    const defaults = await codexInstanceService.getInstanceDefaults();
    const presentation = buildCodexAccountPresentation(account, t);
    const displayName = presentation.displayName || account.email || account.id;
    const instanceHash = md5(`${account.id}|${normalizedWorkingDir}`).substring(
      0,
      12,
    );
    const instanceName = sanitizeCodexCliInstanceName(
      `${displayName} CLI ${instanceHash.substring(0, 6)}`,
    );
    const userDataDir = joinFilePath(defaults.rootDir, `cli-${instanceHash}`);

    return await codexInstanceService.createInstance({
      name: instanceName,
      userDataDir,
      workingDir: normalizedWorkingDir,
      extraArgs: "",
      bindAccountId: account.id,
      launchMode: "cli",
      copySourceInstanceId: "__default__",
      initMode: "copy",
    });
  };

  const resolveCodexCliInstanceForApiService = async (
    workingDir: string,
  ): Promise<InstanceProfile> => {
    const normalizedWorkingDir = normalizePathForCompare(workingDir);
    const instances = await codexInstanceService.listInstances();
    const existing = instances.find(
      (instance) =>
        !instance.isDefault &&
        (instance.launchMode ?? "app") === "cli" &&
        instance.bindAccountId === CODEX_API_SERVICE_BIND_ID &&
        normalizePathForCompare(instance.workingDir) === normalizedWorkingDir,
    );
    if (existing) {
      return existing;
    }

    const defaults = await codexInstanceService.getInstanceDefaults();
    const instanceHash = md5(
      `${CODEX_API_SERVICE_BIND_ID}|${normalizedWorkingDir}`,
    ).substring(0, 12);
    const instanceName = sanitizeCodexCliInstanceName(
      `${t("codex.localAccess.title", "API 服务")} CLI ${instanceHash.substring(0, 6)}`,
    );
    const userDataDir = joinFilePath(
      defaults.rootDir,
      `cli-api-service-${instanceHash}`,
    );

    return await codexInstanceService.createInstance({
      name: instanceName,
      userDataDir,
      workingDir: normalizedWorkingDir,
      extraArgs: "",
      bindAccountId: CODEX_API_SERVICE_BIND_ID,
      launchMode: "cli",
      copySourceInstanceId: "__default__",
      initMode: "copy",
    });
  };

  const handleLaunchCodexCli = async (account: CodexAccount) => {
    if (cliLaunchingAccountId) return;
    setMessage(null);
    setCliLaunchingAccountId(account.id);
    try {
      const selected = await openFileDialog({
        directory: true,
        multiple: false,
        title: t("codex.cli.selectWorkingDir", "选择 Codex CLI 工作目录"),
      });
      if (!selected || typeof selected !== "string") {
        return;
      }

      const instance = await resolveCodexCliInstanceForAccount(
        account,
        selected,
      );
      const prepared = await codexInstanceService.startInstance(instance.id);
      const result =
        await codexInstanceService.executeCodexInstanceLaunchCommand(
          prepared.id,
        );
      await codexInstanceStore.refreshInstances();
      setMessage({
        text: result || t("codex.cli.launchSuccess", "已启动 Codex CLI"),
      });
    } catch (e) {
      setMessage({
        text: t(
          "codex.cli.launchFailed",
          "启动 Codex CLI 失败: {{error}}",
        ).replace("{{error}}", String(e).replace(/^Error:\s*/, "")),
        tone: "error",
      });
    } finally {
      setCliLaunchingAccountId(null);
    }
  };

  const handleLaunchLocalAccessCli = async () => {
    if (cliLaunchingAccountId) return;
    if (!localAccessCollection) {
      setMessage({
        text: t("codex.localAccess.testUnavailable", "当前 API 服务地址不可用"),
        tone: "error",
      });
      return;
    }
    setMessage(null);
    setCliLaunchingAccountId(CODEX_API_SERVICE_BIND_ID);
    try {
      const selected = await openFileDialog({
        directory: true,
        multiple: false,
        title: t("codex.cli.selectWorkingDir", "选择 Codex CLI 工作目录"),
      });
      if (!selected || typeof selected !== "string") {
        return;
      }

      const instance = await resolveCodexCliInstanceForApiService(selected);
      const prepared = await codexInstanceService.startInstance(instance.id);
      const result =
        await codexInstanceService.executeCodexInstanceLaunchCommand(
          prepared.id,
        );
      await codexInstanceStore.refreshInstances();
      setMessage({
        text: result || t("codex.cli.launchSuccess", "已启动 Codex CLI"),
      });
    } catch (e) {
      setMessage({
        text: t(
          "codex.cli.launchFailed",
          "启动 Codex CLI 失败: {{error}}",
        ).replace("{{error}}", String(e).replace(/^Error:\s*/, "")),
        tone: "error",
      });
    } finally {
      setCliLaunchingAccountId(null);
    }
  };

  const handleImportFromLocal = async () => {
    page.setAddStatus("loading");
    page.setAddMessage(t("codex.import.importing", "正在导入本地账号..."));
    try {
      const account = await codexService.importCodexFromLocal();
      await fetchAccounts();
      await new Promise((resolve) => setTimeout(resolve, 180));
      await fetchAccounts();
      await emitAccountsChanged({
        platformId: "codex",
        reason: "import",
      });
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

  const handleImportFromFiles = async () => {
    let unlistenProgress: UnlistenFn | undefined;
    try {
      const selected = await openFileDialog({
        multiple: true,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!selected || (Array.isArray(selected) && selected.length === 0))
        return;
      const paths = Array.isArray(selected) ? selected : [selected];
      page.setAddStatus("loading");
      page.setAddMessage(
        t("modals.import.importingFiles", { count: paths.length }),
      );

      unlistenProgress = await listen<{
        current: number;
        total: number;
        email: string;
      }>("codex:file-import-progress", (event) => {
        const { current, total, email } = event.payload ?? {};
        if (current > 0 && total > 0) {
          const label = email ? ` ${email}` : "";
          page.setAddMessage(
            `${t("modals.import.importingFiles", { count: total })} ${current}/${total}${label}`,
          );
        }
      });

      const result = await codexService.importCodexFromFiles(paths);
      const { imported, failed } = result;
      await fetchAccounts();
      if (imported.length > 0) {
        await emitAccountsChanged({
          platformId: "codex",
          reason: "import",
        });
      }
      if (imported.length === 0 && failed.length === 0) {
        page.setAddStatus("error");
        page.setAddMessage(t("modals.import.noAccountsFound"));
      } else if (failed.length > 0) {
        const failedList = failed.map((f) => f.email).join(", ");
        page.setAddStatus(imported.length > 0 ? "success" : "error");
        page.setAddMessage(
          `${t("messages.importSuccess", { count: imported.length })}，${t("messages.importPartialFailed", { failCount: failed.length, failList: failedList })}`,
        );
      } else {
        page.setAddStatus("success");
        page.setAddMessage(
          t("messages.importSuccess", { count: imported.length }),
        );
      }
    } catch (e) {
      page.setAddStatus("error");
      page.setAddMessage(t("messages.importFailed", { error: String(e) }));
    } finally {
      if (unlistenProgress) unlistenProgress();
    }
  };

  const handleSelectApiProviderPreset = useCallback((providerId: string) => {
    setApiProviderPresetId(providerId);
    const preset = findCodexApiProviderPresetById(providerId);
    if (!preset || preset.baseUrls.length === 0) return;
    setApiBaseUrlInput(preset.baseUrls[0]);
  }, []);

  const handleSelectManagedProvider = useCallback(
    (providerId: string) => {
      setApiProviderPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
      setManagedProviderId(providerId);
      const provider = managedProviders.find((item) => item.id === providerId);
      if (!provider) return;
      setApiBaseUrlInput(provider.baseUrl);
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

  const handleSelectEditingApiProviderPreset = useCallback(
    (providerId: string) => {
      setEditingApiProviderPresetId(providerId);
      const preset = findCodexApiProviderPresetById(providerId);
      if (!preset || preset.baseUrls.length === 0) return;
      setEditingApiBaseUrlCredentialsValue(preset.baseUrls[0]);
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
      );
      setMessage({
        text: t("codex.quickSwitch.success", {
          defaultValue: "已切换到供应商：{{provider}}",
          provider: selectedQuickSwitchProvider.name,
        }),
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
    const providerPayload = buildApiProviderPayload(
      apiBaseUrlInput,
      apiProviderPresetId,
      managedProviderId,
      newManagedProviderNameInput,
    );

    page.setAddStatus("loading");
    page.setAddMessage(t("common.shared.token.importing", "正在导入..."));
    try {
      const account = await codexService.addCodexAccountWithApiKey(
        validation.apiKey,
        validation.apiBaseUrl,
        providerPayload.apiProviderMode,
        providerPayload.apiProviderId,
        providerPayload.apiProviderName,
      );
      if (
        validation.apiBaseUrl &&
        providerPayload.apiProviderMode === "custom" &&
        providerPayload.apiProviderId !== COCKPIT_API_PROVIDER_ID
      ) {
        try {
          await upsertCodexModelProviderFromCredential({
            providerId: providerPayload.apiProviderId ?? null,
            providerName: providerPayload.apiProviderName ?? null,
            apiBaseUrl: validation.apiBaseUrl,
            apiKey: validation.apiKey,
          });
          await reloadManagedProviders();
        } catch (providerErr) {
          console.warn(
            "[CodexModelProviders] 添加账号后写入供应商失败",
            providerErr,
          );
        }
      }
      await fetchAccounts();
      await fetchCurrentAccount();
      await emitAccountsChanged({
        platformId: "codex",
        reason: "import",
      });
      page.setAddStatus("success");
      page.setAddMessage(
        t("codex.import.successMsg", "导入成功: {{email}}").replace(
          "{{email}}",
          maskAccountText(account.email),
        ),
      );
      setApiKeyInput("");
      setApiBaseUrlInput(DEFAULT_CODEX_API_BASE_URL);
      setApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
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

  const handleTokenImport = async () => {
    const trimmed = tokenInput.trim();
    if (!trimmed) {
      page.setAddStatus("error");
      page.setAddMessage(
        t("common.shared.token.empty", "请输入 Token 或 JSON"),
      );
      return;
    }
    page.setAddStatus("loading");
    page.setAddMessage(t("common.shared.token.importing", "正在导入..."));
    try {
      const imported = await codexService.importCodexFromJson(trimmed);
      await fetchAccounts();
      if (imported.length > 0) {
        await emitAccountsChanged({
          platformId: "codex",
          reason: "import",
        });
      }
      page.setAddStatus("success");
      page.setAddMessage(
        t(
          "common.shared.token.importSuccessMsg",
          "成功导入 {{count}} 个账号",
        ).replace("{{count}}", String(imported.length)),
      );
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
        setMessage({ text: t("fingerprints.messages.renamed", "已重命名") });
      } catch (e) {
        setMessage({
          text: `${t("fingerprints.messages.renameFailed", "重命名失败")}: ${String(e)}`,
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
    const providerPayload = buildApiProviderPayload(
      editingApiBaseUrlCredentialsValue,
      editingApiProviderPresetId,
      editingManagedProviderId,
      editingNewManagedProviderNameInput,
    );

    setSavingApiKeyCredentials(true);
    try {
      await updateApiKeyCredentials(
        accountId,
        validation.apiKey,
        validation.apiBaseUrl,
        providerPayload.apiProviderMode,
        providerPayload.apiProviderId,
        providerPayload.apiProviderName,
      );
      if (
        validation.apiBaseUrl &&
        providerPayload.apiProviderMode === "custom" &&
        providerPayload.apiProviderId !== COCKPIT_API_PROVIDER_ID
      ) {
        try {
          await upsertCodexModelProviderFromCredential({
            providerId: providerPayload.apiProviderId ?? null,
            providerName: providerPayload.apiProviderName ?? null,
            apiBaseUrl: validation.apiBaseUrl,
            apiKey: validation.apiKey,
          });
          await reloadManagedProviders();
        } catch (providerErr) {
          console.warn(
            "[CodexModelProviders] 更新凭据后写入供应商失败",
            providerErr,
          );
        }
      }
      setMessage({ text: t("instances.messages.updated", "实例已更新") });
      setEditingApiKeyCredentialsId(null);
      setEditingApiKeyCredentialsValue("");
      setEditingApiKeyCredentialsVisible(false);
      setEditingApiBaseUrlCredentialsValue(DEFAULT_CODEX_API_BASE_URL);
      setEditingApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
      setEditingManagedProviderId("");
      setEditingManagedProviderApiKeyId("");
      setEditingNewManagedProviderNameInput("");
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
    editingApiProviderPresetId,
    editingManagedProviderId,
    editingNewManagedProviderNameInput,
    reloadManagedProviders,
    setMessage,
    t,
    upsertCodexModelProviderFromCredential,
    updateApiKeyCredentials,
    validateApiKeyCredentialInputs,
  ]);

  // ─── Platform-specific: Presentation ─────────────────────────────────

  const resolveQuotaErrorMeta = useCallback(
    (quotaError?: CodexQuotaErrorInfo) => {
      if (!quotaError?.message) {
        return {
          statusCode: "",
          errorCode: "",
          displayText: "",
          rawMessage: "",
          isRefreshRequestFailure: false,
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
      const statusCode =
        rawMessage.match(/API 返回错误\s+(\d{3})/i)?.[1] ||
        rawMessage.match(/status[=: ]+(\d{3})/i)?.[1] ||
        "";
      const errorCode =
        quotaError.code ||
        rawMessage.match(/\[error_code:([^\]]+)\]/)?.[1] ||
        rawMessage.match(/error_code[=:]\s*([^,\]\s]+)/i)?.[1] ||
        "";
      const authFailureText =
        formatCodexAuthFailureMessage(normalizedRawMessage);
      const displayText =
        authFailureText !== normalizedRawMessage
          ? authFailureText
          : errorCode ||
            (isRefreshRequestFailure
              ? t("codex.quotaError.requestFailedManualRetry", {
                  error: requestErrorMessage,
                })
              : normalizedRawMessage);
      return {
        statusCode,
        errorCode,
        displayText,
        rawMessage,
        isRefreshRequestFailure,
      };
    },
    [formatCodexAuthFailureMessage, t],
  );

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
        errorCode === "refresh_token_reused" ||
        errorCode === "refresh_token_expired" ||
        errorCode === "refresh_token_invalidated" ||
        errorCode === "token_invalidated" ||
        errorCode === "invalid_grant" ||
        errorCode === "invalid_token" ||
        rawMessage.includes("refresh_token_reused") ||
        rawMessage.includes("refresh_token_expired") ||
        rawMessage.includes("refresh_token_invalidated") ||
        rawMessage.includes("token_invalidated") ||
        rawMessage.includes("refresh_token 已被其它客户端或实例使用过") ||
        rawMessage.includes("your authentication token has been invalidated") ||
        rawMessage.includes("401 unauthorized") ||
        rawMessage.includes("invalid_grant") ||
        rawMessage.includes("token 已过期且无 refresh_token") ||
        rawMessage.includes("缺少 refresh_token") ||
        rawMessage.includes("token 已过期且刷新失败") ||
        rawMessage.includes("刷新 token 失败")
      );
    },
    [],
  );

  const accountPresentations = useMemo(() => {
    const map = new Map<
      string,
      ReturnType<typeof buildCodexAccountPresentation>
    >();
    accounts.forEach((a) => map.set(a.id, buildCodexAccountPresentation(a, t)));
    return map;
  }, [accounts, t]);

  const resolvePresentation = useCallback(
    (account: CodexAccount) =>
      accountPresentations.get(account.id) ??
      buildCodexAccountPresentation(account, t),
    [accountPresentations, t],
  );

  const resolveSubscriptionPresentation = useCallback(
    (account: CodexAccount) =>
      getCodexSubscriptionPresentation(account.subscription_active_until, t),
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
    (account: CodexAccount) => Boolean(account.quota_error),
    [],
  );

  const localAccessCollection = localAccessState?.collection ?? null;
  const localAccessAccountIdSet = useMemo(
    () => new Set(localAccessCollection?.accountIds ?? []),
    [localAccessCollection?.accountIds],
  );
  const localAccessAccounts = useMemo(
    () =>
      (localAccessCollection?.accountIds ?? [])
        .map((accountId) =>
          accounts.find((account) => account.id === accountId),
        )
        .filter((account): account is CodexAccount => Boolean(account)),
    [accounts, localAccessCollection?.accountIds],
  );
  const localAccessQuotaPoolSummary = useMemo(
    () => summarizeCodexQuotaPool(localAccessAccounts),
    [localAccessAccounts],
  );
  const localAccessQuotaPoolLabels = useMemo(
    () => ({
      hourly: t("codex.localAccess.quotaPool.hourlyShort", "5h"),
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
  const localAccessBusy =
    localAccessSaving ||
    localAccessTesting ||
    localAccessStarting ||
    localAccessRefreshing ||
    localAccessPortKilling;

  const resolveLocalAccessBaseUrl = useCallback(() => {
    if (!localAccessCollection)
      return localAccessState?.baseUrl || CODEX_LOCAL_ACCESS_FALLBACK_BASE_URL;
    return (
      localAccessState?.baseUrl ||
      `http://127.0.0.1:${localAccessCollection.port}/v1`
    );
  }, [localAccessCollection, localAccessState?.baseUrl]);

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

  const openLocalAccessMemberPicker = useCallback(() => {
    setLocalAccessModalMode("members");
    setShowLocalAccessModal(true);
  }, []);

  const handleHideLocalAccessEntry = useCallback(() => {
    setShowLocalAccessHideConfirm(true);
  }, []);

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

  const handleSaveLocalAccessAccounts = useCallback(
    async (
      accountIds: string[],
      options?: { restrictFreeAccounts?: boolean },
    ) => {
      setLocalAccessSaving(true);
      try {
        const restrictFreeAccounts = options?.restrictFreeAccounts ?? true;
        const accountById = new Map(
          accounts.map((account) => [account.id, account]),
        );
        const filteredAccountIds = accountIds.filter((accountId) => {
          const account = accountById.get(accountId);
          if (!account) return false;
          if (isCodexApiKeyAccount(account)) return false;
          if (
            restrictFreeAccounts &&
            isCodexExplicitFreePlanType(account.plan_type)
          ) {
            return false;
          }
          return true;
        });
        const nextState =
          await codexLocalAccessService.saveCodexLocalAccessAccounts(
            filteredAccountIds,
            restrictFreeAccounts,
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

  const handleRemoveLocalAccessAccount = useCallback(
    async (accountId: string) => {
      if (!localAccessCollection) return;
      try {
        await handleSaveLocalAccessAccounts(
          localAccessCollection.accountIds.filter((id) => id !== accountId),
          {
            restrictFreeAccounts:
              localAccessCollection.restrictFreeAccounts ?? true,
          },
        );
      } catch (error) {
        setMessage({
          text: t("messages.actionFailed", {
            action: t("accounts.groups.removeFromGroup"),
            error: String(error).replace(/^Error:\s*/, ""),
          }),
          tone: "error",
        });
      }
    },
    [handleSaveLocalAccessAccounts, localAccessCollection, setMessage, t],
  );

  const tierCounts = useMemo(() => {
    const counts = {
      all: overviewAccounts.length,
      VALID: 0,
      FREE: 0,
      PLUS: 0,
      PRO: 0,
      TEAM: 0,
      ENTERPRISE: 0,
      ERROR: 0,
    };
    overviewAccounts.forEach((a) => {
      if (!isAbnormalAccount(a)) {
        counts.VALID += 1;
      }
      const tier = resolvePlanKey(a);
      if (tier in counts) counts[tier as keyof typeof counts] += 1;
      if (a.quota_error) counts.ERROR += 1;
    });
    return counts;
  }, [isAbnormalAccount, overviewAccounts, resolvePlanKey]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => [
      { value: "FREE", label: `FREE (${tierCounts.FREE})` },
      { value: "PLUS", label: `PLUS (${tierCounts.PLUS})` },
      { value: "PRO", label: `PRO (${tierCounts.PRO})` },
      { value: "TEAM", label: `TEAM (${tierCounts.TEAM})` },
      { value: "ENTERPRISE", label: `ENTERPRISE (${tierCounts.ENTERPRISE})` },
      { value: "ERROR", label: `ERROR (${tierCounts.ERROR})` },
      buildValidAccountsFilterOption(t, tierCounts.VALID),
    ],
    [t, tierCounts],
  );

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
    const existingAccountIds = new Set(accounts.map((account) => account.id));
    const hasStaleAccountIds = codexGroups.some((group) =>
      group.accountIds.some((accountId) => !existingAccountIds.has(accountId)),
    );
    if (!hasStaleAccountIds) {
      return;
    }

    void (async () => {
      await cleanupDeletedCodexAccounts(existingAccountIds);
      await reloadCodexGroups();
    })();
  }, [accounts, codexGroups, reloadCodexGroups]);

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

  const handleTestLocalAccess = useCallback(async () => {
    if (!localAccessCollection) {
      throw new Error(
        t("codex.localAccess.testUnavailable", "当前 API 服务地址不可用"),
      );
    }

    const baseUrl = resolveLocalAccessBaseUrl();
    if (!baseUrl) {
      throw new Error(
        t("codex.localAccess.testUnavailable", "当前 API 服务地址不可用"),
      );
    }

    setLocalAccessTesting(true);
    let timeoutId: number | null = null;

    try {
      const controller = new AbortController();
      timeoutId = window.setTimeout(() => controller.abort(), 10000);
      const response = await fetch(`${baseUrl}/models`, {
        method: "GET",
        headers: {
          Authorization: `Bearer ${localAccessCollection.apiKey}`,
        },
        signal: controller.signal,
      });
      const rawText = await response.text();
      let payload: Record<string, unknown> | null = null;
      if (rawText) {
        try {
          payload = JSON.parse(rawText) as Record<string, unknown>;
        } catch {
          payload = null;
        }
      }

      if (!response.ok) {
        const errorText =
          (typeof payload?.error === "string" && payload.error) ||
          rawText ||
          response.statusText ||
          `HTTP ${response.status}`;
        throw new Error(errorText);
      }

      const modelCount = Array.isArray(payload?.data) ? payload.data.length : 0;
      return modelCount;
    } catch (error) {
      const rawError = String(error).replace(/^Error:\s*/, "");
      const normalizedError =
        rawError === "AbortError"
          ? t("codex.localAccess.testTimeout", "测试超时，请确认本地服务已启动")
          : rawError;
      throw new Error(normalizedError);
    } finally {
      if (timeoutId != null) {
        window.clearTimeout(timeoutId);
      }
      setLocalAccessTesting(false);
    }
  }, [localAccessCollection, resolveLocalAccessBaseUrl, t]);

  const handleActivateLocalAccess = useCallback(
    async (options?: { showSuccessMessage?: boolean }) => {
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
      setLocalAccessStarting(true);
      try {
        const nextState =
          await codexLocalAccessService.activateCodexLocalAccess();
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
      setMessage({
        text: t("messages.actionFailed", {
          action: t("codex.localAccess.toggleService", "切换 API 服务"),
          error: String(error).replace(/^Error:\s*/, ""),
        }),
        tone: "error",
      });
    }
  }, [handleToggleLocalAccessEnabled, setMessage, t]);

  const handleQuickActivateLocalAccess = useCallback(async () => {
    try {
      const currentKind = await resolveCurrentCodexLaunchCredentialKind();
      const state = await handleActivateLocalAccess();
      if (!state) {
        return;
      }
      if (
        shouldShowApiSwitchVisibilityNotice(currentKind, "api-service") &&
        currentKind
      ) {
        openApiSwitchVisibilityNotice({
          from: currentKind,
          to: "api-service",
        });
      }
    } catch (error) {
      setMessage({
        text: t("messages.actionFailed", {
          action: t("codex.localAccess.activateAction", "启动 API 服务"),
          error: String(error).replace(/^Error:\s*/, ""),
        }),
        tone: "error",
      });
    }
  }, [
    handleActivateLocalAccess,
    openApiSwitchVisibilityNotice,
    resolveCurrentCodexLaunchCredentialKind,
    setMessage,
    shouldShowApiSwitchVisibilityNotice,
    t,
  ]);

  const handleQuickRefreshLocalAccessQuota = useCallback(async () => {
    if (!localAccessCollection) return;
    const targetIds = localAccessCollection.accountIds.filter((accountId) => {
      const account = accounts.find((item) => item.id === accountId);
      return Boolean(account && !isCodexApiKeyAccount(account));
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
      const results = await Promise.allSettled(
        targetIds.map((accountId) => refreshQuota(accountId)),
      );
      const successCount = results.filter(
        (result) => result.status === "fulfilled",
      ).length;

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

      const firstFailure = results.find(
        (result): result is PromiseRejectedResult =>
          result.status === "rejected",
      );
      setMessage({
        text: t("codex.refreshFailed", {
          error: String(firstFailure?.reason ?? "").replace(/^Error:\s*/, ""),
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
    refreshQuota,
    setMessage,
    t,
  ]);

  // ─── Filtering & Sorting ────────────────────────────────────────────
  const customSortOrderIndex = useMemo(() => {
    const map = new Map<string, number>();
    customSortOrder.forEach((accountId, index) => {
      map.set(accountId, index);
    });
    return map;
  }, [customSortOrder]);
  const overviewCurrentAccountId = localAccessLaunchCurrent
    ? null
    : (currentAccount?.id ?? null);

  const compareAccountsBySort = useCallback(
    (a: CodexAccount, b: CodexAccount) => {
      if (sortBy === "custom") {
        const aIndex =
          customSortOrderIndex.get(a.id) ?? Number.MAX_SAFE_INTEGER;
        const bIndex =
          customSortOrderIndex.get(b.id) ?? Number.MAX_SAFE_INTEGER;
        if (aIndex !== bIndex) {
          return aIndex - bIndex;
        }
        return b.created_at - a.created_at;
      }

      const currentFirstDiff = compareCurrentAccountFirst(
        a.id,
        b.id,
        overviewCurrentAccountId,
      );
      if (currentFirstDiff !== 0) {
        return currentFirstDiff;
      }

      if (sortBy === "created_at") {
        const diff = b.created_at - a.created_at;
        return sortDirection === "desc" ? diff : -diff;
      }
      if (sortBy === "weekly_reset" || sortBy === "hourly_reset") {
        const aR =
          sortBy === "weekly_reset"
            ? (a.quota?.weekly_reset_time ?? null)
            : (a.quota?.hourly_reset_time ?? null);
        const bR =
          sortBy === "weekly_reset"
            ? (b.quota?.weekly_reset_time ?? null)
            : (b.quota?.hourly_reset_time ?? null);
        if (aR == null && bR == null) return 0;
        if (aR == null) return 1;
        if (bR == null) return -1;
        return sortDirection === "desc" ? bR - aR : aR - bR;
      }
      if (sortBy === "subscription_expiry") {
        const aR = isCodexApiKeyAccount(a)
          ? null
          : resolveSubscriptionPresentation(a).timestampMs;
        const bR = isCodexApiKeyAccount(b)
          ? null
          : resolveSubscriptionPresentation(b).timestampMs;
        if (aR == null && bR == null) return 0;
        if (aR == null) return 1;
        if (bR == null) return -1;
        return sortDirection === "desc" ? bR - aR : aR - bR;
      }
      const aV =
        sortBy === "weekly"
          ? (a.quota?.weekly_percentage ?? -1)
          : (a.quota?.hourly_percentage ?? -1);
      const bV =
        sortBy === "weekly"
          ? (b.quota?.weekly_percentage ?? -1)
          : (b.quota?.hourly_percentage ?? -1);
      return sortDirection === "desc" ? bV - aV : aV - bV;
    },
    [
      customSortOrderIndex,
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

  const filteredAccounts = useMemo(() => {
    let result = [...overviewAccounts];
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((a) =>
        resolvePresentation(a).displayName.toLowerCase().includes(query),
      );
    }
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } =
        splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        result = result.filter((account) => !isAbnormalAccount(account));
      }
      if (selectedTypes.size > 0) {
        result = result.filter((a) => {
          if (selectedTypes.has("ERROR") && a.quota_error) {
            return true;
          }
          return selectedTypes.has(resolvePlanKey(a));
        });
      }
    }
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((a) =>
        (a.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag)),
      );
    }
    // 分组筛选 — 仅保留仍存在于 codexGroups 中的 ID，防止已删除分组导致空筛选
    if (groupFilter.length > 0) {
      const existingGroupIds = new Set(codexGroups.map((g) => g.id));
      const activeFilter = groupFilter.filter((id) => existingGroupIds.has(id));
      if (activeFilter.length > 0) {
        const groupAccountIds = new Set<string>();
        const selectedGroupIds = new Set(activeFilter);
        for (const group of codexGroups) {
          if (selectedGroupIds.has(group.id)) {
            for (const aid of group.accountIds) groupAccountIds.add(aid);
          }
        }
        result = result.filter((a) => groupAccountIds.has(a.id));
      }
    }
    if (activeGroupId) {
      const scopedGroup = codexGroups.find(
        (group) => group.id === activeGroupId,
      );
      if (!scopedGroup) {
        return [];
      }
      const scopedIds = new Set(scopedGroup.accountIds);
      result = result.filter((account) => scopedIds.has(account.id));
    }
    result.sort(compareAccountsBySort);
    return result;
  }, [
    activeGroupId,
    codexGroups,
    compareAccountsBySort,
    filterTypes,
    groupFilter,
    isAbnormalAccount,
    normalizeTag,
    overviewAccounts,
    resolvePlanKey,
    resolvePresentation,
    searchQuery,
    tagFilter,
  ]);

  const filteredIds = useMemo(
    () => filteredAccounts.map((account) => account.id),
    [filteredAccounts],
  );
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

  useEffect(() => {
    const teamAccountIds = filteredAccounts
      .filter(
        (account) =>
          !hasCodexAccountStructure(account) ||
          (isCodexTeamLikePlan(account.plan_type) &&
            !hasCodexAccountName(account)),
      )
      .map((account) => account.id);
    if (teamAccountIds.length === 0) return;
    void hydrateAccountProfilesIfNeeded(teamAccountIds);
  }, [filteredAccounts, hydrateAccountProfilesIfNeeded]);

  const resolveGroupLabel = (groupKey: string) =>
    groupKey === untaggedKey
      ? t("accounts.defaultGroup", "默认分组")
      : groupKey;

  const resolveCompactQuotaItems = useCallback(
    (presentation: ReturnType<typeof buildCodexAccountPresentation>) => {
      const standardQuotaItems = presentation.quotaItems.filter(
        (item) => item.key !== "code_review",
      );
      const first = standardQuotaItems[0];
      const primary =
        standardQuotaItems.find((item) => item.key === "primary") ?? first;
      const secondary =
        standardQuotaItems.find((item) => item.key === "secondary") ??
        standardQuotaItems.find((item) => item.key !== primary?.key);

      return [
        {
          key: "primary",
          valueText: primary?.valueText ?? "--",
          quotaClass: primary?.quotaClass ?? "unknown",
          titleText: primary?.hintText || primary?.label || "",
        },
        {
          key: "secondary",
          valueText: secondary?.valueText ?? "--",
          quotaClass: secondary?.quotaClass ?? "unknown",
          titleText: secondary?.hintText || secondary?.label || "",
        },
      ];
    },
    [],
  );

  // ─── Render helpers ──────────────────────────────────────────────────

  const renderCompactRows = (
    items: typeof filteredAccounts,
    groupKey?: string,
  ) =>
    items.map((account) => {
      const presentation = resolvePresentation(account);
      const isCurrent = overviewCurrentAccountId === account.id;
      const isSelected = selected.has(account.id);
      const isApiKeyAccount = isCodexApiKeyAccount(account);
      const compactQuotaItems = resolveCompactQuotaItems(presentation);
      const subscriptionInfo = resolveSubscriptionPresentation(account);
      const showCompactExpiry =
        !isApiKeyAccount && subscriptionInfo.bucket !== "active";
      return (
        <div
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`codex-compact-row ${isCurrent ? "current" : ""} ${isSelected ? "selected" : ""}`}
        >
          <div className="codex-compact-select">
            <input
              type="checkbox"
              checked={isSelected}
              onChange={() => toggleSelect(account.id)}
            />
          </div>
          <span
            className="codex-compact-email"
            title={maskAccountText(presentation.displayName)}
          >
            {maskAccountText(presentation.displayName)}
          </span>
          <div className="codex-compact-quotas">
            {compactQuotaItems.map((item) => (
              <span
                key={`${account.id}-${item.key}`}
                className={`codex-compact-quota codex-compact-quota-${item.key}`}
                title={item.titleText}
              >
                <span className="codex-compact-dot" />
                <span
                  className={`codex-compact-quota-value ${item.quotaClass}`}
                >
                  {item.valueText}
                </span>
              </span>
            ))}
            {showCompactExpiry && (
              <span
                className={`codex-compact-expiry ${subscriptionInfo.tone}`}
                title={subscriptionInfo.titleText}
              >
                {subscriptionInfo.valueText}
              </span>
            )}
          </div>
          <button
            className={`codex-compact-note-btn ${account.account_note?.trim() ? "has-note" : ""}`}
            onClick={() => openAccountNoteModal(account)}
            title={
              account.account_note?.trim() ||
              t("codex.accountNote.emptyTitle", "填写账号备注")
            }
            aria-label={t("codex.accountNote.title", "账号备注")}
          >
            <FileText size={13} />
          </button>
          <button
            className={`codex-compact-switch-btn ${!isCurrent ? "success" : ""}`}
            onClick={() => handleSwitch(account.id)}
            disabled={!!switching}
            title={t("codex.switch", "切换")}
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
      const isNewApiAccount = isCodexNewApiAccount(account);
      const isEditingApiKeyName =
        isApiKeyAccount && editingApiKeyNameId === account.id;
      const isSavingApiKeyName = savingApiKeyNameId === account.id;
      const planClass = presentation.planClass || "unknown";
      const isSelected = selected.has(account.id);
      const quotaItems =
        isApiKeyAccount && !isNewApiAccount
          ? []
          : showCodeReviewQuota
            ? presentation.quotaItems
            : presentation.quotaItems.filter(
                (item) => item.key !== "code_review",
              );
      const reauthErrorMeta = resolveQuotaErrorMeta(
        account.requires_reauth && account.reauth_reason
          ? {
              message: account.reauth_reason,
              timestamp: account.token_updated_at || account.last_used,
            }
          : undefined,
      );
      const quotaErrorMeta = resolveQuotaErrorMeta(account.quota_error);
      const accountIssueMeta = reauthErrorMeta.rawMessage
        ? reauthErrorMeta
        : quotaErrorMeta;
      const hasQuotaError = Boolean(accountIssueMeta.rawMessage);
      const isQuotaRefreshNotice =
        !reauthErrorMeta.rawMessage &&
        quotaErrorMeta.isRefreshRequestFailure &&
        !quotaErrorMeta.statusCode &&
        !quotaErrorMeta.errorCode;
      const accountIssueBadge = reauthErrorMeta.rawMessage
        ? t("codex.authError.badge", "授权异常")
        : isQuotaRefreshNotice
          ? t("codex.quotaError.refreshFailedBadge", "刷新失败")
          : accountIssueMeta.statusCode ||
            t("codex.quotaError.badge", "配额异常");
      const showReauthorizeAction =
        !isApiKeyAccount &&
        hasQuotaError &&
        shouldOfferReauthorizeAction(accountIssueMeta);
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
      const accountTags = (account.tags || [])
        .map((tag) => tag.trim())
        .filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isInLocalAccess = localAccessAccountIdSet.has(account.id);
      const subscriptionInfo = resolveSubscriptionPresentation(account);
      const isSubscriptionInfoMissing = subscriptionInfo.bucket === "missing";
      return (
        <div
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`codex-account-card ${isCurrent ? "current" : ""} ${isSelected ? "selected" : ""} ${isNewApiAccount ? "new-api-exclusive" : ""}`}
        >
          <div className="card-top">
            <div className="card-select">
              <input
                type="checkbox"
                checked={isSelected}
                onChange={() => toggleSelect(account.id)}
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
            {hasQuotaError && (
              <span
                className={`codex-status-pill ${isQuotaRefreshNotice ? "quota-refresh" : "quota-error"}`}
                title={accountIssueMeta.rawMessage}
              >
                {isQuotaRefreshNotice ? (
                  <Info size={12} />
                ) : (
                  <CircleAlert size={12} />
                )}
                {accountIssueBadge}
              </span>
            )}
            <span className={`tier-badge ${planClass}`}>
              {presentation.planLabel}
            </span>
          </div>
          {(meta.accountContextText ||
            isInLocalAccess ||
            account.account_note?.trim()) && (
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
                <span className="group-account-badge is-current">
                  {t("codex.localAccess.modal.selected", "已加入 API 服务")}
                </span>
              )}
              {renderAccountNoteButton(account)}
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
              <div className="account-sub-line">
                <span className="codex-login-subline" title={apiBaseUrlLine}>
                  {apiBaseUrlLine}
                </span>
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
          <div className="codex-quota-section">
            {isApiKeyAccount && !isNewApiAccount ? (
              <div className="quota-empty">
                <span>{t("common.shared.quota.noData", "暂无配额数据")}</span>
              </div>
            ) : (
              <>
                {hasQuotaError && (
                  <div
                    className={`quota-error-inline ${isQuotaRefreshNotice ? "quota-refresh-notice" : ""}`}
                    title={accountIssueMeta.rawMessage}
                  >
                    {isQuotaRefreshNotice ? (
                      <Info size={14} />
                    ) : (
                      <CircleAlert size={14} />
                    )}
                    <span>{accountIssueMeta.displayText}</span>
                    {showReauthorizeAction && (
                      <button
                        className="btn btn-sm btn-outline"
                        onClick={() => openAddModal("oauth")}
                        title={t("common.shared.addModal.oauth", "OAuth 授权")}
                      >
                        {t("common.shared.addModal.oauth", "OAuth 授权")}
                      </button>
                    )}
                  </div>
                )}
                {quotaItems.map((item) => {
                  const QuotaIcon =
                    item.key === "secondary"
                      ? Calendar
                      : item.key === "code_review"
                        ? BookOpen
                        : item.key === "new_api_quota"
                          ? Database
                          : Clock;
                  return (
                    <div
                      key={item.key}
                      className="quota-item"
                      title={item.hintText}
                    >
                      <div className="quota-header">
                        <QuotaIcon size={14} />
                        <span className="quota-label">{item.label}</span>
                        <span className={`quota-pct ${item.quotaClass}`}>
                          {item.valueText}
                        </span>
                      </div>
                      <div className="quota-bar-track">
                        <div
                          className={`quota-bar ${item.quotaClass}`}
                          style={{ width: `${item.percentage}%` }}
                        />
                      </div>
                      {item.resetText && (
                        <span className="quota-reset">{item.resetText}</span>
                      )}
                    </div>
                  );
                })}
                {quotaItems.length === 0 && (
                  <div className="quota-empty">
                    {t("common.shared.quota.noData", "暂无配额数据")}
                  </div>
                )}
              </>
            )}
          </div>
          {!isApiKeyAccount && (
            <div
              className={`codex-subscription-footer ${subscriptionInfo.tone}`}
              title={subscriptionInfo.titleText}
            >
              <div className="codex-subscription-footer-main">
                <Calendar size={14} />
                {isSubscriptionInfoMissing ? (
                  <strong>{subscriptionInfo.valueText}</strong>
                ) : (
                  <>
                    <span>{t("codex.subscription.label", "有效期")}</span>
                    <strong>{subscriptionInfo.valueText}</strong>
                  </>
                )}
              </div>
              {subscriptionInfo.timestampMs != null && (
                <span className="codex-subscription-footer-date">
                  {subscriptionInfo.detailText}
                </span>
              )}
            </div>
          )}
          <div className="codex-card-bottom">
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-footer">
              <div className="card-actions">
                <button
                  className="card-action-btn"
                  onClick={() => void handleLaunchCodexCli(account)}
                  disabled={cliLaunchingAccountId === account.id}
                  title={t("codex.cli.quickLaunch", "CLI 快速启动")}
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
                {!isNewApiAccount && (
                  <button
                    className="card-action-btn"
                    onClick={() => openTagModal(account.id)}
                    title={t("accounts.editTags", "编辑标签")}
                  >
                    <Tag size={14} />
                  </button>
                )}
                {!isNewApiAccount && (
                  <button
                    className={`card-action-btn ${account.account_note?.trim() ? "active" : ""}`}
                    onClick={() => openAccountNoteModal(account)}
                    title={
                      account.account_note?.trim() ||
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
                    onClick={() => openQuickSwitchProviderModal(account)}
                    title={t("codex.quickSwitch.action", "快速切换供应商")}
                  >
                    <Repeat size={14} />
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
                  disabled={!!switching}
                  title={t("codex.switch", "切换")}
                >
                  {switching === account.id ? (
                    <RefreshCw size={14} className="loading-spinner" />
                  ) : (
                    <Play size={14} />
                  )}
                </button>
                {(!isApiKeyAccount || isNewApiAccount) && (
                  <button
                    className="card-action-btn"
                    onClick={() => handleRefresh(account.id)}
                    disabled={refreshing === account.id}
                    title={t("common.shared.refreshQuota", "刷新配额")}
                  >
                    <RotateCw
                      size={14}
                      className={
                        refreshing === account.id ? "loading-spinner" : ""
                      }
                    />
                  </button>
                )}
                {!isNewApiAccount && (
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
                )}
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
    const previewAccounts = localAccessAccounts.slice(0, 3);
    const hiddenCount = Math.max(
      0,
      localAccessAccounts.length - previewAccounts.length,
    );
    const showLocalAccessEmptyState = previewAccounts.length === 0;
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
    const localAccessSummaryMeta = t("codex.localAccess.summaryMeta", {
      count: localAccessState?.memberCount ?? 0,
      defaultValue: "{{count}} 个账号 · 本机/局域网",
    });
    const localAccessEmptyMessage = t(
      "codex.localAccess.emptyMembers",
      "当前集合暂无账号",
    );

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
              <div className="folder-inline-icon codex-local-access-icon">
                <Server size={24} />
              </div>
              <div className="folder-inline-info">
                <div className="codex-local-access-title-row">
                  <span className="folder-inline-name">
                    {t("codex.localAccess.title", "API 服务")}
                  </span>
                </div>
                <span className="folder-inline-count">
                  {t("codex.localAccess.memberOnlyLocal", "本机/局域网")}
                </span>
              </div>
            </>
          ) : (
            <button
              type="button"
              className="codex-local-access-summary-trigger"
              onClick={() =>
                setLocalAccessDetailsExpanded((current) => !current)
              }
              title={
                showLocalAccessDetails
                  ? t("codex.localAccess.collapseDetails", "收起详情")
                  : t("codex.localAccess.expandDetails", "展开详情")
              }
            >
              <div className="folder-inline-icon codex-local-access-icon">
                <Server size={24} />
              </div>
              <div className="folder-inline-info">
                <div className="codex-local-access-title-row">
                  <span className="folder-inline-name">
                    {t("codex.localAccess.title", "API 服务")}
                  </span>
                  <span className="codex-local-access-summary-text">
                    {localAccessSummaryMeta}
                  </span>
                </div>
                <span className="folder-inline-count">
                  {t("codex.localAccess.memberOnlyLocal", "本机/局域网")}
                </span>
              </div>
            </button>
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
                <span className="codex-local-access-label">
                  {t("codex.localAccess.baseUrl", "地址")}
                </span>
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
            </div>

            <div className="folder-inline-preview codex-local-access-preview">
              {showLocalAccessEmptyState ? (
                <div className="codex-local-access-empty-state">
                  <span className="codex-local-access-empty-text">
                    {localAccessEmptyMessage}
                  </span>
                  <button
                    type="button"
                    className="codex-local-access-empty-action"
                    onClick={openLocalAccessMemberPicker}
                    title={t("common.shared.addAccount", "添加账号")}
                    disabled={localAccessBusy}
                  >
                    <FolderPlus size={14} />
                    <span>{t("common.shared.addAccount", "添加账号")}</span>
                  </button>
                </div>
              ) : (
                <>
                  {previewAccounts.map((account) => {
                    const presentation = resolvePresentation(account);
                    const hourlyQuota = presentation.quotaItems.find(
                      (item) => item.key === "primary",
                    );
                    const weeklyQuota = presentation.quotaItems.find(
                      (item) => item.key === "secondary",
                    );
                    return (
                      <div
                        key={`local-access-${account.id}`}
                        className="folder-preview-item codex-local-access-member"
                      >
                        <span
                          className="folder-preview-email codex-local-access-member-email"
                          title={maskAccountText(presentation.displayName)}
                        >
                          {maskAccountText(presentation.displayName)}
                        </span>
                        <span
                          className={`codex-local-access-member-text codex-local-access-member-quota ${hourlyQuota?.quotaClass || "unknown"}`}
                          title={hourlyQuota?.hintText || hourlyQuota?.label}
                        >
                          {hourlyQuota?.valueText || "-"}
                        </span>
                        <span
                          className={`codex-local-access-member-text codex-local-access-member-quota ${weeklyQuota?.quotaClass || "unknown"}`}
                          title={weeklyQuota?.label}
                        >
                          {weeklyQuota?.valueText || "-"}
                        </span>
                        <span
                          className={`codex-local-access-member-plan tier-badge ${presentation.planClass || "unknown"}`}
                        >
                          {presentation.planLabel}
                        </span>
                        <button
                          type="button"
                          className="folder-preview-remove-btn"
                          onClick={() =>
                            void handleRemoveLocalAccessAccount(account.id)
                          }
                          title={t("accounts.groups.removeFromGroup")}
                          aria-label={`${t("accounts.groups.removeFromGroup")}: ${maskAccountText(presentation.displayName)}`}
                          disabled={localAccessBusy}
                        >
                          <LogOut size={12} />
                        </button>
                      </div>
                    );
                  })}
                  {hiddenCount > 0 && (
                    <button
                      type="button"
                      className="folder-preview-item more"
                      onClick={openLocalAccessMemberPicker}
                      title={t(
                        "codex.localAccess.modal.manageMembers",
                        "管理成员",
                      )}
                      aria-label={t(
                        "codex.localAccess.modal.manageMembers",
                        "管理成员",
                      )}
                    >
                      +{hiddenCount}
                    </button>
                  )}
                </>
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
                    <span>
                      {localAccessQuotaPoolLabels.hourly}{" "}
                      {formatCodexQuotaPoolPercent(item.hourly)}
                    </span>
                    <span>
                      {localAccessQuotaPoolLabels.weekly}{" "}
                      {formatCodexQuotaPoolPercent(item.weekly)}
                    </span>
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

            {localAccessState?.lastError && (
              <div className="quota-error-inline">
                <CircleAlert size={14} />
                <span>{localAccessState.lastError}</span>
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
                {t("codex.localAccess.footerHint", "监听本机与局域网")}
              </span>
              <div className="card-footer codex-local-access-footer">
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
                    onClick={() => void handleQuickRefreshLocalAccessQuota()}
                    title={t("common.shared.refreshQuota", "刷新配额")}
                    disabled={localAccessBusy || !localAccessCollection}
                  >
                    <RotateCw
                      size={14}
                      className={localAccessRefreshing ? "loading-spinner" : ""}
                    />
                  </button>
                  <button
                    className="card-action-btn success"
                    onClick={() => void handleQuickActivateLocalAccess()}
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
                  </span>
                </div>
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
      const isNewApiAccount = isCodexNewApiAccount(account);
      const isEditingApiKeyName =
        isApiKeyAccount && editingApiKeyNameId === account.id;
      const isSavingApiKeyName = savingApiKeyNameId === account.id;
      const planClass = presentation.planClass || "unknown";
      const quotaItems =
        isApiKeyAccount && !isNewApiAccount
          ? []
          : showCodeReviewQuota
            ? presentation.quotaItems
            : presentation.quotaItems.filter(
                (item) => item.key !== "code_review",
              );
      const reauthErrorMeta = resolveQuotaErrorMeta(
        account.requires_reauth && account.reauth_reason
          ? {
              message: account.reauth_reason,
              timestamp: account.token_updated_at || account.last_used,
            }
          : undefined,
      );
      const quotaErrorMeta = resolveQuotaErrorMeta(account.quota_error);
      const accountIssueMeta = reauthErrorMeta.rawMessage
        ? reauthErrorMeta
        : quotaErrorMeta;
      const hasQuotaError = Boolean(accountIssueMeta.rawMessage);
      const isQuotaRefreshNotice =
        !reauthErrorMeta.rawMessage &&
        quotaErrorMeta.isRefreshRequestFailure &&
        !quotaErrorMeta.statusCode &&
        !quotaErrorMeta.errorCode;
      const accountIssueBadge = reauthErrorMeta.rawMessage
        ? t("codex.authError.badge", "授权异常")
        : isQuotaRefreshNotice
          ? t("codex.quotaError.refreshFailedBadge", "刷新失败")
          : accountIssueMeta.statusCode ||
            t("codex.quotaError.badge", "配额异常");
      const showReauthorizeAction =
        !isApiKeyAccount &&
        hasQuotaError &&
        shouldOfferReauthorizeAction(accountIssueMeta);
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
      const isInLocalAccess = localAccessAccountIdSet.has(account.id);
      const subscriptionInfo = resolveSubscriptionPresentation(account);
      return (
        <tr
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`${isCurrent ? "current" : ""} ${isNewApiAccount ? "new-api-exclusive" : ""}`}
        >
          <td>
            <input
              type="checkbox"
              checked={selected.has(account.id)}
              onChange={() => toggleSelect(account.id)}
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
              </div>
              {(meta.accountContextText ||
                isInLocalAccess ||
                account.account_note?.trim()) && (
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
                    <span className="group-account-badge is-current">
                      {t("codex.localAccess.modal.selected", "已加入 API 服务")}
                    </span>
                  )}
                  {renderAccountNoteButton(account)}
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
                  <div className="account-sub-line codex-account-meta-inline">
                    <span
                      className="codex-login-subline"
                      title={apiBaseUrlLine}
                    >
                      {apiBaseUrlLine}
                    </span>
                  </div>
                </>
              )}
              {hasQuotaError && (
                <div className="account-sub-line">
                  <span
                    className={`codex-status-pill ${isQuotaRefreshNotice ? "quota-refresh" : "quota-error"}`}
                    title={accountIssueMeta.rawMessage}
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
            <span className={`tier-badge ${planClass}`}>
              {presentation.planLabel}
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
                <span
                  className={`codex-subscription-badge ${subscriptionInfo.tone}`}
                >
                  {subscriptionInfo.valueText}
                </span>
                {subscriptionInfo.timestampMs != null && (
                  <span className="codex-subscription-date">
                    {subscriptionInfo.detailText}
                  </span>
                )}
              </div>
            )}
          </td>
          <td>
            {isApiKeyAccount && !isNewApiAccount ? (
              <span className="codex-subscription-table-empty">-</span>
            ) : (
              <>
                <div className="quota-grid">
                  {quotaItems.map((item) => (
                    <div
                      key={item.key}
                      className="quota-item"
                      title={item.hintText}
                    >
                      <div className="quota-header">
                        <span className="quota-name">{item.label}</span>
                        <span className={`quota-value ${item.quotaClass}`}>
                          {item.valueText}
                        </span>
                      </div>
                      <div className="quota-progress-track">
                        <div
                          className={`quota-progress-bar ${item.quotaClass}`}
                          style={{ width: `${item.percentage}%` }}
                        />
                      </div>
                      {item.resetText && (
                        <div className="quota-footer">
                          <span className="quota-reset">{item.resetText}</span>
                        </div>
                      )}
                    </div>
                  ))}
                  {quotaItems.length === 0 && (
                    <span style={{ color: "var(--text-muted)", fontSize: 13 }}>
                      {t("common.shared.quota.noData", "暂无配额数据")}
                    </span>
                  )}
                </div>
                {hasQuotaError && (
                  <div
                    className={`quota-error-inline table ${isQuotaRefreshNotice ? "quota-refresh-notice" : ""}`}
                    title={accountIssueMeta.rawMessage}
                  >
                    {isQuotaRefreshNotice ? (
                      <Info size={12} />
                    ) : (
                      <CircleAlert size={12} />
                    )}
                    <span>{accountIssueMeta.displayText}</span>
                    {showReauthorizeAction && (
                      <button
                        className="btn btn-sm btn-outline"
                        onClick={() => openAddModal("oauth")}
                        title={t("common.shared.addModal.oauth", "OAuth 授权")}
                      >
                        {t("common.shared.addModal.oauth", "OAuth 授权")}
                      </button>
                    )}
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
                disabled={cliLaunchingAccountId === account.id}
                title={t("codex.cli.quickLaunch", "CLI 快速启动")}
              >
                {cliLaunchingAccountId === account.id ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Terminal size={14} />
                )}
              </button>
              {isNewApiAccount && (
                <button
                  className="action-btn"
                  onClick={() => setCockpitApiPanelAccountId(account.id)}
                  title={t("codex.cockpitApi.servicePanel", "服务面板")}
                >
                  <Database size={14} />
                </button>
              )}
              {!isNewApiAccount && (
                <button
                  className="action-btn"
                  onClick={() => openTagModal(account.id)}
                  title={t("accounts.editTags", "编辑标签")}
                >
                  <Tag size={14} />
                </button>
              )}
              {!isNewApiAccount && (
                <button
                  className={`action-btn ${account.account_note?.trim() ? "active" : ""}`}
                  onClick={() => openAccountNoteModal(account)}
                  title={
                    account.account_note?.trim() ||
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
                  onClick={() => openQuickSwitchProviderModal(account)}
                  title={t("codex.quickSwitch.action", "快速切换供应商")}
                >
                  <Repeat size={14} />
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
                disabled={!!switching}
                title={t("codex.switch", "切换")}
              >
                {switching === account.id ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Play size={14} />
                )}
              </button>
              {(!isApiKeyAccount || isNewApiAccount) && (
                <button
                  className="action-btn"
                  onClick={() => handleRefresh(account.id)}
                  disabled={refreshing === account.id}
                  title={t("common.shared.refreshQuota", "刷新配额")}
                >
                  <RotateCw
                    size={14}
                    className={
                      refreshing === account.id ? "loading-spinner" : ""
                    }
                  />
                </button>
              )}
              {!isNewApiAccount && (
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
              )}
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
              </span>
            </div>
          </td>
          <td>
            <div className="folder-table-actions">
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
  const showOverviewSelectionBar =
    !groupByTag && !activeGroupId && paginatedAccounts.length > 0;
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
    const usedPercent = readCockpitApiNumber(usage, "used_percent");
    const requestCount = readCockpitApiNumber(requests, "total");
    const todayCount = readCockpitApiNumber(requests, "today");
    const last7Count = readCockpitApiNumber(requests, "last_7_days");
    const last30Count = readCockpitApiNumber(requests, "last_30_days");
    const totalTokens = readCockpitApiNumber(tokens, "total");
    const totalQuotaDisplay = readCockpitApiString(total, "quota_display");
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
        value: formatCockpitApiInteger(totalTokens),
        meta: `${t("codex.cockpitApi.quotaUsed", "消耗")} ${totalQuotaDisplay || "-"}`,
      },
    ];

    return (
      <div
        className="modal-overlay"
        onClick={() => setCockpitApiPanelAccountId(null)}
      >
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
                {maskAccountText(account.account_name || account.email)}
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
                          <div className="cockpit-api-usage-row" key={modelName}>
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
                                Tokens {formatCockpitApiInteger(modelTokens)}
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
                                Tokens {formatCockpitApiInteger(dayTokens)}
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

  return (
    <div
      className={`codex-accounts-page codex-accounts-page--${overviewLayoutMode}`}
    >
      <CodexOverviewTabsHeader
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={["overview", "providers", "wakeup", "instances", "sessions"]}
      />

      {externalImportProgress.visible && (
        <div
          className="modal-overlay codex-external-import-overlay"
          onClick={() => {
            if (!externalImportRunning) {
              closeExternalImportProgressModal();
            }
          }}
        >
          <div
            className="modal-content codex-external-import-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {t("common.shared.externalImport.titleCodex", "Codex 批量导入")}
              </h2>
              {!externalImportRunning && (
                <button
                  className="modal-close"
                  onClick={closeExternalImportProgressModal}
                  aria-label={t("common.close", "关闭")}
                >
                  <X />
                </button>
              )}
            </div>
            <div className="codex-external-import-body">
              <div className="codex-external-import-main">
                <div className="codex-external-import-primary">
                  <div
                    className={`codex-external-import-status is-${externalImportProgress.status}`}
                  >
                    {externalImportRunning ? (
                      <RefreshCw size={18} className="loading-spinner" />
                    ) : externalImportProgress.status === "success" ? (
                      <Check size={18} />
                    ) : (
                      <CircleAlert size={18} />
                    )}
                    <span>{externalImportProgress.message}</span>
                  </div>

                  <div className="codex-external-import-progress-card">
                    <div className="codex-external-import-progress-head">
                      <span>{externalImportPercent}%</span>
                      <strong>
                        {externalImportProgress.current > 0 &&
                        externalImportProgress.total > 0
                          ? `${externalImportProgress.current}/${externalImportProgress.total}`
                          : ""}
                      </strong>
                    </div>
                    <div className="codex-external-import-progress-track">
                      <div
                        className="codex-external-import-progress-fill"
                        style={{ width: `${externalImportPercent}%` }}
                      />
                    </div>
                  </div>
                </div>

                <div className="codex-external-import-side">
                  <div className="codex-external-import-stats">
                    <div>
                      <span>
                        {t("common.shared.externalImport.total", "总数")}
                      </span>
                      <strong>{externalImportProgress.total}</strong>
                    </div>
                    <div>
                      <span>
                        {t("common.shared.externalImport.success", "成功")}
                      </span>
                      <strong>{externalImportProgress.success}</strong>
                    </div>
                    <div>
                      <span>
                        {t("common.shared.externalImport.failed", "失败")}
                      </span>
                      <strong>{externalImportProgress.failed}</strong>
                    </div>
                  </div>
                </div>
              </div>

              <div className="codex-external-import-steps">
                {externalImportSteps.map((label, index) => {
                  const isDone = externalImportStepIndex > index;
                  const isActive = externalImportStepIndex === index;
                  return (
                    <div
                      key={label}
                      className={`codex-external-import-step ${isDone ? "is-done" : ""} ${isActive ? "is-active" : ""}`}
                    >
                      <span>{isDone ? <Check size={13} /> : index + 1}</span>
                      <strong>{label}</strong>
                    </div>
                  );
                })}
              </div>

              {externalImportProgress.failures.length > 0 && (
                <div className="codex-external-import-errors">
                  <div className="codex-external-import-errors-head">
                    <strong>
                      {t("common.shared.externalImport.errorsTitle", "失败项")}
                    </strong>
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={handleCopyExternalImportErrors}
                    >
                      <Copy size={13} />
                      {t("common.shared.externalImport.copyErrors", "复制错误")}
                    </button>
                  </div>
                  <div className="codex-external-import-error-list">
                    {externalImportProgress.failures.map((item) => (
                      <div
                        key={`${item.index}-${item.label}`}
                        className="codex-external-import-error"
                      >
                        <span>
                          {item.index}. {item.label}
                        </span>
                        <small>{item.error}</small>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
            {!externalImportRunning && (
              <div className="modal-footer codex-external-import-footer">
                <button
                  className="btn btn-secondary"
                  onClick={closeExternalImportProgressModal}
                >
                  {t("common.close", "关闭")}
                </button>
                <button
                  className="btn btn-primary"
                  onClick={handleViewExternalImportAccounts}
                >
                  {t(
                    "common.shared.externalImport.viewAccounts",
                    "查看 Codex 账号",
                  )}
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {renderCockpitApiServicePanel()}

      {activeTab === "overview" && (
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
                <>
                  <button
                    className="btn btn-secondary breadcrumb-remove-btn"
                    onClick={() => setShowAddToCodexGroupModal(true)}
                    title={t("accounts.groups.moveToGroup")}
                  >
                    <FolderPlus size={14} />
                    {t("accounts.groups.moveToGroup")} ({selected.size})
                  </button>
                  <button
                    className="btn btn-secondary breadcrumb-remove-btn"
                    onClick={() => void handleRemoveFromGroup()}
                    title={t("accounts.groups.removeFromGroup")}
                  >
                    <LogOut size={14} />
                    {t("accounts.groups.removeFromGroup")} ({selected.size})
                  </button>
                </>
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
                allLabel={t("common.shared.filter.all", {
                  count: tierCounts.all,
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
                options={[
                  {
                    value: "created_at",
                    label: t("common.shared.sort.createdAt", "按创建时间"),
                  },
                  {
                    value: "weekly",
                    label: t("codex.sort.weekly", "按周配额"),
                  },
                  {
                    value: "hourly",
                    label: t("codex.sort.hourly", "按5小时配额"),
                  },
                  {
                    value: "weekly_reset",
                    label: t("codex.sort.weeklyReset", "按周配额重置时间"),
                  },
                  {
                    value: "hourly_reset",
                    label: t("codex.sort.hourlyReset", "按5小时配额重置时间"),
                  },
                  {
                    value: "subscription_expiry",
                    label: t("codex.sort.subscriptionExpiry", "按订阅有效期"),
                  },
                  {
                    value: "custom",
                    label: t("codex.sort.custom", "自定义顺序"),
                  },
                ]}
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
                onClick={() => openAddModal("oauth")}
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
              {selected.size > 0 && (
                <>
                  <button
                    className="btn btn-secondary icon-only"
                    onClick={() => setShowAddToCodexGroupModal(true)}
                    title={
                      activeGroupId
                        ? t("accounts.groups.moveToGroup")
                        : t("codex.groups.addToGroup", "添加至分组")
                    }
                  >
                    <FolderPlus size={14} />
                  </button>
                  <button
                    className="btn btn-danger icon-only"
                    onClick={handleBatchDelete}
                    title={`${t("common.delete", "删除")} (${selected.size})`}
                  >
                    <Trash2 size={14} />
                  </button>
                </>
              )}
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
                  onClick={() => openAddModal("oauth")}
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
            </div>
          ) : (
            <>
              {showOverviewSelectionBar && (
                <div className="codex-overview-selection-bar">
                  <label className="codex-overview-select-all">
                    <input
                      type="checkbox"
                      checked={isAllPaginatedSelected}
                      onChange={() => toggleSelectAll(paginatedIds)}
                    />
                    <span>{t("common.selectAll", "全选")}</span>
                  </label>
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
                            onChange={() => toggleSelectAll(paginatedIds)}
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
                            <input
                              type="checkbox"
                              checked={isAllPaginatedSelected}
                              onChange={() => toggleSelectAll(paginatedIds)}
                            />
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
                                onChange={() => toggleSelectAll(paginatedIds)}
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

          {showAddModal && (
            <div className="modal-overlay" onClick={closeAddModal}>
              <div
                className="modal-content codex-add-modal"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t("codex.addModal.title", "添加 Codex 账号")}</h2>
                  <button
                    className="modal-close"
                    onClick={closeAddModal}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-tabs">
                  <button
                    className={`modal-tab ${addTab === "oauth" ? "active" : ""}`}
                    onClick={() => openAddModal("oauth")}
                  >
                    <Globe size={14} />
                    <span className="modal-tab-label">
                      {t("common.shared.addModal.oauth", "OAuth Authorization")}
                    </span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === "token" ? "active" : ""}`}
                    onClick={() => openAddModal("token")}
                  >
                    <FileText size={14} />
                    <span className="modal-tab-label">
                      {t("common.shared.addModal.token", "Token / JSON")}
                    </span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === "apikey" ? "active" : ""}`}
                    onClick={() => openAddModal("apikey")}
                  >
                    <KeyRound size={14} />
                    <span className="modal-tab-label">
                      {t("codex.addModal.token", "API Key")}
                    </span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === "import" ? "active" : ""}`}
                    onClick={() => openAddModal("import")}
                  >
                    <Database size={14} />
                    <span className="modal-tab-label">
                      {t("accounts.tabs.import", "本地导入")}
                    </span>
                  </button>
                </div>
                <div className="modal-body">
                  {addTab === "oauth" && (
                    <div className="add-section">
                      <p className="section-desc">
                        {t(
                          "codex.oauth.desc",
                          "通过 OpenAI 官方 OAuth 授权您的 Codex 账号。",
                        )}
                      </p>
                      {oauthPrepareError ? (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>{oauthPrepareError}</span>
                          {oauthPortInUse && (
                            <button
                              className="btn btn-sm btn-outline"
                              onClick={handleReleaseOauthPort}
                            >
                              {t(
                                "codex.oauth.portInUseAction",
                                "Close port and retry",
                              )}
                            </button>
                          )}
                          {!oauthPortInUse && oauthTimeoutInfo && (
                            <button
                              className="btn btn-sm btn-outline"
                              onClick={handleRetryOauthAfterTimeout}
                            >
                              {t("codex.oauth.timeoutRetry", "刷新授权链接")}
                            </button>
                          )}
                        </div>
                      ) : oauthUrl ? (
                        <div className="oauth-url-section">
                          <div className="oauth-link">
                            <label>
                              {t("accounts.oauth.linkLabel", "授权链接")}
                            </label>
                            <div className="oauth-url-box">
                              <input type="text" value={oauthUrl} readOnly />
                              <button onClick={handleCopyOauthUrl}>
                                {oauthUrlCopied ? (
                                  <Check size={16} />
                                ) : (
                                  <Copy size={16} />
                                )}
                              </button>
                            </div>
                          </div>
                          <button
                            className="btn btn-primary btn-full"
                            onClick={
                              isOauthTimeoutState
                                ? handleRetryOauthAfterTimeout
                                : handleOpenOauthUrl
                            }
                          >
                            {isOauthTimeoutState ? (
                              <RefreshCw size={16} />
                            ) : (
                              <Globe size={16} />
                            )}
                            {isOauthTimeoutState
                              ? t("codex.oauth.timeoutRetry", "刷新授权链接")
                              : t(
                                  "common.shared.oauth.openBrowser",
                                  "Open in Browser",
                                )}
                          </button>
                          <div className="oauth-link">
                            <label>
                              {t(
                                "common.shared.oauth.manualCallbackLabel",
                                "手动输入回调地址",
                              )}
                            </label>
                            <div className="oauth-url-box oauth-manual-input">
                              <input
                                type="text"
                                value={oauthCallbackInput}
                                onChange={(e) =>
                                  setOauthCallbackInput(e.target.value)
                                }
                                placeholder={t(
                                  "common.shared.oauth.manualCallbackPlaceholder",
                                  "粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...",
                                )}
                              />
                              <button
                                className="oauth-copy-button"
                                onClick={() =>
                                  void handleSubmitOauthCallbackUrl()
                                }
                                disabled={
                                  oauthCallbackSubmitting ||
                                  !oauthCallbackInput.trim()
                                }
                              >
                                {oauthCallbackSubmitting ? (
                                  <RefreshCw
                                    size={16}
                                    className="loading-spinner"
                                  />
                                ) : (
                                  <Check size={16} />
                                )}
                                <span className="oauth-copy-button-label">
                                  {t(
                                    "accounts.oauth.continue",
                                    "我已授权，继续",
                                  )}
                                </span>
                              </button>
                            </div>
                          </div>
                          {oauthCallbackError && (
                            <div className="add-status error">
                              <CircleAlert size={16} />
                              <span>{oauthCallbackError}</span>
                            </div>
                          )}
                          {isOauthTimeoutState && (
                            <div className="add-status error">
                              <CircleAlert size={16} />
                              <span>
                                {t(
                                  "codex.oauth.timeout",
                                  '授权超时，请点击"刷新授权链接"后重试。',
                                )}
                              </span>
                            </div>
                          )}
                          <p className="oauth-hint">
                            {t(
                              "common.shared.oauth.hint",
                              "Once authorized, this window will update automatically",
                            )}
                          </p>
                        </div>
                      ) : (
                        <div className="oauth-loading">
                          <RefreshCw size={24} className="loading-spinner" />
                          <span>
                            {t("codex.oauth.preparing", "正在准备授权链接...")}
                          </span>
                        </div>
                      )}
                    </div>
                  )}
                  {addTab === "apikey" && (
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
                                className={`api-provider-chip ${managedProviderId === provider.id ? "active" : ""}`}
                                onClick={() =>
                                  handleSelectManagedProvider(provider.id)
                                }
                                type="button"
                              >
                                <span>{provider.name}</span>
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                      {selectedManagedProvider &&
                        selectedManagedProvider.apiKeys.length > 0 && (
                          <div className="oauth-link">
                            <label>
                              {t(
                                "codex.modelProviders.selectSavedApiKey",
                                "已保存 API Key",
                              )}
                            </label>
                            <div className="api-provider-endpoint-list">
                              {selectedManagedProvider.apiKeys.map((item) => (
                                <button
                                  key={item.id}
                                  className={`api-provider-endpoint-chip ${managedProviderApiKeyId === item.id ? "active" : ""}`}
                                  onClick={() =>
                                    handleSelectManagedProviderApiKey(item.id)
                                  }
                                  type="button"
                                >
                                  {item.name ||
                                    t(
                                      "codex.modelProviders.unnamedKey",
                                      "未命名 Key",
                                    )}
                                </button>
                              ))}
                            </div>
                          </div>
                        )}
                      <div className="oauth-link">
                        <label>{t("codex.api.provider.label", "供应商")}</label>
                        <div className="api-provider-chip-list">
                          <button
                            className={`api-provider-chip ${apiProviderPresetId === CODEX_API_PROVIDER_CUSTOM_ID ? "active" : ""}`}
                            onClick={() =>
                              handleSelectApiProviderPreset(
                                CODEX_API_PROVIDER_CUSTOM_ID,
                              )
                            }
                            type="button"
                          >
                            <span>
                              {t("codex.api.provider.custom", "自定义")}
                            </span>
                          </button>
                          {CODEX_API_PROVIDER_PRESETS.map((preset) => (
                            <button
                              key={preset.id}
                              className={`api-provider-chip ${apiProviderPresetId === preset.id ? "active" : ""}`}
                              onClick={() =>
                                handleSelectApiProviderPreset(preset.id)
                              }
                              type="button"
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
                      {selectedApiProviderPreset &&
                        selectedApiProviderPreset.baseUrls.length > 1 && (
                          <div className="oauth-link">
                            <label>
                              {t("codex.api.provider.endpoint", "供应商端点")}
                            </label>
                            <div className="api-provider-endpoint-list">
                              {selectedApiProviderPreset.baseUrls.map(
                                (baseUrl) => (
                                  <button
                                    key={baseUrl}
                                    className={`api-provider-endpoint-chip ${apiBaseUrlInput === baseUrl ? "active" : ""}`}
                                    onClick={() => setApiBaseUrlInput(baseUrl)}
                                    type="button"
                                  >
                                    {baseUrl}
                                  </button>
                                ),
                              )}
                            </div>
                          </div>
                        )}
                      {selectedApiProviderPreset && (
                        <div className="api-provider-hint-block">
                          <p className="api-provider-hint">
                            {t(
                              "codex.api.provider.hint",
                              "已自动填写兼容 Base URL，可继续手动调整。",
                            )}
                          </p>
                          <div className="api-provider-links">
                            {selectedApiProviderPreset.website && (
                              <button
                                className="btn btn-secondary"
                                onClick={() =>
                                  void handleOpenProviderLink(
                                    selectedApiProviderPreset.website || "",
                                  )
                                }
                              >
                                <ExternalLink size={14} />
                                {t("codex.api.provider.website", "官网")}
                              </button>
                            )}
                            {selectedApiProviderPreset.apiKeyUrl && (
                              <button
                                className="btn btn-secondary"
                                onClick={() =>
                                  void handleOpenProviderLink(
                                    selectedApiProviderPreset.apiKeyUrl || "",
                                  )
                                }
                              >
                                <KeyRound size={14} />
                                {selectedApiProviderPreset.id ===
                                COCKPIT_API_PROVIDER_ID
                                  ? t(
                                      "codex.api.provider.getApiKey",
                                      "获取秘钥",
                                    )
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
                            type={apiKeyInputVisible ? "text" : "password"}
                            value={apiKeyInput}
                            onChange={(e) => setApiKeyInput(e.target.value)}
                            autoComplete="off"
                            spellCheck={false}
                          />
                          <button
                            type="button"
                            className="codex-secret-toggle-btn"
                            onClick={() =>
                              setApiKeyInputVisible((visible) => !visible)
                            }
                            title={
                              apiKeyInputVisible
                                ? t("codex.api.hideApiKey", "隐藏 API Key")
                                : t("codex.api.showApiKey", "显示 API Key")
                            }
                            aria-label={
                              apiKeyInputVisible
                                ? t("codex.api.hideApiKey", "隐藏 API Key")
                                : t("codex.api.showApiKey", "显示 API Key")
                            }
                          >
                            {apiKeyInputVisible ? (
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
                            value={apiBaseUrlInput}
                            onChange={(e) => setApiBaseUrlInput(e.target.value)}
                            placeholder={t(
                              "codex.api.baseUrlPlaceholder",
                              "不填写则是官方默认",
                            )}
                          />
                        </div>
                      </div>
                      {apiProviderPresetId !== COCKPIT_API_PROVIDER_ID && (
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
                              value={newManagedProviderNameInput}
                              onChange={(e) =>
                                setNewManagedProviderNameInput(e.target.value)
                              }
                              placeholder={t(
                                "codex.modelProviders.newProviderNamePlaceholder",
                                "不填则按域名自动生成",
                              )}
                            />
                          </div>
                        </div>
                      )}
                      <div className="api-key-add-actions">
                        <button
                          className="btn btn-primary"
                          onClick={() => void handleApiKeyLogin()}
                          disabled={
                            importing ||
                            addStatus === "loading" ||
                            !apiKeyInput.trim()
                          }
                        >
                          {addStatus === "loading" ? (
                            <RefreshCw size={16} className="loading-spinner" />
                          ) : (
                            <KeyRound size={16} />
                          )}
                          {t("common.shared.addAccount", "添加账号")}
                        </button>
                      </div>
                    </div>
                  )}
                  {addTab === "token" && (
                    <div className="add-section">
                      <p className="section-desc">
                        {t(
                          "codex.token.desc",
                          "粘贴 auth.json、账号 JSON 或 refresh_token。",
                        )}
                      </p>
                      <details className="token-format-collapse">
                        <summary className="token-format-collapse-summary">
                          {t(
                            "codex.token.formatSummary",
                            "必填字段与示例（点击展开）",
                          )}
                        </summary>
                        <div className="token-format">
                          <p className="token-format-required">
                            {t(
                              "codex.token.formatRequired",
                              "支持完整 tokens（id_token + access_token）或仅 refresh_token。仅 refresh_token 会先联网换取完整凭据。",
                            )}
                          </p>
                          <div className="token-format-group">
                            <div className="token-format-label">
                              {t(
                                "codex.token.formatSingleLabel",
                                "完整 tokens 示例",
                              )}
                            </div>
                            <pre className="token-format-code">
                              {CODEX_TOKEN_SINGLE_EXAMPLE}
                            </pre>
                          </div>
                          <div className="token-format-group">
                            <div className="token-format-label">
                              {t(
                                "codex.token.formatRefreshOnlyLabel",
                                "仅 refresh_token 示例",
                              )}
                            </div>
                            <pre className="token-format-code">
                              {CODEX_TOKEN_REFRESH_ONLY_EXAMPLE}
                            </pre>
                          </div>
                          <div className="token-format-group">
                            <div className="token-format-label">
                              {t("codex.token.formatBatchLabel", "批量示例")}
                            </div>
                            <pre className="token-format-code">
                              {CODEX_TOKEN_BATCH_EXAMPLE}
                            </pre>
                          </div>
                        </div>
                      </details>
                      <textarea
                        className="token-input"
                        value={tokenInput}
                        onChange={(e) => setTokenInput(e.target.value)}
                        placeholder={t(
                          "codex.token.placeholder",
                          '示例：每行一个 refresh_token，或 {"refresh_token":"rt_..."}',
                        )}
                      />
                      <button
                        className="btn btn-primary btn-full"
                        onClick={handleTokenImport}
                        disabled={importing || !tokenInput.trim()}
                      >
                        {importing ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Download size={16} />
                        )}
                        {t("common.shared.token.import", "Import")}
                      </button>
                    </div>
                  )}
                  {addTab === "import" && (
                    <div className="add-section">
                      <p className="section-desc">
                        {t(
                          "codex.import.localDesc",
                          "从本地已登录的会话中导入 Codex 账号。",
                        )}
                      </p>
                      <button
                        className="btn btn-primary btn-full"
                        onClick={handleImportFromLocal}
                        disabled={importing}
                      >
                        {importing ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Database size={16} />
                        )}
                        {t("codex.local.import", "Get Local Account")}
                      </button>
                      <div style={{ height: 12 }} />
                      <p className="section-desc">
                        {t("modals.import.fromFilesDesc")}
                      </p>
                      <button
                        className="btn btn-secondary btn-full"
                        onClick={handleImportFromFiles}
                        disabled={importing}
                      >
                        {importing ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <FileUp size={16} />
                        )}
                        {t("modals.import.fromFiles")}
                      </button>
                    </div>
                  )}
                  {addStatus !== "idle" && (
                    <div className={`add-status ${addStatus}`}>
                      {addStatus === "success" ? (
                        <Check size={16} />
                      ) : addStatus === "loading" ? (
                        <RefreshCw size={16} className="loading-spinner" />
                      ) : (
                        <CircleAlert size={16} />
                      )}
                      <span>{addMessage}</span>
                      {addTab === "oauth" &&
                        addStatus === "error" &&
                        isOauthTokenExchangeErrorState &&
                        oauthLoginIdRef.current && (
                          <button
                            className="btn btn-sm btn-outline"
                            onClick={() => void handleRetryOauthTokenExchange()}
                            disabled={oauthCallbackSubmitting}
                          >
                            {oauthCallbackSubmitting ? (
                              <RefreshCw
                                size={14}
                                className="loading-spinner"
                              />
                            ) : (
                              <RotateCw size={14} />
                            )}
                            {t("accounts.oauth.continue")}
                          </button>
                        )}
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}

          {quickSwitchAccountId && (
            <div className="modal-overlay" onClick={closeQuickSwitchModal}>
              <div
                className="modal-content codex-add-modal"
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
                          <div className="api-provider-endpoint-list">
                            {selectedQuickSwitchProvider.apiKeys.map((item) => (
                              <button
                                key={item.id}
                                className={`api-provider-endpoint-chip ${quickSwitchApiKeyId === item.id ? "active" : ""}`}
                                onClick={() =>
                                  handleSelectQuickSwitchApiKey(item.id)
                                }
                                type="button"
                                disabled={quickSwitchSubmitting}
                              >
                                {item.name ||
                                  t(
                                    "codex.modelProviders.unnamedKey",
                                    "未命名 Key",
                                  )}
                              </button>
                            ))}
                          </div>
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

          {editingApiKeyCredentialsId && (
            <div
              className="modal-overlay"
              onClick={closeApiKeyCredentialsModal}
            >
              <div
                className="modal-content codex-add-modal"
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
                          <div className="api-provider-endpoint-list">
                            {selectedEditingManagedProvider.apiKeys.map(
                              (item) => (
                                <button
                                  key={item.id}
                                  className={`api-provider-endpoint-chip ${editingManagedProviderApiKeyId === item.id ? "active" : ""}`}
                                  onClick={() =>
                                    handleSelectEditingManagedProviderApiKey(
                                      item.id,
                                    )
                                  }
                                  type="button"
                                  disabled={savingApiKeyCredentials}
                                >
                                  {item.name ||
                                    t(
                                      "codex.modelProviders.unnamedKey",
                                      "未命名 Key",
                                    )}
                                </button>
                              ),
                            )}
                          </div>
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
                                    setEditingApiBaseUrlCredentialsValue(
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
                            setEditingApiKeyCredentialsValue(e.target.value)
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
                            setEditingApiBaseUrlCredentialsValue(e.target.value)
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
            <div
              className="modal-overlay"
              onClick={() => setShowCustomSortModal(false)}
            >
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
                      const quotaItems =
                        isCodexApiKeyAccount(account) &&
                        !isCodexNewApiAccount(account)
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
                                ) : (
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
                    onChange={(value) =>
                      setExportFormat(value as CodexExportFormat)
                    }
                  />
                </div>
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
            <div
              className="modal-overlay codex-local-access-stats-overlay"
              onClick={() => setShowLocalAccessQuotaStatsModal(false)}
            >
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
                            <span>
                              <b>{localAccessQuotaPoolLabels.hourly}</b>
                              <strong>
                                {formatCodexQuotaPoolPercent(item.hourly)}
                              </strong>
                            </span>
                            <span>
                              <b>{localAccessQuotaPoolLabels.weekly}</b>
                              <strong>
                                {formatCodexQuotaPoolPercent(item.weekly)}
                              </strong>
                            </span>
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

          {apiSwitchNoticeContext && (
            <div
              className="modal-overlay codex-local-access-hide-confirm-overlay"
              onClick={closeApiSwitchVisibilityNotice}
            >
              <div
                className="modal codex-local-access-hide-confirm-modal codex-api-switch-notice-modal"
                onClick={(event) => event.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>
                    {t("codex.apiSwitchNotice.title", "Codex 会话不可见")}
                  </h2>
                  <button
                    className="modal-close"
                    onClick={closeApiSwitchVisibilityNotice}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage
                    message={apiSwitchNoticeError}
                    scrollKey={apiSwitchNoticeErrorScrollKey}
                  />
                  <p className="codex-local-access-hide-confirm-desc">
                    {t(
                      "codex.apiSwitchNotice.message",
                      "检测到 Codex 已从 {{from}} 切换到 {{to}}。由于官方机制，API 与账号直接切换后，原有会话可能不会自动显示。正在自动修复会话可见性，后续也可以通过「会话管理」里的「修复可见性」功能修复。",
                      {
                        from: formatCodexLaunchCredentialKindLabel(
                          apiSwitchNoticeContext.from,
                        ),
                        to: formatCodexLaunchCredentialKindLabel(
                          apiSwitchNoticeContext.to,
                        ),
                      },
                    )}
                  </p>
                  {apiSwitchNoticeRepairing && (
                    <div className="codex-api-switch-notice-repair-status is-loading">
                      <RefreshCw size={14} className="loading-spinner" />
                      <span>
                        {t(
                          "codex.apiSwitchNotice.repairing",
                          "正在修复 Codex 会话可见性...",
                        )}
                      </span>
                    </div>
                  )}
                  {apiSwitchNoticeRepairResult && (
                    <div className="codex-api-switch-notice-repair-status is-success">
                      <Check size={14} />
                      <span>{apiSwitchNoticeRepairResult}</span>
                    </div>
                  )}
                </div>
                <div className="modal-footer codex-api-switch-notice-footer">
                  <button
                    className="btn btn-primary"
                    onClick={closeApiSwitchVisibilityNotice}
                  >
                    {t("common.close", "关闭")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {deleteConfirm && (
            <div
              className="modal-overlay"
              onClick={() => !deleting && setDeleteConfirm(null)}
            >
              <div className="modal" onClick={(e) => e.stopPropagation()}>
                <div className="modal-header">
                  <h2>{t("common.confirm")}</h2>
                  <button
                    className="modal-close"
                    onClick={() => !deleting && setDeleteConfirm(null)}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-body">
                  <ModalErrorMessage
                    message={deleteConfirmError}
                    scrollKey={deleteConfirmErrorScrollKey}
                  />
                  <p>{deleteConfirm.message}</p>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={() => setDeleteConfirm(null)}
                    disabled={deleting}
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    className="btn btn-danger"
                    onClick={confirmDelete}
                    disabled={deleting}
                  >
                    {t("common.confirm")}
                  </button>
                </div>
              </div>
            </div>
          )}

          {tagDeleteConfirm && (
            <div
              className="modal-overlay"
              onClick={() => !deletingTag && setTagDeleteConfirm(null)}
            >
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
            <div
              className="modal-overlay"
              onClick={() => {
                if (deletingGroup) return;
                setGroupDeleteConfirm(null);
                setGroupDeleteError(null);
              }}
            >
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
            initialTags={
              accounts.find((a) => a.id === showTagModal)?.tags || []
            }
            availableTags={availableTags}
            onClose={() => setShowTagModal(null)}
            onSave={handleSaveTags}
          />

          {editingAccountNoteAccount && (
            <div className="modal-overlay" onClick={closeAccountNoteModal}>
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
                    disabled={savingAccountNote}
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
                      account: maskAccountText(
                        resolvePresentation(editingAccountNoteAccount)
                          .displayName,
                      ),
                      defaultValue: "给 {{account}} 填写单独展示的账号备注。",
                    })}
                  </p>
                  <label className="codex-account-note-field">
                    <span>{t("codex.accountNote.label", "账号备注")}</span>
                    <textarea
                      className="codex-account-note-textarea"
                      value={editingAccountNoteValue}
                      onChange={(event) => {
                        setEditingAccountNoteValue(event.target.value);
                        setAccountNoteError(null);
                      }}
                      placeholder={t(
                        "codex.accountNote.placeholder",
                        "例如邮箱、密码、辅助邮箱或其他交付备注",
                      )}
                      disabled={savingAccountNote}
                      rows={5}
                      autoFocus
                    />
                  </label>
                </div>
                <div className="modal-footer">
                  <button
                    className="btn btn-secondary"
                    onClick={closeAccountNoteModal}
                    disabled={savingAccountNote}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => void handleSubmitAccountNote()}
                    disabled={savingAccountNote}
                  >
                    {savingAccountNote
                      ? t("common.saving", "保存中...")
                      : t("common.save", "保存")}
                  </button>
                </div>
              </div>
            </div>
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

          <CodexLocalAccessModal
            isOpen={showLocalAccessModal}
            mode={localAccessModalMode}
            state={localAccessState}
            accounts={accounts}
            accountGroups={codexGroups}
            initialSelectedIds={localAccessModalSelectedIds}
            maskAccountText={maskAccountText}
            onClose={() => setShowLocalAccessModal(false)}
            onSaveAccounts={({ accountIds, restrictFreeAccounts }) =>
              handleSaveLocalAccessAccounts(accountIds, {
                restrictFreeAccounts,
              })
            }
            onClearStats={handleClearLocalAccessStats}
            onRefreshStats={reloadLocalAccessState}
            onUpdatePort={handleUpdateLocalAccessPort}
            onUpdateRoutingStrategy={handleUpdateLocalAccessRoutingStrategy}
            onRotateApiKey={handleRotateLocalAccessApiKey}
            onKillPort={handleKillLocalAccessPort}
            onToggleEnabled={handleToggleLocalAccessEnabled}
            onTest={handleTestLocalAccess}
            saving={localAccessSaving}
            testing={localAccessTesting}
            starting={localAccessStarting}
            portCleanupBusy={localAccessPortKilling}
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
      )}

      {activeTab === "instances" && (
        <CodexInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}

      {activeTab === "sessions" && <CodexSessionManager />}

      {activeTab === "providers" && (
        <CodexModelProviderManager
          accounts={accounts}
          onProvidersChanged={setManagedProviders}
          onManageModelPresets={() => {
            setActiveTab("wakeup");
            setWakeupPresetManagerSignal((value) => value + 1);
          }}
        />
      )}

      {activeTab === "wakeup" && (
        <CodexWakeupContent
          accounts={accounts}
          openPresetManagerSignal={wakeupPresetManagerSignal}
          onRefreshAccounts={async () => {
            await fetchAccounts();
            await fetchCurrentAccount();
          }}
        />
      )}
    </div>
  );
}
