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
  Image,
  KeyRound,
  Users,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { CodexIcon } from "../components/icons/CodexIcon";
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from "../stores/usePlatformLayoutStore";
import { getPlatformLabel } from "../utils/platformMeta";
import { presentWindowsOperationError } from "../utils/windowsOperationDialog";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import {
  reconcileCodexApiKeyScopeAccountIds,
} from "../utils/codexApiKeyAccountScope";
import { resolveCodexApiServiceCompatibilityBaseUrls } from "../utils/codexApiServiceCompatibility";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import * as codexInstanceService from "../services/codexInstanceService";
import {
  getCodexAccountGroups,
  type CodexAccountGroup,
} from "../services/codexAccountGroupService";
import type { CodexAccount, CodexApiModelMapping } from "../types/codex";
import { isCodexApiKeyAccount } from "../types/codex";
import { updateCodexAccountApiModelMappings } from "../services/codexService";
import { parseContextWindowDrafts } from "../utils/codexModelContextWindows";
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
  CodexLocalAccessImageGenerationPolicy,
  CodexLocalAccessModelAlias,
  CodexLocalAccessModelPricing,
  CodexLocalAccessRequestKind,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessState,
  CodexLocalAccessStatsWindow,
  CodexLocalAccessTimeoutPreset,
  CodexLocalAccessTimeouts,
  CodexLocalAccessUsageStats,
  CodexLocalAccessUsageEventPage,
} from "../types/codexLocalAccess";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import {
  summarizeCodexQuotaPool,
} from "../utils/codexQuotaPool";
import { filterCodexLocalAccessAccountIds } from "../utils/codexLocalAccessAccounts";
import {
  isCodexLocalAccessRiskNoticeDismissed,
} from "../utils/codexLocalAccessRiskNotice";
import { requestCodexOpenAddAccount } from "../utils/codexAddAccountRequest";
import { scrollElementTo } from "../utils/reducedMotion";
import { useCodexAccountOverviewMemberView } from "../hooks/useCodexAccountOverviewMemberView";
import {
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "../utils/codexStatsRange";
import {
  persistCodexStatsRangeSelection,
  readCodexStatsRangeSelection,
} from "../utils/codexStatsRangePreference";
import "./CodexApiServicePage.css";
import { CodexApiServiceView } from "./CodexApiServiceView";


type ServiceTab = "overview" | "keys" | "accounts" | "models" | "logs";
type StatsLogTab = "accounts" | "logs" | "models" | "keys";
export type CopyField =
  | "baseUrl"
  | "lanBaseUrl"
  | "apiKey"
  | "modelId"
  | `compat:${string}`
  | `apiKey:${string}`;
export type RequestLogKindFilter = "all" | CodexLocalAccessRequestKind;
export type RequestLogStatusFilter = "all" | "success" | "failed";
export type RequestLogGatewayModeFilter = "all" | "legacy" | "sidecar";
type BuiltinTimeoutPresetId = "long_wait" | "short_wait";
type TimeoutPresetId = BuiltinTimeoutPresetId | string;

interface ApiKeyPolicyDraft {
  tokenLimit: string;
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

function parseTokenLimitDraft(value: string): number {
  const normalized = value.trim().toLowerCase().replace(/[,_\s]/g, "");
  if (!normalized) return 0;
  const match = normalized.match(/^(\d+(?:\.\d+)?)([kmb])?$/);
  if (!match) return Number.NaN;
  const amount = Number(match[1]);
  const multiplier =
    match[2] === "k"
      ? 1_000
      : match[2] === "m"
        ? 1_000_000
        : match[2] === "b"
          ? 1_000_000_000
          : 1;
  const tokens = amount * multiplier;
  if (
    !Number.isSafeInteger(tokens) ||
    tokens < 0 ||
    (tokens > 0 && tokens < 1)
  ) {
    return Number.NaN;
  }
  return tokens;
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
    tokenLimit: apiKey.tokenLimit ? String(apiKey.tokenLimit) : "",
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
    draft.tokenLimit !== persisted.tokenLimit ||
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

const DEEPSEEK_OFFICIAL_API_MODEL_MAPPINGS: CodexApiModelMapping[] = [
  { client_model: "gpt-5.6-sol", upstream_model: "deepseek-v4-flash" },
  { client_model: "gpt-5.6-terra", upstream_model: "deepseek-v4-pro" },
  { client_model: "deepseek-v4-flash", upstream_model: "deepseek-v4-flash" },
  { client_model: "deepseek-v4-pro", upstream_model: "deepseek-v4-pro" },
];

type AccountModelMappingDraft = {
  clientModel: string;
  upstreamModel: string;
  contextWindow: string;
};

function mappingDraftsFromAccount(account: CodexAccount): AccountModelMappingDraft[] {
  const windows = account.api_model_context_windows ?? {};
  const rows = (account.api_model_mappings ?? []).map((item) => ({
    clientModel: item.client_model,
    upstreamModel: item.upstream_model,
    contextWindow:
      windows[item.client_model] != null
        ? String(windows[item.client_model])
        : windows[item.upstream_model] != null
          ? String(windows[item.upstream_model])
          : "",
  }));
  return rows.length > 0
    ? rows
    : [{ clientModel: "", upstreamModel: "", contextWindow: "" }];
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
  mode: RequestLogGatewayModeFilter | null | undefined,
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
    return t("instances.defaultName", "Default Instance");
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
    return t("codex.apiService.logs.instanceUnknown", "Instance -");
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

export function useCodexApiServicePageController() {
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
  const [statsSelection, setStatsSelection] = useState(() =>
    readCodexStatsRangeSelection(STATS_RANGE_STORAGE_KEY),
  );
  const { key: statsRange, range: statsTimeRange } = statsSelection;
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
  const [sidecarRestarting, setSidecarRestarting] = useState(false);
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
  const [accountModelMappingsOpen, setAccountModelMappingsOpen] =
    useState(false);
  const [accountModelMappingDrafts, setAccountModelMappingDrafts] = useState<
    Record<string, AccountModelMappingDraft[]>
  >({});
  const [accountModelMappingError, setAccountModelMappingError] = useState("");
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
  const [sessionAffinityDraft, setSessionAffinityDraft] = useState(true);
  const [sessionAffinityTtlDraft, setSessionAffinityTtlDraft] =
    useState("3600");
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
  const [statsDetailsRefreshing, setStatsDetailsRefreshing] = useState(false);
  const [requestLogRefreshNonce, setRequestLogRefreshNonce] = useState(0);
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
      if (!stats || statsRange === "custom" || statsRange === "rolling7d") {
        return filteredStatsWindow;
      }
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
  const mappingMemberAccounts = useMemo(
    () => memberAccounts.filter((account) => isCodexApiKeyAccount(account)),
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
    totals && totals.successCount > 0
      ? totals.totalLatencyMs / totals.successCount
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

  const handleRefreshStatsDetails = useCallback(async () => {
    if (statsDetailsRefreshing) return;
    setStatsDetailsRefreshing(true);
    setStatsRangeError("");
    setRequestLogError("");
    try {
      const [nextStats] = await Promise.all([
        codexLocalAccessService.queryCodexLocalAccessStats(
          statsTimeRange.startAt,
          statsTimeRange.endAt,
        ),
        reloadState(),
      ]);
      if (mountedRef.current) {
        setFilteredStatsWindow(nextStats);
        setRequestLogRefreshNonce((value) => value + 1);
      }
    } catch (refreshError) {
      if (mountedRef.current) {
        setStatsRangeError(String(refreshError).replace(/^Error:\s*/, ""));
      }
    } finally {
      if (mountedRef.current) setStatsDetailsRefreshing(false);
    }
  }, [reloadState, statsDetailsRefreshing, statsTimeRange.endAt, statsTimeRange.startAt]);

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
    let disposed = false;
    let unlistenTauri: (() => void) | null = null;
    void listen("codex-local-access-state-updated", onUpdated).then((dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlistenTauri = dispose;
    });
    return () => {
      disposed = true;
      mountedRef.current = false;
      window.removeEventListener("codex-local-access-state-updated", onUpdated);
      unlistenTauri?.();
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
    const selection = { key, range };
    setStatsSelection(selection);
    persistCodexStatsRangeSelection(STATS_RANGE_STORAGE_KEY, selection);
  };

  const handleCustomStatsRangeApply = (range: CodexStatsTimeRange) => {
    const selection = { key: "custom" as const, range };
    setStatsSelection(selection);
    persistCodexStatsRangeSelection(STATS_RANGE_STORAGE_KEY, selection);
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
        statsRange:
          statsRange === "custom" || statsRange === "rolling7d"
            ? null
            : statsRange,
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
    requestLogRefreshNonce,
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
    setSessionAffinityDraft(collection?.sessionAffinity ?? true);
    setSessionAffinityTtlDraft(
      formatSeconds(collection?.sessionAffinityTtlMs ?? 60 * 60 * 1000),
    );
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
    collection?.sessionAffinity,
    collection?.sessionAffinityTtlMs,
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
      if (
        presentWindowsOperationError({
          error: err,
          operation: "unknown",
          summary: successText,
          retry: async () => {
            await task();
            setNotice(successText);
          },
        })
      ) {
        return;
      }
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
      if (
        presentWindowsOperationError({
          error: err,
          operation: "start_sidecar",
          summary: t("codex.localAccess.activateAction", "启动 API 服务"),
          retry: async () => {
            const next = await codexLocalAccessService.activateCodexLocalAccess();
            if (!mountedRef.current) return;
            setState(next);
            await fetchCurrentAccount();
            await refreshApiServiceCurrent();
          },
        })
      ) {
        return;
      }
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
      const retryKillPort = async () => {
        const result = await codexLocalAccessService.killCodexLocalAccessPort();
        setState(result.state);
      };
      if (
        presentWindowsOperationError({
          error: err,
          operation: "stop_process",
          summary: t("codex.localAccess.killPortTitle", "清理 API 服务端口"),
          retry: retryKillPort,
          manualContinue: retryKillPort,
        })
      ) {
        return;
      }
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setPortKilling(false);
    }
  };

  const handleRestartSidecar = async () => {
    const confirmed = await confirmDialog(
      t(
        "codex.localAccess.restartConfirmMessage",
        "将仅重启 API 服务 Sidecar，不修改账号、Token、API Key 或账号池配置。正在进行中的请求可能中断，确认继续吗？",
      ),
      {
        title: t("codex.localAccess.restartTitle", "重启 API 服务"),
        kind: "warning",
        okLabel: t("codex.localAccess.restartAction", "重启 Sidecar"),
        cancelLabel: t("common.cancel", "取消"),
      },
    );
    if (!confirmed) return;
    setSidecarRestarting(true);
    setError("");
    setNotice("");
    try {
      const next =
        await codexLocalAccessService.restartCodexLocalAccessSidecar();
      setState(next);
      setNotice(
        t("codex.localAccess.restartSuccess", "API 服务 Sidecar 已重启"),
      );
    } catch (err) {
      if (
        presentWindowsOperationError({
          error: err,
          operation: "start_sidecar",
          summary: t("codex.localAccess.restartTitle", "重启 API 服务"),
          retry: async () => {
            const next = await codexLocalAccessService.restartCodexLocalAccessSidecar();
            setState(next);
          },
        })
      ) {
        return;
      }
      setError(String(err).replace(/^Error:\s*/, ""));
    } finally {
      setSidecarRestarting(false);
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

  const saveMembers = async (
    accountIds: string[],
    restrictFreeAccounts: boolean,
    backupAccountIds?: string[],
    preferredAccountIds?: string[],
    imageGenerationAccountPolicies?: Record<string, CodexLocalAccessImageGenerationPolicy>,
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
    const nextPreferredAccountIds = (preferredAccountIds ?? []).filter((id) =>
      filteredAccountIdSet.has(id),
    );

    const next = await codexLocalAccessService.saveCodexLocalAccessAccounts(
      filteredAccountIds,
      restrictFreeAccounts,
      nextBackupAccountIds,
      nextPreferredAccountIds,
      undefined,
      undefined,
      imageGenerationAccountPolicies,
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
    preferredAccountIds?: string[],
  ) => {
    await runAction(
      () =>
        saveMembers(
          accountIds,
          restrictFreeAccounts,
          backupAccountIds,
          preferredAccountIds,
        ),
      t("codex.localAccess.saveSuccess", "API 服务集合已更新"),
    );
  };

  const handleSaveMembersFromModal = async (
    accountIds: string[],
    restrictFreeAccounts: boolean,
    backupAccountIds?: string[],
    preferredAccountIds?: string[],
    imageGenerationAccountPolicies?: Record<string, CodexLocalAccessImageGenerationPolicy>,
  ) => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await saveMembers(
        accountIds,
        restrictFreeAccounts,
        backupAccountIds,
        preferredAccountIds,
        imageGenerationAccountPolicies,
      );
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
    const preferredAccountIds = (collection.customRoutingRules ?? [])
      .filter((rule) => rule.isPreferred && remainingSet.has(rule.accountId))
      .map((rule) => rule.accountId);
    await handleSaveMembers(
      remainingIds,
      collection.restrictFreeAccounts,
      backupAccountIds,
      preferredAccountIds,
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
    const tokenLimit = parseTokenLimitDraft(draft.tokenLimit);
    if (!Number.isFinite(tokenLimit)) {
      setError(
        t(
          "codex.apiService.keys.tokenLimitInvalid",
          "Enter a valid token limit, for example 10m or 10000000",
        ),
      );
      return;
    }
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
            tokenLimit,
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

  const resetAccountModelMappingDrafts = () => {
    const next: Record<string, AccountModelMappingDraft[]> = {};
    mappingMemberAccounts.forEach((account) => {
      next[account.id] = mappingDraftsFromAccount(account);
    });
    setAccountModelMappingDrafts(next);
    setAccountModelMappingError("");
  };

  const handleOpenAccountModelMappings = () => {
    resetAccountModelMappingDrafts();
    setAccountModelMappingsOpen(true);
  };

  const handleCloseAccountModelMappings = () => {
    setAccountModelMappingsOpen(false);
    setAccountModelMappingError("");
  };

  const updateAccountModelMappingDraft = (
    accountId: string,
    index: number,
    field: keyof AccountModelMappingDraft,
    value: string,
  ) => {
    setAccountModelMappingDrafts((current) => {
      const rows = [...(current[accountId] ?? [{ clientModel: "", upstreamModel: "", contextWindow: "" }])];
      rows[index] = { ...rows[index], [field]: value };
      return { ...current, [accountId]: rows };
    });
    setAccountModelMappingError("");
  };

  const addAccountModelMappingRow = (accountId: string) => {
    setAccountModelMappingDrafts((current) => ({
      ...current,
      [accountId]: [
        ...(current[accountId] ?? []),
        { clientModel: "", upstreamModel: "", contextWindow: "" },
      ],
    }));
    setAccountModelMappingError("");
  };

  const removeAccountModelMappingRow = (accountId: string, index: number) => {
    setAccountModelMappingDrafts((current) => {
      const rows = (current[accountId] ?? []).filter((_, rowIndex) => rowIndex !== index);
      return {
        ...current,
        [accountId]: rows.length > 0 ? rows : [{ clientModel: "", upstreamModel: "", contextWindow: "" }],
      };
    });
    setAccountModelMappingError("");
  };

  const fillDeepSeekAccountModelMappings = (accountId: string) => {
    setAccountModelMappingDrafts((current) => ({
      ...current,
      [accountId]: DEEPSEEK_OFFICIAL_API_MODEL_MAPPINGS.map((item) => ({
        clientModel: item.client_model,
        upstreamModel: item.upstream_model,
        contextWindow: "",
      })),
    }));
    setAccountModelMappingError("");
  };

  const handleSaveAccountModelMappings = async () => {
    setAccountModelMappingError("");
    const payloads: Array<{
      accountId: string;
      mappings: CodexApiModelMapping[];
      windows: Record<string, number>;
    }> = [];
    for (const account of mappingMemberAccounts) {
      const rows = accountModelMappingDrafts[account.id] ?? [];
      const mappings: CodexApiModelMapping[] = [];
      const drafts: Record<string, string> = {
        ...(account.api_model_context_windows
          ? Object.fromEntries(
              Object.entries(account.api_model_context_windows).map(
                ([model, window]) => [model, String(window)],
              ),
            )
          : {}),
      };
      const seen = new Set<string>();
      for (const row of rows) {
        const clientModel = row.clientModel.trim();
        const upstreamModel = row.upstreamModel.trim();
        if (!clientModel && !upstreamModel) continue;
        if (!clientModel || !upstreamModel) {
          setAccountModelMappingError(
            t(
              "codex.apiService.accountModelMappings.incomplete",
              "请补全请求模型和发送模型",
            ),
          );
          return;
        }
        const key = clientModel.toLowerCase();
        if (seen.has(key)) {
          setAccountModelMappingError(
            t(
              "codex.apiService.accountModelMappings.duplicate",
              "请求模型不能重复",
            ),
          );
          return;
        }
        seen.add(key);
        mappings.push({ client_model: clientModel, upstream_model: upstreamModel });
        drafts[clientModel] = row.contextWindow ?? "";
      }
      const parsedWindows = parseContextWindowDrafts(
        drafts,
        Object.keys(drafts),
      );
      if (!parsedWindows.ok) {
        setAccountModelMappingError(
          t(
            "codex.api.modelCatalog.contextWindowInvalid",
            "上下文窗口必须是大于 0 的整数",
          ),
        );
        return;
      }
      payloads.push({
        accountId: account.id,
        mappings,
        windows: parsedWindows.windows,
      });
    }
    await runAction(
      async () => {
        for (const payload of payloads) {
          await updateCodexAccountApiModelMappings(
            payload.accountId,
            payload.mappings,
            payload.windows,
          );
        }
        await fetchAccounts();
        setAccountModelMappingsOpen(false);
      },
      t(
        "codex.apiService.accountModelMappings.saveSuccess",
        "账号模型映射已保存",
      ),
    );
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
    const sessionAffinityTtlSeconds = parseIntegerDraft(
      sessionAffinityTtlDraft,
      60,
      86400,
    );
    if (sessionAffinityTtlSeconds === null) {
      setError(
        t("codex.apiService.validation.numberRange", {
          min: 60,
          max: 86400,
          defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
        }),
      );
      return;
    }
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
            sessionAffinity: sessionAffinityDraft,
            sessionAffinityTtlMs: sessionAffinityTtlSeconds * 1000,
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
      const max = key === "websocketIdleTimeoutMs" ? 1800 : 600;
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
      : statsRange === "rolling7d"
        ? t("codex.sessionUsage.range.7d", "近 7 天")
      : statsRange === "weekly"
        ? t("codex.apiService.statsRange.thisWeek", "This week")
        : statsRange === "monthly"
          ? t("codex.apiService.statsRange.thisMonth", "This month")
          : `${statsTimeRange.startInput} - ${statsTimeRange.endInput}`;
  const requestLogKindOptions: Array<{
    value: RequestLogKindFilter;
    label: string;
  }> = [
    { value: "all", label: t("codex.apiService.logs.allKinds", "All Types") },
    { value: "text", label: t("codex.localAccess.requestKind.text", "Text") },
    {
      value: "image_generation",
      label: t("codex.localAccess.requestKind.imageGeneration", "Image Gen"),
    },
    {
      value: "image_edit",
      label: t("codex.localAccess.requestKind.imageEdit", "Image Edit"),
    },
    { value: "other", label: t("codex.localAccess.requestKind.other", "Other") },
  ];
  const requestLogStatusOptions: Array<{
    value: RequestLogStatusFilter;
    label: string;
  }> = [
    { value: "all", label: t("codex.apiService.logs.allStatuses", "All Statuses") },
    {
      value: "success",
      label: t("codex.localAccess.requestLogSuccess", "Success"),
    },
    { value: "failed", label: t("codex.localAccess.requestLogFailed", "Failed") },
  ];
  const requestLogInstanceOptions = useMemo(() => {
    const options: Array<{ value: string; label: string }> = [
      {
        value: "all",
        label: t("codex.apiService.logs.allInstances", "All Instances"),
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
      label: t("codex.apiService.logs.allGatewayModes", "All Modes"),
    },
    {
      value: "sidecar",
      label: t("codex.localAccess.gatewayModeNewLabel", "API Service-New"),
    },
    {
      value: "legacy",
      label: t("codex.localAccess.gatewayModeOldLabel", "API Service-Old"),
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

  return {
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
  };
}

/** 组合业务 Controller 与独立 View，保持原组件公开调用入口不变。 */
export function CodexApiServicePage() {
  const controller = useCodexApiServicePageController();
  return <CodexApiServiceView {...controller} />;
}

export default CodexApiServicePage;
