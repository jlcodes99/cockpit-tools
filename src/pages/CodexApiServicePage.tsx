import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Activity,
  BadgeDollarSign,
  ChevronDown,
  Check,
  CircleAlert,
  Copy,
  Eye,
  EyeOff,
  FolderPlus,
  Image,
  KeyRound,
  Pin,
  PinOff,
  Play,
  Plus,
  Power,
  RefreshCw,
  Route,
  Send,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
  Undo2,
  Users,
  Wrench,
  X,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { CodexIcon } from "../components/icons/CodexIcon";
import { ManualHelpIconButton } from "../components/ManualHelpIconButton";
import { PlatformGroupSwitcher } from "../components/platform/PlatformGroupSwitcher";
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from "../stores/usePlatformLayoutStore";
import { getPlatformLabel } from "../utils/platformMeta";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import {
  isCodexApiKeyScopeAccountActive,
  reconcileCodexApiKeyScopeAccountIds,
  selectCodexApiKeyScopeAccounts,
} from "../utils/codexApiKeyAccountScope";
import { resolveCodexApiServiceCompatibilityBaseUrls } from "../utils/codexApiServiceCompatibility";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import * as codexInstanceService from "../services/codexInstanceService";
import {
  getCodexAccountGroups,
  type CodexAccountGroup,
} from "../services/codexAccountGroupService";
import type { CodexAccount } from "../types/codex";
import {
  CODEX_API_SERVICE_BIND_ID,
  type InstanceProfile,
} from "../types/instance";
import type {
  CodexLocalAccessAddressKind,
  CodexLocalAccessAccountModelRule,
  CodexLocalAccessApiKey,
  CodexLocalAccessChatMessage,
  CodexLocalAccessChatStreamEvent,
  CodexLocalAccessClientBaseUrlHost,
  CodexLocalAccessCollection,
  CodexLocalAccessCustomRoutingRule,
  CodexLocalAccessGatewayMode,
  CodexLocalAccessModelAlias,
  CodexLocalAccessModelPricing,
  CodexLocalAccessRequestKind,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessScope,
  CodexLocalAccessState,
  CodexLocalAccessStatsWindow,
  CodexLocalAccessTimeoutPreset,
  CodexLocalAccessTimeouts,
  CodexLocalAccessUsageStats,
  CodexLocalAccessUsageEventPage,
} from "../types/codexLocalAccess";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import {
  formatCodexQuotaPoolPercent,
  formatCodexQuotaPoolWindowLabel,
  summarizeCodexQuotaPool,
} from "../utils/codexQuotaPool";
import { filterCodexLocalAccessAccountIds } from "../utils/codexLocalAccessAccounts";
import {
  isCodexLocalAccessRiskNoticeDismissed,
} from "../utils/codexLocalAccessRiskNotice";
import { requestCodexOpenAddAccount } from "../utils/codexAddAccountRequest";
import { scrollElementTo } from "../utils/reducedMotion";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { CodexLocalAccessModal } from "../components/CodexLocalAccessModal";
import { CodexAccountPoolHealthModal } from "../components/CodexAccountPoolHealthModal";
import { CodexStatsRangePicker } from "../components/CodexStatsRangePicker";
import { PaginationControls } from "../components/PaginationControls";
import { useCodexAccountOverviewMemberView } from "../hooks/useCodexAccountOverviewMemberView";
import {
  buildCodexStatsTimeRange,
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "../utils/codexStatsRange";
import "./CodexApiServicePage.css";

type ServiceTab = "overview" | "keys" | "accounts" | "models" | "logs";
type StatsLogTab = "accounts" | "logs" | "models" | "keys";
type CopyField =
  | "baseUrl"
  | "lanBaseUrl"
  | "apiKey"
  | "modelId"
  | `compat:${string}`
  | `apiKey:${string}`;
type RequestLogKindFilter = "all" | CodexLocalAccessRequestKind;
type RequestLogStatusFilter = "all" | "success" | "failed";
type RequestLogGatewayModeFilter = "all" | CodexLocalAccessGatewayMode;
type BuiltinTimeoutPresetId = "long_wait" | "short_wait";
type TimeoutPresetId = BuiltinTimeoutPresetId | string;

interface ApiKeyPolicyDraft {
  modelPrefix: string;
  allowedModels: string;
  excludedModels: string;
  inheritAccountPool: boolean;
  accountIds: string[];
}

interface TestChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  latencyMs?: number | null;
  failureTitle?: string;
  failureDetail?: string;
}

interface ModelPricingRow
  extends Omit<
    CodexLocalAccessModelPricing,
    "inputUsdPerMillion" | "outputUsdPerMillion"
  > {
  inputUsdPerMillion: number | null;
  outputUsdPerMillion: number | null;
  hasPreset: boolean;
  custom: boolean;
}

interface ModelPricingDraft {
  modelId: string;
  longContextThresholdTokens: string;
  inputUsdPerMillion: string;
  cachedInputUsdPerMillion: string;
  outputUsdPerMillion: string;
  standardLongInputUsdPerMillion: string;
  standardLongCachedInputUsdPerMillion: string;
  standardLongOutputUsdPerMillion: string;
  priorityInputUsdPerMillion: string;
  priorityCachedInputUsdPerMillion: string;
  priorityOutputUsdPerMillion: string;
  hasPreset: boolean;
  custom: boolean;
}

type ModelPricingRepricePhase =
  | "started"
  | "running"
  | "completed"
  | "failed"
  | "superseded";

interface ModelPricingRepriceProgress {
  jobId: number;
  phase: ModelPricingRepricePhase;
  total: number;
  processed: number;
  updated: number;
  modelIds: string[];
  message?: string;
}

const ADDRESS_KIND_STORAGE_KEY = "agtools.codex.local_access.address_kind.v1";
const STATS_RANGE_STORAGE_KEY = "agtools.codex.api_service.stats_range.v1";
const REQUEST_LOG_PAGE_SIZE_STORAGE_KEY =
  "agtools.codex.api_service.request_log_page_size.v1";
const REQUEST_LOG_PAGE_SIZE_OPTIONS = [20, 50, 100] as const;
const FALLBACK_BASE_URL = "http://127.0.0.1:1455/v1";

function normalizeAddressKind(
  value: string | null | undefined,
): CodexLocalAccessAddressKind {
  return value === "lan" ? "lan" : "local";
}

function readStoredAddressKind(): CodexLocalAccessAddressKind {
  try {
    return normalizeAddressKind(localStorage.getItem(ADDRESS_KIND_STORAGE_KEY));
  } catch {
    return "local";
  }
}

function persistAddressKind(value: CodexLocalAccessAddressKind): void {
  try {
    localStorage.setItem(ADDRESS_KIND_STORAGE_KEY, value);
  } catch {
    // ignore storage failures
  }
}

function normalizeStatsRange(value: string | null | undefined): CodexStatsRangeKey {
  if (value === "weekly" || value === "monthly") return value;
  return "daily";
}

function readStoredStatsRange(): CodexStatsRangeKey {
  try {
    return normalizeStatsRange(localStorage.getItem(STATS_RANGE_STORAGE_KEY));
  } catch {
    return "daily";
  }
}

function persistStatsRange(value: CodexStatsRangeKey): void {
  try {
    localStorage.setItem(STATS_RANGE_STORAGE_KEY, value);
  } catch {
    // ignore storage failures
  }
}

function normalizeRequestLogPageSize(value: number): number {
  return REQUEST_LOG_PAGE_SIZE_OPTIONS.includes(
    value as (typeof REQUEST_LOG_PAGE_SIZE_OPTIONS)[number],
  )
    ? value
    : REQUEST_LOG_PAGE_SIZE_OPTIONS[0];
}

function readStoredRequestLogPageSize(): number {
  try {
    const raw = localStorage.getItem(REQUEST_LOG_PAGE_SIZE_STORAGE_KEY);
    const parsed = raw
      ? Number.parseInt(raw, 10)
      : REQUEST_LOG_PAGE_SIZE_OPTIONS[0];
    return normalizeRequestLogPageSize(parsed);
  } catch {
    return REQUEST_LOG_PAGE_SIZE_OPTIONS[0];
  }
}

function persistRequestLogPageSize(value: number): void {
  try {
    localStorage.setItem(REQUEST_LOG_PAGE_SIZE_STORAGE_KEY, String(value));
  } catch {
    // ignore storage failures
  }
}

function formatCompactNumber(value: number): string {
  return new Intl.NumberFormat("en", {
    notation: value >= 1000 ? "compact" : "standard",
    maximumFractionDigits: value >= 1000 ? 1 : 0,
  }).format(value || 0);
}

function formatLatencyMs(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "--";
  if (value >= 1000) return `${(value / 1000).toFixed(2)}s`;
  return `${Math.round(value)}ms`;
}

function createTestChatMessage(
  role: TestChatMessage["role"],
  content: string,
  extra: Partial<Omit<TestChatMessage, "id" | "role" | "content">> = {},
): TestChatMessage {
  return {
    id: `${role}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    role,
    content,
    ...extra,
  };
}

function formatUsdCost(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "$0.00";
  if (value < 0.000001) return "<$0.000001";
  if (value < 0.01) return `$${value.toFixed(6)}`;
  if (value < 1) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(2)}`;
}

function formatPriceDraftValue(value: number | null | undefined): string {
  if (!Number.isFinite(value ?? NaN)) return "";
  return String(value);
}

function formatIntegerDraftValue(value: number | null | undefined): string {
  if (!Number.isFinite(value ?? NaN)) return "";
  return String(Math.trunc(value ?? 0));
}

function parsePriceDraftValue(
  value: string,
  allowEmpty: boolean,
): number | null {
  const trimmed = value.trim();
  if (!trimmed) return allowEmpty ? null : Number.NaN;
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed < 0) return Number.NaN;
  return parsed;
}

function parseOptionalPositiveIntegerDraft(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed) || parsed <= 0) return Number.NaN;
  return parsed;
}

function sameOptionalPrice(
  left: number | null | undefined,
  right: number | null | undefined,
): boolean {
  if (left == null && right == null) return true;
  if (left == null || right == null) return false;
  return Math.abs(left - right) < 0.0000001;
}

function modelPricingDraftFromRow(item: ModelPricingRow): ModelPricingDraft {
  return {
    modelId: item.modelId,
    longContextThresholdTokens: formatIntegerDraftValue(
      item.longContextThresholdTokens,
    ),
    inputUsdPerMillion: formatPriceDraftValue(item.inputUsdPerMillion),
    cachedInputUsdPerMillion: formatPriceDraftValue(
      item.cachedInputUsdPerMillion,
    ),
    outputUsdPerMillion: formatPriceDraftValue(item.outputUsdPerMillion),
    standardLongInputUsdPerMillion: formatPriceDraftValue(
      item.standardLongInputUsdPerMillion,
    ),
    standardLongCachedInputUsdPerMillion: formatPriceDraftValue(
      item.standardLongCachedInputUsdPerMillion,
    ),
    standardLongOutputUsdPerMillion: formatPriceDraftValue(
      item.standardLongOutputUsdPerMillion,
    ),
    priorityInputUsdPerMillion: formatPriceDraftValue(
      item.priorityInputUsdPerMillion,
    ),
    priorityCachedInputUsdPerMillion: formatPriceDraftValue(
      item.priorityCachedInputUsdPerMillion,
    ),
    priorityOutputUsdPerMillion: formatPriceDraftValue(
      item.priorityOutputUsdPerMillion,
    ),
    hasPreset: item.hasPreset,
    custom: item.custom,
  };
}

function formatDateTime(value: number | null | undefined): string {
  if (!value || !Number.isFinite(value) || value <= 0) return "--";
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

function cleanRequestLogErrorDetail(value?: string | null): string {
  return (value || "")
    .replace(/<[^>]*>/g, " ")
    .replace(/&nbsp;/gi, " ")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/gi, "&")
    .replace(/\s+/g, " ")
    .trim();
}

function truncateRequestLogErrorDetail(value: string): string {
  const maxLength = 160;
  if (value.length <= maxLength) return value;
  return `${value.slice(0, maxLength - 1).trimEnd()}...`;
}

function maskAccountText(value?: string | null): string {
  return value?.trim() || "-";
}

function parseModelRuleText(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/[\n,]/)
    .map((item) => item.trim())
    .filter(Boolean)
    .filter((item) => {
      const key = item.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

function serializeModelRules(values: string[] | null | undefined): string {
  return (values ?? []).join("\n");
}

function apiKeyInheritsAccountPool(apiKey: CodexLocalAccessApiKey): boolean {
  return (
    apiKey.inheritAccountPool ?? ((apiKey.accountIds?.length ?? 0) === 0)
  );
}

function apiKeyHasFixedAccountScope(
  apiKey: CodexLocalAccessApiKey,
  collection: CodexLocalAccessCollection | null,
): boolean {
  if (apiKey.providerGateway) return true;
  const accountIds = apiKey.accountIds ?? [];
  return Boolean(
    collection?.boundOauthAccountId &&
      collection.accountIds.length === 0 &&
      accountIds.length === 1 &&
      apiKey.id === `provider_gateway_${accountIds[0]}`,
  );
}

function apiKeyPolicyDraftFromValue(
  apiKey: CodexLocalAccessApiKey,
): ApiKeyPolicyDraft {
  return {
    modelPrefix: apiKey.modelPrefix ?? "",
    allowedModels: serializeModelRules(apiKey.allowedModels),
    excludedModels: serializeModelRules(apiKey.excludedModels),
    inheritAccountPool: apiKeyInheritsAccountPool(apiKey),
    accountIds: apiKey.accountIds ?? [],
  };
}

function sameStringList(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function apiKeyPolicyDraftIsDirty(
  apiKey: CodexLocalAccessApiKey,
  draft: ApiKeyPolicyDraft,
): boolean {
  const persisted = apiKeyPolicyDraftFromValue(apiKey);
  return (
    draft.modelPrefix !== persisted.modelPrefix ||
    draft.allowedModels !== persisted.allowedModels ||
    draft.excludedModels !== persisted.excludedModels ||
    draft.inheritAccountPool !== persisted.inheritAccountPool ||
    !sameStringList(draft.accountIds, persisted.accountIds)
  );
}

function toggleStringSelection(values: string[], value: string): string[] {
  return values.includes(value)
    ? values.filter((item) => item !== value)
    : [...values, value];
}

function parseModelAliasText(value: string): CodexLocalAccessModelAlias[] {
  const seen = new Set<string>();
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const fork = /\s(\+|fork)$/i.test(line);
      const cleaned = line.replace(/\s(\+|fork)$/i, "").trim();
      const parts = cleaned.includes("=>")
        ? cleaned.split("=>")
        : cleaned.split(/\s+as\s+/i);
      const sourceModel = parts[0]?.trim() ?? "";
      const alias = parts[1]?.trim() ?? "";
      if (!sourceModel || !alias) return null;
      const key = alias.toLowerCase();
      if (seen.has(key)) return null;
      seen.add(key);
      return { sourceModel, alias, fork };
    })
    .filter((item): item is CodexLocalAccessModelAlias => Boolean(item));
}

function serializeModelAliases(
  values: CodexLocalAccessModelAlias[] | null | undefined,
): string {
  return (values ?? [])
    .map(
      (item) => `${item.sourceModel} => ${item.alias}${item.fork ? " +" : ""}`,
    )
    .join("\n");
}

function formatSeconds(value: number | null | undefined): string {
  if (!Number.isFinite(value ?? NaN) || !value) return "0";
  return String(Math.round((value ?? 0) / 1000));
}

function parseIntegerDraft(
  value: string,
  min: number,
  max: number,
): number | null {
  const parsed = Number(value.trim());
  if (!Number.isInteger(parsed) || parsed < min || parsed > max) return null;
  return parsed;
}

function defaultCodexLocalAccessTimeouts(): CodexLocalAccessTimeouts {
  return {
    legacyRequestReadTimeoutMs: 60000,
    legacyUpstreamConnectTimeoutMs: 60000,
    legacyStreamIdleTimeoutMs: 120000,
    legacyStreamTotalTimeoutMs: 300000,
    sidecarStreamOpenTimeoutMs: 60000,
    sidecarStreamIdleTimeoutMs: 120000,
    sidecarImageStreamOpenTimeoutMs: 60000,
    sidecarImageStreamIdleTimeoutMs: 180000,
    sidecarStreamOpenMaxAttempts: 1,
    sidecarStreamKeepaliveSeconds: 15,
    websocketConnectTimeoutMs: 30000,
    websocketInitialMessageTimeoutMs: 30000,
    websocketIdleTimeoutMs: 300000,
    websocketHeartbeatIntervalMs: 30000,
    upstreamSendRetryAttempts: 3,
    upstreamSendRetryBaseDelayMs: 200,
    upstreamSendRetryMaxDelayMs: 1200,
    singleAccountStatusRetryAttempts: 2,
    singleAccountStatusRetryBaseDelayMs: 300,
    singleAccountStatusRetryMaxDelayMs: 1500,
    sidecarStreamingBootstrapRetries: 1,
  };
}

function shortWaitCodexLocalAccessTimeouts(): CodexLocalAccessTimeouts {
  return {
    ...defaultCodexLocalAccessTimeouts(),
    legacyRequestReadTimeoutMs: 15000,
    legacyUpstreamConnectTimeoutMs: 20000,
    legacyStreamIdleTimeoutMs: 60000,
    legacyStreamTotalTimeoutMs: 180000,
    sidecarStreamOpenTimeoutMs: 10000,
    sidecarStreamIdleTimeoutMs: 60000,
    sidecarImageStreamOpenTimeoutMs: 10000,
    sidecarImageStreamIdleTimeoutMs: 60000,
    sidecarStreamOpenMaxAttempts: 2,
    websocketConnectTimeoutMs: 30000,
    websocketInitialMessageTimeoutMs: 30000,
  };
}

function builtinTimeoutPresetValue(
  id: BuiltinTimeoutPresetId,
): CodexLocalAccessTimeouts {
  return id === "short_wait"
    ? shortWaitCodexLocalAccessTimeouts()
    : defaultCodexLocalAccessTimeouts();
}

function normalizeTimeouts(
  value?: CodexLocalAccessTimeouts | null,
): CodexLocalAccessTimeouts {
  return { ...defaultCodexLocalAccessTimeouts(), ...(value ?? {}) };
}

function timeoutDraftsFromValue(
  value?: CodexLocalAccessTimeouts | null,
): Record<keyof CodexLocalAccessTimeouts, string> {
  const timeouts = normalizeTimeouts(value);
  return {
    legacyRequestReadTimeoutMs: formatSeconds(
      timeouts.legacyRequestReadTimeoutMs,
    ),
    legacyUpstreamConnectTimeoutMs: formatSeconds(
      timeouts.legacyUpstreamConnectTimeoutMs,
    ),
    legacyStreamIdleTimeoutMs: formatSeconds(
      timeouts.legacyStreamIdleTimeoutMs,
    ),
    legacyStreamTotalTimeoutMs: formatSeconds(
      timeouts.legacyStreamTotalTimeoutMs,
    ),
    sidecarStreamOpenTimeoutMs: formatSeconds(
      timeouts.sidecarStreamOpenTimeoutMs,
    ),
    sidecarStreamIdleTimeoutMs: formatSeconds(
      timeouts.sidecarStreamIdleTimeoutMs,
    ),
    sidecarImageStreamOpenTimeoutMs: formatSeconds(
      timeouts.sidecarImageStreamOpenTimeoutMs,
    ),
    sidecarImageStreamIdleTimeoutMs: formatSeconds(
      timeouts.sidecarImageStreamIdleTimeoutMs,
    ),
    sidecarStreamOpenMaxAttempts: String(timeouts.sidecarStreamOpenMaxAttempts),
    sidecarStreamKeepaliveSeconds: String(
      timeouts.sidecarStreamKeepaliveSeconds,
    ),
    websocketConnectTimeoutMs: formatSeconds(
      timeouts.websocketConnectTimeoutMs,
    ),
    websocketInitialMessageTimeoutMs: formatSeconds(
      timeouts.websocketInitialMessageTimeoutMs,
    ),
    websocketIdleTimeoutMs: formatSeconds(timeouts.websocketIdleTimeoutMs),
    websocketHeartbeatIntervalMs: formatSeconds(
      timeouts.websocketHeartbeatIntervalMs,
    ),
    upstreamSendRetryAttempts: String(timeouts.upstreamSendRetryAttempts),
    upstreamSendRetryBaseDelayMs: String(timeouts.upstreamSendRetryBaseDelayMs),
    upstreamSendRetryMaxDelayMs: String(timeouts.upstreamSendRetryMaxDelayMs),
    singleAccountStatusRetryAttempts: String(
      timeouts.singleAccountStatusRetryAttempts,
    ),
    singleAccountStatusRetryBaseDelayMs: String(
      timeouts.singleAccountStatusRetryBaseDelayMs,
    ),
    singleAccountStatusRetryMaxDelayMs: String(
      timeouts.singleAccountStatusRetryMaxDelayMs,
    ),
    sidecarStreamingBootstrapRetries: String(
      timeouts.sidecarStreamingBootstrapRetries,
    ),
  };
}

function requestKindLabel(
  kind: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (kind === "image_generation") {
    return t("codex.localAccess.requestKind.imageGeneration", "生图");
  }
  if (kind === "image_edit") {
    return t("codex.localAccess.requestKind.imageEdit", "改图");
  }
  if (kind === "text") {
    return t("codex.localAccess.requestKind.text", "文本");
  }
  return t("codex.localAccess.requestKind.other", "其他");
}

function gatewayModeLabel(
  mode: CodexLocalAccessGatewayMode | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (mode === "legacy") {
    return t("codex.localAccess.gatewayModeOldLabel", "API 服务-旧");
  }
  if (mode === "sidecar") {
    return t("codex.localAccess.gatewayModeNewLabel", "API 服务-新");
  }
  return t("codex.apiService.logs.gatewayModeUnknown", "模式未知");
}

/** 与后端写入 x-cockpit-instance-id 一致：profile 目录 basename */
function clientInstanceIdFromUserDataDir(userDataDir: string): string {
  const normalized = userDataDir.trim().replace(/[/\\]+$/, "");
  if (!normalized) return "";
  const parts = normalized.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

function instanceDisplayName(
  instance: InstanceProfile,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (instance.isDefault) {
    return t("instances.defaultName", "默认实例");
  }
  const name = instance.name?.trim();
  return name || instance.id;
}

function resolveClientInstanceLabel(
  clientInstanceId: string | null | undefined,
  instances: InstanceProfile[],
  t: ReturnType<typeof useTranslation>["t"],
): string {
  const id = clientInstanceId?.trim() ?? "";
  if (!id) {
    return t("codex.apiService.logs.instanceUnknown", "实例 -");
  }
  const matched = instances.find((instance) => {
    const dirId = clientInstanceIdFromUserDataDir(instance.userDataDir || "");
    return dirId === id || instance.id === id;
  });
  if (matched) {
    return instanceDisplayName(matched, t);
  }
  return id;
}

export function CodexApiServicePage() {
  const { t } = useTranslation();
  const { platformGroups } = usePlatformLayoutStore();
  const {
    accounts,
    accountsLoaded,
    currentAccount,
    fetchAccounts,
    fetchCurrentAccount,
  } = useCodexAccountStore();
  const [state, setState] = useState<CodexLocalAccessState | null>(null);
  const [groups, setGroups] = useState<CodexAccountGroup[]>([]);
  const [activeTab, setActiveTab] = useState<ServiceTab>("overview");
  const [statsLogTab, setStatsLogTab] = useState<StatsLogTab>("logs");
  const [statsRange, setStatsRange] = useState<CodexStatsRangeKey>(() =>
    readStoredStatsRange(),
  );
  const [statsTimeRange, setStatsTimeRange] = useState<CodexStatsTimeRange>(() =>
    buildCodexStatsTimeRange(readStoredStatsRange()),
  );
  const [filteredStatsWindow, setFilteredStatsWindow] =
    useState<CodexLocalAccessStatsWindow | null>(null);
  const [statsRangeError, setStatsRangeError] = useState("");
  const [addressKind, setAddressKind] = useState<CodexLocalAccessAddressKind>(
    () => readStoredAddressKind(),
  );
  const [busy, setBusy] = useState(false);
  const [activating, setActivating] = useState(false);
  const [testDialogOpen, setTestDialogOpen] = useState(false);
  const [testDialogRunning, setTestDialogRunning] = useState(false);
  const [testChatMessages, setTestChatMessages] = useState<TestChatMessage[]>(
    [],
  );
  const [testChatInput, setTestChatInput] = useState("");
  const [testDialogError, setTestDialogError] = useState("");
  const [portKilling, setPortKilling] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [copiedField, setCopiedField] = useState<CopyField | null>(null);
  const [keyVisible, setKeyVisible] = useState(false);
  const [portInput, setPortInput] = useState("");
  const [proxyInput, setProxyInput] = useState("");
  const [selectedModelId, setSelectedModelId] = useState("");
  const [memberModalOpen, setMemberModalOpen] = useState(false);
  const [healthModalOpen, setHealthModalOpen] = useState(false);
  const [apiServiceIsCurrent, setApiServiceIsCurrent] = useState(false);
  const [apiKeyDrafts, setApiKeyDrafts] = useState<Record<string, string>>({});
  const [apiKeyPolicyDrafts, setApiKeyPolicyDrafts] = useState<
    Record<string, ApiKeyPolicyDraft>
  >({});
  const [expandedApiKeyPolicyIds, setExpandedApiKeyPolicyIds] = useState<
    Set<string>
  >(() => new Set());
  const [modelAliasesText, setModelAliasesText] = useState("");
  const [excludedModelsText, setExcludedModelsText] = useState("");
  const [accountModelRulesOpen, setAccountModelRulesOpen] = useState(false);
  const [accountModelRuleDrafts, setAccountModelRuleDrafts] = useState<
    Record<string, string>
  >({});
  const [accountModelRuleSelected, setAccountModelRuleSelected] = useState<
    Set<string>
  >(() => new Set());
  const [accountModelRuleBulkText, setAccountModelRuleBulkText] = useState("");
  const [pricingModalOpen, setPricingModalOpen] = useState(false);
  const [pricingDrafts, setPricingDrafts] = useState<ModelPricingDraft[]>([]);
  const [pricingError, setPricingError] = useState("");
  const [pricingRepriceProgress, setPricingRepriceProgress] =
    useState<ModelPricingRepriceProgress | null>(null);
  const [timeoutsModalOpen, setTimeoutsModalOpen] = useState(false);
  const [timeoutDrafts, setTimeoutDrafts] = useState<
    Record<keyof CodexLocalAccessTimeouts, string>
  >(() => timeoutDraftsFromValue());
  const [timeoutsError, setTimeoutsError] = useState("");
  const [selectedTimeoutPresetId, setSelectedTimeoutPresetId] =
    useState<TimeoutPresetId>("long_wait");
  const [timeoutPresetNameDraft, setTimeoutPresetNameDraft] = useState("");
  const [responsesWebsocketsEnabledDraft, setResponsesWebsocketsEnabledDraft] =
    useState(false);
  const [maxRetryCredentialsDraft, setMaxRetryCredentialsDraft] = useState("0");
  const [maxRetryIntervalDraft, setMaxRetryIntervalDraft] = useState("3");
  const [disableCoolingDraft, setDisableCoolingDraft] = useState(false);
  const [immediateSseResponseDraft, setImmediateSseResponseDraft] = useState(false);
  const [maxConcurrentImageRequestsDraft, setMaxConcurrentImageRequestsDraft] =
    useState("1");
  const [requestLogPage, setRequestLogPage] = useState(1);
  const [requestLogPageSize, setRequestLogPageSize] = useState(() =>
    readStoredRequestLogPageSize(),
  );
  const [requestLogResult, setRequestLogResult] =
    useState<CodexLocalAccessUsageEventPage | null>(null);
  const [requestLogLoading, setRequestLogLoading] = useState(false);
  const [requestLogError, setRequestLogError] = useState("");
  const [requestLogKindFilter, setRequestLogKindFilter] =
    useState<RequestLogKindFilter>("all");
  const [requestLogStatusFilter, setRequestLogStatusFilter] =
    useState<RequestLogStatusFilter>("all");
  const [requestLogGatewayModeFilter, setRequestLogGatewayModeFilter] =
    useState<RequestLogGatewayModeFilter>("all");
  const [requestLogModelQuery, setRequestLogModelQuery] = useState("");
  const [requestLogAccountQuery, setRequestLogAccountQuery] = useState("");
  const [requestLogApiKeyQuery, setRequestLogApiKeyQuery] = useState("");
  const [requestLogInstanceQuery, setRequestLogInstanceQuery] = useState("all");
  const [requestLogErrorQuery, setRequestLogErrorQuery] = useState("");
  const [codexInstances, setCodexInstances] = useState<InstanceProfile[]>([]);
  const mountedRef = useRef(true);
  const stateRequestSeqRef = useRef(0);
  const statsRequestSeqRef = useRef(0);
  const testChatScrollRef = useRef<HTMLDivElement | null>(null);

  const collection = state?.collection ?? null;
  const stats = state?.stats ?? null;
  const memberView = useCodexAccountOverviewMemberView({
    accounts,
    groups,
    currentAccountId: apiServiceIsCurrent ? null : (currentAccount?.id ?? null),
  });
  const builtinTimeoutPresets = useMemo(
    () => [
      {
        id: "long_wait" as const,
        name: t("codex.apiService.timeouts.longWaitPreset", "长等待方案"),
        timeouts: defaultCodexLocalAccessTimeouts(),
        builtin: true,
      },
      {
        id: "short_wait" as const,
        name: t("codex.apiService.timeouts.shortWaitPreset", "短等待方案"),
        timeouts: shortWaitCodexLocalAccessTimeouts(),
        builtin: true,
      },
    ],
    [t],
  );
  const timeoutPresetOptions = useMemo(
    () => [
      ...builtinTimeoutPresets,
      ...(collection?.timeoutPresets ?? []).map((preset) => ({
        ...preset,
        builtin: false,
      })),
    ],
    [builtinTimeoutPresets, collection?.timeoutPresets],
  );
  const selectedTimeoutPreset = timeoutPresetOptions.find(
    (preset) => preset.id === selectedTimeoutPresetId,
  );
  const selectedTimeoutPresetIsCustom = Boolean(
    selectedTimeoutPreset && !selectedTimeoutPreset.builtin,
  );
  const selectedStatsWindow =
    useMemo<CodexLocalAccessStatsWindow | null>(() => {
      if (filteredStatsWindow) return filteredStatsWindow;
      if (!stats || statsRange === "custom") return null;
      return stats[statsRange];
    }, [filteredStatsWindow, stats, statsRange]);
  const apiKeyStatsById = new Map(
    (selectedStatsWindow?.apiKeys ?? []).map((item) => [item.apiKeyId, item]),
  );
  const totals = selectedStatsWindow?.totals;
  const memberIds = collection?.accountIds ?? [];
  const localAccessAccounts = useMemo(() => accounts, [accounts]);
  const memberAccounts = useMemo(
    () =>
      memberIds
        .map((accountId) =>
          localAccessAccounts.find((account) => account.id === accountId),
        )
        .filter((account): account is CodexAccount => Boolean(account)),
    [memberIds, localAccessAccounts],
  );
  const memberAccountIds = useMemo(
    () => memberAccounts.map((account) => account.id),
    [memberAccounts],
  );
  const accountDisplayNames = useMemo(() => {
    const next = new Map<string, string>();
    localAccessAccounts.forEach((account) => {
      const displayName = buildCodexAccountPresentation(account, t).displayName;
      const accountId = account.id.trim();
      const email = account.email.trim();
      if (accountId) next.set(accountId, displayName);
      if (email) next.set(email, displayName);
    });
    return next;
  }, [localAccessAccounts, t]);
  const accountModelRuleCount = collection?.accountModelRules.length ?? 0;
  const accountModelRuleAllSelected =
    memberAccounts.length > 0 &&
    memberAccounts.every((account) => accountModelRuleSelected.has(account.id));
  const healthByAccountId = useMemo(() => {
    const next = new Map<
      string,
      NonNullable<CodexLocalAccessState["accountHealth"]>[number]
    >();
    state?.accountHealth.forEach((item) => next.set(item.accountId, item));
    return next;
  }, [state?.accountHealth]);
  const quotaPoolSummary = useMemo(
    () => summarizeCodexQuotaPool(memberAccounts),
    [memberAccounts],
  );
  const baseUrl = state?.baseUrl || FALLBACK_BASE_URL;
  const displayBaseUrl =
    addressKind === "lan" && state?.lanBaseUrl ? state.lanBaseUrl : baseUrl;
  const accessScope = collection?.accessScope ?? "localhost";
  const clientBaseUrlHost = collection?.clientBaseUrlHost ?? "localhost";
  const routingStrategy = collection?.routingStrategy ?? "auto";
  const gatewayMode = collection?.gatewayMode ?? "sidecar";
  const modelIds = state?.modelIds ?? [];
  const exampleModelId = modelIds[0] ?? "gpt-5.5";
  const exampleApiKey = collection?.apiKey || "<api-key>";
  const compatibilityBaseUrls = useMemo(
    () => resolveCodexApiServiceCompatibilityBaseUrls(displayBaseUrl),
    [displayBaseUrl],
  );
  const compatibilityExamples = useMemo(
    () => [
      {
        id: "openai",
        title: t(
          "codex.apiService.compat.openaiTitle",
          "OpenAI Compatible",
        ),
        endpoint: "/v1/chat/completions",
        note: t(
          "codex.apiService.compat.openaiNote",
          "Base URL uses /v1.",
        ),
        value: [
          `OPENAI_BASE_URL=${compatibilityBaseUrls.openai}`,
          `OPENAI_API_KEY=${exampleApiKey}`,
          `OPENAI_MODEL=${exampleModelId}`,
        ].join("\n"),
      },
      {
        id: "responses",
        title: t("codex.apiService.compat.responsesTitle", "Responses"),
        endpoint: "/v1/responses",
        note: t(
          "codex.apiService.compat.responsesNote",
          "Codex-native Responses entry.",
        ),
        value: [
          `OPENAI_BASE_URL=${compatibilityBaseUrls.openai}`,
          `OPENAI_API_KEY=${exampleApiKey}`,
          `OPENAI_MODEL=${exampleModelId}`,
          "OPENAI_API_ENDPOINT=/responses",
        ].join("\n"),
      },
      {
        id: "anthropic",
        title: t(
          "codex.apiService.compat.anthropicTitle",
          "Anthropic Messages",
        ),
        endpoint: "/v1/messages",
        note: t(
          "codex.apiService.compat.anthropicNote",
          "Use the same service key.",
        ),
        value: [
          `ANTHROPIC_BASE_URL=${compatibilityBaseUrls.root}`,
          `ANTHROPIC_API_KEY=${exampleApiKey}`,
          `ANTHROPIC_MODEL=${exampleModelId}`,
        ].join("\n"),
      },
      {
        id: "gemini",
        title: t("codex.apiService.compat.geminiTitle", "Gemini"),
        endpoint: "/v1beta/models",
        note: t(
          "codex.apiService.compat.geminiNote",
          "Base URL uses /v1beta.",
        ),
        value: [
          `GEMINI_BASE_URL=${compatibilityBaseUrls.gemini}`,
          `GEMINI_API_KEY=${exampleApiKey}`,
          `GEMINI_MODEL=${exampleModelId}`,
        ].join("\n"),
      },
      {
        id: "ollama",
        title: t("codex.apiService.compat.ollamaTitle", "Ollama Bridge"),
        endpoint: "/api/chat",
        note: t(
          "codex.apiService.compat.ollamaNote",
          "Use Authorization: Bearer.",
        ),
        value: [
          `OLLAMA_HOST=${compatibilityBaseUrls.root}`,
          `OLLAMA_API_KEY=${exampleApiKey}`,
          `OLLAMA_MODEL=${exampleModelId}`,
        ].join("\n"),
      },
    ],
    [compatibilityBaseUrls, exampleApiKey, exampleModelId, t],
  );
  const modelPricingRows = useMemo<ModelPricingRow[]>(() => {
    const presetMap = new Map<string, CodexLocalAccessModelPricing>();
    const customMap = new Map<string, CodexLocalAccessModelPricing>();
    (state?.modelPricingPresets ?? []).forEach((item) => {
      presetMap.set(item.modelId.toLowerCase(), item);
    });
    (collection?.modelPricings ?? []).forEach((item) => {
      customMap.set(item.modelId.toLowerCase(), item);
    });
    const modelOrder = new Map<string, number>();
    const ids: string[] = [];
    const pushId = (modelId: string) => {
      const trimmed = modelId.trim();
      const key = trimmed.toLowerCase();
      if (!trimmed || modelOrder.has(key)) return;
      modelOrder.set(key, ids.length);
      ids.push(trimmed);
    };
    (state?.modelPricingPresets ?? []).forEach((item) => pushId(item.modelId));
    modelIds.forEach(pushId);
    (collection?.modelPricings ?? []).forEach((item) => pushId(item.modelId));
    return ids.map((modelId) => {
      const key = modelId.toLowerCase();
      const preset = presetMap.get(key);
      const custom = customMap.get(key);
      const source = custom ?? preset;
      return {
        modelId: source?.modelId ?? modelId,
        longContextThresholdTokens:
          source?.longContextThresholdTokens ??
          preset?.longContextThresholdTokens ??
          null,
        inputUsdPerMillion: source?.inputUsdPerMillion ?? null,
        outputUsdPerMillion: source?.outputUsdPerMillion ?? null,
        cachedInputUsdPerMillion: source?.cachedInputUsdPerMillion ?? null,
        standardLongInputUsdPerMillion:
          source?.standardLongInputUsdPerMillion ?? null,
        standardLongOutputUsdPerMillion:
          source?.standardLongOutputUsdPerMillion ?? null,
        standardLongCachedInputUsdPerMillion:
          source?.standardLongCachedInputUsdPerMillion ?? null,
        priorityInputUsdPerMillion: source?.priorityInputUsdPerMillion ?? null,
        priorityOutputUsdPerMillion: source?.priorityOutputUsdPerMillion ?? null,
        priorityCachedInputUsdPerMillion:
          source?.priorityCachedInputUsdPerMillion ?? null,
        hasPreset: Boolean(preset),
        custom: Boolean(custom),
      };
    });
  }, [collection?.modelPricings, modelIds, state?.modelPricingPresets]);
  const pricingRepricePercent = useMemo(() => {
    if (!pricingRepriceProgress) return 0;
    if (pricingRepriceProgress.phase === "completed") return 100;
    if (pricingRepriceProgress.total <= 0) return 0;
    return Math.max(
      0,
      Math.min(
        100,
        Math.round(
          (pricingRepriceProgress.processed / pricingRepriceProgress.total) *
            100,
        ),
      ),
    );
  }, [pricingRepriceProgress]);
  const pricingRepriceStatusText = useMemo(() => {
    if (!pricingRepriceProgress) return "";
    const processed = Math.min(
      pricingRepriceProgress.processed,
      pricingRepriceProgress.total,
    );
    if (pricingRepriceProgress.phase === "completed") {
      return t("codex.apiService.models.pricingRepriceDone", {
        updated: formatCompactNumber(pricingRepriceProgress.updated),
        defaultValue: "历史估算价值已更新，变更 {{updated}} 条",
      });
    }
    if (pricingRepriceProgress.phase === "failed") {
      return t("codex.apiService.models.pricingRepriceFailedWithMessage", {
        message:
          pricingRepriceProgress.message ||
          t(
            "codex.apiService.models.pricingRepriceFailed",
            "历史估算价值更新失败",
          ),
        defaultValue: "历史估算价值更新失败：{{message}}",
      });
    }
    if (pricingRepriceProgress.phase === "superseded") {
      return t(
        "codex.apiService.models.pricingRepriceSuperseded",
        "已收到新的价格配置，正在切换到最新重算任务",
      );
    }
    return t("codex.apiService.models.pricingRepriceRunning", {
      processed: formatCompactNumber(processed),
      total: formatCompactNumber(pricingRepriceProgress.total),
      updated: formatCompactNumber(pricingRepriceProgress.updated),
      defaultValue:
        "正在更新历史估算价值 {{processed}} / {{total}}，已更新 {{updated}} 条",
    });
  }, [pricingRepriceProgress, t]);
  const pricingRepriceActive =
    pricingRepriceProgress?.phase === "started" ||
    pricingRepriceProgress?.phase === "running" ||
    pricingRepriceProgress?.phase === "superseded";
  const avgLatency =
    totals && totals.requestCount > 0
      ? totals.totalLatencyMs / totals.requestCount
      : 0;
  const successRate =
    totals && totals.requestCount > 0
      ? Math.round((totals.successCount / totals.requestCount) * 100)
      : 0;
  const formatRequestResultDetail = (
    usage?: CodexLocalAccessUsageStats | null,
  ) =>
    t("codex.localAccess.stats.requestsDetail", {
      success: formatCompactNumber(usage?.successCount ?? 0),
      failed: formatCompactNumber(
        Math.max(
          (usage?.failureCount ?? 0) -
            (usage?.clientCanceledCount ?? 0) -
            (usage?.upstreamResponseFailedCount ?? 0) -
            (usage?.streamIncompleteCount ?? 0),
          0,
        ),
      ),
      canceled: formatCompactNumber(usage?.clientCanceledCount ?? 0),
      upstreamFailed: formatCompactNumber(
        usage?.upstreamResponseFailedCount ?? 0,
      ),
      incomplete: formatCompactNumber(usage?.streamIncompleteCount ?? 0),
      defaultValue:
        "成功 {{success}} / 失败 {{failed}} / 取消 {{canceled}} / 上游失败 {{upstreamFailed}} / 流未完成 {{incomplete}}",
    });
  const formatAccountTokenUsage = (
    usage?: CodexLocalAccessUsageStats | null,
  ) => {
    const totalTokens = usage?.totalTokens ?? 0;
    if (totalTokens === 0) {
      return t("codex.localAccess.stats.accountTokens", {
        count: 0,
        defaultValue: "0 Tokens",
      });
    }
    return t("codex.localAccess.stats.accountTokensCompact", {
      value: formatCompactNumber(totalTokens),
      defaultValue: "{{value}}",
    });
  };
  const imageUnavailableCount =
    state?.accountHealth.filter(
      (item) => item.imageGenerationStatus === "unavailable",
    ).length ?? 0;
  const cooldownCount =
    state?.accountHealth.reduce(
      (sum, item) => sum + item.cooldowns.length,
      0,
    ) ?? 0;
  const availableAccountCount =
    state?.accountHealth.filter((item) => item.available).length ??
    memberAccounts.length;

  const currentPlatformId = "codex_api_service" as const;
  const currentGroup = useMemo(
    () => findGroupByPlatform(platformGroups, currentPlatformId),
    [platformGroups],
  );
  const switchOptions = useMemo(
    () =>
      (currentGroup ? currentGroup.platformIds : [currentPlatformId]).map(
        (platformId) => ({
          platformId,
          label: currentGroup
            ? resolveGroupChildName(
                currentGroup,
                platformId,
                getPlatformLabel(platformId, t),
              )
            : getPlatformLabel(platformId, t),
        }),
      ),
    [currentGroup, t],
  );

  const reloadState = useCallback(async () => {
    const requestSeq = ++stateRequestSeqRef.current;
    try {
      const nextState = await codexLocalAccessService.getCodexLocalAccessState();
      if (!mountedRef.current || requestSeq !== stateRequestSeqRef.current) {
        return null;
      }
      setState(nextState);
      setPortInput(
        nextState.collection?.port ? String(nextState.collection.port) : "",
      );
      setProxyInput(nextState.collection?.upstreamProxyUrl ?? "");
      return nextState;
    } catch (error) {
      if (!mountedRef.current || requestSeq !== stateRequestSeqRef.current) {
        return null;
      }
      throw error;
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void reloadState().catch((err) =>
      setError(String(err).replace(/^Error:\s*/, "")),
    );
    void fetchAccounts();
    void fetchCurrentAccount();
    void codexInstanceService
      .listInstances()
      .then((instances) => {
        const defaultInstance = instances.find((instance) => instance.isDefault);
        if (mountedRef.current) {
          setCodexInstances(instances);
          setApiServiceIsCurrent(
            defaultInstance?.bindAccountId === CODEX_API_SERVICE_BIND_ID,
          );
        }
      })
      .catch(() => {
        if (mountedRef.current) {
          setCodexInstances([]);
          setApiServiceIsCurrent(false);
        }
      });
    void getCodexAccountGroups()
      .then(setGroups)
      .catch(() => setGroups([]));
    const onUpdated = () => {
      void reloadState();
    };
    window.addEventListener("codex-local-access-state-updated", onUpdated);
    return () => {
      mountedRef.current = false;
      window.removeEventListener("codex-local-access-state-updated", onUpdated);
    };
  }, [fetchAccounts, fetchCurrentAccount, reloadState]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<ModelPricingRepriceProgress>(
      "codex-local-access-model-pricing-reprice",
      (event) => {
        if (disposed) return;
        const progress = event.payload;
        setPricingRepriceProgress((current) => {
          if (current && progress.jobId < current.jobId) {
            return current;
          }
          return progress;
        });
        if (progress.phase === "completed") {
          void reloadState();
          setPricingModalOpen(false);
          setPricingRepriceProgress(null);
          setNotice(
            t(
              "codex.apiService.models.pricingRepriceCompleted",
              "历史估算价值已更新",
            ),
          );
          return;
        }
        if (progress.phase === "failed") {
          const message =
            progress.message ||
            t(
              "codex.apiService.models.pricingRepriceFailed",
              "历史估算价值更新失败",
            );
          setPricingError(message);
          setError(message);
        }
      },
    )
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch((err) => {
        if (!disposed) {
          setError(String(err).replace(/^Error:\s*/, ""));
        }
      });
    return () => {
      disposed = true;
      if (unlisten) {
        unlisten();
      }
    };
  }, [reloadState, t]);

  useEffect(() => {
    persistStatsRange(statsRange);
  }, [statsRange]);

  useEffect(() => {
    const requestSeq = ++statsRequestSeqRef.current;
    setStatsRangeError("");
    void codexLocalAccessService
      .queryCodexLocalAccessStats(statsTimeRange.startAt, statsTimeRange.endAt)
      .then((window) => {
        if (!mountedRef.current || requestSeq !== statsRequestSeqRef.current) return;
        setFilteredStatsWindow(window);
      })
      .catch((err) => {
        if (!mountedRef.current || requestSeq !== statsRequestSeqRef.current) return;
        setStatsRangeError(String(err).replace(/^Error:\s*/, ""));
      });
  }, [statsTimeRange.endAt, statsTimeRange.startAt, stats?.updatedAt]);

  const handleStatsPresetChange = (
    key: Exclude<CodexStatsRangeKey, "custom">,
    range: CodexStatsTimeRange,
  ) => {
    setStatsRange(key);
    setStatsTimeRange(range);
  };

  const handleCustomStatsRangeApply = (range: CodexStatsTimeRange) => {
    setStatsRange("custom");
    setStatsTimeRange(range);
  };

  useEffect(() => {
    persistAddressKind(addressKind);
  }, [addressKind]);

  useEffect(() => {
    if (
      !collection?.enabled ||
      (!state?.preparing &&
        !state?.refreshingAccounts &&
        (state?.running || Boolean(state?.lastError)))
    ) {
      return undefined;
    }
    const timer = window.setInterval(() => {
      void reloadState();
    }, 750);
    return () => window.clearInterval(timer);
  }, [
    collection?.enabled,
    reloadState,
    state?.preparing,
    state?.refreshingAccounts,
    state?.running,
    state?.lastError,
  ]);

  useEffect(() => {
    persistRequestLogPageSize(requestLogPageSize);
  }, [requestLogPageSize]);

  useEffect(() => {
    setRequestLogPage(1);
  }, [
    statsRange,
    statsTimeRange.startAt,
    statsTimeRange.endAt,
    requestLogPageSize,
    requestLogKindFilter,
    requestLogStatusFilter,
    requestLogGatewayModeFilter,
    requestLogModelQuery,
    requestLogAccountQuery,
    requestLogApiKeyQuery,
    requestLogInstanceQuery,
    requestLogErrorQuery,
  ]);

  useEffect(() => {
    if (activeTab !== "logs" || statsLogTab !== "logs") return;
    let disposed = false;
    setRequestLogLoading(true);
    setRequestLogError("");
    const success =
      requestLogStatusFilter === "success"
        ? true
        : requestLogStatusFilter === "failed"
          ? false
          : null;
    void codexLocalAccessService
      .queryCodexLocalAccessRequestLogs({
        page: requestLogPage,
        pageSize: requestLogPageSize,
        statsRange: statsRange === "custom" ? null : statsRange,
        startAt: statsTimeRange.startAt,
        endAt: statsTimeRange.endAt,
        modelQuery: requestLogModelQuery,
        accountQuery: requestLogAccountQuery,
        apiKeyQuery: requestLogApiKeyQuery,
        instanceQuery:
          requestLogInstanceQuery === "all" ? null : requestLogInstanceQuery,
        gatewayMode:
          requestLogGatewayModeFilter === "all"
            ? null
            : requestLogGatewayModeFilter,
        requestKind:
          requestLogKindFilter === "all" ? null : requestLogKindFilter,
        success,
        errorCategory: requestLogErrorQuery,
      })
      .then((result) => {
        if (disposed) return;
        setRequestLogResult(result);
        if (result.page !== requestLogPage) {
          setRequestLogPage(result.page);
        }
      })
      .catch((err) => {
        if (!disposed) {
          setRequestLogError(String(err).replace(/^Error:\s*/, ""));
        }
      })
      .finally(() => {
        if (!disposed) {
          setRequestLogLoading(false);
        }
      });
    return () => {
      disposed = true;
    };
  }, [
    activeTab,
    statsLogTab,
    statsRange,
    statsTimeRange.startAt,
    statsTimeRange.endAt,
    requestLogPage,
    requestLogPageSize,
    requestLogKindFilter,
    requestLogStatusFilter,
    requestLogGatewayModeFilter,
    requestLogModelQuery,
    requestLogAccountQuery,
    requestLogApiKeyQuery,
    requestLogInstanceQuery,
    requestLogErrorQuery,
    stats?.updatedAt,
  ]);

  useEffect(() => {
    setApiKeyDrafts(
      Object.fromEntries(
        (collection?.apiKeys ?? []).map((apiKey) => [apiKey.id, apiKey.label]),
      ),
    );
    setApiKeyPolicyDrafts((currentDrafts) =>
      Object.fromEntries(
        (collection?.apiKeys ?? []).map((apiKey) => {
          const currentDraft = currentDrafts[apiKey.id];
          return [
            apiKey.id,
            currentDraft && apiKeyPolicyDraftIsDirty(apiKey, currentDraft)
              ? currentDraft
              : apiKeyPolicyDraftFromValue(apiKey),
          ];
        }),
      ),
    );
  }, [collection?.apiKeys]);

  useEffect(() => {
    setModelAliasesText(serializeModelAliases(collection?.modelAliases));
    setExcludedModelsText(serializeModelRules(collection?.excludedModels));
    setAccountModelRuleDrafts(
      Object.fromEntries(
        (collection?.accountModelRules ?? []).map((rule) => [
          rule.accountId,
          serializeModelRules(rule.excludedModels),
        ]),
      ),
    );
    setAccountModelRuleSelected(new Set());
    setAccountModelRuleBulkText("");
    setResponsesWebsocketsEnabledDraft(
      collection?.responsesWebsocketsEnabled ?? false,
    );
    setMaxRetryCredentialsDraft(String(collection?.maxRetryCredentials ?? 0));
    setMaxRetryIntervalDraft(
      formatSeconds(collection?.maxRetryIntervalMs ?? 3000),
    );
    setDisableCoolingDraft(collection?.disableCooling ?? false);
    setImmediateSseResponseDraft(collection?.immediateSseResponse ?? false);
    setMaxConcurrentImageRequestsDraft(
      String(collection?.maxConcurrentImageRequests ?? 1),
    );
    setTimeoutDrafts(timeoutDraftsFromValue(collection?.timeouts));
    setSelectedTimeoutPresetId(
      collection?.activeTimeoutPresetId || "long_wait",
    );
  }, [
    collection?.modelAliases,
    collection?.excludedModels,
    collection?.accountModelRules,
    collection?.responsesWebsocketsEnabled,
    collection?.maxRetryCredentials,
    collection?.maxRetryIntervalMs,
    collection?.disableCooling,
    collection?.immediateSseResponse,
    collection?.maxConcurrentImageRequests,
    collection?.timeouts,
    collection?.activeTimeoutPresetId,
  ]);

  useEffect(() => {
    if (modelIds.length === 0) {
      setSelectedModelId("");
      return;
    }
    setSelectedModelId((current) =>
      modelIds.includes(current) ? current : modelIds[0],
    );
  }, [modelIds]);

  useEffect(() => {
    if (!testDialogOpen) return;
    const node = testChatScrollRef.current;
    scrollElementTo(node, { top: node?.scrollHeight ?? 0 });
  }, [testChatMessages, testDialogOpen]);

  useEffect(() => {
    if (!pricingModalOpen) return;
    setPricingDrafts(modelPricingRows.map(modelPricingDraftFromRow));
    setPricingError("");
  }, [modelPricingRows, pricingModalOpen]);

  const runAction = async (
    task: () => Promise<unknown>,
    successText: string,
  ) => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await task();
      setNotice(successText);
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const handleCopy = async (field: CopyField, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      window.setTimeout(
        () => setCopiedField((current) => (current === field ? null : current)),
        1200,
      );
    } catch (err) {
      setError(t("common.shared.export.copyFailed", "复制失败，请手动复制"));
      console.error("Failed to copy Codex API service value:", err);
    }
  };

  const toggleApiKeyPolicyExpanded = useCallback((apiKeyId: string) => {
    setExpandedApiKeyPolicyIds((current) => {
      const next = new Set(current);
      if (next.has(apiKeyId)) {
        next.delete(apiKeyId);
      } else {
        next.add(apiKeyId);
      }
      return next;
    });
  }, []);

  const handleToggleEnabled = async () => {
    if (!collection) return;
    if (!collection.enabled) {
      const confirmed = await confirmDialog(
        t(
          "codex.localAccess.riskNotice.desc",
          "当前 Codex API 服务相关功能，本质上属于代理转发使用方式。继续使用即表示您已知悉相关情况，并愿意自行承担可能产生的风险。",
        ),
        {
          title: t("codex.localAccess.riskNotice.title", "使用风险提示"),
          kind: "warning",
          okLabel: t("codex.localAccess.riskNotice.continueStart", "继续启动"),
          cancelLabel: t("common.cancel", "取消"),
        },
      );
      if (!confirmed) return;
    }
    await runAction(
      async () => {
        const next = await codexLocalAccessService.setCodexLocalAccessEnabled(
          !collection.enabled,
        );
        setState(next);
      },
      collection.enabled
        ? t("codex.localAccess.disabledSuccess", "API 服务已停用")
        : t("codex.localAccess.enabledSuccess", "API 服务已启用"),
    );
  };

  const refreshApiServiceCurrent = useCallback(async () => {
    try {
      const instances = await codexInstanceService.listInstances();
      if (!mountedRef.current) return;
      const defaultInstance = instances.find((instance) => instance.isDefault);
      setCodexInstances(instances);
      setApiServiceIsCurrent(
        defaultInstance?.bindAccountId === CODEX_API_SERVICE_BIND_ID,
      );
    } catch {
      if (!mountedRef.current) return;
      setApiServiceIsCurrent(false);
    }
  }, []);

  const handleOpenAddAccount = useCallback(() => {
    requestCodexOpenAddAccount({
      autoJoinApiService: true,
      tab: "oauth",
    });
  }, []);

  const handleActivateService = async () => {
    if (!collection) return;
    if (!collection.enabled) {
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
    if (!isCodexLocalAccessRiskNoticeDismissed()) {
      const confirmedRisk = await confirmDialog(
        t(
          "codex.localAccess.riskNotice.desc",
          "当前 Codex API 服务相关功能，本质上属于代理转发使用方式。继续使用即表示您已知悉相关情况，并愿意自行承担可能产生的风险。",
        ),
        {
          title: t("codex.localAccess.riskNotice.title", "使用风险提示"),
          kind: "warning",
          okLabel: t(
            "codex.localAccess.riskNotice.continueSwitch",
            "继续切号",
          ),
          cancelLabel: t("common.cancel", "取消"),
        },
      );
      if (!confirmedRisk) return;
    }

    setActivating(true);
    setError("");
    setNotice("");
    try {
      const next = await codexLocalAccessService.activateCodexLocalAccess();
      if (!mountedRef.current) return;
      setState(next);
      await fetchCurrentAccount();
      await refreshApiServiceCurrent();
      setNotice(
        t("codex.localAccess.activateSuccess", "已切换到 API 服务"),
      );
    } catch (err) {
      if (!mountedRef.current) return;
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      if (mountedRef.current) {
        setActivating(false);
      }
    }
  };

  const handleOpenTestDialog = () => {
    setTestDialogOpen(true);
    setTestDialogError("");
  };

  const handleCloseTestDialog = () => {
    if (testDialogRunning) return;
    setTestDialogOpen(false);
  };

  const clearTestChat = () => {
    if (testDialogRunning) return;
    setTestChatMessages([]);
    setTestDialogError("");
  };

  const handleSendTestChatMessage = async () => {
    if (testDialogRunning) return;
    const content = testChatInput.trim();
    if (!content) {
      setTestDialogError(
        t("codex.localAccess.testChatInputRequired", "请输入测试消息"),
      );
      return;
    }
    if (!selectedModelId) {
      setTestDialogError(
        t("codex.localAccess.testChatModelRequired", "请选择模型 ID"),
      );
      return;
    }

    const userMessage = createTestChatMessage("user", content);
    const assistantMessage = createTestChatMessage("assistant", "");
    const nextMessages = [...testChatMessages, userMessage, assistantMessage];
    setTestChatMessages(nextMessages);
    setTestChatInput("");
    setTestDialogError("");
    setTestDialogRunning(true);
    const sessionId = `api-service-test-${Date.now()}-${Math.random()
      .toString(36)
      .slice(2, 8)}`;
    let unlisten: (() => void) | null = null;
    try {
      const apiMessages: CodexLocalAccessChatMessage[] = nextMessages
        .filter((message) => !message.failureTitle && message.content.trim())
        .map((message) => ({
          role: message.role,
          content: message.content,
        }));
      unlisten = await listen<CodexLocalAccessChatStreamEvent>(
        "codex-local-access-chat-test-stream",
        (event) => {
          const payload = event.payload;
          if (payload.sessionId !== sessionId) return;
          if (payload.type === "delta") {
            const chunk = payload.content ?? payload.reasoning ?? "";
            if (!chunk) return;
            setTestChatMessages((current) =>
              current.map((message) =>
                message.id === assistantMessage.id
                  ? { ...message, content: `${message.content}${chunk}` }
                  : message,
              ),
            );
            return;
          }
          if (payload.type === "error") {
            setTestChatMessages((current) =>
              current.map((message) =>
                message.id === assistantMessage.id
                  ? {
                      ...message,
                      content: payload.failure.cause,
                      failureTitle: payload.failure.title,
                      failureDetail: payload.failure.suggestion,
                    }
                  : message,
              ),
            );
            return;
          }
          if (payload.type === "done") {
            setTestChatMessages((current) =>
              current.map((message) =>
                message.id === assistantMessage.id
                  ? {
                      ...message,
                      content:
                        message.content ||
                        t(
                          "codex.localAccess.testChatEmptyResponse",
                          "响应为空",
                        ),
                      latencyMs: payload.latencyMs,
                    }
                  : message,
              ),
            );
          }
        },
      );
      await codexLocalAccessService.streamCodexLocalAccessChatTest(
        sessionId,
        selectedModelId,
        apiMessages,
      );
    } catch (err) {
      setTestDialogError(String(err).replace(/^Error:\s*/, ""));
      setTestChatMessages((current) =>
        current.filter((message) => message.id !== assistantMessage.id),
      );
    } finally {
      unlisten?.();
      setTestDialogRunning(false);
    }
  };

  const handleSavePort = async () => {
    const nextPort = Number(portInput.trim());
    if (!Number.isInteger(nextPort) || nextPort <= 0 || nextPort > 65535) {
      setError(
        t("codex.localAccess.portInvalid", "请输入 1 到 65535 之间的端口"),
      );
      return;
    }
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessPort(nextPort);
        setState(next);
      },
      t("codex.localAccess.portSaveSuccess", "API 服务端口已更新"),
    );
  };

  const handleSaveProxy = async () => {
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessUpstreamProxyConfig(
            proxyInput.trim() || null,
          );
        setState(next);
      },
      t("codex.localAccess.upstreamProxySaveSuccess", "API 代理地址已更新"),
    );
  };

  const handleKillPort = async () => {
    setPortKilling(true);
    setError("");
    setNotice("");
    try {
      const result = await codexLocalAccessService.killCodexLocalAccessPort();
      setState(result.state);
      setNotice(
        t("codex.localAccess.killPortSuccessUnknown", "API 服务端口已清理"),
      );
    } catch (err) {
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setPortKilling(false);
    }
  };

  const handleUpdateAccessScope = async (value: string) => {
    const accessScope = value === "lan" ? "lan" : "localhost";
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessAccessScope(
            accessScope,
          );
        setState(next);
      },
      t("codex.localAccess.accessScopeSaveSuccess", "API 服务访问范围已更新"),
    );
  };

  const handleUpdateClientBaseUrlHost = async (value: string) => {
    const host = (
      value === "127.0.0.1" ? value : "localhost"
    ) as CodexLocalAccessClientBaseUrlHost;
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessClientBaseUrlHost(
            host,
          );
        setState(next);
      },
      t("codex.localAccess.clientBaseUrlHostSaveSuccess", "客户端地址已更新"),
    );
  };

  const handleUpdateRouting = async (value: string) => {
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessRoutingStrategy(
            value as CodexLocalAccessRoutingStrategy,
          );
        setState(next);
      },
      t("codex.localAccess.routingSaveSuccess", "API 服务调度策略已更新"),
    );
  };

  const handleUpdateGatewayMode = async (
    value: CodexLocalAccessGatewayMode,
  ) => {
    if (!collection || value === gatewayMode) return;
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessGatewayMode(
            value,
          );
        setState(next);
      },
      t("codex.localAccess.gatewayModeSaveSuccess", "API 服务网关模式已更新"),
    );
  };

  const saveMembers = async (
    accountIds: string[],
    restrictFreeAccounts: boolean,
    backupAccountIds?: string[],
  ) => {
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
    const nextBackupAccountIds = (backupAccountIds ?? []).filter((id) =>
      filteredAccountIdSet.has(id),
    );

    const next = await codexLocalAccessService.saveCodexLocalAccessAccounts(
      filteredAccountIds,
      restrictFreeAccounts,
      nextBackupAccountIds,
    );
    setState(next);
    void fetchAccounts().catch((error) => {
      console.error(
        "Failed to refresh Codex accounts after API service save:",
        error,
      );
    });
  };

  const handleSaveMembers = async (
    accountIds: string[],
    restrictFreeAccounts: boolean,
    backupAccountIds?: string[],
  ) => {
    await runAction(
      () => saveMembers(accountIds, restrictFreeAccounts, backupAccountIds),
      t("codex.localAccess.saveSuccess", "API 服务集合已更新"),
    );
  };

  const handleSaveMembersFromModal = async (
    accountIds: string[],
    restrictFreeAccounts: boolean,
    backupAccountIds?: string[],
  ) => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await saveMembers(accountIds, restrictFreeAccounts, backupAccountIds);
      setNotice(t("codex.localAccess.saveSuccess", "API 服务集合已更新"));
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      throw new Error(message);
    } finally {
      setBusy(false);
    }
  };

  const handleRecoverAccounts = async (accountIds: string[]) => {
    if (busy || accountIds.length === 0) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const next =
        await codexLocalAccessService.recoverCodexLocalAccessAccounts(
          accountIds,
        );
      setState(next);
      setNotice(
        t("codex.localAccess.accountPoolHealth.recoverSuccess", {
          count: accountIds.length,
          defaultValue: "已提交 {{count}} 个账号的恢复操作",
        }),
      );
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      throw new Error(message);
    } finally {
      setBusy(false);
    }
  };

  const handleRemoveMember = async (accountId: string) => {
    if (!collection) return;
    const remainingIds = collection.accountIds.filter(
      (item) => item !== accountId,
    );
    const remainingSet = new Set(remainingIds);
    const backupAccountIds = (collection.customRoutingRules ?? [])
      .filter((rule) => rule.isBackup && remainingSet.has(rule.accountId))
      .map((rule) => rule.accountId);
    await handleSaveMembers(
      remainingIds,
      collection.restrictFreeAccounts,
      backupAccountIds,
    );
  };

  const handleCreateApiKey = async () => {
    const nextIndex = (collection?.apiKeys.length ?? 0) + 1;
    await runAction(
      async () => {
        const next = await codexLocalAccessService.createCodexLocalAccessApiKey(
          t("codex.localAccess.apiKeyDefaultLabel", {
            index: nextIndex,
            defaultValue: "Client {{index}}",
          }),
        );
        setState(next);
      },
      t("codex.localAccess.apiKeyCreateSuccess", "API Key 已创建"),
    );
  };

  const handleSaveApiKeyLabel = async (
    apiKeyId: string,
    currentLabel: string,
  ) => {
    const nextLabel = (apiKeyDrafts[apiKeyId] ?? currentLabel).trim();
    if (!nextLabel || nextLabel === currentLabel) return;
    await runAction(
      async () => {
        const next = await codexLocalAccessService.updateCodexLocalAccessApiKey(
          apiKeyId,
          {
            label: nextLabel,
          },
        );
        setState(next);
      },
      t("codex.localAccess.apiKeyUpdateSuccess", "API Key 已更新"),
    );
  };

  const handleSaveApiKeyPolicy = async (apiKeyId: string) => {
    const draft = apiKeyPolicyDrafts[apiKeyId];
    if (!draft) return;
    const apiKey = collection?.apiKeys.find((item) => item.id === apiKeyId);
    if (!apiKey) return;
    const accountIds = reconcileCodexApiKeyScopeAccountIds({
      accounts: localAccessAccounts,
      restrictFreeAccounts: collection?.restrictFreeAccounts ?? true,
      persistedAccountIds: apiKey.accountIds ?? [],
      draftAccountIds: draft.accountIds,
    });
    if (
      apiKeyHasFixedAccountScope(apiKey, collection) &&
      (draft.inheritAccountPool || accountIds.length === 0)
    ) {
      setError(
        t(
          "codex.apiService.keys.accountScopeFixed",
          "此 Key 已固定绑定账号，不能继承服务池或清空账号范围",
        ),
      );
      return;
    }
    if (!draft.inheritAccountPool && accountIds.length === 0) {
      setError(
        t(
          "codex.apiService.keys.accountScopeRequired",
          "自定义账号池至少需要选择 1 个账号",
        ),
      );
      return;
    }
    await runAction(
      async () => {
        const next = await codexLocalAccessService.updateCodexLocalAccessApiKey(
          apiKeyId,
          {
            modelPrefix: draft.modelPrefix.trim(),
            allowedModels: parseModelRuleText(draft.allowedModels),
            excludedModels: parseModelRuleText(draft.excludedModels),
            accountIds,
            inheritAccountPool: draft.inheritAccountPool,
          },
        );
        setState(next);
        const savedApiKey = next.collection?.apiKeys.find(
          (item) => item.id === apiKeyId,
        );
        if (savedApiKey) {
          setApiKeyPolicyDrafts((drafts) => ({
            ...drafts,
            [apiKeyId]: apiKeyPolicyDraftFromValue(savedApiKey),
          }));
        }
      },
      t("codex.apiService.keys.policySaved", "Key 策略已保存"),
    );
  };

  const handleSetApiKeyAccountPriority = async (
    apiKey: CodexLocalAccessApiKey,
    draft: ApiKeyPolicyDraft,
    accountId: string,
  ) => {
    if (
      draft.inheritAccountPool ||
      !draft.accountIds.includes(accountId) ||
      !apiKey.accountIds?.includes(accountId)
    ) {
      return;
    }
    const priorityRank = (apiKey.priorityAccountIds ?? []).indexOf(accountId);
    const pinned = priorityRank !== 0;
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.setCodexLocalAccessApiKeyAccountPriority(
            apiKey.id,
            accountId,
            pinned,
          );
        setState(next);
      },
      pinned
        ? t("codex.apiService.keys.accountPrioritySaved", "已置顶账号")
        : t("codex.apiService.keys.accountPriorityCleared", "已取消置顶账号"),
    );
  };

  const handleResetApiKeyPolicy = (apiKey: CodexLocalAccessApiKey) => {
    setApiKeyPolicyDrafts((drafts) => ({
      ...drafts,
      [apiKey.id]: apiKeyPolicyDraftFromValue(apiKey),
    }));
    setError("");
  };

  const handleToggleApiKey = async (apiKeyId: string, enabled: boolean) => {
    await runAction(
      async () => {
        const next = await codexLocalAccessService.updateCodexLocalAccessApiKey(
          apiKeyId,
          {
            enabled,
          },
        );
        setState(next);
      },
      t("codex.localAccess.apiKeyUpdateSuccess", "API Key 已更新"),
    );
  };

  const handleRotateApiKey = async (apiKeyId: string) => {
    const confirmed = await confirmDialog(
      t(
        "codex.localAccess.apiKeyRotateConfirm",
        "重置后该 API Key 会立即失效，确认继续吗？",
      ),
      {
        title: t("codex.localAccess.rotateKey", "重置密钥"),
        kind: "warning",
        okLabel: t("common.confirm", "确认"),
        cancelLabel: t("common.cancel", "取消"),
      },
    );
    if (!confirmed) return;
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.rotateCodexLocalAccessNamedApiKey(
            apiKeyId,
          );
        setState(next);
      },
      t("codex.localAccess.apiKeyRotateSuccess", "API Key 已重置"),
    );
  };

  const handleDeleteApiKey = async (apiKeyId: string) => {
    const confirmed = await confirmDialog(
      t("codex.localAccess.apiKeyDeleteConfirm", "确定删除这个 API Key 吗？"),
      {
        title: t("codex.localAccess.apiKeyDelete", "删除 Key"),
        kind: "error",
        okLabel: t("common.delete", "删除"),
        cancelLabel: t("common.cancel", "取消"),
      },
    );
    if (!confirmed) return;
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.deleteCodexLocalAccessApiKey(apiKeyId);
        setState(next);
      },
      t("codex.localAccess.apiKeyDeleteSuccess", "API Key 已删除"),
    );
  };

  const handleClearStats = async () => {
    const confirmed = await confirmDialog(
      t("codex.localAccess.clearStatsConfirm", "确定要清空 API 服务统计吗？"),
      {
        title: t("codex.localAccess.clearStats", "清除统计"),
        kind: "warning",
        okLabel: t("common.confirm", "确认"),
        cancelLabel: t("common.cancel", "取消"),
      },
    );
    if (!confirmed) return;
    await runAction(
      async () => {
        const next = await codexLocalAccessService.clearCodexLocalAccessStats();
        setState(next);
      },
      t("codex.localAccess.clearStatsSuccess", "API 服务统计已清空"),
    );
  };

  const handleSaveModelRules = async () => {
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessModelRules(
            parseModelAliasText(modelAliasesText),
            parseModelRuleText(excludedModelsText),
          );
        setState(next);
      },
      t("codex.apiService.models.rulesSaved", "模型规则已保存"),
    );
  };

  const resetAccountModelRuleDraftsFromCollection = () => {
    setAccountModelRuleDrafts(
      Object.fromEntries(
        (collection?.accountModelRules ?? []).map((rule) => [
          rule.accountId,
          serializeModelRules(rule.excludedModels),
        ]),
      ),
    );
    setAccountModelRuleSelected(new Set());
    setAccountModelRuleBulkText("");
  };

  const handleOpenAccountModelRules = () => {
    resetAccountModelRuleDraftsFromCollection();
    setAccountModelRulesOpen(true);
  };

  const handleCloseAccountModelRules = () => {
    resetAccountModelRuleDraftsFromCollection();
    setAccountModelRulesOpen(false);
  };

  const handleApplyAccountModelRuleBulk = () => {
    if (accountModelRuleSelected.size === 0) return;
    setAccountModelRuleDrafts((drafts) => {
      const next = { ...drafts };
      accountModelRuleSelected.forEach((accountId) => {
        next[accountId] = accountModelRuleBulkText;
      });
      return next;
    });
  };

  const handleSaveAccountModelRules = async () => {
    const rules: CodexLocalAccessAccountModelRule[] = memberAccounts
      .map((account) => ({
        accountId: account.id,
        excludedModels: parseModelRuleText(
          accountModelRuleDrafts[account.id] ?? "",
        ),
      }))
      .filter((rule) => rule.excludedModels.length > 0);

    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessAccountModelRules(
            rules,
          );
        setState(next);
        setAccountModelRulesOpen(false);
      },
      t(
        "codex.apiService.accountModelRules.saveSuccess",
        "账号模型禁用规则已保存",
      ),
    );
  };

  const handleOpenPricingModal = () => {
    setPricingDrafts(modelPricingRows.map(modelPricingDraftFromRow));
    setPricingError("");
    setPricingModalOpen(true);
  };

  const updatePricingDraft = (
    modelId: string,
    field: keyof Omit<ModelPricingDraft, "modelId" | "hasPreset" | "custom">,
    value: string,
  ) => {
    setPricingDrafts((current) =>
      current.map((item) =>
        item.modelId === modelId ? { ...item, [field]: value } : item,
      ),
    );
  };

  const resetPricingDraft = (modelId: string) => {
    const preset = state?.modelPricingPresets.find(
      (item) => item.modelId.toLowerCase() === modelId.toLowerCase(),
    );
    setPricingDrafts((current) =>
      current.map((item) =>
        item.modelId === modelId
          ? {
              ...item,
              longContextThresholdTokens: formatIntegerDraftValue(
                preset?.longContextThresholdTokens ?? null,
              ),
              inputUsdPerMillion: formatPriceDraftValue(
                preset?.inputUsdPerMillion ?? null,
              ),
              cachedInputUsdPerMillion: formatPriceDraftValue(
                preset?.cachedInputUsdPerMillion ?? null,
              ),
              outputUsdPerMillion: formatPriceDraftValue(
                preset?.outputUsdPerMillion ?? null,
              ),
              standardLongInputUsdPerMillion: formatPriceDraftValue(
                preset?.standardLongInputUsdPerMillion ?? null,
              ),
              standardLongCachedInputUsdPerMillion: formatPriceDraftValue(
                preset?.standardLongCachedInputUsdPerMillion ?? null,
              ),
              standardLongOutputUsdPerMillion: formatPriceDraftValue(
                preset?.standardLongOutputUsdPerMillion ?? null,
              ),
              priorityInputUsdPerMillion: formatPriceDraftValue(
                preset?.priorityInputUsdPerMillion ?? null,
              ),
              priorityCachedInputUsdPerMillion: formatPriceDraftValue(
                preset?.priorityCachedInputUsdPerMillion ?? null,
              ),
              priorityOutputUsdPerMillion: formatPriceDraftValue(
                preset?.priorityOutputUsdPerMillion ?? null,
              ),
              custom: false,
            }
          : item,
      ),
    );
  };

  const handleSaveModelPricings = async () => {
    if (pricingRepriceActive) {
      setPricingError(
        t(
          "codex.apiService.models.pricingRepriceSaveBlocked",
          "历史估算价值更新中，完成后再保存",
        ),
      );
      return;
    }
    const presetMap = new Map(
      (state?.modelPricingPresets ?? []).map((item) => [
        item.modelId.toLowerCase(),
        item,
      ]),
    );
    const sameAsPreset = (
      draft: ModelPricingDraft,
      preset: CodexLocalAccessModelPricing,
    ) =>
      sameOptionalPrice(
        parseOptionalPositiveIntegerDraft(draft.longContextThresholdTokens),
        preset.longContextThresholdTokens ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.inputUsdPerMillion, false),
        preset.inputUsdPerMillion,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.outputUsdPerMillion, false),
        preset.outputUsdPerMillion,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.cachedInputUsdPerMillion, true),
        preset.cachedInputUsdPerMillion ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.standardLongInputUsdPerMillion, true),
        preset.standardLongInputUsdPerMillion ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.standardLongOutputUsdPerMillion, true),
        preset.standardLongOutputUsdPerMillion ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.standardLongCachedInputUsdPerMillion, true),
        preset.standardLongCachedInputUsdPerMillion ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.priorityInputUsdPerMillion, true),
        preset.priorityInputUsdPerMillion ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.priorityOutputUsdPerMillion, true),
        preset.priorityOutputUsdPerMillion ?? null,
      ) &&
      sameOptionalPrice(
        parsePriceDraftValue(draft.priorityCachedInputUsdPerMillion, true),
        preset.priorityCachedInputUsdPerMillion ?? null,
      );
    const nextPricings: CodexLocalAccessModelPricing[] = [];
    for (const draft of pricingDrafts) {
      const longContextThresholdTokens = parseOptionalPositiveIntegerDraft(
        draft.longContextThresholdTokens,
      );
      const input = parsePriceDraftValue(draft.inputUsdPerMillion, false);
      const cached = parsePriceDraftValue(draft.cachedInputUsdPerMillion, true);
      const output = parsePriceDraftValue(draft.outputUsdPerMillion, false);
      const standardLongInput = parsePriceDraftValue(
        draft.standardLongInputUsdPerMillion,
        true,
      );
      const standardLongCached = parsePriceDraftValue(
        draft.standardLongCachedInputUsdPerMillion,
        true,
      );
      const standardLongOutput = parsePriceDraftValue(
        draft.standardLongOutputUsdPerMillion,
        true,
      );
      const priorityInput = parsePriceDraftValue(
        draft.priorityInputUsdPerMillion,
        true,
      );
      const priorityCached = parsePriceDraftValue(
        draft.priorityCachedInputUsdPerMillion,
        true,
      );
      const priorityOutput = parsePriceDraftValue(
        draft.priorityOutputUsdPerMillion,
        true,
      );
      const preset = presetMap.get(draft.modelId.toLowerCase());
      const unsetUnknown =
        !preset &&
        draft.longContextThresholdTokens.trim() === "" &&
        draft.inputUsdPerMillion.trim() === "" &&
        draft.cachedInputUsdPerMillion.trim() === "" &&
        draft.outputUsdPerMillion.trim() === "" &&
        draft.standardLongInputUsdPerMillion.trim() === "" &&
        draft.standardLongCachedInputUsdPerMillion.trim() === "" &&
        draft.standardLongOutputUsdPerMillion.trim() === "" &&
        draft.priorityInputUsdPerMillion.trim() === "" &&
        draft.priorityCachedInputUsdPerMillion.trim() === "" &&
        draft.priorityOutputUsdPerMillion.trim() === "";
      if (unsetUnknown) {
        continue;
      }
      // 阈值可空：非长上下文模型（如 gpt-5.4-mini）不填是合法的。
      // 仅当用户填写了内容但不是正整数（解析为 NaN）时才拦截。
      // 若填写了任一长上下文价格档，则必须同时提供合法阈值。
      const hasLongContextTier =
        (standardLongInput != null && Number.isFinite(standardLongInput)) ||
        (standardLongOutput != null && Number.isFinite(standardLongOutput)) ||
        (standardLongCached != null && Number.isFinite(standardLongCached));
      const tokenInvalid = hasLongContextTier
        ? longContextThresholdTokens === null ||
          !Number.isFinite(longContextThresholdTokens)
        : longContextThresholdTokens != null &&
          !Number.isFinite(longContextThresholdTokens);
      const inputInvalid = input === null || !Number.isFinite(input);
      const cachedInvalid = cached !== null && !Number.isFinite(cached);
      const outputInvalid = output === null || !Number.isFinite(output);
      const tierInvalid =
        (standardLongInput !== null && !Number.isFinite(standardLongInput)) ||
        (standardLongOutput !== null &&
          !Number.isFinite(standardLongOutput)) ||
        (standardLongCached !== null && !Number.isFinite(standardLongCached)) ||
        (priorityInput !== null && !Number.isFinite(priorityInput)) ||
        (priorityOutput !== null && !Number.isFinite(priorityOutput)) ||
        (priorityCached !== null && !Number.isFinite(priorityCached));
      if (
        tokenInvalid ||
        inputInvalid ||
        cachedInvalid ||
        outputInvalid ||
        tierInvalid
      ) {
        setPricingError(
          t(
            "codex.apiService.models.pricingInvalid",
            "价格必须是大于或等于 0 的数字，Token 阈值必须是正整数",
          ),
        );
        return;
      }
      const allZero =
        !preset &&
        input === 0 &&
        output === 0 &&
        (cached == null || cached === 0) &&
        standardLongInput === 0 &&
        standardLongOutput === 0 &&
        (standardLongCached == null || standardLongCached === 0) &&
        priorityInput === 0 &&
        priorityOutput === 0 &&
        (priorityCached == null || priorityCached === 0);
      if ((preset && sameAsPreset(draft, preset)) || allZero) {
        continue;
      }
      nextPricings.push({
        modelId: draft.modelId,
        longContextThresholdTokens,
        inputUsdPerMillion: input,
        outputUsdPerMillion: output,
        cachedInputUsdPerMillion: cached,
        standardLongInputUsdPerMillion: standardLongInput,
        standardLongOutputUsdPerMillion: standardLongOutput,
        standardLongCachedInputUsdPerMillion: standardLongCached,
        priorityInputUsdPerMillion: priorityInput,
        priorityOutputUsdPerMillion: priorityOutput,
        priorityCachedInputUsdPerMillion: priorityCached,
        // priority_long_* is not a product tier; keep wire fields cleared.
        priorityLongInputUsdPerMillion: null,
        priorityLongOutputUsdPerMillion: null,
        priorityLongCachedInputUsdPerMillion: null,
      });
    }
    setPricingError("");
    setPricingRepriceProgress(null);
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessModelPricings(
            nextPricings,
          );
        setState(next);
      },
      t("codex.apiService.models.pricingSaved", "价格设置已保存"),
    );
  };

  const handleRepriceRequestLogs = async () => {
    setBusy(true);
    setPricingError("");
    setNotice("");
    try {
      const next =
        await codexLocalAccessService.repriceCodexLocalAccessRequestLogs();
      setState(next);
      setNotice(
        t(
          "codex.apiService.models.pricingRepriced",
          "历史估值已按当前价格重算",
        ),
      );
    } catch (err) {
      setPricingError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const handleSaveRoutingOptions = async () => {
    const maxRetryCredentials = parseIntegerDraft(
      maxRetryCredentialsDraft,
      0,
      8,
    );
    if (maxRetryCredentials === null) {
      setError(
        t("codex.apiService.validation.numberRange", {
          min: 0,
          max: 8,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return;
    }
    const maxRetryIntervalSeconds = parseIntegerDraft(
      maxRetryIntervalDraft,
      0,
      30,
    );
    if (maxRetryIntervalSeconds === null) {
      setError(
        t("codex.apiService.validation.numberRange", {
          min: 0,
          max: 30,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return;
    }
    const maxConcurrentImageRequests = parseIntegerDraft(
      maxConcurrentImageRequestsDraft,
      1,
      16,
    );
    if (maxConcurrentImageRequests === null) {
      setError(
        t("codex.apiService.validation.numberRange", {
          min: 1,
          max: 16,
          defaultValue: "Please enter a number between {{min}} and {{max}}",
        }),
      );
      return;
    }
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessRoutingOptions({
            sessionAffinity: collection?.sessionAffinity ?? true,
            sessionAffinityTtlMs:
              collection?.sessionAffinityTtlMs ?? 60 * 60 * 1000,
            responsesWebsocketsEnabled: responsesWebsocketsEnabledDraft,
            maxRetryCredentials,
            maxRetryIntervalMs: maxRetryIntervalSeconds * 1000,
            disableCooling: disableCoolingDraft,
            immediateSseResponse: immediateSseResponseDraft,
            maxConcurrentImageRequests,
          });
        setState(next);
      },
      t("codex.apiService.routing.optionsSaved", "调度选项已保存"),
    );
  };

  const updateTimeoutDraft = (
    key: keyof CodexLocalAccessTimeouts,
    value: string,
  ) => {
    setTimeoutsError("");
    setTimeoutDrafts((current) => ({ ...current, [key]: value }));
  };

  const handleResetTimeoutDrafts = () => {
    setTimeoutsError("");
    setSelectedTimeoutPresetId("long_wait");
    setTimeoutDrafts(timeoutDraftsFromValue());
  };

  const parseTimeoutDraftPayload = (): CodexLocalAccessTimeouts | null => {
    const secondFields: Array<keyof CodexLocalAccessTimeouts> = [
      "legacyRequestReadTimeoutMs",
      "legacyUpstreamConnectTimeoutMs",
      "legacyStreamIdleTimeoutMs",
      "legacyStreamTotalTimeoutMs",
      "sidecarStreamOpenTimeoutMs",
      "sidecarStreamIdleTimeoutMs",
      "sidecarImageStreamOpenTimeoutMs",
      "sidecarImageStreamIdleTimeoutMs",
      "websocketConnectTimeoutMs",
      "websocketInitialMessageTimeoutMs",
      "websocketIdleTimeoutMs",
      "websocketHeartbeatIntervalMs",
    ];
    const parsedSeconds = new Map<keyof CodexLocalAccessTimeouts, number>();
    for (const key of secondFields) {
      const max =
        key === "legacyStreamTotalTimeoutMs" || key === "websocketIdleTimeoutMs"
          ? 1800
          : 600;
      const parsed = parseIntegerDraft(timeoutDrafts[key], 1, max);
      if (parsed === null) {
        setTimeoutsError(
          t("codex.apiService.validation.numberRange", {
            min: 1,
            max,
            defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
          }),
        );
        return null;
      }
      parsedSeconds.set(key, parsed);
    }
    if (
      (parsedSeconds.get("legacyStreamTotalTimeoutMs") ?? 0) <
      (parsedSeconds.get("legacyStreamIdleTimeoutMs") ?? 0)
    ) {
      setTimeoutsError(
        t(
          "codex.apiService.timeouts.totalGteIdle",
          "旧 API 流总超时不能小于流空闲超时",
        ),
      );
      return null;
    }
    const attempts = parseIntegerDraft(
      timeoutDrafts.sidecarStreamOpenMaxAttempts,
      1,
      3,
    );
    if (attempts === null) {
      setTimeoutsError(
        t("codex.apiService.validation.numberRange", {
          min: 1,
          max: 3,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return null;
    }
    const keepalive = parseIntegerDraft(
      timeoutDrafts.sidecarStreamKeepaliveSeconds,
      0,
      300,
    );
    if (keepalive === null) {
      setTimeoutsError(
        t("codex.apiService.validation.numberRange", {
          min: 0,
          max: 300,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return null;
    }
    const sidecarBootstrapRetries = parseIntegerDraft(
      timeoutDrafts.sidecarStreamingBootstrapRetries,
      0,
      5,
    );
    if (sidecarBootstrapRetries === null) {
      setTimeoutsError(
        t("codex.apiService.validation.numberRange", {
          min: 0,
          max: 5,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return null;
    }
    const upstreamSendRetryAttempts = parseIntegerDraft(
      timeoutDrafts.upstreamSendRetryAttempts,
      0,
      5,
    );
    const singleAccountStatusRetryAttempts = parseIntegerDraft(
      timeoutDrafts.singleAccountStatusRetryAttempts,
      0,
      5,
    );
    if (
      upstreamSendRetryAttempts === null ||
      singleAccountStatusRetryAttempts === null
    ) {
      setTimeoutsError(
        t("codex.apiService.validation.numberRange", {
          min: 0,
          max: 5,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return null;
    }
    const upstreamSendRetryBaseDelayMs = parseIntegerDraft(
      timeoutDrafts.upstreamSendRetryBaseDelayMs,
      50,
      10000,
    );
    const upstreamSendRetryMaxDelayMs = parseIntegerDraft(
      timeoutDrafts.upstreamSendRetryMaxDelayMs,
      50,
      10000,
    );
    const singleAccountStatusRetryBaseDelayMs = parseIntegerDraft(
      timeoutDrafts.singleAccountStatusRetryBaseDelayMs,
      50,
      10000,
    );
    const singleAccountStatusRetryMaxDelayMs = parseIntegerDraft(
      timeoutDrafts.singleAccountStatusRetryMaxDelayMs,
      50,
      10000,
    );
    if (
      upstreamSendRetryBaseDelayMs === null ||
      upstreamSendRetryMaxDelayMs === null ||
      singleAccountStatusRetryBaseDelayMs === null ||
      singleAccountStatusRetryMaxDelayMs === null
    ) {
      setTimeoutsError(
        t("codex.apiService.validation.numberRange", {
          min: 50,
          max: 10000,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return null;
    }
    if (upstreamSendRetryMaxDelayMs < upstreamSendRetryBaseDelayMs) {
      setTimeoutsError(
        t(
          "codex.apiService.timeouts.maxDelayGteBase",
          "最大延迟不能小于基础延迟",
        ),
      );
      return null;
    }
    if (
      singleAccountStatusRetryMaxDelayMs < singleAccountStatusRetryBaseDelayMs
    ) {
      setTimeoutsError(
        t(
          "codex.apiService.timeouts.maxDelayGteBase",
          "最大延迟不能小于基础延迟",
        ),
      );
      return null;
    }
    const payload: CodexLocalAccessTimeouts = {
      legacyRequestReadTimeoutMs:
        (parsedSeconds.get("legacyRequestReadTimeoutMs") ?? 15) * 1000,
      legacyUpstreamConnectTimeoutMs:
        (parsedSeconds.get("legacyUpstreamConnectTimeoutMs") ?? 20) * 1000,
      legacyStreamIdleTimeoutMs:
        (parsedSeconds.get("legacyStreamIdleTimeoutMs") ?? 60) * 1000,
      legacyStreamTotalTimeoutMs:
        (parsedSeconds.get("legacyStreamTotalTimeoutMs") ?? 180) * 1000,
      sidecarStreamOpenTimeoutMs:
        (parsedSeconds.get("sidecarStreamOpenTimeoutMs") ?? 10) * 1000,
      sidecarStreamIdleTimeoutMs:
        (parsedSeconds.get("sidecarStreamIdleTimeoutMs") ?? 60) * 1000,
      sidecarImageStreamOpenTimeoutMs:
        (parsedSeconds.get("sidecarImageStreamOpenTimeoutMs") ?? 10) * 1000,
      sidecarImageStreamIdleTimeoutMs:
        (parsedSeconds.get("sidecarImageStreamIdleTimeoutMs") ?? 60) * 1000,
      sidecarStreamOpenMaxAttempts: attempts,
      sidecarStreamKeepaliveSeconds: keepalive,
      websocketConnectTimeoutMs:
        (parsedSeconds.get("websocketConnectTimeoutMs") ?? 30) * 1000,
      websocketInitialMessageTimeoutMs:
        (parsedSeconds.get("websocketInitialMessageTimeoutMs") ?? 30) * 1000,
      websocketIdleTimeoutMs:
        (parsedSeconds.get("websocketIdleTimeoutMs") ?? 300) * 1000,
      websocketHeartbeatIntervalMs:
        (parsedSeconds.get("websocketHeartbeatIntervalMs") ?? 30) * 1000,
      upstreamSendRetryAttempts,
      upstreamSendRetryBaseDelayMs,
      upstreamSendRetryMaxDelayMs,
      singleAccountStatusRetryAttempts,
      singleAccountStatusRetryBaseDelayMs,
      singleAccountStatusRetryMaxDelayMs,
      sidecarStreamingBootstrapRetries: sidecarBootstrapRetries,
    };
    return payload;
  };

  const handleSaveTimeouts = async () => {
    const payload = parseTimeoutDraftPayload();
    if (!payload) return;
    await runAction(
      async () => {
        const next =
          await codexLocalAccessService.updateCodexLocalAccessTimeouts(
            payload,
            selectedTimeoutPresetId,
          );
        setState(next);
        setTimeoutsModalOpen(false);
      },
      t("codex.apiService.timeouts.saved", "超时与重试已保存"),
    );
  };

  const applyTimeoutPreset = (presetId: TimeoutPresetId) => {
    const builtin = presetId === "long_wait" || presetId === "short_wait";
    const preset = builtin
      ? {
          id: presetId,
          timeouts: builtinTimeoutPresetValue(
            presetId as BuiltinTimeoutPresetId,
          ),
          name: "",
        }
      : collection?.timeoutPresets.find((item) => item.id === presetId);
    if (!preset) return;
    setTimeoutsError("");
    setSelectedTimeoutPresetId(preset.id);
    setTimeoutPresetNameDraft(preset.name);
    setTimeoutDrafts(timeoutDraftsFromValue(preset.timeouts));
  };

  const handleCreateTimeoutPreset = async () => {
    const payload = parseTimeoutDraftPayload();
    if (!payload || !collection) return;
    const name = timeoutPresetNameDraft.trim();
    if (!name) {
      setTimeoutsError(
        t("codex.apiService.timeouts.presetNameRequired", "请输入方案名称"),
      );
      return;
    }
    const now = Date.now();
    const preset: CodexLocalAccessTimeoutPreset = {
      id: `custom_${crypto.randomUUID?.() ?? `${now}_${Math.random().toString(36).slice(2)}`}`,
      name,
      timeouts: payload,
      createdAt: now,
      updatedAt: now,
    };
    await runAction(
      async () => {
        await codexLocalAccessService.updateCodexLocalAccessTimeoutPresets(
          [...(collection.timeoutPresets ?? []), preset],
          preset.id,
        );
        const next =
          await codexLocalAccessService.updateCodexLocalAccessTimeouts(
            payload,
            preset.id,
          );
        setState(next);
        setSelectedTimeoutPresetId(preset.id);
        setTimeoutPresetNameDraft("");
      },
      t("codex.apiService.timeouts.presetSaved", "方案已保存"),
    );
  };

  const handleUpdateTimeoutPreset = async () => {
    const payload = parseTimeoutDraftPayload();
    if (!payload || !collection || !selectedTimeoutPresetIsCustom) return;
    const name = timeoutPresetNameDraft.trim();
    if (!name) {
      setTimeoutsError(
        t("codex.apiService.timeouts.presetNameRequired", "请输入方案名称"),
      );
      return;
    }
    const nextPresets = collection.timeoutPresets.map((preset) =>
      preset.id === selectedTimeoutPresetId
        ? { ...preset, name, timeouts: payload, updatedAt: Date.now() }
        : preset,
    );
    await runAction(
      async () => {
        await codexLocalAccessService.updateCodexLocalAccessTimeoutPresets(
          nextPresets,
          selectedTimeoutPresetId,
        );
        const next =
          await codexLocalAccessService.updateCodexLocalAccessTimeouts(
            payload,
            selectedTimeoutPresetId,
          );
        setState(next);
      },
      t("codex.apiService.timeouts.presetUpdated", "方案已更新"),
    );
  };

  const handleDeleteTimeoutPreset = async () => {
    if (!collection || !selectedTimeoutPresetIsCustom) return;
    const confirmed = await confirmDialog(
      t(
        "codex.apiService.timeouts.deletePresetConfirm",
        "确定删除这个自定义方案吗？",
      ),
      { title: t("codex.apiService.timeouts.deletePresetTitle", "删除方案") },
    );
    if (!confirmed) return;
    const nextPresets = collection.timeoutPresets.filter(
      (preset) => preset.id !== selectedTimeoutPresetId,
    );
    await runAction(
      async () => {
        await codexLocalAccessService.updateCodexLocalAccessTimeoutPresets(
          nextPresets,
          "long_wait",
        );
        const next =
          await codexLocalAccessService.updateCodexLocalAccessTimeouts(
            defaultCodexLocalAccessTimeouts(),
            "long_wait",
          );
        setState(next);
        setSelectedTimeoutPresetId("long_wait");
        setTimeoutPresetNameDraft("");
        setTimeoutDrafts(
          timeoutDraftsFromValue(defaultCodexLocalAccessTimeouts()),
        );
      },
      t("codex.apiService.timeouts.presetDeleted", "方案已删除"),
    );
  };

  const accessScopeOptions = [
    {
      value: "localhost",
      label: t("codex.localAccess.accessScopeLocalhost", "仅本机"),
    },
    { value: "lan", label: t("codex.localAccess.accessScopeLan", "局域网") },
  ];
  const clientBaseUrlHostOptions = [
    { value: "localhost", label: "localhost" },
    { value: "127.0.0.1", label: "127.0.0.1" },
  ];
  const routingOptions = [
    {
      value: "auto",
      label: t("codex.localAccess.routingStrategy.auto", "自动（推荐）"),
    },
    {
      value: "random",
      label: t("codex.localAccess.routingStrategy.random", "随机分散"),
    },
    {
      value: "single_account",
      label: t("codex.localAccess.routingStrategy.singleAccount", "固定首个账号"),
    },
    {
      value: "quota_high_first",
      label: t(
        "codex.localAccess.routingStrategy.quotaHighFirst",
        "优先高配额",
      ),
    },
    {
      value: "quota_low_first",
      label: t("codex.localAccess.routingStrategy.quotaLowFirst", "优先低配额"),
    },
    {
      value: "plan_high_first",
      label: t("codex.localAccess.routingStrategy.planHighFirst", "优先高订阅"),
    },
    {
      value: "plan_low_first",
      label: t("codex.localAccess.routingStrategy.planLowFirst", "优先低订阅"),
    },
    {
      value: "expiry_soon_first",
      label: t(
        "codex.localAccess.routingStrategy.expirySoonFirst",
        "优先近到期",
      ),
    },
    {
      value: "custom",
      label: t("codex.localAccess.routingStrategy.custom", "自定义"),
    },
  ];
  const selectedStatsRangeTitle =
    statsRange === "daily"
      ? t("codex.apiService.statsRange.today", "Today")
      : statsRange === "weekly"
        ? t("codex.apiService.statsRange.thisWeek", "This week")
        : statsRange === "monthly"
          ? t("codex.apiService.statsRange.thisMonth", "This month")
          : `${statsTimeRange.startInput} - ${statsTimeRange.endInput}`;
  const requestLogKindOptions: Array<{
    value: RequestLogKindFilter;
    label: string;
  }> = [
    { value: "all", label: t("codex.apiService.logs.allKinds", "全部类型") },
    { value: "text", label: t("codex.localAccess.requestKind.text", "文本") },
    {
      value: "image_generation",
      label: t("codex.localAccess.requestKind.imageGeneration", "生图"),
    },
    {
      value: "image_edit",
      label: t("codex.localAccess.requestKind.imageEdit", "改图"),
    },
    { value: "other", label: t("codex.localAccess.requestKind.other", "其他") },
  ];
  const requestLogStatusOptions: Array<{
    value: RequestLogStatusFilter;
    label: string;
  }> = [
    { value: "all", label: t("codex.apiService.logs.allStatuses", "全部状态") },
    {
      value: "success",
      label: t("codex.localAccess.requestLogSuccess", "成功"),
    },
    { value: "failed", label: t("codex.localAccess.requestLogFailed", "失败") },
  ];
  const requestLogInstanceOptions = useMemo(() => {
    const options: Array<{ value: string; label: string }> = [
      {
        value: "all",
        label: t("codex.apiService.logs.allInstances", "全部实例"),
      },
    ];
    const seen = new Set<string>(["all"]);
    for (const instance of codexInstances) {
      const value =
        clientInstanceIdFromUserDataDir(instance.userDataDir || "") ||
        instance.id;
      if (!value || seen.has(value)) continue;
      seen.add(value);
      options.push({
        value,
        label: instanceDisplayName(instance, t),
      });
    }
    return options;
  }, [codexInstances, t]);
  const requestLogGatewayModeOptions: Array<{
    value: RequestLogGatewayModeFilter;
    label: string;
  }> = [
    {
      value: "all",
      label: t("codex.apiService.logs.allGatewayModes", "全部模式"),
    },
    {
      value: "sidecar",
      label: t("codex.localAccess.gatewayModeNewLabel", "API 服务-新"),
    },
    {
      value: "legacy",
      label: t("codex.localAccess.gatewayModeOldLabel", "API 服务-旧"),
    },
  ];
  const gatewayModeOptions: Array<{
    value: CodexLocalAccessGatewayMode;
    label: string;
  }> = [
    {
      value: "sidecar",
      label: t("codex.localAccess.gatewayModeNewLabel", "API 服务-新"),
    },
    {
      value: "legacy",
      label: t("codex.localAccess.gatewayModeOldLabel", "API 服务-旧"),
    },
  ];
  const serviceTabs: Array<{
    key: ServiceTab;
    label: string;
    icon: ReactNode;
  }> = [
    {
      key: "overview",
      label: t("codex.apiService.tabs.overview", "服务总览"),
      icon: <CodexIcon className="tab-icon" />,
    },
    {
      key: "keys",
      label: t("codex.apiService.tabs.keys", "客户端 Key"),
      icon: <KeyRound className="tab-icon" />,
    },
    {
      key: "accounts",
      label: t("codex.apiService.tabs.accounts", "账号池"),
      icon: <Users className="tab-icon" />,
    },
    {
      key: "models",
      label: t("codex.apiService.tabs.models", "模型与能力"),
      icon: <Image className="tab-icon" />,
    },
    {
      key: "logs",
      label: t("codex.apiService.tabs.logs", "统计与日志"),
      icon: <Activity className="tab-icon" />,
    },
  ];
  const statsLogTabs: Array<{ key: StatsLogTab; label: string }> = [
    { key: "logs", label: t("codex.localAccess.requestLogTitle", "请求日志") },
    {
      key: "accounts",
      label: t("codex.localAccess.accountStatsTitle", "按账号统计"),
    },
    {
      key: "models",
      label: t("codex.localAccess.modelStatsTitle", "按模型统计"),
    },
    {
      key: "keys",
      label: t("codex.localAccess.apiKeyStatsTitle", "按 Key 统计"),
    },
  ];

  const summaryCards = [
    {
      key: "requests",
      label: t("codex.localAccess.stats.requests", "总请求数"),
      value: formatCompactNumber(totals?.requestCount ?? 0),
      detail: formatRequestResultDetail(totals),
    },
    {
      key: "images",
      label: t("codex.localAccess.stats.images", "图片请求"),
      value: formatCompactNumber(totals?.imageRequestCount ?? 0),
      detail: t("codex.localAccess.stats.imagesDetail", {
        generate: formatCompactNumber(totals?.imageGenerationRequestCount ?? 0),
        edit: formatCompactNumber(totals?.imageEditRequestCount ?? 0),
        blocked: formatCompactNumber(
          totals?.imageGenerationCapabilityFailureCount ?? 0,
        ),
        defaultValue: "生成 {{generate}} / 编辑 {{edit}} / 权限 {{blocked}}",
      }),
    },
    {
      key: "tokens",
      label: t("codex.localAccess.stats.tokens", "总 Token 数"),
      value: formatCompactNumber(totals?.totalTokens ?? 0),
      detail: `${t("codex.localAccess.stats.tokensDetail", {
        input: formatCompactNumber(totals?.inputTokens ?? 0),
        output: formatCompactNumber(totals?.outputTokens ?? 0),
        defaultValue: "输入 {{input}} / 输出 {{output}}",
      })} / ${t("codex.localAccess.stats.cached", "缓存")} ${formatCompactNumber(totals?.cachedTokens ?? 0)}`,
    },
    {
      key: "cost",
      label: t("codex.localAccess.stats.estimatedCost", "估算价值"),
      value: formatUsdCost(totals?.estimatedCostUsd ?? 0),
      detail: t(
        "codex.localAccess.stats.estimatedCostDetail",
        "按当前请求价格快照累计",
      ),
    },
    {
      key: "latency",
      label: t("codex.localAccess.stats.avgLatency", "平均延迟"),
      value: formatLatencyMs(avgLatency),
      detail: t("codex.localAccess.stats.successRate", {
        rate: successRate,
        defaultValue: "成功率 {{rate}}%",
      }),
    },
  ];
  const requestLogEvents = requestLogResult?.events ?? [];
  const requestLogTotal = requestLogResult?.total ?? 0;
  const requestLogCurrentPage = requestLogResult?.page ?? requestLogPage;
  const requestLogTotalPages = requestLogResult?.totalPages ?? 1;
  const requestLogRangeStart =
    requestLogTotal === 0
      ? 0
      : (requestLogCurrentPage - 1) * requestLogPageSize + 1;
  const requestLogRangeEnd =
    requestLogTotal === 0
      ? 0
      : Math.min(requestLogTotal, requestLogCurrentPage * requestLogPageSize);
  const hasRequestLogFilters = Boolean(
    requestLogKindFilter !== "all" ||
    requestLogStatusFilter !== "all" ||
    requestLogGatewayModeFilter !== "all" ||
    requestLogInstanceQuery !== "all" ||
    requestLogModelQuery.trim() ||
    requestLogAccountQuery.trim() ||
    requestLogApiKeyQuery.trim() ||
    requestLogErrorQuery.trim(),
  );
  const clearRequestLogFilters = () => {
    setRequestLogKindFilter("all");
    setRequestLogStatusFilter("all");
    setRequestLogGatewayModeFilter("all");
    setRequestLogModelQuery("");
    setRequestLogAccountQuery("");
    setRequestLogApiKeyQuery("");
    setRequestLogInstanceQuery("all");
    setRequestLogErrorQuery("");
  };

  return (
    <div className="codex-api-service-page">
      <div className="page-top-strip">
        <div className="page-top-strip-left">
          <span className="page-top-strip-label">
            {t("settings.general.account", "账号")}
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
                  <SingleSelectDropdown
                    value={gatewayMode}
                    options={gatewayModeOptions}
                    onChange={(value) =>
                      void handleUpdateGatewayMode(
                        value as CodexLocalAccessGatewayMode,
                      )
                    }
                    className="codex-api-service-title-mode-select"
                    menuClassName="codex-local-access-title-mode-menu"
                    menuWidth={116}
                    menuMaxHeight={120}
                    disabled={
                      busy ||
                      activating ||
                      state?.preparing ||
                      testDialogRunning ||
                      !collection
                    }
                    ariaLabel={t(
                      "codex.localAccess.gatewayModeLabel",
                      "网关模式",
                    )}
                  />
                </div>
              </div>
            </div>
          </div>
          <div className="codex-api-service-hero-actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void reloadState()}
              disabled={busy || activating || testDialogRunning}
            >
              <RefreshCw size={14} />
              {t("codex.localAccess.refreshStats", "刷新统计")}
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
                {t("codex.apiService.usage.title", "用量统计")}
              </strong>
              <span>
                {selectedStatsRangeTitle}
                {stats?.updatedAt
                  ? ` · ${t("codex.apiService.usage.lastRecorded", "最近入账")} ${formatDateTime(stats.updatedAt)}`
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
                        gatewayMode === "legacy"
                          ? "codex.localAccess.upstreamProxyUrlPlaceholderLegacy"
                          : "codex.localAccess.upstreamProxyUrlPlaceholderSidecar",
                        gatewayMode === "legacy"
                          ? "留空依次使用全局代理、环境代理或系统代理"
                          : "留空用全局代理",
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
                          ? t("common.enabled", "已启用")
                          : t("common.disabled", "已停用")}
                      </span>
                      <span className="codex-api-service-key-last-used">
                        <small>
                          {t("codex.apiService.keys.lastUsed", "最近使用")}
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
                          title={t("common.copy", "复制")}
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
                              ? t("common.disable", "停用")
                              : t("common.enable", "启用")
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
                            "轮换 Key",
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
                          title={t("common.delete", "删除")}
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
                            {t("codex.apiService.keys.routingAccounts", "分流账号")}
                          </span>
                          <strong>
                            {persistedInheritAccountPool
                              ? t(
                                  "codex.apiService.keys.accountScopeInheritedCount",
                                  "继承服务池 · {{count}} 个账号",
                                  { count: memberIds.length },
                                )
                              : persistedAccountIds.length === 0
                                ? t(
                                    "codex.apiService.keys.accountScopeUnavailable",
                                    "无可用账号",
                                  )
                                : t(
                                    "codex.apiService.keys.accountScopeCount",
                                    "自定义 · {{selected}}/{{total}} 个账号",
                                    {
                                      selected: persistedAccountIds.length,
                                      total: keySelectableAccountIds.length,
                                    },
                                  )}
                          </strong>
                        </div>
                      </div>
                      <div
                        key={`${statsRange}:${statsTimeRange.startAt}:${statsTimeRange.endAt}`}
                        className="api-key-usage-grid"
                        aria-live="polite"
                        aria-label={`${selectedStatsRangeTitle} Key 用量`}
                      >
                        <div className="api-key-usage-grid-head">
                          <Activity size={14} />
                          <span>{selectedStatsRangeTitle}</span>
                        </div>
                        <div>
                          <span>
                            {t("codex.localAccess.stats.requests", "请求")}
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
                            {t("codex.localAccess.stats.successRateLabel", "成功率")}
                          </span>
                          <strong>{keySuccessRate}%</strong>
                        </div>
                        <div>
                          <span>
                            {t("codex.localAccess.stats.estimatedCost", "估算费用")}
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
                    disabled={busy || !collection || gatewayMode !== "sidecar"}
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
                    disabled={busy || !collection || gatewayMode !== "sidecar"}
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
                    disabled={busy || !collection || gatewayMode !== "sidecar"}
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
                        "模型 ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.accountFilter", "账号")}
                    </span>
                    <input
                      value={requestLogAccountQuery}
                      onChange={(event) =>
                        setRequestLogAccountQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.accountPlaceholder",
                        "邮箱或账号 ID",
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
                        "名称或 ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.instanceFilter", "实例")}
                    </span>
                    <SingleSelectDropdown
                      value={requestLogInstanceQuery}
                      options={requestLogInstanceOptions}
                      onChange={setRequestLogInstanceQuery}
                      ariaLabel={t(
                        "codex.apiService.logs.instanceFilter",
                        "实例",
                      )}
                      placeholder={t(
                        "codex.apiService.logs.allInstances",
                        "全部实例",
                      )}
                    />
                  </label>
                  <label>
                    <span>{t("codex.apiService.logs.kindFilter", "类型")}</span>
                    <SingleSelectDropdown
                      value={requestLogKindFilter}
                      options={requestLogKindOptions}
                      onChange={(value) =>
                        setRequestLogKindFilter(value as RequestLogKindFilter)
                      }
                      ariaLabel={t("codex.apiService.logs.kindFilter", "类型")}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.statusFilter", "状态")}
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
                        "状态",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.gatewayModeFilter", "模式")}
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
                        "模式",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.errorFilter", "错误")}
                    </span>
                    <input
                      value={requestLogErrorQuery}
                      onChange={(event) =>
                        setRequestLogErrorQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.errorPlaceholder",
                        "错误分类",
                      )}
                    />
                  </label>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={clearRequestLogFilters}
                    disabled={!hasRequestLogFilters}
                  >
                    {t("codex.apiService.logs.clearFilters", "清除筛选")}
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
                  {t("codex.apiService.timeouts.legacyTitle", "旧 API 服务")}
                </h3>
                <div className="codex-api-service-policy-grid">
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.requestRead", "请求读取")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.legacyRequestReadTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "legacyRequestReadTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.connect", "上游连接")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={timeoutDrafts.legacyUpstreamConnectTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "legacyUpstreamConnectTimeoutMs",
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
                      value={timeoutDrafts.legacyStreamIdleTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "legacyStreamIdleTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.timeouts.streamTotal", "流总超时")}
                    </span>
                    <input
                      type="number"
                      min={1}
                      max={1800}
                      value={timeoutDrafts.legacyStreamTotalTimeoutMs}
                      onChange={(event) =>
                        updateTimeoutDraft(
                          "legacyStreamTotalTimeoutMs",
                          event.target.value,
                        )
                      }
                    />
                  </label>
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
        onSaveAccounts={({
          accountIds,
          restrictFreeAccounts,
          backupAccountIds,
        }) =>
          handleSaveMembersFromModal(
            accountIds,
            restrictFreeAccounts,
            backupAccountIds,
          )
        }
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
      />
    </div>
  );
}

export default CodexApiServicePage;
