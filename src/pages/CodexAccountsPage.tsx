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
import { createPortal } from "react-dom";
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
  Pause,
  RotateCw,
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
  Wrench,
  Terminal,
  Link2,
  ChevronDown,
  ShieldCheck,
  Minimize2,
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
import { MfaQuickCodeSelect } from "../components/MfaQuickCodeSelect";
import { PaginationControls } from "../components/PaginationControls";
import {
  CodexAccountGroupModal,
  CodexAddToGroupModal,
} from "../components/CodexAccountGroupModal";
import { CodexGroupAccountPickerModal } from "../components/CodexGroupAccountPickerModal";
import { CodexLocalAccessModal } from "../components/CodexLocalAccessModal";
import { CodexAccountPoolHealthModal } from "../components/CodexAccountPoolHealthModal";
import {
  type CodexAccountGroup,
  assignAccountsToCodexGroup,
  cleanupDeletedCodexAccounts,
  deleteCodexGroup,
  getCodexAccountGroups,
  isCodexGroupQuotaRefreshInherit,
  resolveCodexGroupQuotaAutoRefreshMinutes,
  removeAccountsFromCodexGroup,
} from "../services/codexAccountGroupService";
import {
  hasCodexAccountStructure,
  formatCodexLoginProvider,
  getCodexAuthMetadata,
  getCodexPlanFilterKey,
  getCodexSubscriptionPresentationForAccount,
  hasCodexAccountName,
  formatCodexResetTime,
  formatCodexResetTimeAbsolute,
  isCodexApiKeyAccount,
  isCodexAgentIdentityAccount,
  isCodexWebSessionAccount,
  isCodexChatCompletionsApiKeyAccount,
  isCodexNewApiAccount,
  isCodexOpaqueAccessTokenOnlyAccount,
  isCodexPendingOAuthAccount,
  isCodexTeamLikePlan,
  type CodexApiProviderMode,
  type CodexBatchDeleteJobStatus,
  type CodexQuotaErrorInfo,
  type CodexResetCredit,
  type CodexResetCreditsSnapshot,
} from "../types/codex";
import {
  canAddCodexAccountToLocalAccess,
  filterCodexLocalAccessAccountIds,
  isCodexOAuthBindingEligibleAccount,
  resolveImportedCodexAccountIdsForLocalAccess,
} from "../utils/codexLocalAccessAccounts";
import {
  extractCodexQuotaErrorCode,
  extractCodexQuotaErrorStatusCode,
  isBlockingCodexQuotaError,
  isVerboseCodexQuotaErrorMessage,
  summarizeCodexQuotaErrorMessage,
} from "../utils/codexQuotaError";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import {
  readCodexImportSyncApiService,
  writeCodexImportSyncApiService,
} from "../utils/codexImportPreferences";
import { recoverCodexBatchImportStartFromPreview } from "../utils/codexBatchImportQueue";
import {
  CODEX_OPEN_ADD_ACCOUNT_EVENT,
  type CodexOpenAddAccountDetail,
} from "../utils/codexAddAccountRequest";
import {
  CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT,
  getCodexPlanBadgeStyle,
  type CodexPlanBadgeStyle,
} from "../utils/codexPreferences";

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
import { CodexCliLaunchDialog } from "../components/codex/CodexCliLaunchDialog";
import {
  CodexWakeupContent,
  type CodexWakeupTestOpenRequest,
} from "../components/codex/CodexWakeupContent";
import { CodexModelProviderManager } from "../components/codex/CodexModelProviderManager";
import { CodexSpeedSelect } from "../components/codex/CodexSpeedSelect";
import { QuickSettingsPopover } from "../components/QuickSettingsPopover";
import { useProviderAccountsPage } from "../hooks/useProviderAccountsPage";
import { usePlatformRuntimeSupport } from "../hooks/usePlatformRuntimeSupport";
import { useEscClose } from "../hooks/useEscClose";
import { useEnterConfirm } from "../hooks/useEnterConfirm";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from "../components/MultiSelectFilterDropdown";
import { AccountTagFilterDropdown } from "../components/AccountTagFilterDropdown";
import {
  SingleSelectFilterDropdown,
  type SingleSelectFilterOption,
} from "../components/SingleSelectFilterDropdown";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import type {
  CodexAccount,
  CodexAppSpeed,
} from "../types/codex";
import type {
  CodexLocalAccessAddressKind,
  CodexLocalAccessAccountHealth,
  CodexLocalAccessCustomRoutingRule,
  CodexLocalAccessGatewayMode,
  CodexLocalAccessOAuthQuotaReserve,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessScope,
  CodexLocalAccessState,
} from "../types/codexLocalAccess";
import {
  CODEX_API_SERVICE_BIND_ID,
  type InstanceDefaults,
  type InstanceProfile,
} from "../types/instance";
import {
  CODEX_ADDITIONAL_QUOTA_VISIBILITY_CHANGED_EVENT,
  CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
  isCodexAdditionalQuotaVisibleByDefault,
  isCodexCodeReviewQuotaVisibleByDefault,
} from "../utils/codexPreferences";
import {
  findCodexWebSessionImports,
  splitCodexImportPayloads,
} from "../utils/codexJsonImportProgress";
import { emitAccountsChanged } from "../utils/accountSyncEvents";
import {
  CODEX_OVERVIEW_FILTER_FIELDS,
  CODEX_OVERVIEW_FILTER_SCOPE,
  buildCodexOverviewGroupFilterOptions,
  buildCodexOverviewSortOptions,
  buildCodexPlanFilterOptions,
  createCodexOverviewAccountComparator,
  createCodexPlanFilterCounts,
  filterAndSortCodexOverviewAccounts,
  incrementCodexPlanFilterCount,
  isCodexOverviewAccountAbnormal,
  isCodexOverviewAccountSubscriptionExpired,
  isCodexOverviewAccountZeroQuota,
  readCodexCustomSortActive,
  readCodexCustomSortOrder,
  writeCodexCustomSortActive,
  writeCodexCustomSortOrder,
} from "../utils/codexAccountOverview";
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  CODEX_API_PROVIDER_PRESETS,
  COCKPIT_API_BASE_URL,
  COCKPIT_API_PROVIDER_ID,
  COCKPIT_API_PROVIDER_NAME,
  findCodexApiProviderPresetById,
  isCockpitApiProviderBaseUrl,
  resolveCodexApiProviderPresetId,
} from "../utils/codexProviderPresets";
import {
  APIKEY_FUN_PROVIDER_BASE_URL,
  isApiKeyFunProviderBaseUrl,
  normalizeApiKeyFunOfficialUrl,
  resolveApiKeyFunWireApi,
} from "../utils/apikeyFunLinks";
import {
  APIKEY_FUN_PREFILL_EVENT,
  consumeApiKeyFunPrefill,
  type ApiKeyFunPrefillPayload,
} from "../utils/apiKeyFunPrefill";
import { resolveCodexProviderCapabilityProfile } from "../utils/codexProviderGateway";
import {
  formatCodexQuotaPoolPercent,
  formatCodexQuotaPoolWindowLabel,
  summarizeCodexQuotaPool,
} from "../utils/codexQuotaPool";
import {
  findCodexModelProviderById,
  findCodexModelProviderByBaseUrl,
  listCodexModelProviders,
  queryCodexModelProviderUsage,
  saveCodexModelProviderDetectedIntegrationType,
  type CodexModelProvider,
  type CodexModelProviderUsageSummary,
  upsertCodexModelProviderFromCredential,
} from "../services/codexModelProviderService";
import {
  CODEX_API_KEY_USAGE_REFRESHED_EVENT,
  readCodexApiKeyUsageCache,
  writeCodexApiKeyUsageCache,
  type CodexApiKeyUsageState,
} from "../services/codexApiKeyUsageRefreshService";
import {
  isModelProviderUsageUnavailableError,
  listModelProviderModels,
  resolveNewApiQuotaSnapshot,
} from "../services/modelProviderUsageService";
import { useSponsorStore } from "../stores/useSponsorStore";
import type { Sponsor } from "../types/sponsor";
import { buildValidAccountsFilterOption } from "../utils/accountValidityFilter";
import {
  buildPaginatedGroups,
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from "../hooks/usePagination";
import {
  buildCodexExportContent,
  buildCodexExportFileNameBase,
  hasCodexExportAgentIdentity,
  hasCodexExportSensitiveNotes,
  type CodexExportFormat,
} from "../utils/codexExportFormats";
import {
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
import {
  getMfaOtpToken,
  getMfaTimeRemaining,
  loadSavedMfaRecords,
  parseMfaCredentialInput,
  upsertSavedMfaRecord,
  type MfaRecord,
} from "../utils/mfaVault";
import {
  findFirstMailVerificationCode,
  type MailVerificationCodePreview,
} from "../utils/mailVerificationCode";
import md5 from "blueimp-md5";

const CODEX_TOKEN_SINGLE_EXAMPLE = `{
  "tokens": {
    "id_token": "eyJ...",
    "access_token": "eyJ...",
    "refresh_token": "rt_..."
  }
}`;
const CODEX_TOKEN_SESSION_EXAMPLE = `{
  "user": {
    "email": "user@example.com"
  },
  "account": {
    "id": "account-id"
  },
  "accessToken": "eyJ...",
  "authProvider": "openai"
}

{
  "refresh_token": "rt_..."
}

at-your-personal-access-token

{
  "personal_access_token": "at-...",
  "account_id": "workspace-uuid"
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
const OPENAI_OFFICIAL_BASE_URL = "https://api.openai.com/v1";
function parseOAuthQuotaReservePercent(value: string): number | null {
  const normalized = value.trim();
  if (!/^(?:[1-9]\d?|100)$/.test(normalized)) {
    return null;
  }
  return Number(normalized);
}

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
const CODEX_HIDE_RELAY_QUOTA_LEGACY_KEY =
  "agtools.codex.accounts.hide_relay_quota.v1";
const CODEX_LOCAL_ACCESS_EXPANDED_KEY =
  "agtools.codex.local_access_entry_expanded.v1";
const CODEX_LOCAL_ACCESS_ADDRESS_KIND_KEY =
  "agtools.codex.local_access_address_kind.v1";
const CODEX_LOCAL_ACCESS_GATEWAY_GUIDE_DISMISSED_KEY =
  "agtools.codex.api_service.gateway_guide.dismissed.v1";
const DEFAULT_CODEX_API_PROVIDER_ID = OPENAI_OFFICIAL_PRESET_ID;
const DEFAULT_CODEX_API_BASE_URL = OPENAI_OFFICIAL_BASE_URL;
const CODEX_LOCAL_ACCESS_FALLBACK_PORT = 54140;
const CODEX_LOCAL_ACCESS_FALLBACK_BASE_URL = `http://127.0.0.1:${CODEX_LOCAL_ACCESS_FALLBACK_PORT}/v1`;
const CODEX_LOCAL_ACCESS_FALLBACK_API_KEY_MASK = "agt_codex_••••••••••••";
const CODEX_FILTER_PERSISTENCE_SCOPE = CODEX_OVERVIEW_FILTER_SCOPE;
const SEARCH_QUERY_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.searchQuery;
const FILTER_TYPES_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.filterTypes;
const EXPIRY_FILTER_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.expiryFilter;
const GROUP_FILTER_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.groupFilter;
const ACTIVE_GROUP_ID_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.activeGroupId;
const OAUTH_BINDING_PAGE_SIZE_OPTIONS = [10, 20, 50] as const;

type CodexOverviewLayoutMode = "compact" | "list" | "grid";
type OAuthBindingTargetKind = "api_key_account" | "local_access";
type OAuthBindingQuotaReserveFieldErrors = {
  hourlyPercent?: string;
  weeklyPercent?: string;
};
type CodexAccountNoteFormState = {
  note: string;
  twoFactorSecret: string;
  accountPassword: string;
  phoneNumber: string;
  mailUrl: string;
  chatgptAccountId: string;
};

type CodexAccountNoteMailPreviewState = MailVerificationCodePreview & {
  fetchedAt: number;
  truncated: boolean;
  status: "initial" | "changed" | "unchanged";
};

type CodexAccountNoteMailPreviewSnapshot = {
  mailUrl: string;
  code: string;
};

type CodexAccountNoteFieldErrors = {
  twoFactorSecret?: string;
};

const EMPTY_CODEX_ACCOUNT_NOTE_FORM: CodexAccountNoteFormState = {
  note: "",
  twoFactorSecret: "",
  accountPassword: "",
  phoneNumber: "",
  mailUrl: "",
  chatgptAccountId: "",
};

function buildCodexAccountNoteForm(
  account?: CodexAccount | null,
): CodexAccountNoteFormState {
  return {
    note: account?.account_note ?? "",
    twoFactorSecret: account?.two_factor_secret ?? "",
    accountPassword: account?.account_password ?? "",
    phoneNumber: account?.phone_number ?? "",
    mailUrl: account?.mail_url ?? "",
    chatgptAccountId: account?.account_id ?? "",
  };
}

function hasCodexAccountNoteDetails(account?: CodexAccount | null): boolean {
  return Boolean(
    account?.account_note?.trim() ||
      account?.two_factor_secret?.trim() ||
      account?.account_password?.trim() ||
      account?.phone_number?.trim() ||
      account?.mail_url?.trim(),
  );
}

function hasCodexAccountNoteFormDetails(
  form?: CodexAccountNoteFormState | null,
): boolean {
  return Boolean(
    form?.note.trim() ||
      form?.twoFactorSecret.trim() ||
      form?.accountPassword.trim() ||
      form?.phoneNumber.trim() ||
      form?.mailUrl.trim(),
  );
}

function getCodexAccountNoteTitle(account: CodexAccount, fallback: string): string {
  return (
    account.account_note?.trim() ||
    account.two_factor_secret?.trim() ||
    account.account_password?.trim() ||
    account.phone_number?.trim() ||
    account.mail_url?.trim() ||
    fallback
  );
}

function formatCodexAccountNoteMailPreviewTime(timestamp: number): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function formatMfaRecordOption(record: MfaRecord, fallback: string): string {
  const accountName = record.accountName.trim();
  if (accountName) return accountName;
  const secret = record.secret.trim();
  if (!secret) return fallback;
  if (secret.length <= 14) return secret;
  return `${secret.slice(0, 6)}...${secret.slice(-4)}`;
}

function formatMfaSecretPreview(secret: string): string {
  const trimmed = secret.trim();
  if (!trimmed) return "";
  if (trimmed.length <= 12) return trimmed;
  return `${trimmed.slice(0, 5)}...${trimmed.slice(-4)}`;
}

function isPendingOAuthCodexAccount(account?: CodexAccount | null): boolean {
  return isCodexPendingOAuthAccount(account);
}


function isSponsorModelProvider(
  provider: CodexModelProvider | null | undefined,
  sponsorTemplates: SponsorApiProviderTemplate[],
): boolean {
  if (!provider) return false;
  if (provider.sourceTag) {
    return sponsorTemplates.some(
      (template) => template.id === provider.sourceTag,
    );
  }
  const normalizedBaseUrl = normalizeHttpBaseUrl(provider.baseUrl);
  if (!normalizedBaseUrl) return false;
  return sponsorTemplates.some(
    (template) => normalizeHttpBaseUrl(template.baseUrl) === normalizedBaseUrl,
  );
}

interface LocalAccessAccountPoolHealthSummary {
  total: number;
  available: number;
  abnormal: number;
  cooldown: number;
  missing: number;
  authError: number;
  quotaLimited: number;
}

const ABNORMAL_LOCAL_ACCESS_ACCOUNT_FAILURE_CATEGORIES = new Set([
  "auth_unavailable",
  "auth_refresh_failed",
  "account_prepare_failed",
]);

function isAbnormalLocalAccessAccountFailure(
  health?: CodexLocalAccessAccountHealth,
): boolean {
  return Boolean(
    health &&
      ((health.schedulerAvailable === false && !health.cooldowns.length) ||
        (health.consecutiveFailures >= 3 &&
          health.lastFailureCategory &&
          ABNORMAL_LOCAL_ACCESS_ACCOUNT_FAILURE_CATEGORIES.has(
            health.lastFailureCategory,
          ))),
  );
}

function normalizeLocalAccessAddressKind(
  value: string | null | undefined,
): CodexLocalAccessAddressKind {
  return value === "lan" ? "lan" : "local";
}

function readStoredLocalAccessAddressKind(): CodexLocalAccessAddressKind {
  try {
    return normalizeLocalAccessAddressKind(
      localStorage.getItem(CODEX_LOCAL_ACCESS_ADDRESS_KIND_KEY),
    );
  } catch {
    return "local";
  }
}

function persistLocalAccessAddressKind(
  value: CodexLocalAccessAddressKind,
): void {
  try {
    localStorage.setItem(CODEX_LOCAL_ACCESS_ADDRESS_KIND_KEY, value);
  } catch {
    // ignore storage write failures
  }
}

function readLocalAccessGatewayGuideDismissed(): boolean {
  try {
    return (
      localStorage.getItem(CODEX_LOCAL_ACCESS_GATEWAY_GUIDE_DISMISSED_KEY) ===
      "1"
    );
  } catch {
    return false;
  }
}

function persistLocalAccessGatewayGuideDismissed(): void {
  try {
    localStorage.setItem(CODEX_LOCAL_ACCESS_GATEWAY_GUIDE_DISMISSED_KEY, "1");
  } catch {
    // ignore storage write failures
  }
}

const CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY =
  "cockpit.codex.batchImport.sessionId";

type CodexBatchImportFilter = "all" | "ready";

function shouldAutoHideBatchDeleteJob(
  job: CodexBatchDeleteJobStatus | null,
): job is CodexBatchDeleteJobStatus & { status: "completed" } {
  return job?.status === "completed" && job.failed === 0;
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

function readCockpitApiOptionalNumber(
  record: CockpitApiJsonRecord | null,
  key: string,
): number | null {
  const value = record?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function formatCockpitApiInteger(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(
    Math.max(0, value),
  );
}

function formatCockpitApiTokenCount(value: number): string {
  const normalized = Math.max(0, value);
  if (normalized >= 100_000_000) {
    return `${(normalized / 100_000_000).toFixed(normalized >= 1_000_000_000 ? 1 : 2).replace(/\.?0+$/, "")}亿`;
  }
  if (normalized >= 10_000) {
    return `${(normalized / 10_000).toFixed(normalized >= 100_000 ? 1 : 2).replace(/\.?0+$/, "")}万`;
  }
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(
    normalized,
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

function resolveApiKeyUsageMode(
  summary?: CodexModelProviderUsageSummary,
): "new_api" | "sub2api" | null {
  if (!summary) return null;
  if (summary.mode === "new_api" || summary.mode === "sub2api") {
    return summary.mode;
  }
  if (
    typeof summary.todayRequests === "number" ||
    typeof summary.todayTotalTokens === "number"
  ) {
    return "sub2api";
  }
  const detailKeys = new Set((summary.details ?? []).map((item) => item.key));
  if (
    detailKeys.has("todayRequests") ||
    detailKeys.has("todayTokens") ||
    detailKeys.has("remaining")
  ) {
    return "sub2api";
  }
  if (
    detailKeys.has("totalGranted") ||
    detailKeys.has("totalAvailable") ||
    detailKeys.has("expiresAt")
  ) {
    return "new_api";
  }
  return null;
}

interface CodexOverviewGeneralConfig {
  codex_local_access_entry_visible?: boolean;
  codex_hide_relay_quota?: boolean;
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

function isSameHttpBaseUrl(left: string, right: string): boolean {
  const normalizedLeft = normalizeHttpBaseUrl(left)?.toLowerCase();
  const normalizedRight = normalizeHttpBaseUrl(right)?.toLowerCase();
  return Boolean(normalizedLeft && normalizedRight && normalizedLeft === normalizedRight);
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

interface CodexCliLaunchModalState {
  target: "account" | "apiService";
  accountId: string;
  accountLabel: string;
  instanceId: string | null;
  instanceDraft: CodexCliInstanceDraft | null;
  instanceName: string;
  workingDir: string;
  workingDirError: string | null;
  launchCommand: string;
  terminalCommand: string;
  runtimePrepared: boolean;
  preparing: boolean;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
}

interface CodexCliInstanceDraft {
  name: string;
  userDataDir: string;
  workingDir: string;
  extraArgs: string;
  bindAccountId: string;
}

const CODEX_CLI_LAST_WORKING_DIR_KEY = "cockpit.codex.cli.lastWorkingDir";

function readLastCodexCliWorkingDir(): string {
  try {
    return localStorage.getItem(CODEX_CLI_LAST_WORKING_DIR_KEY)?.trim() || "";
  } catch {
    return "";
  }
}

function persistLastCodexCliWorkingDir(value: string): void {
  const workingDir = value.trim();
  if (!workingDir) return;
  try {
    localStorage.setItem(CODEX_CLI_LAST_WORKING_DIR_KEY, workingDir);
  } catch {
    // 工作目录记忆失败不影响 CLI 启动。
  }
}

function maskCodexApiKey(value: string): string {
  const raw = value.trim();
  if (!raw) return raw;
  if (raw.startsWith("sk-")) return "sk-••••••••••••••••";
  return "••••••••••••••••";
}

function parseApiModelCatalogText(value: string): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean)
    .forEach((model) => {
      const key = model.toLowerCase();
      if (seen.has(key)) return;
      seen.add(key);
      models.push(model);
    });
  return models;
}

interface SponsorApiProviderTemplate {
  id: string;
  sponsor: Sponsor;
  name: string;
  baseUrl: string;
  modelCatalog: string[];
  supportsVision: boolean;
  website: string;
  apiKeyUrl: string;
  wireApi?: "responses" | "chat_completions" | null;
  integrationType?: "sub2api" | "new_api" | null;
}

function normalizeSponsorApiProviderTemplates(
  sponsors: Sponsor[] | undefined,
): SponsorApiProviderTemplate[] {
  const templates: SponsorApiProviderTemplate[] = [];
  for (const sponsor of sponsors ?? []) {
    const integration = sponsor.integration;
    if (
      !integration?.enabled ||
      !integration.quickConfigure ||
      !integration.baseUrl?.trim()
    ) {
      continue;
    }
    templates.push({
      id: `relay:${sponsor.id}`,
      sponsor,
      name: sponsor.name,
      baseUrl: integration.baseUrl.trim(),
      modelCatalog: integration.models ?? [],
      supportsVision: integration.supportsVision === true,
      website: normalizeApiKeyFunOfficialUrl(
        integration.website || sponsor.url,
      ),
      apiKeyUrl: normalizeApiKeyFunOfficialUrl(
        integration.apiKeyUrl || sponsor.url,
      ),
      wireApi: resolveApiKeyFunWireApi(
        integration.baseUrl,
        integration.wireApi ?? null,
      ),
      integrationType: integration.type ?? null,
    });
  }
  return templates.sort((a, b) => {
    const priority = a.sponsor.priority - b.sponsor.priority;
    if (priority !== 0) return priority;
    return a.name.localeCompare(b.name);
  });
}

function isRelayApiProviderTemplateId(value?: string | null): boolean {
  return Boolean(value?.startsWith("relay:"));
}

function getDefaultApiProviderPresetId(
  sponsorTemplates: SponsorApiProviderTemplate[],
): string {
  return sponsorTemplates[0]?.id ?? DEFAULT_CODEX_API_PROVIDER_ID;
}

function resolveApiProviderPresetDefaults(
  providerId: string,
  sponsorTemplates: SponsorApiProviderTemplate[],
): { baseUrl: string; providerName: string } {
  const sponsorTemplate = sponsorTemplates.find(
    (template) => template.id === providerId,
  );
  if (sponsorTemplate) {
    return {
      baseUrl: sponsorTemplate.baseUrl,
      providerName: sponsorTemplate.name,
    };
  }
  const preset = findCodexApiProviderPresetById(providerId);
  return {
    baseUrl: preset?.baseUrls[0] ?? DEFAULT_CODEX_API_BASE_URL,
    providerName: "",
  };
}

export function CodexAccountsPage() {
  const isMacOS = usePlatformRuntimeSupport("macos-only");
  const isWindows = usePlatformRuntimeSupport("windows-only");
  const isCliLaunchSupported = isMacOS || isWindows;
  const sponsorModule = useSponsorStore((state) => state.state.sponsorModule);
  const fetchSponsorState = useSponsorStore((state) => state.fetchState);
  const [activeTab, setActiveTab] = useState<CodexTab>("overview");
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
  const [importApiServiceGuideCount, setImportApiServiceGuideCount] =
    useState<number | null>(null);
  const [externalImportSyncError, setExternalImportSyncError] = useState<
    string | null
  >(null);
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

  const dismissLocalAccessGatewayGuide = useCallback(() => {
    persistLocalAccessGatewayGuideDismissed();
    setLocalAccessGatewayGuideDismissed(true);
  }, []);

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
  const [
    localAccessGatewayGuideDismissed,
    setLocalAccessGatewayGuideDismissed,
  ] = useState(readLocalAccessGatewayGuideDismissed);

  const store = useCodexAccountStore();
  const codexInstanceStore = useCodexInstanceStore();
  const [cliLaunchingAccountId, setCliLaunchingAccountId] = useState<
    string | null
  >(null);
  const [cliLaunchModal, setCliLaunchModal] =
    useState<CodexCliLaunchModalState | null>(null);
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
  const [pendingOAuthFieldErrors, setPendingOAuthFieldErrors] =
    useState<CodexAccountNoteFieldErrors & { email?: string }>({});
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
  const [mfaTimeRemaining, setMfaTimeRemaining] = useState(
    getMfaTimeRemaining,
  );
  const [savingAccountNote, setSavingAccountNote] = useState(false);
  const [savingAppSpeedId, setSavingAppSpeedId] = useState<string | null>(null);
  const [apiServiceAppSpeed, setApiServiceAppSpeed] =
    useState<CodexAppSpeed>("standard");
  const [reauthTargetAccount, setReauthTargetAccount] =
    useState<CodexAccount | null>(null);
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
  const [batchImportFilter, setBatchImportFilter] = useState<
    CodexBatchImportFilter
  >("all");
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
      allowEmptyCurrent:
        !!currentAccount && removeIds.has(currentAccount.id),
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
        if (
          nextJob.completed > batchDeleteRefreshedCompletedRef.current
        ) {
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
        Math.round((batchImportProgressCurrent / batchImportProgressTotal) * 100),
      )
    : 0;

  const openCodexAddModal = useCallback(
    (tab: string, targetAccount?: CodexAccount | null) => {
      setReauthTargetAccount(targetAccount ?? null);
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
    const handleOpenAddAccount = (event: Event) => {
      const detail = (event as CustomEvent<CodexOpenAddAccountDetail>).detail;
      if (detail?.autoJoinApiService) {
        setSyncImportedToApiService(true);
        writeCodexImportSyncApiService(true);
      }
      openCodexAddModal(detail?.tab ?? "oauth");
    };
    window.addEventListener(CODEX_OPEN_ADD_ACCOUNT_EVENT, handleOpenAddAccount);
    return () => {
      window.removeEventListener(
        CODEX_OPEN_ADD_ACCOUNT_EVENT,
        handleOpenAddAccount,
      );
    };
  }, [openCodexAddModal]);

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
    const exportFormatSupportsSensitiveNotes = exportFormat !== "sub2api";
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
    exportHasSensitiveNotes && exportFormat !== "sub2api";

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
    switchAccount,
    refreshQuota,
    refreshSubscriptionInfo,
    hydrateAccountProfilesIfNeeded,
    updateAccountName,
    updateApiKeyCredentials,
    updateApiKeyBoundOAuthAccount,
    updateAccountAppSpeed,
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
        ? accounts.find((account) => account.id === resetCreditConfirmAccountId) ??
          null
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
      ? buildCodexAccountPresentation(editingAccountNoteAccount, t).displayName
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
        const response = await codexService.fetchCodexAccountNoteMailUrl(mailUrl);
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
        setAccountNoteError(t("common.shared.export.copyFailed", "复制失败，请手动复制"));
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
  const [apiModelCatalogInput, setApiModelCatalogInput] = useState("");
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
  const [oauthBindingQuotaReserveEditorOpen, setOauthBindingQuotaReserveEditorOpen] =
    useState(false);
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
  const isLocalAccessOAuthBinding =
    oauthBindingTargetKind === "local_access";
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
    ): {
      apiProviderMode: CodexApiProviderMode;
      apiProviderId?: string;
      apiProviderName?: string;
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
          accountName: managedProvider.name,
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
          apiModelCatalog: preset.modelCatalog,
          apiWireApi: resolveCodexProviderCapabilityProfile({
            presetId: preset.id,
            baseUrl: normalizedBaseUrl,
            wireApi: null,
          }).wireApi,
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
        findCodexModelProviderById(
          managedProviders,
          account.api_provider_id,
        ) ??
        findCodexModelProviderByBaseUrl(
          managedProviders,
          account.api_base_url ?? "",
        )
      )?.id ?? null;
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
    if (accounts.length === 0) {
      return;
    }
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
    if (!managedProviderId) {
      skipManagedProviderApiKeyAutofillRef.current = false;
      setManagedProviderApiKeyId("");
      return;
    }
    const matched = findCodexModelProviderById(managedProviders, managedProviderId);
    if (!matched || !isSameHttpBaseUrl(matched.baseUrl, apiBaseUrlInput)) {
      skipManagedProviderApiKeyAutofillRef.current = false;
      setManagedProviderId("");
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
  }, [apiBaseUrlInput, apiProviderPresetId, managedProviderId, managedProviders]);

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
    if (!editingManagedProviderId) {
      setEditingManagedProviderApiKeyId("");
      return;
    }
    const matched = findCodexModelProviderById(
      managedProviders,
      editingManagedProviderId,
    );
    if (
      !matched ||
      !isSameHttpBaseUrl(matched.baseUrl, editingApiBaseUrlCredentialsValue)
    ) {
      setEditingManagedProviderId("");
      setEditingManagedProviderApiKeyId("");
      return;
    }
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
      await fetchAccounts();
      await fetchCurrentAccount();
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
    },
    [
      assignCodexAccountsToTargetGroup,
      fetchAccounts,
      fetchCurrentAccount,
      reauthTargetAccountId,
      syncImportedAccountsToApiService,
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

  // ─── Codex-specific: Switch / Import ─────────────────────────────────

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
    [
      resolveBoundOAuthAccount,
      setOauthBindingError,
    ],
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
    (
      field: keyof OAuthBindingQuotaReserveFieldErrors,
      rawValue: string,
    ) => {
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
      const flowStartedAt = performance.now();
      console.info("[Codex Switch][UI] button loading started", {
        accountId,
      });
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
        console.info("[Codex Switch][UI] button loading finished", {
          accountId,
          elapsedMs: Math.round(performance.now() - flowStartedAt),
        });
      }
    },
    [
      maskAccountText,
      setMessage,
      switchAccount,
      t,
    ],
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

  const handleSwitch = async (accountId: string) => {
    const account = codexAccountsRef.current.find((item) => item.id === accountId);
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
    try {
      await executeCodexAccountSwitch(accountId);
    } catch (e) {
      setMessage({
        text: t("codex.switchFailed", {
          error: formatCodexAuthFailureMessage(e),
        }),
        tone: "error",
      });
    }
  };

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
          normalizePathForCompare(instance.workingDir) ===
            normalizedWorkingDir,
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
        : modal.accountId;
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
  ) => {
    if (cliLaunchModal || cliLaunchingAccountId) return;
    setMessage(null);
    const modal: CodexCliLaunchModalState = {
      target,
      accountId,
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

  const handleLaunchCodexCli = (account: CodexAccount) => {
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
    const presentation = buildCodexAccountPresentation(account, t);
    openCodexCliLaunchModal(
      "account",
      account.id,
      presentation.displayName || account.email || account.id,
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
        setCliLaunchModal((prev) =>
          prev ? { ...prev, copied: false } : prev,
        );
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
  }, [
    selectedTerminal,
  ]);

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
        const recoveredPreview =
          await codexService.getCodexBatchImportPreview(started.sessionId);
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
        return;
      }
      const sponsorTemplate = sponsorApiProviderTemplates.find(
        (template) => template.id === providerId,
      );
      if (sponsorTemplate) {
        setApiBaseUrlInput(sponsorTemplate.baseUrl);
        setNewManagedProviderNameInput(sponsorTemplate.name);
        setApiModelCatalogInput(sponsorTemplate.modelCatalog.join("\n"));
        return;
      }
      const preset = findCodexApiProviderPresetById(providerId);
      if (!preset || preset.baseUrls.length === 0) return;
      setApiBaseUrlInput(preset.baseUrls[0]);
      setNewManagedProviderNameInput("");
      setApiModelCatalogInput((preset.modelCatalog ?? []).join("\n"));
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
      }
      const preset = findCodexApiProviderPresetById(providerId);
      if (!preset || preset.baseUrls.length === 0) return;
      setEditingApiBaseUrlCredentialsValue(preset.baseUrls[0]);
      setEditingApiModelCatalogInput((preset.modelCatalog ?? []).join("\n"));
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
      ),
      apiModelCatalog: apiModelCatalogDraft,
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
            supportsVision: providerPayload.sponsorTemplate?.supportsVision,
            website: providerPayload.sponsorTemplate?.website,
            apiKeyUrl: providerPayload.sponsorTemplate?.apiKeyUrl,
            wireApi: providerPayload.sponsorTemplate?.wireApi,
            integrationType: providerPayload.sponsorTemplate?.integrationType,
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
            accountName: savedProvider.name,
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
        t("codex.import.successMsg", "导入成功: {{email}}").replace(
          "{{email}}",
          maskAccountText(account.email),
        ),
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
      page.setAddMessage(t("common.shared.token.empty", "请输入 Token 或 JSON"));
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
          imported.push(...(await codexService.importCodexFromJson(payloads[index])));
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
      return (
        <div className="account-sub-line codex-provider-inline-line codex-oauth-binding-line">
          <span
            className="codex-login-subline codex-provider-inline-text"
            title={line}
          >
            {line}
          </span>
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
    [maskAccountText, openOAuthBindingModal, resolveBoundOAuthAccount, t],
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
      if (
        !isCodexApiKeyAccount(account) ||
        isCodexNewApiAccount(account)
      ) {
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
      if (isCodexChatCompletionsApiKeyAccount(account)) {
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
        isCodexChatCompletionsApiKeyAccount(account)
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
      if (
        state?.loading ||
        state?.unavailable ||
        apiKeyUsageInFlightRef.current.has(account.id)
      ) {
        return false;
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
  }, [apiKeyUsageMap]);

  useEffect(() => {
    const syncUsageCache = () => setApiKeyUsageMap(readCodexApiKeyUsageCache());
    window.addEventListener(CODEX_API_KEY_USAGE_REFRESHED_EVENT, syncUsageCache);
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
        .filter((account) => isCodexChatCompletionsApiKeyAccount(account))
        .map((account) => account.id),
    );
    setApiKeyUsageMap((previous) => {
      let changed = false;
      const next: Record<string, CodexApiKeyUsageState> = {};
      for (const [accountId, state] of Object.entries(previous)) {
        if (accountIds.has(accountId) && !chatCompletionsAccountIds.has(accountId)) {
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
  }, [refreshApiKeyUsageByAccountId]);

  const formatApiKeyUsageMoney = useCallback(
    (value?: number | null, unit?: string | null): string => {
      if (typeof value !== "number" || !Number.isFinite(value)) return "-";
      const normalizedUnit = unit?.trim() || "USD";
      const formatted = value.toFixed(value >= 100 ? 0 : 2);
      return normalizedUnit === "USD"
        ? `$${formatted}`
        : `${formatted} ${normalizedUnit}`;
    },
    [],
  );

  const formatApiKeyUsageBalance = useCallback(
    (summary?: CodexModelProviderUsageSummary): string | null => {
      if (
        typeof summary?.balance !== "number" ||
        !Number.isFinite(summary.balance)
      ) {
        return null;
      }
      return formatApiKeyUsageMoney(summary.balance, summary.unit);
    },
    [formatApiKeyUsageMoney],
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
      if (item.key === "quotaUnlimited" || item.key === "modelLimitsEnabled") {
        if (raw === "true")
          return t("codex.modelProviders.usage.booleanTrue", "是");
        if (raw === "false")
          return t("codex.modelProviders.usage.booleanFalse", "否");
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

  const renderApiKeyUsagePanel = useCallback(
    (
      account: CodexAccount,
      provider: CodexModelProvider | null,
      variant: "card" | "table" = "card",
    ): ReactElement => {
      if (isCodexChatCompletionsApiKeyAccount(account)) {
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
      const isNewApiUsage = usageMode === "new_api";
      const isSub2ApiUsage = usageMode === "sub2api";
      const usedPercent = formatApiKeyUsagePercent(summary);
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
                {isNewApiUsage ? (
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
              {isNewApiUsage ? (
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
      formatApiKeyUsageBalance,
      formatApiKeyUsageQuotaValue,
      formatApiKeyUsageDetailByKey,
      canRefreshApiKeyUsage,
      refreshApiKeyUsage,
      setApiKeyUsageDetailAccountId,
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
    setEditingApiModelCatalogError(null);
    const editingAccount = accounts.find((account) => account.id === accountId);
    const providerPayload = {
      ...buildApiProviderPayload(
        editingApiBaseUrlCredentialsValue,
        editingApiProviderPresetId,
        editingManagedProviderId,
        editingNewManagedProviderNameInput,
      ),
      apiModelCatalog: editingApiModelCatalogDraft,
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
            supportsVision: providerPayload.sponsorTemplate?.supportsVision,
            website: providerPayload.sponsorTemplate?.website,
            apiKeyUrl: providerPayload.sponsorTemplate?.apiKeyUrl,
            wireApi: providerPayload.apiWireApi,
            supportsWebsockets: providerPayload.apiSupportsWebsockets,
            integrationType: providerPayload.sponsorTemplate?.integrationType,
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
      setMessage({ text: t("instances.messages.updated", "实例已更新") });
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
      const errorCode = extractCodexQuotaErrorCode(
        rawMessage,
        quotaError.code,
      );
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
    (accountName: string, message: string) => {
      const text = message.trim();
      if (!text) return;
      setQuotaErrorDetail({
        accountName: accountName.trim() || t("common.unknown", "未知"),
        message: text,
      });
    },
    [t],
  );

  const renderQuotaErrorInline = useCallback(
    (options: {
      accountName: string;
      displayText: string;
      rawMessage: string;
      isVerbose: boolean;
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
        isRefreshNotice = false,
        showReauthorize = false,
        onReauthorize,
        table = false,
      } = options;
      const showDetailAction =
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
              onClick={() => openQuotaErrorDetail(accountName, rawMessage)}
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
    return createPortal(
      <div className="modal-overlay">
        <div
          className="modal-content codex-quota-error-detail-modal"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="modal-header">
            <h3>{t("codex.quotaError.detailTitle", "错误详情")}</h3>
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
            <pre className="codex-quota-error-detail-text">
              {quotaErrorDetail.message}
            </pre>
          </div>
          <div className="modal-footer">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(quotaErrorDetail.message)
                  .catch(() => undefined);
              }}
            >
              {t("common.copy", "复制")}
            </button>
            <button
              type="button"
              className="btn btn-primary"
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
      const summary: LocalAccessAccountPoolHealthSummary = {
        total: localAccessCollection?.accountIds.length ?? 0,
        available: 0,
        abnormal: 0,
        cooldown: 0,
        missing: 0,
        authError: 0,
        quotaLimited: 0,
      };

      (localAccessCollection?.accountIds ?? []).forEach((accountId) => {
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
        if (isBlockingCodexQuotaError(account.quota_error)) {
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
        summary.available += 1;
      });

      return summary;
    }, [
      accounts,
      localAccessCollection?.accountIds,
      localAccessState?.accountHealth,
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
    localAccessPortKilling;
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

  const renderAddLocalAccessAccountButton = (
    account: CodexAccount,
    className: string,
    iconSize = 14,
  ) => {
    if (!canDirectlyAddLocalAccessAccount(account)) return null;
    const label = t(
      "codex.localAccess.entryAction",
      "添加至 API 服务",
    );
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
        const backupAccountIds = (options?.backupAccountIds ?? []).filter((id) =>
          filteredAccountIdSet.has(id),
        );
        const preferredAccountIds = (
          options?.preferredAccountIds ?? []
        ).filter((id) => filteredAccountIdSet.has(id));
        const nextState =
          await codexLocalAccessService.saveCodexLocalAccessAccounts(
            filteredAccountIds,
            restrictFreeAccounts,
            backupAccountIds,
            preferredAccountIds,
            options?.sessionAffinity,
            options?.sessionAffinityTtlMs,
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

  useEffect(() => {
    const existingAccountIds = new Set(accounts.map((account) => account.id));
    const hasStaleAccountIds = codexGroups.some((group) =>
      group.accountIds.some((accountId) => !existingAccountIds.has(accountId)),
    );
    if (!hasStaleAccountIds) {
      return;
    }

    void (async () => {
      try {
        await cleanupDeletedCodexAccounts(existingAccountIds);
        await reloadCodexGroups();
      } catch (error) {
        console.error(
          "Failed to clean up deleted Codex accounts from groups:",
          error,
        );
      }
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

  const handleUpdateLocalAccessGatewayMode = useCallback(
    async (gatewayMode: CodexLocalAccessGatewayMode) => {
      if (
        !localAccessCollection ||
        localAccessCollection.gatewayMode === gatewayMode
      ) {
        return;
      }
      setLocalAccessSaving(true);
      try {
        const nextState =
          await codexLocalAccessService.updateCodexLocalAccessGatewayMode(
            gatewayMode,
          );
        setLocalAccessState(nextState);
        setMessage({
          text: t(
            "codex.localAccess.gatewayModeSaveSuccess",
            "API 服务网关模式已更新",
          ),
        });
        dismissLocalAccessGatewayGuide();
        return nextState;
      } catch (error) {
        console.error("Failed to update local access gateway mode:", error);
        throw new Error(String(error).replace(/^Error:\s*/, ""));
      } finally {
        setLocalAccessSaving(false);
      }
    },
    [dismissLocalAccessGatewayGuide, localAccessCollection, setMessage, t],
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
      const flowStartedAt = performance.now();
      console.info("[Codex API Service Switch][UI] button loading started");
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
        console.info(
          "[Codex API Service Switch][UI] button loading finished",
          { elapsedMs: Math.round(performance.now() - flowStartedAt) },
        );
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
      const state = await handleActivateLocalAccess();
      if (!state) {
        return;
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
  }, [handleActivateLocalAccess, setMessage, t]);

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
      const successCount = await codexService.refreshCodexQuotasBatch(targetIds);

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
          error: String(error ?? "").replace(/^Error:\s*/, "") ||
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

  // ─── Filtering & Sorting ────────────────────────────────────────────
  const overviewCurrentAccountId = localAccessLaunchCurrent
    ? null
    : (currentAccount?.id ?? null);

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
      filteredAccounts
        .filter(isAbnormalAccount)
        .map((account) => account.id),
    [filteredAccounts, isAbnormalAccount],
  );
  // Full overview set of auth-failed accounts for export (#992), not limited to current page filter.
  const authFailedExportAccountIds = useMemo(
    () =>
      overviewAccounts
        .filter(isAbnormalAccount)
        .map((account) => account.id),
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
        defaultValue:
          "确定要删除当前范围内的 {{count}} 条 ERROR 账号吗？",
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
      const job = await codexService.pauseCodexBatchDelete(batchDeleteJob.jobId);
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
          { respectGroupQuotaRefresh: false },
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
            error: String(error ?? "").replace(/^Error:\s*/, "") ||
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

  const renderResetCreditControls = (account: CodexAccount) => {
    if (isCodexApiKeyAccount(account) || isCodexAgentIdentityAccount(account)) return null;

    const creditDetails = getResetCreditDetails(account);
    const availableCount = getResetCreditsAvailable(account);
    if (availableCount == null && creditDetails.length === 0) return null;

    const displayCount =
      availableCount ??
      creditDetails.filter(isAvailableResetCredit).length;
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
      const isAgentIdentityAccount = isCodexAgentIdentityAccount(account);
      const switchOrLaunchBlockedReason =
        getCodexSwitchOrLaunchBlockedReason(account);
      const isChatCompletionsApiKey =
        isCodexChatCompletionsApiKeyAccount(account);
      const compactQuotaItems = resolveCompactQuotaItems(presentation);
      const subscriptionInfo = resolveSubscriptionPresentation(account);
      const showCompactExpiry =
        !isApiKeyAccount && !isAgentIdentityAccount && subscriptionInfo.bucket !== "active";
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
          <div className="codex-compact-quotas">
            {!isChatCompletionsApiKey &&
              compactQuotaItems.map((item) => (
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
      const isSelected = selected.has(account.id);
      const quotaItems = resolveVisibleQuotaItems(
        presentation,
        isApiKeyAccount,
        isNewApiAccount,
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
      const showApiKeyUsagePanel =
        isApiKeyAccount &&
        !isNewApiAccount &&
        !isChatCompletionsApiKey &&
        !hideRelayQuota;
      const isSub2ApiUsageAccount =
        showApiKeyUsagePanel &&
        (apiKeyUsageMode === "sub2api" ||
          apiKeyUsageProvider?.integrationType === "sub2api");
      const isQuotaAwareApiKeyAccount =
        showApiKeyUsagePanel &&
        !isSponsorApiKeyAccount &&
        (apiKeyUsageMode !== null ||
          apiKeyUsageProvider?.integrationType === "new_api" ||
          apiKeyUsageProvider?.integrationType === "sub2api");
      const shouldRenderQuotaSection =
        (!hideRelayQuota && showApiKeyUsagePanel) ||
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
            {hasQuotaError && (
              <span
                className={`codex-status-pill ${isQuotaRefreshNotice ? "quota-refresh" : "quota-error"}`}
                title={accountIssueMeta.displayText}
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
                <span className="group-account-badge is-current">
                  {t("codex.localAccess.modal.selected", "已加入 API 服务")}
                </span>
              )}
              {!isInLocalAccess && canAddToLocalAccess && (
                <button
                  type="button"
                  className="group-account-badge codex-local-access-inline-add"
                  onClick={() => void handleAddLocalAccessAccount(account.id)}
                  disabled={addingLocalAccessAccountId === account.id}
                  title={t(
                    "codex.localAccess.entryAction",
                    "添加至 API 服务",
                  )}
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
                {isSub2ApiUsageAccount && (
                  <button
                    type="button"
                    className="codex-provider-inline-switch"
                    onClick={() => setApiKeyUsageDetailAccountId(account.id)}
                    title={t("codex.modelProviders.usage.detailTitle", "服务面板")}
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
                {!isPendingOAuthAccount &&
                  hasQuotaError &&
                  renderQuotaErrorInline({
                    accountName: presentation.displayName,
                    displayText: accountIssueMeta.displayText,
                    rawMessage: accountIssueMeta.rawMessage,
                    isVerbose: accountIssueMeta.isVerbose,
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
                {quotaItems.map((item) => {
                  const QuotaIcon =
                    item.key === "secondary" || item.key.endsWith(":secondary")
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
    const localAccessGatewayMode =
      localAccessCollection?.gatewayMode ?? "sidecar";
    const localAccessGatewayModeOptions = [
      {
        value: "sidecar",
        label: t("codex.localAccess.gatewayModeNewLabel", "API 服务-新"),
      },
      {
        value: "legacy",
        label: t("codex.localAccess.gatewayModeOldLabel", "API 服务-旧"),
      },
    ];
    const showLocalAccessGatewayGuide = !localAccessGatewayGuideDismissed;
    const renderLocalAccessGatewayGuide = () =>
      showLocalAccessGatewayGuide ? (
        <div
          className="codex-local-access-gateway-guide"
          role="dialog"
          aria-label={t(
            "codex.localAccess.gatewayGuideTitle",
            "这里可以切换网关",
          )}
          onClick={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            className="codex-local-access-gateway-guide-close"
            onClick={dismissLocalAccessGatewayGuide}
            aria-label={t("common.close", "关闭")}
          >
            <X size={12} />
          </button>
          <div className="codex-local-access-gateway-guide-title">
            {t("codex.localAccess.gatewayGuideTitle", "这里可以切换网关")}
          </div>
          <p>
            {t(
              "codex.localAccess.gatewayGuideDesc",
              "默认使用新网关。如果遇到兼容性问题或客户端请求异常，可以在这里切换到旧网关。",
            )}
          </p>
          <button
            type="button"
            className="codex-local-access-gateway-guide-action"
            onClick={dismissLocalAccessGatewayGuide}
          >
            {t("codex.localAccess.gatewayGuideAction", "我知道了")}
          </button>
        </div>
      ) : null;

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
                  <div
                    className="codex-local-access-title-mode-select"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <SingleSelectDropdown
                      value={localAccessGatewayMode}
                      options={localAccessGatewayModeOptions}
                      onChange={(value) =>
                        void handleUpdateLocalAccessGatewayMode(
                          value as CodexLocalAccessGatewayMode,
                        )
                      }
                      disabled={!localAccessCollection || localAccessBusy}
                      menuClassName="codex-local-access-title-mode-menu"
                      menuWidth={116}
                      menuMaxHeight={120}
                      ariaLabel={t(
                        "codex.localAccess.gatewayModeLabel",
                        "网关模式",
                      )}
                    />
                    {renderLocalAccessGatewayGuide()}
                  </div>
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
                  <div
                    className="codex-local-access-title-mode-select"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <SingleSelectDropdown
                      value={localAccessGatewayMode}
                      options={localAccessGatewayModeOptions}
                      onChange={(value) =>
                        void handleUpdateLocalAccessGatewayMode(
                          value as CodexLocalAccessGatewayMode,
                        )
                      }
                      disabled={!localAccessCollection || localAccessBusy}
                      menuClassName="codex-local-access-title-mode-menu"
                      menuWidth={116}
                      menuMaxHeight={120}
                      ariaLabel={t(
                        "codex.localAccess.gatewayModeLabel",
                        "网关模式",
                      )}
                    />
                    {renderLocalAccessGatewayGuide()}
                  </div>
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
                  localAccessAccountPoolHealthSummary.cooldown === 0
                    ? t("codex.localAccess.accountPoolHealth.allAvailable", {
                        count: localAccessAccountPoolHealthSummary.total,
                        defaultValue: "全部可用 {{count}}",
                      })
                    : t("codex.localAccess.accountPoolHealth.availableRatio", {
                        available: localAccessAccountPoolHealthSummary.available,
                        total: localAccessAccountPoolHealthSummary.total,
                        defaultValue: "可用 {{available}}/{{total}}",
                      })}
                </span>
                {(localAccessAccountPoolHealthSummary.abnormal > 0 ||
                  localAccessAccountPoolHealthSummary.cooldown > 0) && (
                  <span className="codex-local-access-health-summary-value">
                    {t("codex.localAccess.accountPoolHealth.issueSummary", {
                      abnormal: localAccessAccountPoolHealthSummary.abnormal,
                      cooldown: localAccessAccountPoolHealthSummary.cooldown,
                      defaultValue: "异常 {{abnormal}} · 冷却 {{cooldown}}",
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
                {isVerboseCodexQuotaErrorMessage(localAccessState.lastError) && (
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
                            onClick={() =>
                              setImportApiServiceGuideCount(null)
                            }
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
                            onClick={() =>
                              setImportApiServiceGuideCount(null)
                            }
                          >
                            {t("codex.importApiService.later", "稍后")}
                          </button>
                        </div>
                      )}
                    <button
                      className="card-action-btn success"
                      onClick={() => {
                        setImportApiServiceGuideCount(null);
                        void handleQuickActivateLocalAccess();
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
      const quotaItems = resolveVisibleQuotaItems(
        presentation,
        isApiKeyAccount,
        isNewApiAccount,
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
      const showApiKeyUsagePanel =
        isApiKeyAccount &&
        !isNewApiAccount &&
        !isChatCompletionsApiKey &&
        !hideRelayQuota;
      const isSub2ApiUsageAccount =
        showApiKeyUsagePanel &&
        (apiKeyUsageMode === "sub2api" ||
          apiKeyUsageProvider?.integrationType === "sub2api");
      const isQuotaAwareApiKeyAccount =
        showApiKeyUsagePanel &&
        !isSponsorApiKeyAccount &&
        (apiKeyUsageMode !== null ||
          apiKeyUsageProvider?.integrationType === "new_api" ||
          apiKeyUsageProvider?.integrationType === "sub2api");
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
                    <span className="group-account-badge is-current">
                      {t("codex.localAccess.modal.selected", "已加入 API 服务")}
                    </span>
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
                    {isSub2ApiUsageAccount && (
                      <button
                        type="button"
                        className="codex-provider-inline-switch"
                        onClick={() => setApiKeyUsageDetailAccountId(account.id)}
                        title={t("codex.modelProviders.usage.detailTitle", "服务面板")}
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
                    title={accountIssueMeta.displayText}
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
                    displayText: accountIssueMeta.displayText,
                    rawMessage: accountIssueMeta.rawMessage,
                    isVerbose: accountIssueMeta.isVerbose,
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
      (provider?.integrationType === "sub2api" ? "sub2api" : null);
    if (!usageMode) return null;
    const coreDetailKeys =
      usageMode === "new_api"
        ? new Set(["mode", "totalGranted", "totalAvailable", "expiresAt"])
        : usageMode === "sub2api"
          ? new Set(["mode", "remaining", "todayRequests", "todayTokens"])
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
              value: formatApiKeyUsageMoney(
                newApiQuota.granted,
                summary?.unit,
              ),
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
          : [];
    const summaryGridClassName =
      usageMode === "sub2api" || usageMode === "new_api"
        ? "cockpit-api-summary-grid compact"
        : "cockpit-api-summary-grid";

    return (
      <div
        className="modal-overlay"
      >
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
      <div
        className="modal-overlay"
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

  return (
    <div
      className={`codex-accounts-page codex-accounts-page--${overviewLayoutMode}`}
    >
      <CodexOverviewTabsHeader
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={[
          "overview",
          "providers",
          "wakeup",
          "instances",
          "sessions",
        ]}
      />

      {batchImportOpen && createPortal(
          <div className="modal-overlay codex-batch-import-overlay">
            <div
              className="modal-content codex-batch-import-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <div>
                  <h2>{t("codex.batchImport.title", "Codex 批量导入")}</h2>
                  <p className="codex-batch-import-subtitle">
                    {batchImportResult
                      ? t("codex.batchImport.resultSubtitle", "导入结果")
                      : batchImportProgress?.phase === "importing" ||
                          batchImportProgress?.phase === "finalizing"
                        ? t(
                            "codex.batchImport.importSubtitle",
                            "正在写入选中的账号",
                          )
                      : batchImportBusy
                        ? activeBatchImportCheckQuota
                          ? t(
                              "codex.batchImport.scanSubtitle",
                              "正在逐条解析并检查账号",
                            )
                          : t(
                              "codex.batchImport.parseSubtitle",
                              "正在解析账号文件",
                            )
                        : batchImportPreview
                          ? t(
                              "codex.batchImport.previewSubtitle",
                              "选择要写入的账号",
                            )
                          : activeBatchImportCheckQuota
                            ? t(
                                "codex.batchImport.scanSubtitle",
                                "正在逐条解析并检查账号",
                              )
                            : t(
                                "codex.batchImport.parseSubtitle",
                                "正在解析账号文件",
                              )}
                  </p>
                </div>
                <div className="codex-batch-import-header-actions">
                  {batchImportBusy && (
                    <button
                      type="button"
                      className="btn btn-secondary compact"
                      onClick={() => setBatchImportOpen(false)}
                    >
                      <Minimize2 size={14} />
                      {t("codex.batchImport.runInBackground", "后台执行")}
                    </button>
                  )}
                  <button
                    className="modal-close"
                    onClick={() => void handleCloseBatchImport()}
                  >
                    <X size={18} />
                  </button>
                </div>
              </div>

              <div className="codex-batch-import-body">
                {batchImportError && (
                  <div className="codex-batch-import-error">
                    <CircleAlert size={16} />
                    <span>{batchImportError}</span>
                  </div>
                )}

                {!batchImportResult && (
                  <div className="codex-batch-import-progress-panel">
                    <div className="codex-batch-import-progress-head">
                      <span>
                        {batchImportProgress?.phase === "cancelling"
                          ? t("codex.batchImport.cancelling", "正在取消...")
                          : batchImportBusy
                            ? batchImportProgress?.phase === "importing" ||
                              batchImportProgress?.phase === "finalizing"
                              ? t("codex.batchImport.importing", "导入中")
                              : activeBatchImportCheckQuota
                                ? t("codex.batchImport.scanning", "扫描中")
                                : t("codex.batchImport.parsing", "解析中")
                            : batchImportPreview?.status === "cancelled"
                              ? t("codex.batchImport.cancelled", "已取消")
                              : batchImportPreview
                                ? activeBatchImportCheckQuota
                                  ? t("codex.batchImport.scanDone", "扫描完成")
                                  : t("codex.batchImport.parseDone", "解析完成")
                                : activeBatchImportCheckQuota
                                  ? t("codex.batchImport.scanning", "扫描中")
                                  : t("codex.batchImport.parsing", "解析中")}
                      </span>
                      <strong>
                        {batchImportProgressCurrent}/{batchImportProgressTotal}
                      </strong>
                    </div>
                    <div className="codex-batch-import-progress-track">
                      <div
                        className="codex-batch-import-progress-fill"
                        style={{
                          width: `${batchImportProgressPercent}%`,
                        }}
                      />
                    </div>
                    {batchImportProgress?.currentLabel && (
                      <div className="codex-batch-import-current">
                        {t("codex.batchImport.current", "当前")}：
                        {maskAccountText(batchImportProgress.currentLabel)}
                      </div>
                    )}
                  </div>
                )}

                {batchImportResult ? (
                    <div className="codex-batch-import-result">
                    {batchImportResult.cancelled && (
                      <div className="codex-batch-import-cancelled-note">
                        {t(
                          "codex.batchImport.importCancelledSummary",
                          "导入已取消，已处理 {{processed}}/{{total}} 个账号。",
                        )
                          .replace("{{processed}}", String(batchImportResult.processed))
                          .replace("{{total}}", String(batchImportResult.total))}
                      </div>
                    )}
                    <div className="codex-batch-import-stat-grid">
                      <div>
                        <span>{t("codex.batchImport.imported", "已导入")}</span>
                        <strong>{batchImportResult.imported.length}</strong>
                      </div>
                      <div>
                        <span>{t("codex.batchImport.failed", "失败")}</span>
                        <strong>{batchImportResult.failed.length}</strong>
                      </div>
                    </div>
                    {batchImportResult.failed.length > 0 && (
                      <div className="codex-batch-import-list compact">
                        {batchImportResult.failed.map((item) => (
                          <div
                            className="codex-batch-import-row"
                            key={item.email}
                          >
                            <div>
                              <strong>{maskAccountText(item.email)}</strong>
                              <small>{item.error}</small>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                ) : batchImportPreview ? (
                  <>
                    <div className="codex-batch-import-stat-grid">
                      <div>
                        <span>
                          {t("codex.batchImport.groups.ready", "可导入")}
                        </span>
                        <strong>{batchImportCounts.ready}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.batchImport.groups.quotaFailed", "异常")}
                        </span>
                        <strong>{batchImportCounts.quotaFailed}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.batchImport.groups.existing", "已存在")}
                        </span>
                        <strong>{batchImportCounts.existing}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.batchImport.groups.invalid", "无效账号")}
                        </span>
                        <strong>{batchImportCounts.invalid}</strong>
                      </div>
                    </div>

                    <div className="codex-batch-import-toolbar">
                      <div className="codex-batch-import-toolbar-main">
                        <span>{batchImportSelectedCountLabel}</span>
                        <label className="codex-batch-import-check-toggle">
                          <input
                            type="checkbox"
                            checked={batchImportCheckQuota}
                            disabled={batchImportBusy}
                            onChange={(event) =>
                              void handleBatchImportCheckQuotaChange(
                                event.target.checked,
                              )
                            }
                          />
                          <span className="codex-batch-import-check-switch" />
                          <span>
                            {t(
                              "codex.batchImport.checkQuotaToggle",
                              "导入前检测账号",
                            )}
                          </span>
                        </label>
                        <label className="codex-batch-import-check-toggle">
                          <input
                            type="checkbox"
                            checked={syncImportedToApiService}
                            disabled={batchImportBusy}
                            onChange={(event) =>
                              handleSyncImportedToApiServiceChange(
                                event.target.checked,
                              )
                            }
                          />
                          <span className="codex-batch-import-check-switch" />
                          <span>
                            {t(
                              "codex.importApiService.toggle",
                              "同步加入 API 服务",
                            )}
                          </span>
                        </label>
                      </div>
                      <div className="codex-batch-import-actions">
                        <button
                          type="button"
                          className="btn btn-secondary compact"
                          disabled={
                            batchImportBusy ||
                            batchImportSelectableIds.length === 0
                          }
                          onClick={selectAllBatchImportAccounts}
                        >
                          {t(
                            "codex.batchImport.selectAllAccounts",
                            "选择全部账号",
                          )}
                        </button>
                        <button
                          type="button"
                          className="btn btn-secondary compact"
                          disabled={
                            batchImportBusy ||
                            batchImportCounts.ready +
                              batchImportCounts.existing ===
                              0
                          }
                          onClick={selectReadyBatchImportAccounts}
                        >
                          {t("codex.batchImport.selectReady", "选择正常账号")}
                        </button>
                        <button
                          type="button"
                          className="btn btn-secondary compact"
                          disabled={
                            batchImportBusy ||
                            batchImportSelectedSelectableCount === 0
                          }
                          onClick={clearBatchImportSelection}
                        >
                          {t("codex.batchImport.clearSelection", "取消选择")}
                        </button>
                      </div>
                    </div>

                    <div className="codex-batch-import-tags-row">
                      <label
                        className="codex-batch-import-tags-label"
                        htmlFor="codex-batch-import-tags"
                      >
                        {t(
                          "codex.batchImport.bulkTagsLabel",
                          "导入后批量打标",
                        )}
                      </label>
                      <input
                        id="codex-batch-import-tags"
                        type="text"
                        className="codex-batch-import-tags-input"
                        value={batchImportTagsInput}
                        disabled={batchImportBusy}
                        onChange={(event) =>
                          setBatchImportTagsInput(event.target.value)
                        }
                        placeholder={t(
                          "codex.batchImport.bulkTagsPlaceholder",
                          "可选，多个标签用逗号或空格分隔",
                        )}
                      />
                    </div>

                    <div className="codex-batch-import-list">
                      {[...batchImportVisibleItems].reverse().map((item) => {
                        const selectable = batchImportSelectableIdSet.has(
                          item.itemId,
                        );
                        const checked =
                          selectable &&
                          batchImportSelectedIds.includes(item.itemId);
                        return (
                          <label
                            className={`codex-batch-import-row status-${item.status}`}
                            key={item.itemId}
                          >
                            <input
                              type="checkbox"
                              checked={checked}
                              disabled={!selectable || batchImportBusy}
                              onChange={() => toggleBatchImportItem(item.itemId)}
                            />
                            <div className="codex-batch-import-row-main">
                              <div className="codex-batch-import-row-title">
                                <strong>{maskAccountText(item.label)}</strong>
                                <span>{item.accountType}</span>
                              </div>
                              <div className="codex-batch-import-row-meta">
                                <span>{item.source}</span>
                                {item.provider && <span>{item.provider}</span>}
                                {item.status === "ready" && (
                                  <span>
                                    {activeBatchImportCheckQuota
                                      ? t(
                                          "codex.batchImport.quotaOk",
                                          "账号正常",
                                        )
                                      : t(
                                          "codex.batchImport.groups.ready",
                                          "可导入",
                                        )}
                                  </span>
                                )}
                                {item.status === "quota_failed" && (
                                  <span>
                                    {t("codex.batchImport.quotaFailed", "异常")}
                                  </span>
                                )}
                                {item.status === "existing" && (
                                  <span>
                                    {t(
                                      "codex.batchImport.groups.existing",
                                      "已存在",
                                    )}
                                  </span>
                                )}
                                {item.status === "invalid" && (
                                  <span>
                                    {t(
                                      "codex.batchImport.groups.invalid",
                                      "无效账号",
                                    )}
                                  </span>
                                )}
                              </div>
                              {(item.quotaError || item.error) && (
                                <small className="codex-batch-import-row-error">
                                  {item.quotaError || item.error}
                                </small>
                              )}
                            </div>
                          </label>
                        );
                      })}
                    </div>
                  </>
                ) : (
                  <div className="codex-batch-import-empty">
                    <RefreshCw size={18} className="loading-spinner" />
                    {t("codex.batchImport.preparing", "正在准备导入任务...")}
                  </div>
                )}
              </div>

              <div className="modal-footer codex-batch-import-footer">
                {batchImportResult ? (
                  <button
                    className="btn btn-primary"
                    onClick={() => void handleCloseBatchImport()}
                  >
                    {t("common.shared.close", "关闭")}
                  </button>
                ) : (
                  <>
                    <button
                      className="btn btn-secondary"
                      onClick={() =>
                        batchImportBusy
                          ? void handleCancelBatchImport()
                          : void handleCloseBatchImport()
                      }
                      disabled={batchImportProgress?.phase === "cancelling"}
                    >
                      {batchImportBusy
                        ? batchImportProgress?.phase === "importing" ||
                          batchImportProgress?.phase === "finalizing"
                          ? t("codex.batchImport.cancelImport", "取消导入")
                          : activeBatchImportCheckQuota
                            ? t("codex.batchImport.cancelScan", "取消扫描")
                            : t("codex.batchImport.cancelParse", "取消解析")
                        : t("common.shared.close", "关闭")}
                    </button>
                    {!batchImportBusy &&
                      batchImportPreview?.status === "cancelled" && (
                        <button
                          className="btn btn-secondary"
                          onClick={() => void handleResumeBatchImport()}
                        >
                          <RefreshCw size={16} />
                          {activeBatchImportCheckQuota
                            ? t("codex.batchImport.resumeScan", "继续扫描")
                            : t("codex.batchImport.resumeParse", "继续解析")}
                        </button>
                      )}
                    {!batchImportBusy && (
                      <button
                        className="btn btn-primary"
                        onClick={() => void handleConfirmBatchImport()}
                        disabled={
                          !batchImportPreview ||
                          batchImportSelectedSelectableCount === 0
                        }
                      >
                        <Download size={16} />
                        {activeBatchImportCheckQuota
                          ? t(
                              "codex.batchImport.importChecked",
                              "导入已检测账号",
                            )
                          : t(
                              "codex.batchImport.directImport",
                              "不检测，直接导入",
                            )}
                        {batchImportSelectedSelectableCount > 0
                          ? ` (${batchImportSelectedSelectableCount})`
                          : ""}
                      </button>
                    )}
                    {!batchImportBusy && (
                      <button
                        className="btn btn-success"
                        onClick={() =>
                          void handleConfirmBatchImport({
                            addToApiService: true,
                          })
                        }
                        disabled={
                          localAccessSaving ||
                          !batchImportPreview ||
                          batchImportSelectedSelectableCount === 0
                        }
                      >
                        {localAccessSaving ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Database size={16} />
                        )}
                        {activeBatchImportCheckQuota
                          ? t(
                              "codex.batchImport.importCheckedAndAddToApiService",
                              "导入已检测账号并添加到 API 服务",
                            )
                          : t(
                              "codex.batchImport.directImportAndAddToApiService",
                              "直接导入并添加到 API 服务",
                            )}
                        {batchImportSelectedSelectableCount > 0
                          ? ` (${batchImportSelectedSelectableCount})`
                          : ""}
                      </button>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>,
          document.body,
      )}

      {externalImportProgress.visible && (
        <div
          className="modal-overlay codex-external-import-overlay"
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
              {externalImportSyncError && (
                <div className="codex-import-api-service-error" role="alert">
                  <CircleAlert size={16} />
                  <span>
                    {t(
                      "codex.importApiService.syncFailed",
                      "账号已导入，但加入 API 服务失败：{{error}}",
                    ).replace("{{error}}", externalImportSyncError)}
                  </span>
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
      {renderApiKeyUsageDetailModal()}
      {renderQuotaErrorDetailModal()}

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
                    <span className={`codex-batch-delete-job__status ${batchDeleteJob.status}`}>
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
                        {t("codex.batchImport.hiddenTask", "Codex 批量导入进行中")}
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
                            : t("codex.batchImport.preparing", "正在准备导入任务...")}
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

          {showAddModal && createPortal(
            <div className="modal-overlay">
              <div
                className="modal-content codex-add-modal codex-account-add-modal"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t("codex.addModal.title", "添加 Codex 账号")}</h2>
                  <button
                    className="modal-close"
                    onClick={closeCodexAddModal}
                    disabled={importing}
                    aria-label={t("common.close", "关闭")}
                  >
                    <X />
                  </button>
                </div>
                <div className="modal-tabs">
                  <button
                    className={`modal-tab ${addTab === "oauth" ? "active" : ""}`}
                    onClick={() => openCodexAddModal("oauth")}
                    disabled={importing}
                  >
                    <Globe size={14} />
                    <span className="modal-tab-label">
                      {t("common.shared.addModal.oauth", "OAuth Authorization")}
                    </span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === "token" ? "active" : ""}`}
                    onClick={() => openCodexAddModal("token")}
                    disabled={importing}
                  >
                    <FileText size={14} />
                    <span className="modal-tab-label">
                      {t("common.shared.addModal.token", "Token / JSON")}
                    </span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === "apikey" ? "active" : ""}`}
                    onClick={() => openCodexAddModal("apikey")}
                    disabled={importing}
                  >
                    <KeyRound size={14} />
                    <span className="modal-tab-label">
                      {t("codex.addModal.token", "API Key")}
                    </span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === "import" ? "active" : ""}`}
                    onClick={() => openCodexAddModal("import")}
                    disabled={importing}
                  >
                    <Database size={14} />
                    <span className="modal-tab-label">
                      {t("accounts.tabs.import", "本地导入")}
                    </span>
                  </button>
                </div>
                <div className="modal-body">
                  {codexAddTargetGroup && !reauthTargetAccount && (
                    <div className="codex-add-target-group-hint">
                      <FolderPlus size={14} />
                      <span>
                        {t("codex.addModal.targetGroup", {
                          defaultValue: "将添加到分组：{{group}}",
                          group: codexAddTargetGroup.name,
                        })}
                      </span>
                    </div>
                  )}
                  {addTab !== "oauth" && <MfaQuickCodeSelect />}
                  {addTab === "oauth" && (
                    <div className="add-section">
                      {reauthTargetEmail && (
                        <div className="oauth-link codex-reauth-email-block">
                          <label>
                            {t(
                              "codex.oauth.reauthEmailLabel",
                              "本次重新授权账号",
                            )}
                          </label>
                          <div className="oauth-url-box">
                            <input
                              type="text"
                              value={reauthTargetEmail}
                              readOnly
                              aria-label={t(
                                "codex.oauth.reauthEmailLabel",
                                "本次重新授权账号",
                              )}
                            />
                            <button
                              type="button"
                              onClick={() => void handleCopyReauthEmail()}
                              title={
                                reauthEmailCopied
                                  ? t("common.copied", "已复制")
                                  : t("common.copy", "复制")
                              }
                              aria-label={
                                reauthEmailCopied
                                  ? t("common.copied", "已复制")
                                  : t("common.copy", "复制")
                              }
                            >
                              {reauthEmailCopied ? (
                                <Check size={16} />
                              ) : (
                                <Copy size={16} />
                              )}
                            </button>
                          </div>
                        </div>
                      )}
                      {reauthTargetAccount && (
                        <div className="codex-reauth-note-summary">
                          {renderAccountNoteButton(reauthTargetAccount)}
                        </div>
                      )}
                      {shouldShowPendingOAuthDraftForm && (
                        <div className="codex-pending-oauth-draft">
                          <div className="oauth-link">
                            <label>
                              {t(
                                "codex.pendingAuth.emailLabel",
                                "待授权账号",
                              )}
                            </label>
                            <div className="oauth-url-box oauth-manual-input">
                              <input
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
                                disabled={savingPendingOAuthAccount}
                              />
                            </div>
                            {pendingOAuthFieldErrors.email && (
                              <span className="codex-account-note-field-error">
                                {pendingOAuthFieldErrors.email}
                              </span>
                            )}
                            {pendingOAuthFieldErrors.twoFactorSecret && (
                              <span className="codex-account-note-field-error">
                                {pendingOAuthFieldErrors.twoFactorSecret}
                              </span>
                            )}
                          </div>
                          <button
                            type="button"
                            className={`codex-account-note-chip ${pendingOAuthHasNoteDetails ? "has-note" : "empty-note"}`}
                            onClick={openPendingOAuthNoteModal}
                            disabled={savingPendingOAuthAccount}
                          >
                            <FileText size={12} />
                            <span>
                              {pendingOAuthHasNoteDetails
                                ? t("codex.accountNote.short", "账号备注")
                                : t("codex.accountNote.addShort", "加备注")}
                            </span>
                          </button>
                          <button
                            type="button"
                            className="btn btn-secondary btn-full"
                            onClick={() => void handleSavePendingOAuthAccount()}
                            disabled={
                              savingPendingOAuthAccount ||
                              !pendingOAuthEmailInput.trim()
                            }
                          >
                            {savingPendingOAuthAccount ? (
                              <RefreshCw size={16} className="loading-spinner" />
                            ) : (
                              <FileText size={16} />
                            )}
                            {t(
                              "codex.pendingAuth.saveDraft",
                              "保存待授权卡片",
                            )}
                          </button>
                        </div>
                      )}
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
                          {!isOauthTimeoutState && isMacOS && (
                            <button
                              type="button"
                              className="btn btn-secondary btn-full"
                              onClick={() =>
                                void handleOpenOauthIncognitoWindow()
                              }
                            >
                              <ShieldCheck size={16} />
                              {t(
                                "common.shared.oauth.incognitoWindow",
                                "无痕窗口",
                              )}
                            </button>
                          )}
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
                          {sponsorApiProviderTemplates.map((template) => (
                            <button
                              key={template.id}
                              className={`api-provider-chip sponsor ${apiProviderPresetId === template.id ? "active" : ""}`}
                              onClick={() =>
                                handleSelectApiProviderPreset(template.id)
                              }
                              type="button"
                            >
                              <span>{template.name}</span>
                              <Star
                                size={12}
                                className="api-provider-chip-badge"
                              />
                            </button>
                          ))}
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
                      {selectedSponsorApiProviderTemplate && (
                        <div className="api-provider-hint-block sponsor">
                          <p className="api-provider-hint">
                            {t(
                              "codex.modelProviders.sponsorHint",
                              "已按专属中转站配置自动填写兼容服务地址。输入 API Key 后，卡片会自动查询余额和用量。",
                            )}
                          </p>
                          <div className="api-provider-links">
                            {selectedSponsorApiProviderTemplate.website && (
                              <button
                                className="btn btn-secondary"
                                onClick={() =>
                                  void handleOpenProviderLink(
                                    selectedSponsorApiProviderTemplate.website,
                                  )
                                }
                              >
                                <ExternalLink size={14} />
                                {t("codex.api.provider.website", "官网")}
                              </button>
                            )}
                            {selectedSponsorApiProviderTemplate.apiKeyUrl && (
                              <button
                                className="btn btn-secondary"
                                onClick={() =>
                                  void handleOpenProviderLink(
                                    selectedSponsorApiProviderTemplate.apiKeyUrl,
                                  )
                                }
                              >
                                <KeyRound size={14} />
                                {t(
                                  "codex.api.provider.apiKeyPage",
                                  "API Key 页面",
                                )}
                              </button>
                            )}
                          </div>
                        </div>
                      )}
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
                                    onClick={() =>
                                      handleApiBaseUrlInputChange(baseUrl)
                                    }
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
                            onChange={(e) =>
                              handleApiKeyInputChange(e.target.value)
                            }
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
                            onChange={(e) =>
                              handleApiBaseUrlInputChange(e.target.value)
                            }
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
                      {apiProviderPresetId !== OPENAI_OFFICIAL_PRESET_ID && (
                        <>
                          <div className="api-model-catalog-panel">
                            <div className="api-model-catalog-header">
                              <label htmlFor="codex-api-model-catalog-add">
                                {t("codex.api.modelCatalog.label", "模型列表")}
                              </label>
                              <span className="api-model-catalog-count">
                                {t("codex.api.modelCatalog.count", {
                                  defaultValue: "{{count}} 个模型",
                                  count: apiModelCatalogDraft.length,
                                })}
                              </span>
                            </div>
                            <textarea
                              id="codex-api-model-catalog-add"
                              className="form-input api-model-catalog-input"
                              rows={6}
                              value={apiModelCatalogInput}
                              onChange={(event) => {
                                setApiModelCatalogInput(event.target.value);
                                setApiModelCatalogError(null);
                              }}
                              placeholder={t(
                                "codex.api.modelCatalog.placeholder",
                                "每行填写一个模型 ID，也可以使用逗号分隔。",
                              )}
                              disabled={addStatus === "loading"}
                              aria-describedby="codex-api-model-catalog-add-hint"
                            />
                            <div className="api-model-catalog-toolbar">
                              <p
                                id="codex-api-model-catalog-add-hint"
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
                                  void handleFetchApiModelCatalog()
                                }
                                disabled={
                                  apiModelCatalogFetching ||
                                  addStatus === "loading" ||
                                  !apiKeyInput.trim()
                                }
                              >
                                <RefreshCw
                                  size={14}
                                  className={
                                    apiModelCatalogFetching
                                      ? "loading-spinner"
                                      : undefined
                                  }
                                />
                                {apiModelCatalogFetching
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
                            {apiModelCatalogError && (
                              <div
                                className="add-status error api-model-catalog-error"
                              >
                                <CircleAlert size={16} />
                                <span>{apiModelCatalogError}</span>
                              </div>
                            )}
                          </div>
                          {apiModelCatalogSyncAvailable && (
                            <label
                              className="codex-import-api-service-toggle api-model-catalog-sync-toggle"
                            >
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
                                checked={apiSyncModelCatalogToCodex}
                                disabled={addStatus === "loading"}
                                onChange={(event) => {
                                  setApiSyncModelCatalogToCodex(
                                    event.target.checked,
                                  );
                                  setApiModelCatalogError(null);
                                }}
                              />
                              <span className="codex-import-api-service-switch" />
                            </label>
                          )}
                        </>
                      )}
                      <div className="api-key-add-actions">
                        <button
                          className="btn btn-primary"
                          onClick={() => void handleApiKeyLogin()}
                          disabled={
                            importing ||
                            addStatus === "loading" ||
                            apiModelCatalogFetching ||
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
                          "粘贴 auth.json、账号 JSON、Sub2API JSON、accessToken、个人访问令牌 at-… 或 refresh_token。",
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
                              "支持 session JSON、完整 tokens（id_token + access_token）、Sub2API 导出 JSON、仅 accessToken、个人访问令牌 at-… / personal_access_token，或仅 refresh_token。仅 refresh_token 会先联网换取完整凭据；无 refresh 的 at-… 按 personal_access_token 形态落盘。",
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
                                "session / accessToken / at- / refresh_token 示例",
                              )}
                            </div>
                            <pre className="token-format-code">
                              {CODEX_TOKEN_SESSION_EXAMPLE}
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
                        disabled={importing}
                        placeholder={t(
                          "codex.token.placeholder",
                          '示例：session JSON、accessToken、at-… 个人访问令牌、Sub2API JSON，或 {"personal_access_token":"at-..."}',
                        )}
                      />
                      <label className="codex-import-api-service-toggle">
                        <span className="codex-import-api-service-toggle-copy">
                          <strong>
                            {t(
                              "codex.importApiService.toggle",
                              "同步加入 API 服务",
                            )}
                          </strong>
                          <small>
                            {t(
                              "codex.importApiService.description",
                              "导入成功后，将符合条件的账号加入 API 服务账号池。",
                            )}
                          </small>
                        </span>
                        <input
                          type="checkbox"
                          checked={syncImportedToApiService}
                          disabled={importing}
                          onChange={(event) =>
                            handleSyncImportedToApiServiceChange(
                              event.target.checked,
                            )
                          }
                        />
                        <span className="codex-import-api-service-switch" />
                      </label>
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
                        {tokenImportProgress
                          ? `${tokenImportProgress.current}/${tokenImportProgress.total}`
                          : t("common.shared.token.import", "Import")}
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
                      <label className="codex-import-api-service-toggle">
                        <span className="codex-import-api-service-toggle-copy">
                          <strong>
                            {t(
                              "codex.importApiService.toggle",
                              "同步加入 API 服务",
                            )}
                          </strong>
                          <small>
                            {t(
                              "codex.importApiService.description",
                              "导入成功后，将符合条件的账号加入 API 服务账号池。",
                            )}
                          </small>
                        </span>
                        <input
                          type="checkbox"
                          checked={syncImportedToApiService}
                          disabled={importing}
                          onChange={(event) =>
                            handleSyncImportedToApiServiceChange(
                              event.target.checked,
                            )
                          }
                        />
                        <span className="codex-import-api-service-switch" />
                      </label>
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
            </div>,
            document.body,
          )}

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
                            onPageSizeChange={oauthBindingPagination.setPageSize}
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

          {oauthBindingQuotaReserveEditorOpen &&
            isLocalAccessOAuthBinding && (
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
                              {
                                oauthBindingQuotaReserveFieldErrors.hourlyPercent
                              }
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
                              {
                                oauthBindingQuotaReserveFieldErrors.weeklyPercent
                              }
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
            <div
              className="modal-overlay"
            >
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
                            handleEditingApiKeyCredentialsChange(
                              e.target.value,
                            )
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
                            <div
                              className="add-status error api-model-catalog-error"
                            >
                              <CircleAlert size={16} />
                              <span>{editingApiModelCatalogError}</span>
                            </div>
                          )}
                        </div>
                        {editingApiModelCatalogSyncAvailable && (
                          <label
                            className="codex-import-api-service-toggle api-model-catalog-sync-toggle"
                          >
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
            <div
              className="modal-overlay"
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
            <div
              className="modal-overlay codex-local-access-stats-overlay"
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
                            {item.windows.map((window) => (
                              <span key={window.key}>
                                <b>
                                  {formatCodexQuotaPoolWindowLabel(
                                    window.label,
                                    localAccessQuotaPoolLabels.weekly,
                                  )}
                                </b>
                                <strong>
                                  {formatCodexQuotaPoolPercent(window.percentage)}
                                </strong>
                              </span>
                            ))}
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
                          key={credit.id || `${credit.status || "credit"}-${index}`}
                        >
                          <span
                            className={`codex-reset-credit-confirm-detail-status ${getResetCreditStatusTone(credit)}`}
                          >
                            {getResetCreditStatusLabel(credit)}
                          </span>
                          <span>
                            {t("codex.quota.resetCreditGrantedAt", "发放")}
                            ：
                            <strong>
                              {formatResetCreditAbsoluteTime(credit.granted_at)}
                            </strong>
                          </span>
                          <span>
                            {t("codex.quota.resetCreditExpiresAt", "到期")}
                            ：
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
                      t(
                        "codex.quota.resetCreditDialogAction",
                        "重置使用次数",
                      )
                    )}
                  </button>
                </div>
              </div>
            </div>
          )}

          {deleteConfirm && (
            <div
              className="modal-overlay"
            >
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
            <div
              className="modal-overlay"
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
            resetKey={showTagModal}
            initialTags={
              accounts.find((a) => a.id === showTagModal)?.tags || []
            }
            availableTags={availableTags}
            onClose={() => setShowTagModal(null)}
            onSave={handleSaveTags}
          />

          {activeAccountNoteMode && createPortal(
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
                        type={accountNotePasswordVisible ? "text" : "password"}
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
                      {t("codex.accountNote.twoFactorSecretLabel", "2FA 秘钥")}
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
                        disabled={activeAccountNoteSaving || savedMfaRecords.length === 0}
                        aria-label={t("mfaQuick.selectLabel", "选择 2FA 秘钥")}
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
                    {accountNoteMfaPickerOpen && savedMfaRecords.length > 0 ? (
                      <div
                        className="codex-account-note-mfa-picker"
                        role="listbox"
                        aria-label={t("mfaQuick.selectLabel", "选择 2FA 秘钥")}
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
                                {remark ? <em title={remark}>{remark}</em> : null}
                              </span>
                              <span className="codex-account-note-mfa-option__side">
                                {isSelected ? <Check size={14} /> : null}
                                {token || formatMfaSecretPreview(record.secret)}
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
                    <span>{t("codex.accountNote.mailUrlLabel", "邮件地址")}</span>
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
                        aria-label={t("codex.accountNote.mailPreviewRefresh", "刷新邮件")}
                        title={t("codex.accountNote.mailPreviewRefresh", "刷新邮件")}
                      >
                        <RefreshCw
                          size={14}
                          className={
                            accountNoteMailPreviewLoading ? "loading-spinner" : ""
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
                        aria-label={t("codex.accountNote.mailPreviewOpen", "浏览器查看")}
                        title={t("codex.accountNote.mailPreviewOpen", "浏览器查看")}
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
                        {t("codex.accountNote.mailPreviewLoading", "读取邮件中...")}
                      </div>
                    ) : accountNoteMailPreviewError ? (
                      <span className="codex-account-note-field-error">
                        {accountNoteMailPreviewError}
                      </span>
                    ) : accountNoteMailPreview ? (
                      <div
                        key={`${accountNoteMailPreview.code}-${accountNoteMailPreview.fetchedAt}`}
                        className={`codex-account-note-mail-preview ${
                          accountNoteMailPreview.status === "changed" ? "is-changed" : ""
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
                            ? t("codex.accountNote.mailPreviewStatusChanged", {
                                defaultValue: "新验证码 · {{time}}",
                                time: formatCodexAccountNoteMailPreviewTime(
                                  accountNoteMailPreview.fetchedAt,
                                ),
                              })
                            : accountNoteMailPreview.status === "unchanged"
                              ? t("codex.accountNote.mailPreviewStatusUnchanged", {
                                  defaultValue: "未变化 · {{time}}",
                                  time: formatCodexAccountNoteMailPreviewTime(
                                    accountNoteMailPreview.fetchedAt,
                                  ),
                                })
                              : t("codex.accountNote.mailPreviewStatusInitial", {
                                  defaultValue: "获取于 {{time}}",
                                  time: formatCodexAccountNoteMailPreviewTime(
                                    accountNoteMailPreview.fetchedAt,
                                  ),
                                })}
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
            }) =>
              handleSaveLocalAccessAccounts(accountIds, {
                restrictFreeAccounts,
                backupAccountIds,
                preferredAccountIds,
                sessionAffinity,
                sessionAffinityTtlMs,
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

      {cliLaunchModal && (
        <CodexCliLaunchDialog
          subjectLabel={t("instances.columns.account", "账号")}
          subjectValue={cliLaunchModal.accountLabel}
          workingDir={{
            value: cliLaunchModal.workingDir,
            error: cliLaunchModal.workingDirError,
            onChange: updateCodexCliWorkingDir,
            onBlur: () => {
              if (
                cliLaunchModal.workingDir.trim() &&
                !cliLaunchModal.instanceId &&
                !cliLaunchModal.preparing &&
                !cliLaunchModal.executing
              ) {
                void prepareCodexCliLaunch(cliLaunchModal);
              }
            },
            onChoose: () => void handleChooseCodexCliWorkingDir(),
          }}
          terminal={selectedTerminal}
          terminalOptions={terminalOptions}
          onTerminalChange={setSelectedTerminal}
          command={cliLaunchModal.terminalCommand}
          commandPlaceholder={
            cliLaunchModal.preparing
              ? t("common.loading", "加载中...")
              : t("codex.cli.selectWorkingDir", "选择 Codex CLI 工作目录")
          }
          preparing={cliLaunchModal.preparing}
          copied={cliLaunchModal.copied}
          executing={cliLaunchModal.executing}
          successMessage={cliLaunchModal.executeMessage}
          errorMessage={cliLaunchModal.executeError}
          onClose={closeCliLaunchModal}
          onCopy={() => void handleCopyCodexCliCommand()}
          onExecute={() => void handleExecuteCodexCli()}
          showCancelButton
        />
      )}

      {activeTab === "instances" && (
        <CodexInstancesContent
          accountsForSelect={sortedAccountsForInstances}
        />
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

      {activeTab !== "wakeup" && fullQuotaWakeupOpenRequest && (
        <CodexWakeupContent
          accounts={accounts}
          openTestRequest={fullQuotaWakeupOpenRequest}
          modalOnly
          onRefreshAccounts={async () => {
            await fetchAccounts();
            await fetchCurrentAccount();
          }}
        />
      )}
    </div>
  );
}
