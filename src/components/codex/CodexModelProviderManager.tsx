import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { homeDir, join } from "@tauri-apps/api/path";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ArrowDownWideNarrow,
  ArrowDown,
  ArrowUp,
  Check,
  CircleAlert,
  ChevronDown,
  Copy,
  Clock,
  Database,
  ExternalLink,
  GripVertical,
  HelpCircle,
  KeyRound,
  Link2,
  LayoutGrid,
  Pencil,
  Plus,
  Rows3,
  Star,
  Trash2,
  X,
  Search,
  Settings,
  Activity,
  RefreshCw,
  RotateCw,
  Play,
} from "lucide-react";
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from "../MultiSelectFilterDropdown";
import { SingleSelectFilterDropdown } from "../SingleSelectFilterDropdown";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { AccountTagFilterDropdown } from "../AccountTagFilterDropdown";
import { PaginationControls } from "../PaginationControls";
import { useEscClose } from "../../hooks/useEscClose";
import type { CodexAccount } from "../../types/codex";
import type { InstanceProfile } from "../../types/instance";
import {
  CODEX_API_SERVICE_BIND_ID,
  CODEX_PROVIDER_GATEWAY_BIND_PREFIX,
  buildCodexProviderGatewayBindId,
} from "../../types/instance";
import type {
  CodexLocalAccessState,
  CodexLocalAccessTestFailure,
} from "../../types/codexLocalAccess";
import {
  addCodexAccountWithApiKey,
  deleteCodexAccounts,
  getCurrentCodexAccount,
  listCodexAccounts,
  syncCodexApiKeyProviderAccounts,
  updateCodexAccountName,
  updateCodexApiKeyCredentials,
  updateCodexApiKeyBoundOAuthAccount,
} from "../../services/codexService";
import { useDeepSeekDirectModelPrompt } from "./DeepSeekDirectModelModal";
import {
  isDeepSeekAccount,
  resolveDeepSeekBindAccountId,
} from "../../utils/codexDeepSeekAccess";
import {
  contextWindowDraftsFromRecord,
  parseContextWindowDrafts,
} from "../../utils/codexModelContextWindows";
import { CodexModelContextWindowTable } from "./CodexModelContextWindowTable";
import {
  getCodexLocalAccessState,
} from "../../services/codexLocalAccessService";
import {
  listInstances as listCodexInstances,
  startInstance as startCodexInstance,
  updateInstance as updateCodexInstance,
} from "../../services/codexInstanceService";
import {
  addApiKeyToCodexModelProvider,
  cancelCodexModelProviderChatTest,
  countCodexModelProviderReferences,
  createCodexModelProvider,
  deleteCodexModelProvider,
  listCodexModelProviders,
  normalizeCodexModelProviderBaseUrl,
  removeApiKeyFromCodexModelProvider,
  renameApiKeyOnCodexModelProvider,
  updateApiKeyOnCodexModelProvider,
  queryCodexModelProviderUsage,
  saveCodexModelProviderDetectedIntegrationType,
  testCodexModelProviderConnection,
  testCodexModelProviderChatBatch,
  type CodexModelProvider,
  type CodexModelProviderApiKey,
  type CodexModelProviderChatTestProgressPayload,
  type CodexModelProviderChatTestRecord,
  type CodexModelProviderChatTestTarget,
  type CodexModelProviderUsageSummary,
  updateCodexModelProvider,
} from "../../services/codexModelProviderService";
import {
  CODEX_API_KEY_USAGE_REFRESHED_EVENT,
  readCodexApiKeyUsageCache,
} from "../../services/codexApiKeyUsageRefreshService";
import {
  formatModelProviderUsageMoney,
  resolveNewApiQuotaSnapshot,
} from "../../services/modelProviderUsageService";
import { useSponsorStore } from "../../stores/useSponsorStore";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import type { Sponsor } from "../../types/sponsor";
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  CODEX_API_PROVIDER_PRESETS,
  DEEPSEEK_API_PROVIDER_ID,
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from "../../utils/codexProviderPresets";
import {
  normalizeApiKeyFunOfficialUrl,
  resolveApiKeyFunWireApi,
} from "../../utils/apikeyFunLinks";
import {
  getCodexPlanFilterKey,
  getCodexSubscriptionPresentation,
  isCodexApiKeyAccount,
} from "../../types/codex";
import { buildCodexAccountPresentation } from "../../presentation/platformAccountPresentation";
import {
  buildPaginationPageSizeStorageKey,
  usePagination,
} from "../../hooks/usePagination";
import {
  splitValidityFilterValues,
} from "../../utils/accountValidityFilter";
import {
  buildCodexPlanFilterOptions,
  createCodexPlanFilterCounts,
  incrementCodexPlanFilterCount,
} from "../../utils/codexAccountOverview";
import {
  resolveCodexProviderCapabilityProfile,
  type CodexProviderEnableModePreference,
  type CodexProviderWireApi,
} from "../../utils/codexProviderGateway";
import { emitAccountsChanged } from "../../utils/accountSyncEvents";
import {
  resolveCodexModelProviderAccountName,
  shouldSyncCodexModelProviderAccountName,
} from "../../utils/codexModelProviderAccountName";
import { findCodexAccountsReferencingModelProvider } from "../../utils/codexModelProviderAccountSync";
import { CodexQuickConfigCard } from "./CodexQuickConfigCard";
import {
  CodexServicePanelModal,
  type CodexServicePanelActionItem,
  type CodexServicePanelMetricItem,
} from "./CodexServicePanelModal";

const DEFAULT_INSTANCE_ID = "__default__";
const OAUTH_BINDING_PAGE_SIZE_OPTIONS = [10, 20, 50] as const;
type OAuthBindingSortBy = "account" | "created_at" | "last_used" | "plan";
type ProviderSortBy = "name" | "created_at" | "custom";
type InstanceSortField = "createdAt" | "lastLaunchedAt";
type InstanceSortDirection = "asc" | "desc";

interface CodexModelProviderManagerProps {
  accounts: CodexAccount[];
  onProvidersChanged?: (providers: CodexModelProvider[]) => void;
  onManageModelPresets?: () => void;
}

function maskApiKey(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  if (trimmed.length <= 8) return `${trimmed.slice(0, 2)}****`;
  return `${trimmed.slice(0, 4)}****${trimmed.slice(-4)}`;
}

function parseModelCatalogText(value: string): string[] {
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

function parseVisionModelText(value: string): Record<string, { supportsVision: boolean }> {
  const capabilities: Record<string, { supportsVision: boolean }> = {};
  value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean)
    .forEach((model) => {
      capabilities[model.toLowerCase()] = { supportsVision: true };
    });
  return capabilities;
}

function visionModelTextFromCapabilities(
  capabilities?: Record<string, { supportsVision?: boolean }>,
): string {
  if (!capabilities) return "";
  return Object.entries(capabilities)
    .filter(([, capability]) => capability.supportsVision === true)
    .map(([model]) => model)
    .sort()
    .join("\n");
}

function isSponsorProvider(
  provider: CodexModelProvider,
  sponsorTemplates: SponsorProviderTemplate[],
): boolean {
  if (provider.sourceTag) {
    return sponsorTemplates.some((template) => template.id === provider.sourceTag);
  }
  const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(provider.baseUrl);
  return sponsorTemplates.some(
    (template) =>
      normalizeCodexModelProviderBaseUrl(template.baseUrl) === normalizedBaseUrl,
  );
}

function readCodexInstanceSortPreference(): {
  field: InstanceSortField;
  direction: InstanceSortDirection;
} {
  const sortField = localStorage.getItem("agtools.codex.instances.sort_field");
  const sortDirection = localStorage.getItem("agtools.codex.instances.sort_direction");
  return {
    field: sortField === "lastLaunchedAt" ? "lastLaunchedAt" : "createdAt",
    direction: sortDirection === "desc" ? "desc" : "asc",
  };
}

const CODEX_PROVIDER_CUSTOM_SORT_ORDER_KEY =
  "agtools.codex.modelProviders.custom_sort_order.v1";
const CODEX_PROVIDER_CUSTOM_SORT_ACTIVE_KEY =
  "agtools.codex.modelProviders.custom_sort_active.v1";
const PROVIDER_USAGE_CACHE_KEY = "agtools.codex.modelProviders.usage.cache.v1";

type ProviderUsageState = {
  loading: boolean;
  summary?: CodexModelProviderUsageSummary;
  error?: string;
  unavailable?: boolean;
  updatedAt?: number;
};

function readProviderUsageCache(): Record<string, ProviderUsageState> {
  try {
    const raw = localStorage.getItem(PROVIDER_USAGE_CACHE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (!parsed || typeof parsed !== "object") return {};
    const next: Record<string, ProviderUsageState> = {};
    Object.entries(parsed).forEach(([providerId, value]) => {
      if (!value || typeof value !== "object") return;
      const item = value as {
        summary?: CodexModelProviderUsageSummary;
        error?: string;
        unavailable?: boolean;
        updatedAt?: number;
      };
      next[providerId] = {
        loading: false,
        summary: item.summary,
        error: typeof item.error === "string" ? item.error : undefined,
        unavailable: item.unavailable === true,
        updatedAt:
          typeof item.updatedAt === "number" && Number.isFinite(item.updatedAt)
            ? item.updatedAt
            : undefined,
      };
    });
    return next;
  } catch {
    return {};
  }
}

function writeProviderUsageCache(value: Record<string, ProviderUsageState>): void {
  try {
    localStorage.setItem(
      PROVIDER_USAGE_CACHE_KEY,
      JSON.stringify(
        Object.fromEntries(
          Object.entries(value).map(([providerId, item]) => [
            providerId,
            {
              summary: item.summary,
              error: item.error,
              unavailable: item.unavailable === true,
              updatedAt: item.updatedAt,
            },
          ]),
        ),
      ),
    );
  } catch {
    // ignore persistence failures
  }
}

function getAccountUsageForProviderApiKey(
  provider: CodexModelProvider,
  apiKey: CodexModelProviderApiKey,
  accounts: CodexAccount[],
): ProviderUsageState | null {
  const normalizedProviderBaseUrl = normalizeCodexModelProviderBaseUrl(
    provider.baseUrl,
  );
  if (!normalizedProviderBaseUrl || !apiKey.apiKey.trim()) return null;

  const matchedAccount = accounts.find(
    (account) =>
      isCodexApiKeyAccount(account) &&
      account.openai_api_key?.trim() === apiKey.apiKey.trim() &&
      normalizeCodexModelProviderBaseUrl(account.api_base_url ?? "") ===
        normalizedProviderBaseUrl,
  );
  if (!matchedAccount) return null;

  const accountUsage = readCodexApiKeyUsageCache()[matchedAccount.id];
  if (!accountUsage) return null;
  return {
    loading: false,
    summary: accountUsage.summary,
    error: accountUsage.error,
    unavailable: accountUsage.unavailable,
    updatedAt: accountUsage.updatedAt,
  };
}

function readCodexProviderCustomSortOrder(): string[] {
  try {
    const raw = localStorage.getItem(CODEX_PROVIDER_CUSTOM_SORT_ORDER_KEY);
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

function writeCodexProviderCustomSortOrder(providerIds: string[]): void {
  try {
    localStorage.setItem(
      CODEX_PROVIDER_CUSTOM_SORT_ORDER_KEY,
      JSON.stringify(providerIds),
    );
  } catch {
    // ignore persistence failures
  }
}

function readCodexProviderCustomSortActive(): boolean {
  try {
    return localStorage.getItem(CODEX_PROVIDER_CUSTOM_SORT_ACTIVE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeCodexProviderCustomSortActive(active: boolean): void {
  try {
    localStorage.setItem(
      CODEX_PROVIDER_CUSTOM_SORT_ACTIVE_KEY,
      active ? "1" : "0",
    );
  } catch {
    // ignore persistence failures
  }
}

interface ProviderFormState {
  providerId: string | null;
  name: string;
  baseUrl: string;
  modelCatalogText: string;
  modelContextWindowsDraft: Record<string, string>;
  supportsVision: boolean;
  visionModelText: string;
  visionRoutingModel: string;
  website: string;
  apiKeyUrl: string;
  wireApi: CodexProviderWireApi;
  supportsWebsockets: boolean;
  enableModePreference: CodexProviderEnableModePreference;
  integrationType: "sub2api" | "new_api" | "";
  newApiKeyName: string;
  newApiKey: string;
}

interface EditingApiKeyState {
  providerId: string;
  apiKeyId: string;
  originalApiKey: string;
  apiKey: string;
  name: string;
}

const EMPTY_FORM: ProviderFormState = {
  providerId: null,
  name: "",
  baseUrl: "",
  modelCatalogText: "",
  modelContextWindowsDraft: {},
  supportsVision: false,
  visionModelText: "",
  visionRoutingModel: "",
  website: "",
  apiKeyUrl: "",
  wireApi: "responses",
  supportsWebsockets: false,
  enableModePreference: "direct",
  integrationType: "",
  newApiKeyName: "",
  newApiKey: "",
};

interface SponsorProviderTemplate {
  id: string;
  sponsor: Sponsor;
  name: string;
  baseUrl: string;
  modelCatalog: string[];
  supportsVision: boolean;
  website: string;
  apiKeyUrl: string;
  wireApi?: CodexProviderWireApi | null;
  integrationType?: "sub2api" | "new_api" | null;
}

interface ProviderPreviewPaths {
  providerStorePath: string;
  codexConfigPath: string;
  codexAuthPath: string;
}

type ProviderBatchTestStatus =
  | "pending"
  | "running"
  | "success"
  | "error"
  | "cancelled";
type ProviderBatchTestFilter = "all" | ProviderBatchTestStatus;

type ProviderBatchTestRecordView = CodexModelProviderChatTestRecord & {
  status: ProviderBatchTestStatus;
};

interface ProviderBatchTestSession {
  runId: string;
  total: number;
  completed: number;
  successCount: number;
  failureCount: number;
  running: boolean;
  cancelled: boolean;
  startedAt: number;
  records: ProviderBatchTestRecordView[];
  errorText?: string;
}

function resolveProviderApiKeyLabel(
  apiKey: CodexModelProviderApiKey,
  fallbackName: string,
  unnamedLabel: string,
): string {
  const name = apiKey.name?.trim();
  const label = name || fallbackName || unnamedLabel;
  return `${label}：${maskApiKey(apiKey.apiKey)}`;
}

const DEFAULT_PROVIDER_PREVIEW_PATHS: ProviderPreviewPaths = {
  providerStorePath: "~/.antigravity_cockpit/codex_model_providers.json",
  codexConfigPath: "~/.codex/config.toml",
  codexAuthPath: "~/.codex/auth.json",
};

function resolveDefaultProviderWireApi(
  presetId?: string | null,
  templateWireApi?: CodexProviderWireApi | null,
): CodexProviderWireApi {
  if (templateWireApi === "chat_completions" || templateWireApi === "responses") {
    return templateWireApi;
  }
  if (presetId && resolveCodexProviderCapabilityProfile({ presetId, baseUrl: "", wireApi: null }).wireApi === "chat_completions") {
    return "chat_completions";
  }
  return "responses";
}

function resolveEnableModePreferenceForWireApi(
  wireApi: CodexProviderWireApi,
  _presetId?: string | null,
): CodexProviderEnableModePreference {
  if (wireApi === "chat_completions") return "gateway";
  return "direct";
}

function resolveGatewayModeByWireApi(
  wireApi?: CodexProviderWireApi | null,
  _presetId?: string | null,
): "direct" | "gateway" {
  if (wireApi === "chat_completions") return "gateway";
  return "direct";
}

function resolveProviderWireApi(provider: CodexModelProvider): CodexProviderWireApi {
  return resolveCodexProviderCapabilityProfile({
    presetId: resolveCodexApiProviderPresetId(provider.baseUrl),
    baseUrl: provider.baseUrl,
    wireApi: provider.wireApi,
  }).wireApi;
}

const RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY = [
  "gpt-5.5",
  "gpt-5.4",
  "gpt-5",
  "gpt-4.1",
  "gpt-4o",
];

function isImageGenerationModelId(modelId: string): boolean {
  const lower = modelId.trim().toLowerCase();
  return (
    lower.startsWith("gpt-image") ||
    lower.startsWith("dall-e") ||
    lower.includes("image-gen")
  );
}

function selectProviderBatchTestModelId(
  wireApi: CodexProviderWireApi,
  modelCatalog?: string[] | null,
): string | null {
  const models = (modelCatalog ?? [])
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
  if (models.length === 0) return null;

  if (wireApi === "responses") {
    for (const preferred of RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY) {
      const model = models.find(
        (item) => item.toLowerCase() === preferred.toLowerCase(),
      );
      if (model) return model;
    }
    const textModel = models.find((item) => !isImageGenerationModelId(item));
    if (textModel) return textModel;
  }

  return models[0] ?? null;
}

function formatDateTime(value: number): string {
  return new Date(value).toLocaleString();
}

function formatDurationMs(value?: number | null): string {
  if (value === null || value === undefined) return "-";
  if (value < 1000) return `${value}ms`;
  return `${(value / 1000).toFixed(value < 10000 ? 1 : 0)}s`;
}

function createProviderBatchTestRunId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `provider-test-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function toProviderBatchTestRecordView(
  record: CodexModelProviderChatTestRecord,
): ProviderBatchTestRecordView {
  return {
    ...record,
    status: record.success ? "success" : "error",
  };
}

export function CodexModelProviderManager({
  accounts,
  onProvidersChanged,
}: CodexModelProviderManagerProps) {
  const { t } = useTranslation();
  const updateAccountInstanceAccess = useCodexAccountStore(
    (state) => state.updateAccountInstanceAccess,
  );
  const sponsorModule = useSponsorStore((state) => state.state.sponsorModule);
  const fetchSponsorState = useSponsorStore((state) => state.fetchState);
  const [providers, setProviders] = useState<CodexModelProvider[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{
    text: string;
    tone: "success" | "error";
  } | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [showQuickConfigModal, setShowQuickConfigModal] = useState(false);
  const [saving, setSaving] = useState(false);
  const [enablingProviderId, setEnablingProviderId] = useState<string | null>(
    null,
  );
  const deepSeekStart = useDeepSeekDirectModelPrompt();
  const [testingProviderId, setTestingProviderId] = useState<string | null>(
    null,
  );
  const [formError, setFormError] = useState<string | null>(null);
  const [form, setForm] = useState<ProviderFormState>(EMPTY_FORM);
  const [currentAccount, setCurrentAccount] = useState<CodexAccount | null>(
    null,
  );
  const [codexInstances, setCodexInstances] = useState<InstanceProfile[]>([]);
  const [localAccessState, setLocalAccessState] =
    useState<CodexLocalAccessState | null>(null);
  const [lastEnabledProviderId, setLastEnabledProviderId] = useState<
    string | null
  >(null);
  const [previewPaths, setPreviewPaths] = useState<ProviderPreviewPaths>(
    DEFAULT_PROVIDER_PREVIEW_PATHS,
  );
  const [selectedPresetId, setSelectedPresetId] = useState<string>(
    CODEX_API_PROVIDER_CUSTOM_ID,
  );
  const [selectedSponsorTemplateId, setSelectedSponsorTemplateId] = useState<string | null>(null);
  const [providerUsageMap, setProviderUsageMap] = useState<
    Record<string, ProviderUsageState>
  >(() => readProviderUsageCache());
  const [providerUsageRefreshingAll, setProviderUsageRefreshingAll] =
    useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [providerDetailId, setProviderDetailId] = useState<string | null>(null);
  const [selectedProviderApiKeyMap, setSelectedProviderApiKeyMap] = useState<
    Record<string, string>
  >({});
  const [apiKeyPickerProviderId, setApiKeyPickerProviderId] = useState<
    string | null
  >(null);
  const [instancePickerProviderId, setInstancePickerProviderId] = useState<
    string | null
  >(null);
  const [pickerSearchQuery, setPickerSearchQuery] = useState("");
  const [providerOauthPickerId, setProviderOauthPickerId] = useState<string | null>(
    null,
  );
  const [providerOauthSaving, setProviderOauthSaving] = useState(false);
  const [providerOauthSelectedAccountId, setProviderOauthSelectedAccountId] =
    useState("");
  const [providerOauthSearchQuery, setProviderOauthSearchQuery] = useState("");
  const [providerOauthFilterTypes, setProviderOauthFilterTypes] = useState<
    string[]
  >([]);
  const [providerOauthTagFilter, setProviderOauthTagFilter] = useState<string[]>(
    [],
  );
  const [providerOauthSortBy, setProviderOauthSortBy] =
    useState<OAuthBindingSortBy>("last_used");
  const [providerOauthSortDirection, setProviderOauthSortDirection] = useState<
    "asc" | "desc"
  >("desc");
  const [oauthAccounts, setOauthAccounts] = useState<CodexAccount[]>([]);
  const [selectedProviderIds, setSelectedProviderIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [providerViewMode, setProviderViewMode] = useState<"grid" | "compact">(
    "grid",
  );
  const [providerSortBy, setProviderSortBy] = useState<ProviderSortBy>(
    () => (readCodexProviderCustomSortActive() ? "custom" : "created_at"),
  );
  const [providerSortDirection, setProviderSortDirection] = useState<"asc" | "desc">("asc");
  const [providerCustomSortOrder, setProviderCustomSortOrder] = useState<
    string[]
  >(readCodexProviderCustomSortOrder);
  const [showProviderCustomSortModal, setShowProviderCustomSortModal] =
    useState(false);
  const [
    draggedProviderCustomSortId,
    setDraggedProviderCustomSortId,
  ] = useState<string | null>(null);
  const [
    providerCustomSortDropTargetId,
    setProviderCustomSortDropTargetId,
  ] = useState<string | null>(null);
  const [providerNameFilter, setProviderNameFilter] = useState<
    string[]
  >([]);
  const [batchTestModalOpen, setBatchTestModalOpen] = useState(false);
  const [batchTestStep, setBatchTestStep] = useState<"select" | "results">("select");
  const [batchTestSearchQuery, setBatchTestSearchQuery] = useState("");
  const [batchTestSelectedProviderIds, setBatchTestSelectedProviderIds] =
    useState<Set<string>>(() => new Set());
  const [batchTestSession, setBatchTestSession] =
    useState<ProviderBatchTestSession | null>(null);
  const [batchTestFilter, setBatchTestFilter] =
    useState<ProviderBatchTestFilter>("all");
  const [batchTestError, setBatchTestError] = useState<string | null>(null);
  const [batchTestDeleting, setBatchTestDeleting] = useState(false);
  const [batchTestCancelling, setBatchTestCancelling] = useState(false);
  const [batchTestResultSelectedProviderIds, setBatchTestResultSelectedProviderIds] =
    useState<Set<string>>(() => new Set());
  /** Empty string = auto-pick per provider catalog / discovery. */
  const [batchTestModelId, setBatchTestModelId] = useState("");
  const [batchTestModelCustom, setBatchTestModelCustom] = useState("");
  const [existingApiKeySearchQuery, setExistingApiKeySearchQuery] =
    useState("");
  const [editingApiKey, setEditingApiKey] = useState<EditingApiKeyState | null>(
    null,
  );
  const cancelledBatchTestRunIdsRef = useRef<Set<string>>(new Set());

  const sponsorProviderTemplates = useMemo<SponsorProviderTemplate[]>(() => {
    const sponsors = sponsorModule?.sponsors ?? [];
    const templates: SponsorProviderTemplate[] = [];
    for (const sponsor of sponsors) {
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
        website: normalizeApiKeyFunOfficialUrl(integration.website || sponsor.url),
        apiKeyUrl: normalizeApiKeyFunOfficialUrl(integration.apiKeyUrl || sponsor.url),
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
  }, [sponsorModule?.sponsors]);

  const providerCustomSortOrderIndex = useMemo(() => {
    const map = new Map<string, number>();
    providerCustomSortOrder.forEach((providerId, index) => {
      map.set(providerId, index);
    });
    return map;
  }, [providerCustomSortOrder]);

  const filteredProviders = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    let result = providers.filter((provider) => {
      const haystack = [
        provider.name,
        provider.baseUrl,
        provider.website || "",
        provider.apiKeyUrl || "",
      ]
        .join(" ")
        .toLowerCase();
      return !query || haystack.includes(query);
    });
    if (providerNameFilter.length > 0) {
      const filterSet = new Set(providerNameFilter);
      result = result.filter((provider) =>
        filterSet.has(provider.name.trim().toLowerCase()),
      );
    }
    result = [...result].sort((a, b) => {
      if (providerSortBy === "custom") {
        const aIndex =
          providerCustomSortOrderIndex.get(a.id) ?? Number.MAX_SAFE_INTEGER;
        const bIndex =
          providerCustomSortOrderIndex.get(b.id) ?? Number.MAX_SAFE_INTEGER;
        if (aIndex !== bIndex) {
          return aIndex - bIndex;
        }
        const createdDiff = (b.createdAt || 0) - (a.createdAt || 0);
        if (createdDiff !== 0) return createdDiff;
        return a.name.localeCompare(b.name);
      }
      const direction = providerSortDirection === "asc" ? 1 : -1;
      if (providerSortBy === "created_at") {
        return direction * ((a.createdAt || 0) - (b.createdAt || 0));
      }
      return direction * a.name.localeCompare(b.name);
    });
    return result;
  }, [
    providers,
    providerNameFilter,
    providerCustomSortOrderIndex,
    providerSortBy,
    providerSortDirection,
    searchQuery,
  ]);

  const providerFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => {
      const counts = new Map<string, { label: string; count: number }>();
      providers.forEach((provider) => {
        const label = provider.name.trim() || t("common.unknown", "未知");
        const value = label.toLowerCase();
        const previous = counts.get(value);
        counts.set(value, {
          label: previous?.label ?? label,
          count: (previous?.count ?? 0) + 1,
        });
      });
      return [...counts.entries()]
        .map(([value, item]) => ({
          value,
          label: item.label,
          count: item.count,
        }))
        .sort((a, b) => a.label.localeCompare(b.label));
    },
    [providers, t],
  );

  const filteredProviderIds = useMemo(
    () => filteredProviders.map((item) => item.id),
    [filteredProviders],
  );
  const isProviderCustomSortActive = providerSortBy === "custom";
  const providerCustomSortProviders = useMemo(() => {
    const providerMap = new Map(
      providers.map((provider) => [provider.id, provider]),
    );
    const result: CodexModelProvider[] = [];
    const seen = new Set<string>();

    providerCustomSortOrder.forEach((providerId) => {
      const provider = providerMap.get(providerId);
      if (!provider || seen.has(providerId)) return;
      result.push(provider);
      seen.add(providerId);
    });

    providers.forEach((provider) => {
      if (seen.has(provider.id)) return;
      result.push(provider);
      seen.add(provider.id);
    });

    return result;
  }, [providerCustomSortOrder, providers]);
  const providerCustomSortProviderIds = useMemo(
    () => providerCustomSortProviders.map((provider) => provider.id),
    [providerCustomSortProviders],
  );
  const moveProviderCustomSortProvider = useCallback(
    (providerId: string, direction: "up" | "down") => {
      const currentIndex = providerCustomSortProviderIds.indexOf(providerId);
      if (currentIndex < 0) return;
      const targetIndex =
        direction === "up" ? currentIndex - 1 : currentIndex + 1;
      if (
        targetIndex < 0 ||
        targetIndex >= providerCustomSortProviderIds.length
      ) {
        return;
      }
      const next = [...providerCustomSortProviderIds];
      const [moved] = next.splice(currentIndex, 1);
      next.splice(targetIndex, 0, moved);
      setProviderCustomSortOrder(next);
    },
    [providerCustomSortProviderIds],
  );
  const stopProviderCustomSortDragging = useCallback(() => {
    setDraggedProviderCustomSortId(null);
    setProviderCustomSortDropTargetId(null);
  }, []);
  const handleProviderCustomSortDragStart = useCallback(
    (event: ReactMouseEvent, providerId: string) => {
      if (event.button !== 0) return;
      event.preventDefault();
      event.stopPropagation();
      setDraggedProviderCustomSortId(providerId);
      setProviderCustomSortDropTargetId(null);
    },
    [],
  );
  const handleProviderCustomSortDragMove = useCallback(
    (targetProviderId: string) => {
      if (!draggedProviderCustomSortId) return;
      if (draggedProviderCustomSortId === targetProviderId) {
        setProviderCustomSortDropTargetId(null);
        return;
      }
      const fromIndex = providerCustomSortProviderIds.indexOf(
        draggedProviderCustomSortId,
      );
      const toIndex = providerCustomSortProviderIds.indexOf(targetProviderId);
      if (fromIndex < 0 || toIndex < 0) return;
      setProviderCustomSortDropTargetId(targetProviderId);
      const next = [...providerCustomSortProviderIds];
      const [moved] = next.splice(fromIndex, 1);
      next.splice(toIndex, 0, moved);
      setProviderCustomSortOrder(next);
    },
    [draggedProviderCustomSortId, providerCustomSortProviderIds],
  );
  const resetProviderCustomSortOrder = useCallback(() => {
    setProviderCustomSortOrder(providers.map((provider) => provider.id));
  }, [providers]);
  const handleProviderSortByChange = useCallback((value: string) => {
    const nextSortBy: ProviderSortBy =
      value === "custom" || value === "name" || value === "created_at"
        ? value
        : "created_at";
    setProviderSortBy(nextSortBy);
    if (nextSortBy === "custom") {
      setShowProviderCustomSortModal(true);
    }
  }, []);
  const isAllProvidersSelected = useMemo(
    () =>
      filteredProviderIds.length > 0 &&
      filteredProviderIds.every((id) => selectedProviderIds.has(id)),
    [filteredProviderIds, selectedProviderIds],
  );

  const reloadProviders = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await listCodexModelProviders();
      setProviders(next);
      onProvidersChanged?.(next);
    } catch (err) {
      setError(
        t("codex.modelProviders.loadFailed", {
          defaultValue: "加载模型供应商失败：{{error}}",
          error: String(err),
        }),
      );
    } finally {
      setLoading(false);
    }
  }, [onProvidersChanged, t]);

  const reloadCurrentAccount = useCallback(async () => {
    try {
      setCurrentAccount(await getCurrentCodexAccount());
    } catch {
      setCurrentAccount(null);
    }
  }, []);

  const reloadLocalAccessState = useCallback(async () => {
    try {
      setLocalAccessState(await getCodexLocalAccessState());
    } catch {
      setLocalAccessState(null);
    }
  }, []);

  const reloadCodexInstances = useCallback(async () => {
    try {
      const next = await listCodexInstances();
      setCodexInstances(next);
    } catch {
      setCodexInstances([]);
    }
  }, []);

  useEffect(() => {
    void reloadProviders();
    void reloadCurrentAccount();
    void reloadLocalAccessState();
    void reloadCodexInstances();
    void fetchSponsorState();
    void listCodexAccounts()
      .then((items) => setOauthAccounts(items.filter((item) => item.auth_mode !== "apikey")))
      .catch(() => setOauthAccounts([]));
  }, [
    fetchSponsorState,
    reloadProviders,
    reloadCurrentAccount,
    reloadLocalAccessState,
    reloadCodexInstances,
  ]);

  useEffect(() => {
    writeProviderUsageCache(providerUsageMap);
  }, [providerUsageMap]);

  useEffect(() => {
    if (providers.length === 0) {
      return;
    }
    const providerIds = providers.map((provider) => provider.id);
    const providerIdSet = new Set(providerIds);
    setProviderCustomSortOrder((previous) => {
      const next = previous.filter((providerId) =>
        providerIdSet.has(providerId),
      );
      const seen = new Set(next);
      for (const providerId of providerIds) {
        if (!seen.has(providerId)) {
          next.push(providerId);
          seen.add(providerId);
        }
      }
      const unchanged =
        next.length === previous.length &&
        next.every((providerId, index) => providerId === previous[index]);
      return unchanged ? previous : next;
    });
  }, [providers]);

  useEffect(() => {
    writeCodexProviderCustomSortOrder(providerCustomSortOrder);
  }, [providerCustomSortOrder]);

  useEffect(() => {
    writeCodexProviderCustomSortActive(providerSortBy === "custom");
  }, [providerSortBy]);

  useEffect(() => {
    if (!showProviderCustomSortModal || !draggedProviderCustomSortId) return;
    const handleMouseUp = () => {
      setDraggedProviderCustomSortId(null);
      setProviderCustomSortDropTargetId(null);
    };
    window.addEventListener("mouseup", handleMouseUp);
    return () => window.removeEventListener("mouseup", handleMouseUp);
  }, [draggedProviderCustomSortId, showProviderCustomSortModal]);

  useEffect(() => {
    if (!showProviderCustomSortModal) {
      setDraggedProviderCustomSortId(null);
      setProviderCustomSortDropTargetId(null);
    }
  }, [showProviderCustomSortModal]);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const home = await homeDir();
        const [providerStorePath, codexConfigPath, codexAuthPath] =
          await Promise.all([
            join(home, ".antigravity_cockpit", "codex_model_providers.json"),
            join(home, ".codex", "config.toml"),
            join(home, ".codex", "auth.json"),
          ]);
        if (cancelled) return;
        setPreviewPaths({
          providerStorePath,
          codexConfigPath,
          codexAuthPath,
        });
      } catch {
        // ignore path resolution failures and keep fallback preview paths
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    void listen<CodexModelProviderChatTestProgressPayload>(
      "codex://model-provider-test-progress",
      (event) => {
        const payload = event.payload;
        setBatchTestSession((current) => {
          if (!current || current.runId !== payload.runId) return current;
          const cancellationRequested = cancelledBatchTestRunIdsRef.current.has(
            payload.runId,
          );
          const batchCancelled = payload.phase === "batch_cancelled";
          if (cancellationRequested && !batchCancelled) return current;

          const nextRecords = current.records.map((record) => {
            if (
              batchCancelled &&
              (record.status === "pending" || record.status === "running")
            ) {
              return {
                ...record,
                status: "cancelled" as ProviderBatchTestStatus,
              };
            }
            if (
              payload.phase === "provider_started" &&
              record.providerId === payload.currentProviderId
            ) {
              return { ...record, status: "running" as ProviderBatchTestStatus };
            }
            if (
              payload.phase === "provider_completed" &&
              payload.item &&
              record.providerId === payload.item.providerId
            ) {
              return toProviderBatchTestRecordView(payload.item);
            }
            if (
              payload.phase === "provider_started" &&
              record.status === "running" &&
              record.providerId !== payload.currentProviderId
            ) {
              return { ...record, status: "pending" as ProviderBatchTestStatus };
            }
            return record;
          });

          return {
            ...current,
            total: payload.total,
            completed: payload.completed,
            successCount: payload.successCount,
            failureCount: payload.failureCount,
            running: payload.running,
            cancelled: current.cancelled || batchCancelled,
            records: nextRecords,
          };
        });
      },
    ).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  const providerReferenceMap = useMemo(() => {
    const map = new Map<string, number>();
    providers.forEach((provider) => {
      map.set(
        provider.id,
        countCodexModelProviderReferences(provider, accounts),
      );
    });
    return map;
  }, [accounts, providers]);

  const displayInstances = useMemo(() => {
    const source =
      codexInstances.length > 0
        ? codexInstances
        : [
            {
              id: DEFAULT_INSTANCE_ID,
              name: "",
              userDataDir: "",
              extraArgs: "",
              createdAt: 0,
              running: false,
              isDefault: true,
            } as InstanceProfile,
          ];
    const sortPreference = readCodexInstanceSortPreference();
    return [...source].sort((a, b) => {
      if (a.isDefault && !b.isDefault) return -1;
      if (!a.isDefault && b.isDefault) return 1;
      const av =
        sortPreference.field === "createdAt"
          ? a.createdAt || 0
          : a.lastLaunchedAt || 0;
      const bv =
        sortPreference.field === "createdAt"
          ? b.createdAt || 0
          : b.lastLaunchedAt || 0;
      return sortPreference.direction === "asc" ? av - bv : bv - av;
    });
  }, [codexInstances]);

  const resolveInstanceById = useCallback(
    (instanceId: string): InstanceProfile | null =>
      displayInstances.find((item) => item.id === instanceId) ??
      displayInstances.find((item) => item.id === DEFAULT_INSTANCE_ID) ??
      null,
    [displayInstances],
  );

  const getProviderInstanceId = useCallback(
    (provider: CodexModelProvider): string => {
      const selected = provider.boundInstanceId ?? DEFAULT_INSTANCE_ID;
      return resolveInstanceById(selected)?.id ?? DEFAULT_INSTANCE_ID;
    },
    [resolveInstanceById],
  );

  const getSelectedProviderApiKey = useCallback(
    (provider: CodexModelProvider): CodexModelProviderApiKey | null => {
      const selectedId = selectedProviderApiKeyMap[provider.id];
      if (selectedId) {
        const matched = provider.apiKeys.find((item) => item.id === selectedId);
        if (matched) return matched;
      }
      return provider.apiKeys[0] ?? null;
    },
    [selectedProviderApiKeyMap],
  );

  const syncProviderUsageFromAccountCache = useCallback(() => {
    setProviderUsageMap((previous) => {
      let changed = false;
      const next = { ...previous };

      for (const provider of providers) {
        const apiKey = getSelectedProviderApiKey(provider);
        if (!apiKey) continue;
        const accountUsage = getAccountUsageForProviderApiKey(
          provider,
          apiKey,
          accounts,
        );
        if (!accountUsage) continue;

        const existing = previous[provider.id];
        if ((existing?.updatedAt ?? 0) > (accountUsage.updatedAt ?? 0)) {
          continue;
        }
        if (
          existing?.summary === accountUsage.summary &&
          existing?.error === accountUsage.error &&
          existing?.unavailable === accountUsage.unavailable &&
          existing?.updatedAt === accountUsage.updatedAt
        ) {
          continue;
        }
        next[provider.id] = accountUsage;
        changed = true;
      }

      return changed ? next : previous;
    });
  }, [accounts, getSelectedProviderApiKey, providers]);

  useEffect(() => {
    syncProviderUsageFromAccountCache();
    window.addEventListener(
      CODEX_API_KEY_USAGE_REFRESHED_EVENT,
      syncProviderUsageFromAccountCache,
    );
    return () =>
      window.removeEventListener(
        CODEX_API_KEY_USAGE_REFRESHED_EVENT,
        syncProviderUsageFromAccountCache,
      );
  }, [syncProviderUsageFromAccountCache]);

  const providerBatchTestVisibleProviders = useMemo(() => {
    const query = batchTestSearchQuery.trim().toLowerCase();
    if (!query) return filteredProviders;
    return filteredProviders.filter((provider) =>
      [provider.name, provider.baseUrl, provider.website ?? "", provider.apiKeyUrl ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(query),
    );
  }, [batchTestSearchQuery, filteredProviders]);

  const providerBatchTestSelectableIds = useMemo(
    () =>
      providerBatchTestVisibleProviders
        .filter((provider) => !!getSelectedProviderApiKey(provider))
        .map((provider) => provider.id),
    [getSelectedProviderApiKey, providerBatchTestVisibleProviders],
  );

  const isAllBatchTestProvidersSelected = useMemo(
    () =>
      providerBatchTestSelectableIds.length > 0 &&
      providerBatchTestSelectableIds.every((id) => batchTestSelectedProviderIds.has(id)),
    [batchTestSelectedProviderIds, providerBatchTestSelectableIds],
  );

  const batchTestSelectedCount = useMemo(
    () =>
      providers.filter(
        (provider) =>
          batchTestSelectedProviderIds.has(provider.id) &&
          !!getSelectedProviderApiKey(provider),
      ).length,
    [batchTestSelectedProviderIds, getSelectedProviderApiKey, providers],
  );

  const batchTestModelOptions = useMemo(() => {
    const seen = new Set<string>();
    const models: string[] = [];
    for (const provider of providers) {
      if (!batchTestSelectedProviderIds.has(provider.id)) continue;
      for (const raw of provider.modelCatalog ?? []) {
        const model = raw.trim();
        if (!model || isImageGenerationModelId(model)) continue;
        const key = model.toLowerCase();
        if (seen.has(key)) continue;
        seen.add(key);
        models.push(model);
      }
    }
    models.sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
    return [
      {
        value: "",
        label: t(
          "codex.modelProviders.batchTest.modelAuto",
          "自动选择（按目录/探测）",
        ),
      },
      ...models.map((model) => ({ value: model, label: model })),
      {
        value: "__custom__",
        label: t("codex.modelProviders.batchTest.modelCustom", "自定义模型…"),
      },
    ];
  }, [batchTestSelectedProviderIds, providers, t]);

  const resolvedBatchTestModel = useMemo(() => {
    if (batchTestModelId === "__custom__") {
      return batchTestModelCustom.trim() || null;
    }
    const trimmed = batchTestModelId.trim();
    return trimmed || null;
  }, [batchTestModelCustom, batchTestModelId]);

  const openBatchTestModal = useCallback(() => {
    const defaultSource =
      selectedProviderIds.size > 0
        ? filteredProviders.filter((provider) => selectedProviderIds.has(provider.id))
        : filteredProviders;
    const initialIds = defaultSource
      .filter((provider) => !!getSelectedProviderApiKey(provider))
      .map((provider) => provider.id);
    setBatchTestSelectedProviderIds(new Set(initialIds));
    setBatchTestResultSelectedProviderIds(new Set());
    setBatchTestSearchQuery("");
    setBatchTestFilter("all");
    setBatchTestModelId("");
    setBatchTestModelCustom("");
    setBatchTestError(null);
    setBatchTestCancelling(false);
    setBatchTestSession(null);
    setBatchTestStep("select");
    setBatchTestModalOpen(true);
  }, [filteredProviders, getSelectedProviderApiKey, selectedProviderIds]);

  const markBatchTestCancelled = useCallback((runId: string) => {
    setBatchTestSession((current) => {
      if (!current || current.runId !== runId) return current;
      const records = current.records.map((record) =>
        record.status === "pending" || record.status === "running"
          ? { ...record, status: "cancelled" as ProviderBatchTestStatus }
          : record,
      );
      return {
        ...current,
        completed: records.filter(
          (record) => record.status === "success" || record.status === "error",
        ).length,
        running: false,
        cancelled: true,
        records,
      };
    });
  }, []);

  const requestBatchTestCancellation = useCallback(
    async (runId: string) => {
      if (cancelledBatchTestRunIdsRef.current.has(runId)) return;
      cancelledBatchTestRunIdsRef.current.add(runId);
      setBatchTestCancelling(true);
      try {
        const accepted = await cancelCodexModelProviderChatTest(runId);
        if (accepted) {
          markBatchTestCancelled(runId);
        } else {
          cancelledBatchTestRunIdsRef.current.delete(runId);
        }
      } catch (err) {
        cancelledBatchTestRunIdsRef.current.delete(runId);
        setBatchTestError(String(err));
      } finally {
        setBatchTestCancelling(false);
      }
    },
    [markBatchTestCancelled],
  );

  const closeBatchTestModal = useCallback(() => {
    const runId = batchTestSession?.running ? batchTestSession.runId : null;
    if (runId) {
      void requestBatchTestCancellation(runId);
    }
    setBatchTestModalOpen(false);
    setBatchTestError(null);
  }, [batchTestSession?.runId, batchTestSession?.running, requestBatchTestCancellation]);

  useEscClose(batchTestModalOpen, closeBatchTestModal);

  const toggleBatchTestProvider = useCallback((providerId: string) => {
    setBatchTestSelectedProviderIds((previous) => {
      const next = new Set(previous);
      if (next.has(providerId)) {
        next.delete(providerId);
      } else {
        next.add(providerId);
      }
      return next;
    });
  }, []);

  const toggleAllVisibleBatchTestProviders = useCallback(() => {
    setBatchTestSelectedProviderIds((previous) => {
      const next = new Set(previous);
      const allSelected =
        providerBatchTestSelectableIds.length > 0 &&
        providerBatchTestSelectableIds.every((id) => next.has(id));
      providerBatchTestSelectableIds.forEach((id) => {
        if (allSelected) {
          next.delete(id);
        } else {
          next.add(id);
        }
      });
      return next;
    });
  }, [providerBatchTestSelectableIds]);

  const buildBatchTestTarget = useCallback(
    (provider: CodexModelProvider): CodexModelProviderChatTestTarget | null => {
      const apiKey = getSelectedProviderApiKey(provider);
      if (!apiKey) return null;
      return {
        providerId: provider.id,
        providerName: provider.name,
        baseUrl: provider.baseUrl,
        apiKeyId: apiKey.id,
        apiKeyName: apiKey.name || provider.name,
        apiKey: apiKey.apiKey,
        wireApi: resolveProviderWireApi(provider),
        modelCatalog: provider.modelCatalog ?? [],
      };
    },
    [getSelectedProviderApiKey],
  );

  const buildBatchTestPendingRecord = useCallback(
    (
      provider: CodexModelProvider,
      target: CodexModelProviderChatTestTarget,
      runStartedAt: number,
      explicitModel?: string | null,
    ): ProviderBatchTestRecordView => {
      const wireApi = target.wireApi ?? resolveProviderWireApi(provider);
      const modelId =
        explicitModel?.trim() ||
        selectProviderBatchTestModelId(wireApi, target.modelCatalog);
      return {
        providerId: target.providerId,
        providerName: target.providerName,
        apiKeyId: target.apiKeyId,
        apiKeyName: target.apiKeyName,
        wireApi,
        accessMode: "gateway",
        modelId,
        success: false,
        prompt: "",
        reply: null,
        error: null,
        durationMs: null,
        timestamp: runStartedAt,
        status: "pending",
      };
    },
    [],
  );

  const resolveBoundOAuthAccount = useCallback(
    (provider: CodexModelProvider): CodexAccount | null => {
      const boundId = (provider.boundOauthAccountId || "").trim();
      if (!boundId) return null;
      return oauthAccounts.find((item) => item.id === boundId) ?? null;
    },
    [oauthAccounts],
  );

  const maskAccountText = useCallback((value?: string | null): string => {
    const trimmed = (value || "").trim();
    if (!trimmed) return t("common.none", "暂无");
    if (trimmed.includes("@")) {
      const [name, domain] = trimmed.split("@");
      if (!domain) return trimmed;
      if (name.length <= 2) return `${name[0] || ""}***@${domain}`;
      return `${name.slice(0, 2)}***@${domain}`;
    }
    if (trimmed.length <= 6) return trimmed;
    return `${trimmed.slice(0, 3)}***${trimmed.slice(-2)}`;
  }, [t]);

  const resolvePresentation = useCallback(
    (account: CodexAccount) => buildCodexAccountPresentation(account, t),
    [t],
  );

  const resolvePlanKey = useCallback(
    (account: CodexAccount) => getCodexPlanFilterKey(account),
    [],
  );

  const normalizeTag = useCallback((tag: string) => tag.trim().toLowerCase(), []);

  const getInstanceName = useCallback(
    (instance: InstanceProfile | null): string => {
      if (!instance || instance.id === DEFAULT_INSTANCE_ID) {
        return t("codex.modelProviders.instance.default", "默认实例");
      }
      return instance.name;
    },
    [t],
  );

  const isInstanceReady = useCallback(
    (instance: InstanceProfile | null): boolean =>
      !instance ||
      instance.id === DEFAULT_INSTANCE_ID ||
      instance.initialized !== false,
    [],
  );

  const currentEditingProvider = useMemo(
    () =>
      form.providerId
        ? (providers.find((item) => item.id === form.providerId) ?? null)
        : null,
    [form.providerId, providers],
  );
  const selectedPreset = useMemo(
    () => findCodexApiProviderPresetById(selectedPresetId),
    [selectedPresetId],
  );
  const selectedSponsorTemplate = useMemo(
    () =>
      sponsorProviderTemplates.find((template) => template.id === selectedSponsorTemplateId) ??
      null,
    [selectedSponsorTemplateId, sponsorProviderTemplates],
  );
  const openCreateModal = useCallback(() => {
    setNotice(null);
    setFormError(null);
    setExistingApiKeySearchQuery("");
    setEditingApiKey(null);
    setForm({
      ...EMPTY_FORM,
      wireApi: resolveDefaultProviderWireApi(CODEX_API_PROVIDER_CUSTOM_ID),
      enableModePreference: resolveEnableModePreferenceForWireApi(
        resolveDefaultProviderWireApi(CODEX_API_PROVIDER_CUSTOM_ID),
      ),
    });
    setSelectedPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
    setSelectedSponsorTemplateId(null);
    setShowModal(true);
  }, []);

  const toggleProviderSelected = useCallback((providerId: string) => {
    setSelectedProviderIds((previous) => {
      const next = new Set(previous);
      if (next.has(providerId)) {
        next.delete(providerId);
      } else {
        next.add(providerId);
      }
      return next;
    });
  }, []);

  const toggleSelectAllProviders = useCallback((providerIds: string[]) => {
    setSelectedProviderIds((previous) => {
      const next = new Set(previous);
      const allSelected =
        providerIds.length > 0 && providerIds.every((id) => next.has(id));
      providerIds.forEach((id) => {
        if (allSelected) {
          next.delete(id);
        } else {
          next.add(id);
        }
      });
      return next;
    });
  }, []);

  const openEditModal = useCallback((provider: CodexModelProvider) => {
    setNotice(null);
    setFormError(null);
    setExistingApiKeySearchQuery("");
    setEditingApiKey(null);
    const resolvedWireApi = resolveProviderWireApi(provider);
    setForm({
      providerId: provider.id,
      name: provider.name,
      baseUrl: provider.baseUrl,
      modelCatalogText: (provider.modelCatalog ?? []).join("\n"),
      modelContextWindowsDraft: contextWindowDraftsFromRecord(
        provider.modelContextWindows,
        provider.modelCatalog ?? [],
      ),
      supportsVision: provider.supportsVision === true,
      visionModelText: visionModelTextFromCapabilities(provider.modelCapabilities),
      visionRoutingModel: provider.visionRoutingModel ?? "",
      website: provider.website ?? "",
      apiKeyUrl: provider.apiKeyUrl ?? "",
      wireApi: resolvedWireApi,
      supportsWebsockets:
        resolvedWireApi === "responses" && provider.supportsWebsockets === true,
      enableModePreference:
        provider.enableModePreference ??
        resolveEnableModePreferenceForWireApi(
          resolvedWireApi,
          resolveCodexApiProviderPresetId(provider.baseUrl),
        ),
      integrationType: provider.integrationType ?? "",
      newApiKeyName: "",
      newApiKey: "",
    });
    setSelectedPresetId(resolveCodexApiProviderPresetId(provider.baseUrl));
    setSelectedSponsorTemplateId(null);
    setShowModal(true);
  }, []);

  const closeModal = useCallback(() => {
    if (saving) return;
    setEditingApiKey(null);
    setShowModal(false);
    setFormError(null);
  }, [saving]);

  useEscClose(showModal, closeModal);

  const mutateForm = useCallback((patch: Partial<ProviderFormState>) => {
    setForm((prev) => ({ ...prev, ...patch }));
  }, []);

  useEffect(() => {
    const resolved = resolveCodexApiProviderPresetId(form.baseUrl);
    setSelectedPresetId((prev) => (prev === resolved ? prev : resolved));
  }, [form.baseUrl]);

  const handleSelectProviderPreset = useCallback(
    (presetId: string) => {
      setSelectedPresetId(presetId);
      setSelectedSponsorTemplateId(null);
      if (presetId === CODEX_API_PROVIDER_CUSTOM_ID) {
        mutateForm({ supportsWebsockets: false });
        return;
      }
      const preset = findCodexApiProviderPresetById(presetId);
      if (!preset) return;
      const wireApi = resolveDefaultProviderWireApi(preset.id);
      mutateForm({
        name: preset.name,
        baseUrl: preset.baseUrls[0] ?? "",
        modelCatalogText: (preset.modelCatalog ?? []).join("\n"),
        modelContextWindowsDraft: contextWindowDraftsFromRecord(
          undefined,
          preset.modelCatalog ?? [],
        ),
        supportsVision: false,
        visionModelText: "",
        visionRoutingModel: "",
        website: preset.website ?? "",
        apiKeyUrl: preset.apiKeyUrl ?? "",
        wireApi,
        supportsWebsockets: false,
        enableModePreference: resolveEnableModePreferenceForWireApi(
          wireApi,
          preset.id,
        ),
        integrationType: "",
      });
    },
    [mutateForm],
  );

  const handleSelectSponsorTemplate = useCallback(
    (template: SponsorProviderTemplate) => {
      setSelectedSponsorTemplateId(template.id);
      setSelectedPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
      const wireApi = resolveDefaultProviderWireApi(
        null,
        template.wireApi ?? null,
      );
      mutateForm({
        name: template.name,
        baseUrl: template.baseUrl,
        modelCatalogText: template.modelCatalog.join("\n"),
        modelContextWindowsDraft: contextWindowDraftsFromRecord(
          undefined,
          template.modelCatalog,
        ),
        supportsVision: template.supportsVision,
        visionModelText: "",
        visionRoutingModel: "",
        website: template.website,
        apiKeyUrl: template.apiKeyUrl,
        wireApi,
        supportsWebsockets: false,
        enableModePreference: resolveEnableModePreferenceForWireApi(wireApi),
        integrationType: template.integrationType ?? "",
      });
    },
    [mutateForm],
  );

  const handleSelectPresetEndpoint = useCallback(
    (baseUrl: string) => {
      setSelectedSponsorTemplateId(null);
      mutateForm({ baseUrl });
    },
    [mutateForm],
  );

  const parseServiceError = useCallback(
    (err: unknown): string => {
      const raw = String(err ?? "");
      if (raw.includes("PROVIDER_NAME_REQUIRED")) {
        return t(
          "codex.modelProviders.validation.nameRequired",
          "供应商名称不能为空",
        );
      }
      if (raw.includes("PROVIDER_BASE_URL_INVALID")) {
        return t(
          "codex.modelProviders.validation.baseUrlInvalid",
          "Base URL 格式无效",
        );
      }
      if (raw.includes("PROVIDER_BASE_URL_EXISTS")) {
        return t(
          "codex.modelProviders.validation.baseUrlExists",
          "该 Base URL 已存在",
        );
      }
      if (raw.includes("PROVIDER_NOT_FOUND")) {
        return t(
          "codex.modelProviders.validation.providerNotFound",
          "供应商不存在",
        );
      }
      return raw.replace(/^Error:\s*/, "");
    },
    [t],
  );

  const formatProviderTestFailure = useCallback(
    (failure: CodexLocalAccessTestFailure): string => {
      const titleByStage: Record<string, string> = {
        credential: t(
          "codex.modelProviders.testFailure.credential",
          "API Key 不可用",
        ),
        url: t("codex.modelProviders.testFailure.url", "Base URL 无效"),
        network: t(
          "codex.modelProviders.testFailure.network",
          "网络连接失败",
        ),
        models: t(
          "codex.modelProviders.testFailure.models",
          "模型列表接口异常",
        ),
        parse: t(
          "codex.modelProviders.testFailure.parse",
          "响应解析失败",
        ),
      };
      const suggestionByCode: Record<string, string> = {
        add_api_key: t(
          "codex.modelProviders.testSuggestion.addApiKey",
          "请先为该供应商添加 API Key，然后再测试连接。",
        ),
        check_base_url: t(
          "codex.modelProviders.testSuggestion.checkBaseUrl",
          "请检查 Base URL 是否包含正确版本路径，例如 /v1。",
        ),
        check_network: t(
          "codex.modelProviders.testSuggestion.checkNetwork",
          "请检查供应商地址、网络代理、防火墙或上游服务状态。",
        ),
        check_api_key: t(
          "codex.modelProviders.testSuggestion.checkApiKey",
          "请检查 API Key 是否有效、权限是否包含模型列表接口。",
        ),
        check_provider_status: t(
          "codex.modelProviders.testSuggestion.checkProviderStatus",
          "请检查供应商服务状态、网络代理或供应商接口兼容性。",
        ),
        check_openai_compatible_models: t(
          "codex.modelProviders.testSuggestion.checkModelsApi",
          "请确认该供应商提供 OpenAI 兼容的模型列表响应。",
        ),
      };
      const detail =
        failure.status !== null
          ? t("codex.modelProviders.testFailure.httpStatus", {
              defaultValue: "HTTP {{status}}",
              status: failure.status,
            })
          : failure.cause;
      const title = titleByStage[failure.stage] ?? failure.title;
      const suggestion =
        suggestionByCode[failure.suggestion] ?? failure.suggestion;
      return t("codex.modelProviders.testFailure.message", {
        defaultValue: "{{title}}：{{detail}}。{{suggestion}}",
        title,
        detail,
        suggestion,
      });
    },
    [t],
  );

  const providerBatchTestCounts = useMemo(() => {
    const records = batchTestSession?.records ?? [];
    return {
      all: records.length,
      pending: records.filter((record) => record.status === "pending").length,
      running: records.filter((record) => record.status === "running").length,
      success: records.filter((record) => record.status === "success").length,
      error: records.filter((record) => record.status === "error").length,
      cancelled: records.filter((record) => record.status === "cancelled").length,
    };
  }, [batchTestSession?.records]);

  const providerBatchTestFilterOptions = useMemo(
    () => [
      {
        key: "all" as const,
        label: t("codex.modelProviders.batchTest.status.all", "全部"),
        count: providerBatchTestCounts.all,
        tone: "all",
      },
      {
        key: "success" as const,
        label: t("codex.modelProviders.batchTest.status.success", "成功"),
        count: providerBatchTestCounts.success,
        tone: "success",
      },
      {
        key: "error" as const,
        label: t("codex.modelProviders.batchTest.status.error", "失败"),
        count: providerBatchTestCounts.error,
        tone: "error",
      },
      {
        key: "running" as const,
        label: t("codex.modelProviders.batchTest.status.running", "测试中"),
        count: providerBatchTestCounts.running,
        tone: "running",
      },
      {
        key: "pending" as const,
        label: t("codex.modelProviders.batchTest.status.pending", "等待中"),
        count: providerBatchTestCounts.pending,
        tone: "pending",
      },
      {
        key: "cancelled" as const,
        label: t("common.cancelled", "已取消"),
        count: providerBatchTestCounts.cancelled,
        tone: "cancelled",
      },
    ],
    [providerBatchTestCounts, t],
  );

  const filteredProviderBatchTestRecords = useMemo(() => {
    const records = batchTestSession?.records ?? [];
    if (batchTestFilter === "all") return records;
    return records.filter((record) => record.status === batchTestFilter);
  }, [batchTestFilter, batchTestSession?.records]);

  const toggleBatchTestResultProvider = useCallback((providerId: string) => {
    setBatchTestResultSelectedProviderIds((previous) => {
      const next = new Set(previous);
      if (next.has(providerId)) {
        next.delete(providerId);
      } else {
        next.add(providerId);
      }
      return next;
    });
  }, []);

  const toggleAllVisibleBatchTestResults = useCallback(() => {
    setBatchTestResultSelectedProviderIds((previous) => {
      const visibleIds = filteredProviderBatchTestRecords.map((record) => record.providerId);
      const next = new Set(previous);
      const allSelected =
        visibleIds.length > 0 && visibleIds.every((id) => next.has(id));
      visibleIds.forEach((id) => {
        if (allSelected) {
          next.delete(id);
        } else {
          next.add(id);
        }
      });
      return next;
    });
  }, [filteredProviderBatchTestRecords]);

  const selectFailedBatchTestResults = useCallback(() => {
    setBatchTestResultSelectedProviderIds(
      new Set(
        (batchTestSession?.records ?? [])
          .filter((record) => record.status === "error")
          .map((record) => record.providerId),
      ),
    );
  }, [batchTestSession?.records]);

  const formatProviderBatchTestErrorMessage = useCallback(
    (error?: string | null) => {
      const text = (error ?? "").trim();
      if (!text) {
        return t(
          "codex.modelProviders.batchTest.unknownError",
          "未知错误",
        );
      }
      if (
        text
          .toLowerCase()
          .includes("this account only allows codex official clients")
      ) {
        return t(
          "codex.modelProviders.batchTest.officialClientOnlyError",
          "该供应商限制只允许 Codex 官方客户端请求；当前一键测试已走本地网关，但上游仍拒绝了这次测试请求。请以官方 Codex 实际启动后的表现为准。",
        );
      }
      return text;
    },
    [t],
  );

  const handleStartBatchProviderTest = useCallback(async () => {
    if (batchTestSession?.running) return;
    const selectedProviders = providers.filter((provider) =>
      batchTestSelectedProviderIds.has(provider.id),
    );
    const targets: CodexModelProviderChatTestTarget[] = [];
    const pendingRecords: ProviderBatchTestRecordView[] = [];
    const startedAt = Date.now();
    for (const provider of selectedProviders) {
      const target = buildBatchTestTarget(provider);
      if (!target) continue;
      targets.push(target);
      pendingRecords.push(
        buildBatchTestPendingRecord(
          provider,
          target,
          startedAt,
          resolvedBatchTestModel,
        ),
      );
    }
    if (targets.length === 0) {
      setBatchTestError(
        t(
          "codex.modelProviders.batchTest.emptySelection",
          "请选择至少一个带 API Key 的供应商。",
        ),
      );
      return;
    }
    const runId = createProviderBatchTestRunId();
    cancelledBatchTestRunIdsRef.current.delete(runId);
    setBatchTestCancelling(false);
    setNotice(null);
    setBatchTestError(null);
    setBatchTestResultSelectedProviderIds(new Set());
    setBatchTestFilter("all");
    setBatchTestStep("results");
    setBatchTestSession({
      runId,
      total: targets.length,
      completed: 0,
      successCount: 0,
      failureCount: 0,
      running: true,
      cancelled: false,
      startedAt,
      records: pendingRecords,
    });
    try {
      const result = await testCodexModelProviderChatBatch({
        targets,
        runId,
        model: resolvedBatchTestModel,
      });
      if (cancelledBatchTestRunIdsRef.current.has(result.runId)) {
        cancelledBatchTestRunIdsRef.current.delete(result.runId);
        return;
      }
      setBatchTestSession((current) => {
        if (!current || current.runId !== result.runId) return current;
        return {
          ...current,
          total: result.records.length,
          completed: result.records.length,
          successCount: result.successCount,
          failureCount: result.failureCount,
          running: false,
          cancelled: false,
          records: result.records.map(toProviderBatchTestRecordView),
        };
      });
    } catch (err) {
      if (cancelledBatchTestRunIdsRef.current.has(runId)) {
        cancelledBatchTestRunIdsRef.current.delete(runId);
        return;
      }
      const errorText = parseServiceError(err);
      setBatchTestError(errorText);
      setBatchTestSession((current) =>
        current
          ? {
              ...current,
              running: false,
              cancelled: false,
              errorText,
              records: current.records.map((record) =>
                record.status === "running"
                  ? { ...record, status: "error", error: errorText }
                  : record,
              ),
            }
          : current,
      );
    }
  }, [
    batchTestSelectedProviderIds,
    batchTestSession?.running,
    buildBatchTestPendingRecord,
    buildBatchTestTarget,
    parseServiceError,
    providers,
    resolvedBatchTestModel,
    t,
  ]);

  const handleDeleteBatchTestResults = useCallback(async () => {
    if (batchTestDeleting || batchTestSession?.running) return;
    const providerIds = Array.from(batchTestResultSelectedProviderIds);
    if (providerIds.length === 0) return;
    const providerIdSet = new Set(providerIds);
    const linkedAccountIds = accounts
      .filter(
        (account) =>
          isCodexApiKeyAccount(account) &&
          !!account.api_provider_id &&
          providerIdSet.has(account.api_provider_id),
      )
      .map((account) => account.id);
    const confirmed = await confirmDialog(
      t("codex.modelProviders.batchTest.deleteConfirm", {
        defaultValue:
          "确认删除选中的 {{providerCount}} 个供应商吗？会同时删除 {{accountCount}} 个精确关联的 Codex API Key 账号。",
        providerCount: providerIds.length,
        accountCount: linkedAccountIds.length,
      }),
      {
        title: t("common.delete", "删除"),
        kind: "warning",
        okLabel: t("common.delete", "删除"),
        cancelLabel: t("common.cancel", "取消"),
      },
    );
    if (!confirmed) return;

    setBatchTestDeleting(true);
    setBatchTestError(null);
    try {
      if (linkedAccountIds.length > 0) {
        await deleteCodexAccounts(linkedAccountIds);
        await emitAccountsChanged({
          platformId: "codex",
          reason: "delete",
        });
      }
      for (const providerId of providerIds) {
        await deleteCodexModelProvider(providerId);
      }
      setSelectedProviderIds((previous) => {
        const next = new Set(previous);
        providerIds.forEach((providerId) => next.delete(providerId));
        return next;
      });
      setBatchTestSelectedProviderIds((previous) => {
        const next = new Set(previous);
        providerIds.forEach((providerId) => next.delete(providerId));
        return next;
      });
      setBatchTestResultSelectedProviderIds(new Set());
      setBatchTestSession((current) => {
        if (!current) return current;
        const nextRecords = current.records.filter(
          (record) => !providerIdSet.has(record.providerId),
        );
        return {
          ...current,
          total: nextRecords.length,
          completed: nextRecords.filter(
            (record) => record.status === "success" || record.status === "error",
          ).length,
          successCount: nextRecords.filter((record) => record.status === "success").length,
          failureCount: nextRecords.filter((record) => record.status === "error").length,
          records: nextRecords,
        };
      });
      await reloadProviders();
      setNotice({
        tone: "success",
        text: t("codex.modelProviders.batchTest.deleteSuccess", {
          defaultValue:
            "已删除 {{providerCount}} 个供应商和 {{accountCount}} 个关联账号。",
          providerCount: providerIds.length,
          accountCount: linkedAccountIds.length,
        }),
      });
    } catch (err) {
      setBatchTestError(
        t("codex.modelProviders.batchTest.deleteFailed", {
          defaultValue: "删除选中项失败：{{error}}",
          error: parseServiceError(err),
        }),
      );
    } finally {
      setBatchTestDeleting(false);
    }
  }, [
    accounts,
    batchTestDeleting,
    batchTestResultSelectedProviderIds,
    batchTestSession?.running,
    parseServiceError,
    reloadProviders,
    t,
  ]);

  const handleSaveProvider = useCallback(async () => {
    if (saving) return;
    setFormError(null);
    setNotice(null);

    const name = form.name.trim();
    const baseUrl = form.baseUrl.trim();
    const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(baseUrl);
    const newApiKey = form.newApiKey.trim();
    const modelCatalog = parseModelCatalogText(form.modelCatalogText);
    const parsedWindows = parseContextWindowDrafts(
      form.modelContextWindowsDraft,
      modelCatalog,
    );
    if (!parsedWindows.ok) {
      setFormError(
        t(
          "codex.api.modelCatalog.contextWindowInvalid",
          "上下文窗口必须是大于 0 的整数",
        ),
      );
      return;
    }
    const modelCapabilities = parseVisionModelText(form.visionModelText);
    const visionRoutingModel = form.visionRoutingModel.trim();
    const isCreate = !form.providerId;
    const existingKeyCount = currentEditingProvider?.apiKeys.length ?? 0;

    if (!name) {
      setFormError(
        t("codex.modelProviders.validation.nameRequired", "供应商名称不能为空"),
      );
      return;
    }
    if (!normalizedBaseUrl) {
      setFormError(
        t(
          "codex.modelProviders.validation.baseUrlInvalid",
          "Base URL 格式无效",
        ),
      );
      return;
    }
    if (isCreate && !newApiKey) {
      setFormError(
        t(
          "codex.modelProviders.validation.apiKeyRequiredOnCreate",
          "新增供应商时必须至少填写一个 API Key",
        ),
      );
      return;
    }
    if (!isCreate && existingKeyCount === 0 && !newApiKey) {
      setFormError(
        t(
          "codex.modelProviders.validation.apiKeyRequiredWhenEmpty",
          "当前供应商没有可用 API Key，请先添加一个",
        ),
      );
      return;
    }

    setSaving(true);
    try {
      let savedProvider: CodexModelProvider | null = null;
      if (!form.providerId) {
        savedProvider = await createCodexModelProvider({
          name,
          baseUrl,
          sourceTag: selectedSponsorTemplate?.id,
          modelCatalog,
          modelContextWindows: parsedWindows.windows,
          supportsVision: form.supportsVision,
          modelCapabilities,
          visionRoutingModel,
          website: form.website,
          apiKeyUrl: form.apiKeyUrl,
          wireApi: form.wireApi,
          supportsWebsockets: form.supportsWebsockets,
          enableModePreference: form.enableModePreference,
          integrationType: form.integrationType || undefined,
          initialApiKey: newApiKey || undefined,
          initialApiKeyName: form.newApiKeyName,
        });
      } else {
        savedProvider = await updateCodexModelProvider(form.providerId, {
          name,
          baseUrl,
          sourceTag: selectedSponsorTemplate?.id ?? null,
          modelCatalog,
          modelContextWindows: parsedWindows.windows,
          supportsVision: form.supportsVision,
          modelCapabilities,
          visionRoutingModel,
          website: form.website,
          apiKeyUrl: form.apiKeyUrl,
          wireApi: form.wireApi,
          supportsWebsockets: form.supportsWebsockets,
          enableModePreference: form.enableModePreference,
          integrationType: form.integrationType || null,
        });
        if (newApiKey) {
          savedProvider = await addApiKeyToCodexModelProvider(
            form.providerId,
            newApiKey,
            form.newApiKeyName,
          );
        }
      }
      if (savedProvider && newApiKey) {
        try {
          const usageSummary = await queryCodexModelProviderUsage({
            baseUrl: savedProvider.baseUrl,
            apiKey: newApiKey,
            integrationType: savedProvider.integrationType ?? null,
          });
          setProviderUsageMap((previous) => ({
            ...previous,
            [savedProvider.id]: { loading: false, summary: usageSummary },
          }));
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
      }
      if (savedProvider && currentEditingProvider) {
        const linkedAccountIds = findCodexAccountsReferencingModelProvider(
          currentEditingProvider,
          accounts,
        );
        if (linkedAccountIds.length > 0) {
          const presetId = resolveCodexApiProviderPresetId(savedProvider.baseUrl);
          const isOpenAIOfficial = presetId === "openai_official";
          const wireApi = resolveProviderWireApi(savedProvider);
          const updatedAccountCount = await syncCodexApiKeyProviderAccounts({
            accountIds: linkedAccountIds,
            apiBaseUrl: savedProvider.baseUrl,
            apiProviderMode: isOpenAIOfficial ? "openai_builtin" : "custom",
            apiProviderId:
              presetId === CODEX_API_PROVIDER_CUSTOM_ID
                ? savedProvider.id
                : presetId,
            apiProviderName: savedProvider.name,
            apiModelCatalog: savedProvider.modelCatalog,
            apiModelContextWindows: savedProvider.modelContextWindows,
            apiWireApi: wireApi,
            apiSupportsWebsockets:
              !isOpenAIOfficial &&
              wireApi === "responses" &&
              savedProvider.supportsWebsockets === true,
            apiSupportsVision: savedProvider.supportsVision === true,
            apiModelVisionSupport: Object.fromEntries(
              Object.entries(savedProvider.modelCapabilities ?? {}).map(
                ([model, capability]) => [
                  model,
                  capability.supportsVision === true,
                ],
              ),
            ),
            apiVisionRoutingModel: savedProvider.visionRoutingModel,
          });
          if (updatedAccountCount > 0) {
            await emitAccountsChanged({
              platformId: "codex",
              reason: "provider-snapshot-sync",
            });
          }
        }
      }
      await reloadProviders();
      setShowModal(false);
      setForm(EMPTY_FORM);
      setFormError(null);
      setNotice({
        tone: "success",
        text:
          Object.keys(parsedWindows.windows).length > 0
            ? `${t("codex.modelProviders.saveSuccess", "模型供应商已保存")} ${t(
                "codex.api.modelCatalog.restartHint",
                "模型目录已更新。若 Codex 正在运行，请重启后生效。",
              )}`
            : t("codex.modelProviders.saveSuccess", "模型供应商已保存"),
      });
    } catch (err) {
      setFormError(parseServiceError(err));
    } finally {
      setSaving(false);
    }
  }, [
    accounts,
    currentEditingProvider,
    currentEditingProvider?.apiKeys.length,
    form,
    parseServiceError,
    reloadProviders,
    saving,
    selectedSponsorTemplate?.id,
    t,
  ]);

  const handleDeleteProvider = useCallback(
    async (provider: CodexModelProvider) => {
      const referenceCount = providerReferenceMap.get(provider.id) ?? 0;
      if (referenceCount > 0) {
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.deleteBlocked", {
            defaultValue: "该供应商已被 {{count}} 个账号引用，禁止删除。",
            count: referenceCount,
          }),
        });
        return;
      }
      const confirmed = await confirmDialog(
        t("codex.modelProviders.confirmDelete", {
          defaultValue: "确认删除供应商「{{name}}」吗？",
          name: provider.name,
        }),
        {
          title: t("common.confirm", "确认"),
          kind: "warning",
          okLabel: t("common.delete", "删除"),
          cancelLabel: t("common.cancel", "取消"),
        },
      );
      if (!confirmed) return;
      try {
        await deleteCodexModelProvider(provider.id);
        await reloadProviders();
      } catch (err) {
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.deleteFailed", {
            defaultValue: "删除供应商失败：{{error}}",
            error: parseServiceError(err),
          }),
        });
      }
    },
    [parseServiceError, providerReferenceMap, reloadProviders, t],
  );

  const handleDeleteApiKey = useCallback(
    async (provider: CodexModelProvider, apiKey: CodexModelProviderApiKey) => {
      try {
        await removeApiKeyFromCodexModelProvider(provider.id, apiKey.id);
        await reloadProviders();
      } catch (err) {
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.deleteApiKeyFailed", {
            defaultValue: "删除 API Key 失败：{{error}}",
            error: parseServiceError(err),
          }),
        });
      }
    },
    [parseServiceError, reloadProviders, t],
  );

  const handleSaveApiKeyEdit = useCallback(async () => {
    if (!editingApiKey || saving) return;
    const provider = providers.find((item) => item.id === editingApiKey.providerId);
    if (!provider) return;

    const nextApiKey = editingApiKey.apiKey.trim();
    if (!nextApiKey) {
      setNotice({
        tone: "error",
        text: t("codex.modelProviders.validation.apiKeyRequired", "API Key 不能为空"),
      });
      return;
    }

    setSaving(true);
    try {
      const savedProvider = await updateApiKeyOnCodexModelProvider(
        provider.id,
        editingApiKey.apiKeyId,
        nextApiKey,
        editingApiKey.name,
      );
      const previousApiKey = editingApiKey.originalApiKey.trim();
      const normalizedProviderBaseUrl = normalizeCodexModelProviderBaseUrl(
        savedProvider.baseUrl,
      );
      const linkedAccounts = accounts.filter(
        (account) =>
          isCodexApiKeyAccount(account) &&
          account.openai_api_key?.trim() === previousApiKey &&
          normalizeCodexModelProviderBaseUrl(account.api_base_url ?? "") ===
            normalizedProviderBaseUrl,
      );
      const presetId = resolveCodexApiProviderPresetId(savedProvider.baseUrl);
      const isOpenAIOfficial = presetId === "openai_official";
      const wireApi = resolveProviderWireApi(savedProvider);
      const apiProviderMode = isOpenAIOfficial ? "openai_builtin" : "custom";
      const apiProviderId =
        presetId === CODEX_API_PROVIDER_CUSTOM_ID ? savedProvider.id : presetId;
      for (const account of linkedAccounts) {
        await updateCodexApiKeyCredentials(
          account.id,
          nextApiKey,
          savedProvider.baseUrl,
          apiProviderMode,
          apiProviderId,
          savedProvider.name,
          savedProvider.modelCatalog,
          savedProvider.supportsVision === true,
          Object.fromEntries(
            Object.entries(savedProvider.modelCapabilities ?? {}).map(
              ([model, capability]) => [model, capability.supportsVision === true],
            ),
          ),
          savedProvider.visionRoutingModel,
          wireApi,
          !isOpenAIOfficial &&
            wireApi === "responses" &&
            savedProvider.supportsWebsockets === true,
          account.api_sync_model_catalog_to_codex,
          account.account_name,
          account.api_model_context_windows,
        );
      }
      await reloadProviders();
      if (linkedAccounts.length > 0) {
        await emitAccountsChanged({
          platformId: "codex",
          reason: "provider-api-key-updated",
        });
      }
      setEditingApiKey(null);
      setNotice({
        tone: "success",
        text: t("codex.modelProviders.updateApiKeySuccess", "API Key 已更新"),
      });
    } catch (err) {
      const raw = String(err ?? "");
      const message = raw.includes("API_KEY_EXISTS")
        ? t(
            "codex.modelProviders.validation.apiKeyExists",
            "该 API Key 已存在于当前供应商",
          )
        : raw.includes("API_KEY_REQUIRED")
          ? t(
              "codex.modelProviders.validation.apiKeyRequired",
              "API Key 不能为空",
            )
          : t("codex.modelProviders.updateApiKeyFailed", {
              defaultValue: "更新 API Key 失败：{{error}}",
              error: raw.replace(/^Error:\s*/, ""),
            });
      setNotice({ tone: "error", text: message });
    } finally {
      setSaving(false);
    }
  }, [accounts, editingApiKey, providers, reloadProviders, saving, t]);

  const handleRenameApiKey = useCallback(
    async (provider: CodexModelProvider, apiKey: CodexModelProviderApiKey) => {
      const next = window.prompt(
        t("codex.modelProviders.renameApiKeyPrompt", "重命名 API Key"),
        apiKey.name || "",
      );
      if (next === null) return;
      try {
        const previousName = apiKey.name;
        await renameApiKeyOnCodexModelProvider(provider.id, apiKey.id, next);
        const normalizedProviderBaseUrl = normalizeCodexModelProviderBaseUrl(
          provider.baseUrl,
        );
        const nextName = resolveCodexModelProviderAccountName(provider.name, next);
        const accountsToRename = accounts.filter(
          (account) =>
            isCodexApiKeyAccount(account) &&
            account.openai_api_key?.trim() === apiKey.apiKey.trim() &&
            (account.api_provider_id === provider.id ||
              normalizeCodexModelProviderBaseUrl(account.api_base_url ?? "") ===
                normalizedProviderBaseUrl) &&
            shouldSyncCodexModelProviderAccountName(
              account.account_name,
              provider.name,
              previousName,
            ),
        );
        if (accountsToRename.length > 0) {
          await Promise.all(
            accountsToRename.map((account) =>
              updateCodexAccountName(account.id, nextName),
            ),
          );
          await emitAccountsChanged({
            platformId: "codex",
            reason: "provider-api-key-rename",
          });
        }
        await reloadProviders();
        setNotice({
          tone: "success",
          text: t("codex.modelProviders.renameApiKeySuccess", "API Key 已重命名"),
        });
      } catch (err) {
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.renameApiKeyFailed", {
            defaultValue: "重命名 API Key 失败：{{error}}",
            error: parseServiceError(err),
          }),
        });
      }
    },
    [accounts, parseServiceError, reloadProviders, t],
  );

  const handleBatchDeleteProviders = useCallback(async () => {
    const ids = Array.from(selectedProviderIds);
    if (ids.length === 0) return;
    const confirmed = await confirmDialog(
      t("codex.modelProviders.batchDeleteConfirm", {
        defaultValue: "确定要删除选中的 {{count}} 个供应商吗？",
        count: ids.length,
      }),
      {
        title: t("common.delete", "删除"),
        kind: "warning",
      },
    );
    if (!confirmed) return;
    setNotice(null);
    try {
      for (const id of ids) {
        await deleteCodexModelProvider(id);
      }
      setSelectedProviderIds(new Set());
      await reloadProviders();
      setNotice({
        tone: "success",
        text: t("codex.modelProviders.batchDeleteSuccess", {
          defaultValue: "已删除 {{count}} 个供应商",
          count: ids.length,
        }),
      });
    } catch (err) {
      setNotice({
        tone: "error",
        text: t("codex.modelProviders.deleteFailed", {
          defaultValue: "删除供应商失败：{{error}}",
          error: parseServiceError(err),
        }),
      });
    }
  }, [deleteCodexModelProvider, parseServiceError, reloadProviders, selectedProviderIds, t]);

  const handleProviderInstanceChange = useCallback(
    async (provider: CodexModelProvider, instanceId: string) => {
      setNotice(null);
      setLastEnabledProviderId(null);
      setProviders((previous) =>
        previous.map((item) =>
          item.id === provider.id
            ? { ...item, boundInstanceId: instanceId }
            : item,
        ),
      );
      try {
        const updated = await updateCodexModelProvider(provider.id, {
          boundInstanceId: instanceId,
        });
        setProviders((previous) =>
          previous.map((item) => (item.id === updated.id ? updated : item)),
        );
        await reloadProviders();
      } catch (err) {
        await reloadProviders();
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.instance.saveFailed", {
            defaultValue: "保存实例绑定失败：{{error}}",
            error: parseServiceError(err),
          }),
        });
      }
    },
    [parseServiceError, reloadProviders, t],
  );

  const isOAuthBindingEligibleAccount = useCallback(
    (account: CodexAccount): boolean =>
      Boolean((account.tokens?.refresh_token || "").trim()),
    [],
  );

  const providerOauthTarget = useMemo(
    () =>
      providerOauthPickerId
        ? (providers.find((item) => item.id === providerOauthPickerId) ?? null)
        : null,
    [providerOauthPickerId, providers],
  );

  const providerOauthAccounts = useMemo(
    () => oauthAccounts.filter((account) => !isCodexApiKeyAccount(account)),
    [oauthAccounts],
  );

  const providerOauthEligibleAccounts = useMemo(
    () => providerOauthAccounts.filter(isOAuthBindingEligibleAccount),
    [isOAuthBindingEligibleAccount, providerOauthAccounts],
  );

  const selectedProviderOauthAccount = useMemo(
    () =>
      providerOauthEligibleAccounts.find(
        (item) => item.id === providerOauthSelectedAccountId,
      ) ?? null,
    [providerOauthEligibleAccounts, providerOauthSelectedAccountId],
  );

  const providerOauthHasExistingBinding = useMemo(
    () => Boolean(providerOauthTarget?.boundOauthAccountId?.trim()),
    [providerOauthTarget?.boundOauthAccountId],
  );

  const providerOauthTierCounts = useMemo(() => {
    const counts = createCodexPlanFilterCounts(
      providerOauthEligibleAccounts.length,
    );
    providerOauthEligibleAccounts.forEach((account) => {
      incrementCodexPlanFilterCount(counts, resolvePlanKey(account));
    });
    return counts;
  }, [providerOauthEligibleAccounts, resolvePlanKey]);

  const providerOauthTierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      buildCodexPlanFilterOptions(providerOauthTierCounts, {
        includeError: false,
        pendingLabel: t("codex.pendingAuth.badge", "待授权"),
      }),
    [providerOauthTierCounts, t],
  );

  const providerOauthAvailableTags = useMemo(() => {
    const tagSet = new Set<string>();
    providerOauthEligibleAccounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const trimmed = tag.trim();
        if (trimmed) tagSet.add(trimmed);
      });
    });
    return Array.from(tagSet).sort((a, b) => a.localeCompare(b));
  }, [providerOauthEligibleAccounts]);

  const toggleProviderOAuthFilterTypeValue = useCallback((value: string) => {
    setProviderOauthFilterTypes((prev) =>
      prev.includes(value)
        ? prev.filter((item) => item !== value)
        : [...prev, value],
    );
  }, []);

  const toggleProviderOAuthTagFilterValue = useCallback((tag: string) => {
    setProviderOauthTagFilter((prev) =>
      prev.includes(tag) ? prev.filter((item) => item !== tag) : [...prev, tag],
    );
  }, []);

  const providerOauthFilteredAccounts = useMemo(() => {
    let result = [...providerOauthEligibleAccounts];
    const query = providerOauthSearchQuery.trim().toLowerCase();
    if (query) {
      result = result.filter((account) => {
        const presentation = resolvePresentation(account);
        const searchable = [
          presentation.displayName,
          account.email,
          account.account_name,
          account.account_id,
          account.organization_id,
          account.plan_type,
          ...(account.tags || []),
        ]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        return searchable.includes(query);
      });
    }
    if (providerOauthFilterTypes.length > 0) {
      const { selectedTypes } =
        splitValidityFilterValues(providerOauthFilterTypes);
      if (selectedTypes.size > 0) {
        result = result.filter((account) => {
          if (selectedTypes.has("ERROR") && account.quota_error) return true;
          return selectedTypes.has(resolvePlanKey(account));
        });
      }
    }
    if (providerOauthTagFilter.length > 0) {
      const selectedTags = new Set(providerOauthTagFilter.map(normalizeTag));
      result = result.filter((account) =>
        (account.tags || [])
          .map(normalizeTag)
          .some((tag) => selectedTags.has(tag)),
      );
    }
    result.sort((a, b) => {
      if (providerOauthSortBy === "created_at") {
        const diff = b.created_at - a.created_at;
        return providerOauthSortDirection === "desc" ? diff : -diff;
      }
      if (providerOauthSortBy === "last_used") {
        const diff = b.last_used - a.last_used;
        return providerOauthSortDirection === "desc" ? diff : -diff;
      }
      if (providerOauthSortBy === "plan") {
        const diff = resolvePresentation(a).planLabel.localeCompare(
          resolvePresentation(b).planLabel,
        );
        return providerOauthSortDirection === "desc" ? -diff : diff;
      }
      const diff = resolvePresentation(a).displayName.localeCompare(
        resolvePresentation(b).displayName,
      );
      return providerOauthSortDirection === "desc" ? -diff : diff;
    });
    return result;
  }, [
    normalizeTag,
    providerOauthEligibleAccounts,
    providerOauthFilterTypes,
    providerOauthSearchQuery,
    providerOauthSortBy,
    providerOauthSortDirection,
    providerOauthTagFilter,
    resolvePlanKey,
    resolvePresentation,
  ]);

  const providerOauthPagination = usePagination({
    items: providerOauthFilteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey("CodexProviderOAuthBinding"),
    pageSizeOptions: OAUTH_BINDING_PAGE_SIZE_OPTIONS,
    defaultPageSize: OAUTH_BINDING_PAGE_SIZE_OPTIONS[0],
  });

  const handleProviderOauthBindingChange = useCallback(
    async (
      provider: CodexModelProvider,
      boundOauthAccountId: string | null,
    ) => {
      setProviderOauthSaving(true);
      setNotice(null);
      try {
        await updateCodexModelProvider(provider.id, {
          boundOauthAccountId,
        });
        await reloadProviders();
        setNotice({
          tone: "success",
          text: boundOauthAccountId
            ? t("codex.api.oauthBinding.saveSuccess", "OAuth 绑定已更新")
            : t("codex.api.oauthBinding.clearSuccess", "OAuth 绑定已解除"),
        });
        setProviderOauthPickerId(null);
      } catch (err) {
        setNotice({
          tone: "error",
          text: boundOauthAccountId
            ? t("codex.api.oauthBinding.saveFailed", {
                defaultValue: "OAuth 绑定失败：{{error}}",
                error: parseServiceError(err),
              })
            : t("codex.api.oauthBinding.clearFailed", {
                defaultValue: "解除 OAuth 绑定失败：{{error}}",
                error: parseServiceError(err),
              }),
        });
      } finally {
        setProviderOauthSaving(false);
      }
    },
    [parseServiceError, reloadProviders, t],
  );

  useEffect(() => {
    if (!providerOauthTarget) {
      setProviderOauthSelectedAccountId("");
      setProviderOauthSearchQuery("");
      setProviderOauthFilterTypes([]);
      setProviderOauthTagFilter([]);
      setProviderOauthSortBy("last_used");
      setProviderOauthSortDirection("desc");
      return;
    }
    const bound = resolveBoundOAuthAccount(providerOauthTarget);
    setProviderOauthSelectedAccountId(
      bound && isOAuthBindingEligibleAccount(bound) ? bound.id : "",
    );
    setProviderOauthSearchQuery("");
    setProviderOauthFilterTypes([]);
    setProviderOauthTagFilter([]);
    setProviderOauthSortBy("last_used");
    setProviderOauthSortDirection("desc");
  }, [
    isOAuthBindingEligibleAccount,
    providerOauthTarget,
    resolveBoundOAuthAccount,
  ]);

  useEffect(() => {
    if (!providerOauthTarget) return;
    providerOauthPagination.setCurrentPage(1);
  }, [
    providerOauthFilterTypes,
    providerOauthPagination.setCurrentPage,
    providerOauthSearchQuery,
    providerOauthSortBy,
    providerOauthSortDirection,
    providerOauthTagFilter,
    providerOauthTarget,
  ]);

  const isCurrentProviderActive = useCallback(
    (
      provider: CodexModelProvider,
      targetInstance: InstanceProfile | null,
    ): boolean => {
      const targetInstanceId = targetInstance?.id ?? DEFAULT_INSTANCE_ID;
      if (lastEnabledProviderId === `${targetInstanceId}:${provider.id}`) {
        return true;
      }
      const normalizedProviderBaseUrl = normalizeCodexModelProviderBaseUrl(
        provider.baseUrl,
      );
      const selectedBindAccountId = targetInstance?.bindAccountId ?? null;
      const providerGatewayAccountId =
        selectedBindAccountId?.startsWith(CODEX_PROVIDER_GATEWAY_BIND_PREFIX)
          ? selectedBindAccountId.slice(CODEX_PROVIDER_GATEWAY_BIND_PREFIX.length)
          : null;

      if (
        selectedBindAccountId &&
        selectedBindAccountId !== CODEX_API_SERVICE_BIND_ID
      ) {
        if (providerGatewayAccountId) {
          const boundGatewayAccount = accounts.find(
            (account) => account.id === providerGatewayAccountId,
          );
          return (
            boundGatewayAccount?.auth_mode === "apikey" &&
            normalizeCodexModelProviderBaseUrl(
              boundGatewayAccount.api_base_url ?? "",
            ) === normalizedProviderBaseUrl
          );
        }
        const boundAccount = accounts.find(
          (account) => account.id === selectedBindAccountId,
        );
        return (
          boundAccount?.auth_mode === "apikey" &&
          normalizeCodexModelProviderBaseUrl(boundAccount.api_base_url ?? "") ===
            normalizedProviderBaseUrl
        );
      }

      const directActive =
        targetInstanceId === DEFAULT_INSTANCE_ID &&
        currentAccount?.auth_mode === "apikey" &&
        normalizeCodexModelProviderBaseUrl(currentAccount.api_base_url ?? "") ===
          normalizedProviderBaseUrl;
      if (directActive) return true;

      const gatewayAccountIds = new Set(
        localAccessState?.collection?.accountIds ?? [],
      );
      if (
        !localAccessState?.collection?.enabled ||
        gatewayAccountIds.size === 0 ||
        selectedBindAccountId !== CODEX_API_SERVICE_BIND_ID
      ) {
        return false;
      }
      return accounts.some((account) => {
        if (!gatewayAccountIds.has(account.id)) return false;
        if (account.auth_mode !== "apikey") return false;
        return (
          normalizeCodexModelProviderBaseUrl(account.api_base_url ?? "") ===
          normalizedProviderBaseUrl
        );
      });
    },
    [
      accounts,
      currentAccount,
      lastEnabledProviderId,
      localAccessState,
    ],
  );

  const handleEnableProvider = useCallback(
    async (
      provider: CodexModelProvider,
      apiKey: CodexModelProviderApiKey,
      instanceId: string,
      instanceName: string,
    ) => {
      if (enablingProviderId) return;
      setNotice(null);
      const presetId = resolveCodexApiProviderPresetId(provider.baseUrl);
      const isOpenAIOfficial = presetId === "openai_official";
      const wireApi = resolveProviderWireApi(provider);
      const deepSeekDraft =
        accounts.find(
          (item) =>
            item.auth_mode === "apikey" &&
            item.openai_api_key === apiKey.apiKey &&
            isDeepSeekAccount(item),
        ) ?? {
          api_provider_id: presetId,
          api_base_url: provider.baseUrl,
          api_wire_api: wireApi,
        };
      let deepSeekChoice: Awaited<ReturnType<typeof deepSeekStart.requestStart>> =
        null;
      if (isDeepSeekAccount(deepSeekDraft)) {
        deepSeekChoice = await deepSeekStart.requestStart(
          deepSeekDraft,
          instanceName,
        );
        if (!deepSeekChoice) return;
      }
      setEnablingProviderId(provider.id);
      try {
        const enableMode = resolveGatewayModeByWireApi(wireApi, presetId);
        const account = await addCodexAccountWithApiKey(
          apiKey.apiKey,
          provider.baseUrl,
          isOpenAIOfficial ? "openai_builtin" : "custom",
          presetId === CODEX_API_PROVIDER_CUSTOM_ID ? provider.id : presetId,
          provider.name,
          provider.modelCatalog,
          provider.supportsVision === true,
          Object.fromEntries(
            Object.entries(provider.modelCapabilities ?? {}).map(([model, capability]) => [
              model,
              capability.supportsVision === true,
            ]),
          ),
          provider.visionRoutingModel,
          undefined,
          wireApi,
          provider.supportsWebsockets,
          undefined,
          provider.modelContextWindows,
        );
        await updateCodexApiKeyBoundOAuthAccount(
          account.id,
          provider.boundOauthAccountId?.trim() || null,
        );
        const startedAccount = deepSeekChoice
          ? await updateAccountInstanceAccess(
              account.id,
              deepSeekChoice.accessMode,
              deepSeekChoice.modelId,
            )
          : account;

        await updateCodexInstance({
          instanceId,
          bindAccountId: isDeepSeekAccount(startedAccount)
            ? resolveDeepSeekBindAccountId(startedAccount)
            : isOpenAIOfficial || enableMode === "direct"
              ? startedAccount.id
              : buildCodexProviderGatewayBindId(startedAccount.id),
          followLocalAccount: false,
        });
        await startCodexInstance(instanceId);

        await reloadCurrentAccount();
        await reloadLocalAccessState();
        await reloadCodexInstances();
        setLastEnabledProviderId(`${instanceId}:${provider.id}`);
        setNotice({
          tone: "success",
          text: t("codex.modelProviders.enableSuccess", {
            defaultValue:
              "已启用 {{name}}，并启动 {{instance}}。",
            name: provider.name,
            instance: instanceName,
          }),
        });
      } catch (err) {
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.enableFailed", {
            defaultValue: "启用供应商失败：{{error}}",
            error: parseServiceError(err),
          }),
        });
      } finally {
        setEnablingProviderId(null);
      }
    },
    [
      accounts,
      deepSeekStart.requestStart,
      enablingProviderId,
      updateAccountInstanceAccess,
      parseServiceError,
      reloadCurrentAccount,
      reloadCodexInstances,
      reloadLocalAccessState,
      t,
    ],
  );

  const handleTestProvider = useCallback(
    async (
      provider: CodexModelProvider,
      apiKey: CodexModelProviderApiKey,
      wireApi: CodexProviderWireApi,
    ) => {
      if (testingProviderId) return;
      setNotice(null);
      setTestingProviderId(provider.id);
      try {
        const result = await testCodexModelProviderConnection({
          baseUrl: provider.baseUrl,
          apiKey: apiKey.apiKey,
          wireApi,
        });
        if (result.failure) {
          setNotice({
            tone: "error",
            text: formatProviderTestFailure(result.failure),
          });
          return;
        }
        setNotice({
          tone: "success",
          text: t("codex.modelProviders.testSuccess", {
            defaultValue:
              "供应商连接正常：{{protocol}}，{{model}}，{{latency}}。",
            protocol:
              wireApi === "chat_completions"
                ? t(
                    "codex.modelProviders.wireApi.chatCompletions",
                    "Chat Completions",
                  )
                : t(
                    "codex.modelProviders.wireApi.responses",
                    "Responses 原生",
                  ),
            model: result.output ?? result.modelId ?? provider.name,
            latency:
              result.latencyMs === null
                ? "-"
                : `${Math.max(0, Math.round(result.latencyMs))}ms`,
          }),
        });
      } catch (err) {
        setNotice({
          tone: "error",
          text: t("codex.modelProviders.testFailed", {
            defaultValue: "测试供应商失败：{{error}}",
            error: parseServiceError(err),
          }),
        });
      } finally {
        setTestingProviderId(null);
      }
    },
    [formatProviderTestFailure, parseServiceError, t, testingProviderId],
  );

  const refreshProviderUsage = useCallback(
    async (provider: CodexModelProvider, apiKey?: CodexModelProviderApiKey | null) => {
      if (!apiKey) return;
      setProviderUsageMap((previous) => ({
        ...previous,
        [provider.id]: {
          ...previous[provider.id],
          loading: true,
          error: undefined,
          unavailable: false,
        },
      }));
      try {
        const summary = await queryCodexModelProviderUsage({
          baseUrl: provider.baseUrl,
          apiKey: apiKey.apiKey,
          integrationType: provider.integrationType ?? null,
        });
        if (
          (summary.mode === "sub2api" || summary.mode === "new_api") &&
          summary.mode !== provider.integrationType
        ) {
          await saveCodexModelProviderDetectedIntegrationType(provider.id, summary.mode);
          await reloadProviders();
        }
        setProviderUsageMap((previous) => ({
          ...previous,
          [provider.id]: { loading: false, summary, updatedAt: Date.now() },
        }));
      } catch (err) {
        const errorMessage = parseServiceError(err);
        const unavailable =
          errorMessage.includes("PROVIDER_USAGE_DETECT_FAILED") ||
          errorMessage.includes("PROVIDER_USAGE_HTTP_404") ||
          errorMessage.includes("PROVIDER_USAGE_TYPE_UNSUPPORTED");
        setProviderUsageMap((previous) => ({
          ...previous,
          [provider.id]: {
            loading: false,
            summary: previous[provider.id]?.summary,
            error: unavailable ? undefined : errorMessage,
            unavailable,
            updatedAt: Date.now(),
          },
        }));
      }
    },
    [parseServiceError, reloadProviders],
  );

  const refreshAllProviderUsage = useCallback(async () => {
    if (providerUsageRefreshingAll) return;
    const refreshTargets = providers
      .map((provider) => ({
        provider,
        apiKey: getSelectedProviderApiKey(provider),
      }))
      .filter(
        (item): item is {
          provider: CodexModelProvider;
          apiKey: CodexModelProviderApiKey;
        } => Boolean(item.apiKey),
      );
    if (refreshTargets.length === 0) return;
    setProviderUsageRefreshingAll(true);
    try {
      await Promise.all(
        refreshTargets.map(({ provider, apiKey }) =>
          refreshProviderUsage(provider, apiKey),
        ),
      );
    } finally {
      setProviderUsageRefreshingAll(false);
    }
  }, [
    getSelectedProviderApiKey,
    providerUsageRefreshingAll,
    providers,
    refreshProviderUsage,
  ]);

  const formatUsageMoney = useCallback(
    (value?: number | null, unit?: string | null): string =>
      formatModelProviderUsageMoney(value ?? undefined, unit ?? undefined),
    [],
  );

  const formatUsageQuotaValue = useCallback(
    (
      summary: CodexModelProviderUsageSummary | undefined,
      value?: number | null,
    ): string => {
      if (summary?.quotaUnlimited === true) {
        return t("codex.modelProviders.usage.unlimitedQuota", "无限额度");
      }
      return formatUsageMoney(value, summary?.unit);
    },
    [formatUsageMoney, t],
  );

  const formatUsageDetailLabel = useCallback(
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
        accessUntil: t("codex.modelProviders.usage.fields.accessUntil", "可用至"),
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
        totalUsage: t("codex.modelProviders.usage.fields.totalUsage", "累计消耗"),
        isAvailable: t("codex.modelProviders.usage.fields.isAvailable", "余额可用"),
        currency: t("codex.modelProviders.usage.fields.currency", "币种"),
        totalBalance: t("codex.modelProviders.usage.fields.totalBalance", "总余额"),
        grantedBalance: t("codex.modelProviders.usage.fields.grantedBalance", "赠金余额"),
        toppedUpBalance: t("codex.modelProviders.usage.fields.toppedUpBalance", "充值余额"),
      };
      return labels[key] ?? fallback;
    },
    [t],
  );

  const formatUsageDetailValue = useCallback(
    (item: { key: string; value: string }, unit?: string | null): string => {
      const raw = item.value.trim();
      const numeric = Number(raw);
      if (
        Number.isFinite(numeric) &&
        (item.key.includes("Tokens") ||
          item.key === "todayTokens" ||
          item.key === "totalTokens")
      ) {
        return numeric.toLocaleString("en-US");
      }
      if (Number.isFinite(numeric) && item.key === "accessUntil") {
        return numeric > 0 ? formatDateTime(numeric * 1000) : "-";
      }
      if (Number.isFinite(numeric) && item.key === "expiresAt") {
        return numeric > 0 ? formatDateTime(numeric * 1000) : "-";
      }
      if (
        Number.isFinite(numeric) &&
        (item.key === "intervalExpiresAt" || item.key === "weeklyExpiresAt")
      ) {
        return numeric > 0 ? formatDateTime(numeric * 1000) : "-";
      }
      if (
        item.key === "quotaUnlimited" ||
        item.key === "modelLimitsEnabled" ||
        item.key === "isAvailable"
      ) {
        if (raw === "true") return t("codex.modelProviders.usage.booleanTrue", "是");
        if (raw === "false") return t("codex.modelProviders.usage.booleanFalse", "否");
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
        return formatUsageMoney(numeric, unit);
      }
      if (Number.isFinite(numeric) && ["totalGranted", "totalAvailable"].includes(item.key)) {
        return formatUsageMoney(numeric, unit);
      }
      if (Number.isFinite(numeric) && item.key === "totalUsage") {
        return formatUsageMoney(numeric / 100, unit);
      }
      if (
        Number.isFinite(numeric) &&
        (item.key.includes("Requests") ||
          item.key === "todayRequests" ||
          item.key === "totalRequests")
      ) {
        return numeric.toLocaleString("en-US");
      }
      return raw || "-";
    },
    [formatUsageMoney, t],
  );

  return (
    <div className="codex-provider-manager-page">
      {notice && (
        <div
          className={`message-bar ${notice.tone === "error" ? "error" : "success"}`}
        >
          {notice.text}
          <button
            onClick={() => setNotice(null)}
            aria-label={t("common.close", "关闭")}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {showQuickConfigModal && (
        <CodexQuickConfigCard onClose={() => setShowQuickConfigModal(false)} />
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search className="search-icon" size={16} />
            <input
              type="text"
              placeholder={t("common.search", "搜索...")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
          <div className="view-switcher">
            <button
              className={`view-btn ${providerViewMode === "compact" ? "active" : ""}`}
              onClick={() => setProviderViewMode("compact")}
              title={t("accounts.view.compact", "紧凑视图")}
            >
              <Rows3 size={16} />
            </button>
            <button
              className={`view-btn ${providerViewMode === "grid" ? "active" : ""}`}
              onClick={() => setProviderViewMode("grid")}
              title={t("common.shared.view.grid", "卡片视图")}
            >
              <LayoutGrid size={16} />
            </button>
          </div>
          <MultiSelectFilterDropdown
            options={providerFilterOptions}
            selectedValues={providerNameFilter}
            allLabel={t("common.shared.filter.all", {
              count: providers.length,
            })}
            filterLabel={t("common.shared.filterLabel", "筛选")}
            clearLabel={t("accounts.clearFilter", "清空筛选")}
            emptyLabel={t("common.none", "暂无")}
            ariaLabel={t("common.shared.filterLabel", "筛选")}
            onToggleValue={(value) =>
              setProviderNameFilter((previous) =>
                previous.includes(value)
                  ? previous.filter((item) => item !== value)
                  : [...previous, value],
              )
            }
            onClear={() => setProviderNameFilter([])}
          />
          <SingleSelectFilterDropdown
            value={providerSortBy}
            options={[
              {
                value: "name",
                label: t("common.shared.sort.name", "按名称"),
              },
              {
                value: "created_at",
                label: t("common.shared.sort.createdAt", "按创建时间"),
              },
              {
                value: "custom",
                label: t("codex.modelProviders.sort.custom", "自定义顺序"),
              },
            ]}
            ariaLabel={t("common.shared.sortLabel", "排序")}
            icon={<ArrowDownWideNarrow size={14} />}
            onChange={handleProviderSortByChange}
          />
          {!isProviderCustomSortActive && (
            <button
              className="sort-direction-btn"
              onClick={() =>
                setProviderSortDirection((previous) =>
                  previous === "asc" ? "desc" : "asc",
                )
              }
              title={t("common.shared.sort.toggleDirection", "切换排序方向")}
            >
              {providerSortDirection === "desc" ? "⬇" : "⬆"}
            </button>
          )}
        </div>
        <div className="toolbar-right">
          <button
            className="btn btn-secondary icon-only"
            onClick={() => void refreshAllProviderUsage()}
            disabled={
              providerUsageRefreshingAll ||
              providers.every((provider) => !getSelectedProviderApiKey(provider))
            }
            title={t("common.shared.refreshQuota", "刷新配额")}
          >
            <RefreshCw
              size={14}
              className={providerUsageRefreshingAll ? "loading-spinner" : undefined}
            />
          </button>
          <button
            className="btn btn-primary icon-only"
            onClick={openCreateModal}
            title={t("codex.modelProviders.add", "新增供应商")}
          >
            <Plus size={14} />
          </button>
          <button
            className="btn btn-secondary icon-only"
            onClick={() => setShowQuickConfigModal(true)}
            title={t("codex.modelProviders.quickConfig.title", "当前 Codex 配置")}
          >
            <Settings size={14} />
          </button>
          {selectedProviderIds.size > 0 && (
            <button
              className="btn btn-danger icon-only"
              onClick={() => void handleBatchDeleteProviders()}
              title={`${t("common.delete", "删除")} (${selectedProviderIds.size})`}
            >
              <Trash2 size={14} />
            </button>
          )}
        </div>
      </div>

      {error && (
        <div className="add-status error">
          <CircleAlert size={16} />
          <span>{error}</span>
        </div>
      )}

      {filteredProviderIds.length > 0 && (
        <div className="codex-overview-selection-bar">
          <div className="codex-overview-selection-left">
            <label className="codex-overview-select-all">
              <input
                type="checkbox"
                checked={isAllProvidersSelected}
                onChange={() => toggleSelectAllProviders(filteredProviderIds)}
              />
              <span>{t("common.selectAll", "全选")}</span>
            </label>
            {selectedProviderIds.size > 0 && (
              <span className="codex-overview-selected-count">
                {t("codex.modelProviders.batchTest.selectedCount", {
                  defaultValue: "已选 {{count}} 个",
                  count: selectedProviderIds.size,
                })}
              </span>
            )}
          </div>
          <div className="codex-overview-selection-actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={openBatchTestModal}
              disabled={filteredProviders.every((provider) => !getSelectedProviderApiKey(provider))}
              title={t(
                "codex.modelProviders.batchTest.entryHint",
                "批量测试供应商真实对话能力",
              )}
            >
              <Activity size={14} />
              {t("codex.modelProviders.batchTest.entry", "一键测试")}
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="section-desc">{t("common.loading", "加载中...")}</div>
      ) : providers.length === 0 ? (
        <div className="empty-state">
          <h3>{t("codex.modelProviders.emptyTitle", "暂无模型供应商")}</h3>
          <p>
            {t(
              "codex.modelProviders.emptyDesc",
              "点击右上角“新增供应商”开始维护。",
            )}
          </p>
        </div>
      ) : filteredProviders.length === 0 ? (
        <div className="empty-state">
          <h3>{t("codex.modelProviders.noMatchTitle", "没有匹配的供应商")}</h3>
          <p>{t("common.shared.noMatch.desc", "请尝试调整搜索或筛选条件")}</p>
        </div>
      ) : (
        <div className={`codex-provider-grid ${providerViewMode === "compact" ? "compact" : ""}`}>
          {filteredProviders.map((provider) => {
            const presetId = resolveCodexApiProviderPresetId(provider.baseUrl);
            const capabilityProfile = resolveCodexProviderCapabilityProfile({
              presetId,
              baseUrl: provider.baseUrl,
              wireApi: provider.wireApi,
            });
            const primaryApiKey = getSelectedProviderApiKey(provider);
            const enabling = enablingProviderId === provider.id;
            const testing = testingProviderId === provider.id;
            const usageState = providerUsageMap[provider.id];
            const usageSummary = usageState?.summary;
            const usagePrimaryText = usageSummary
              ? formatUsageQuotaValue(
                  usageSummary,
                  usageSummary.quotaRemaining ??
                    usageSummary.remaining ??
                    usageSummary.balance,
                )
              : "-";
            const usageRequestText =
              usageSummary?.todayRequests != null
                ? String(usageSummary.todayRequests)
                : "-";
            const targetInstanceId = getProviderInstanceId(provider);
            const targetInstance = resolveInstanceById(targetInstanceId);
            const targetInstanceName = getInstanceName(targetInstance);
            const targetInstanceReady = isInstanceReady(targetInstance);
            const active = isCurrentProviderActive(provider, targetInstance);
            const sponsorProvider = isSponsorProvider(
              provider,
              sponsorProviderTemplates,
            );
            const selectedApiKeyLine = primaryApiKey
              ? `${t("codex.addModal.token", "API Key")}：${maskApiKey(
                  primaryApiKey.apiKey,
                )}`
              : `${t("codex.addModal.token", "API Key")}：${t(
                  "common.none",
                  "暂无",
                )}`;
            const oauthBindingLine = `${t(
              "codex.api.oauthBinding.label",
              "OAuth 绑定",
            )}：${
              resolveBoundOAuthAccount(provider)?.account_name ||
              resolveBoundOAuthAccount(provider)?.email ||
              resolveBoundOAuthAccount(provider)?.id ||
              t("codex.api.oauthBinding.unbound", "未绑定")
            }`;
            const providerLine = `${t("codex.api.provider.label", "供应商")}：${
              provider.name
            }`;
            const apiBaseUrlLine = `${t("codex.api.baseUrl", "Base URL")}：${
              provider.baseUrl
            }`;
            const usageMode =
              usageSummary?.mode === "new_api" ||
              usageSummary?.mode === "sub2api" ||
              usageSummary?.mode === "deepseek" ||
              usageSummary?.mode === "token_plan"
                ? usageSummary.mode
                : provider.integrationType ?? null;
            const deepSeekDetailValue = (key: string) => {
              const item = usageSummary?.details?.find((detail) => detail.key === key);
              return item ? formatUsageDetailValue(item, usageSummary?.unit) : "-";
            };
            const {
              granted: totalGranted,
              available: totalAvailable,
              expiresAt,
            } = resolveNewApiQuotaSnapshot(usageSummary);
            const tokenPlanResetDetail = usageSummary?.details?.find((detail) =>
              ["intervalExpiresAt", "weeklyExpiresAt", "expiresAt"].includes(
                detail.key,
              ),
            );
            const tokenPlanResetText = tokenPlanResetDetail
              ? formatUsageDetailValue(
                  tokenPlanResetDetail,
                  usageSummary?.unit,
                )
              : "-";
            const progressPercent =
              usageMode === "new_api" &&
              totalGranted != null &&
              totalAvailable != null &&
              totalGranted > 0
                ? Math.max(
                    0,
                    Math.min(
                      100,
                      Math.round(
                        ((totalGranted - totalAvailable) / totalGranted) * 100,
                      ),
                    ),
                  )
                : usageSummary?.quotaUnlimited
                  ? 100
                  : 0;
            return (
              <div
                className={`codex-account-card codex-provider-card ${active ? "current" : ""} ${sponsorProvider ? "sponsor-api-account" : ""}`}
                key={provider.id}
              >
                <div className="card-top">
                  <div className="card-select">
                    <input
                      type="checkbox"
                      checked={selectedProviderIds.has(provider.id)}
                      onChange={() => toggleProviderSelected(provider.id)}
                    />
                  </div>
                  <span className="account-email" title={provider.name}>
                    {provider.name}
                  </span>
                </div>
                <div className="account-sub-line">
                  {provider.apiKeys.length > 0 && primaryApiKey ? (
                    <div className="codex-provider-inline-line codex-provider-api-key-line">
                      <div
                        className="codex-api-key-reveal-line codex-provider-api-key-trigger"
                        title={selectedApiKeyLine}
                      >
                        <span className="codex-login-subline">
                          {selectedApiKeyLine}
                        </span>
                        <button
                          type="button"
                          className="codex-provider-inline-icon-btn"
                          onClick={() => {
                            void navigator.clipboard.writeText(primaryApiKey.apiKey);
                            setNotice({
                              tone: "success",
                              text: t(
                                "codex.modelProviders.apiKeyCopied",
                                "API Key 已复制",
                              ),
                            });
                          }}
                          title={t("common.copy", "复制")}
                        >
                          <Copy size={12} />
                        </button>
                        {provider.apiKeys.length > 1 && (
                          <button
                            type="button"
                            className="codex-provider-inline-dropdown-btn"
                            onClick={() => {
                              setPickerSearchQuery("");
                              setApiKeyPickerProviderId(provider.id);
                            }}
                            title={t("codex.modelProviders.existingApiKeys", "已有 API Key")}
                          >
                            <ChevronDown size={12} />
                          </button>
                        )}
                      </div>
                    </div>
                  ) : (
                    <span
                      className="codex-login-subline codex-provider-inline-text"
                      title={selectedApiKeyLine}
                    >
                      {selectedApiKeyLine}
                    </span>
                  )}
                </div>
                <div className="account-sub-line codex-provider-inline-line codex-oauth-binding-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={oauthBindingLine}
                  >
                    {oauthBindingLine}
                  </span>
                  <button
                    type="button"
                    className="codex-provider-inline-switch codex-oauth-binding-action"
                    onClick={() => {
                      setPickerSearchQuery("");
                      setProviderOauthPickerId(provider.id);
                    }}
                    title={t("codex.api.oauthBinding.action", "绑定 OAuth")}
                  >
                    <Link2 size={11} />
                    {resolveBoundOAuthAccount(provider)
                      ? t("common.detail", "详情")
                      : t("codex.api.oauthBinding.actionShort", "绑定")}
                  </button>
                </div>
                <div className="account-sub-line codex-provider-inline-line">
                  <span
                    className="codex-login-subline codex-provider-inline-text"
                    title={providerLine}
                  >
                    {providerLine}
                  </span>
                  <button
                    type="button"
                    className="codex-provider-inline-switch"
                    onClick={() => setProviderDetailId(provider.id)}
                    title={t("codex.quickSwitch.inlineAction", "切换")}
                  >
                    {t("codex.quickSwitch.inlineAction", "切换")}
                  </button>
                </div>
                <div className="account-sub-line">
                  <span className="codex-login-subline" title={apiBaseUrlLine}>
                    {apiBaseUrlLine}
                  </span>
                </div>
                <div className="codex-quota-section">
                  {usageMode === "deepseek" ? (
                    <div className="codex-api-key-usage-panel sub2api">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.totalBalance", "总余额")}</span>
                          <strong>{usagePrimaryText}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.grantedBalance", "赠金余额")}</span>
                          <strong>{deepSeekDetailValue("grantedBalance")}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.toppedUpBalance", "充值余额")}</span>
                          <strong>{deepSeekDetailValue("toppedUpBalance")}</strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "token_plan" ? (
                    <div className="codex-api-key-usage-panel token-plan">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.remaining",
                              "Remaining",
                            )}
                          </span>
                          <strong>{usagePrimaryText}</strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.planName",
                              "Plan",
                            )}
                          </span>
                          <strong>{usageSummary?.planName || "-"}</strong>
                        </div>
                        <div>
                          <span>
                            {t(
                              "codex.modelProviders.usage.fields.expiresAt",
                              "Next Reset",
                            )}
                          </span>
                          <strong>{tokenPlanResetText}</strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "sub2api" ? (
                    <div className="codex-api-key-usage-panel sub2api">
                      <div className="codex-api-key-usage-grid">
                        <div>
                          <span>{t("codex.modelProviders.usage.accountBalance", "账户余额")}</span>
                          <strong>{usagePrimaryText}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.todayRequests", "今日请求")}</span>
                          <strong>{usageRequestText}</strong>
                        </div>
                        <div>
                          <span>{t("codex.modelProviders.usage.fields.todayTokens", "今日 Token")}</span>
                          <strong>
                            {typeof usageSummary?.todayTotalTokens === "number"
                              ? usageSummary.todayTotalTokens.toLocaleString("en-US")
                              : "-"}
                          </strong>
                        </div>
                      </div>
                    </div>
                  ) : usageMode === "new_api" ? (
                    <div className="codex-api-key-usage-panel">
                      <div
                        className="quota-item codex-api-key-quota-item new-api"
                        title={`${t("codex.cockpitApi.balance", "额度")}：${
                          usageSummary?.quotaUnlimited
                            ? t("codex.newApi.quota.unlimited", "不限量")
                            : totalAvailable != null && totalGranted != null
                              ? `${formatUsageMoney(totalAvailable, usageSummary?.unit)} / ${formatUsageMoney(totalGranted, usageSummary?.unit)}`
                              : totalAvailable != null
                                ? formatUsageMoney(totalAvailable, usageSummary?.unit)
                                : "-"
                        }`}
                      >
                        <div className="quota-header">
                          <Database size={14} />
                          <span className="quota-label">
                            {t("codex.cockpitApi.balance", "额度")}
                          </span>
                          <span className="quota-pct high">
                            {usageSummary?.quotaUnlimited
                              ? t("codex.newApi.quota.unlimited", "不限量")
                              : totalAvailable != null && totalGranted != null
                                ? `${formatUsageMoney(totalAvailable, usageSummary?.unit)} / ${formatUsageMoney(totalGranted, usageSummary?.unit)}`
                                : totalAvailable != null
                                  ? formatUsageMoney(totalAvailable, usageSummary?.unit)
                                : "-"}
                          </span>
                        </div>
                        <div className="quota-bar-track">
                          <div
                            className="quota-bar high"
                            style={{ width: `${progressPercent}%` }}
                          />
                        </div>
                        <span className="quota-reset">
                          {expiresAt != null && expiresAt > 0
                            ? `${t("codex.modelProviders.usage.fields.expiresAt", "过期时间")} ${new Date(expiresAt * 1000).toLocaleDateString()}`
                            : t("dashboard.noData", "暂无数据")}
                        </span>
                      </div>
                    </div>
                  ) : (
                    <div className="codex-api-key-usage-panel empty">
                      {t("codex.modelProviders.usage.noKey", "暂无可查询额度")}
                    </div>
                  )}
                </div>
                <div className="codex-card-bottom">
                  <span className="card-date">
                    {new Date(provider.updatedAt || provider.createdAt).toLocaleString()}
                  </span>
                  <button
                    type="button"
                    className="codex-speed-select codex-provider-instance-select codex-provider-instance-trigger"
                    onClick={() => {
                      setPickerSearchQuery("");
                      setInstancePickerProviderId(provider.id);
                    }}
                    title={targetInstanceName}
                  >
                    <span>{targetInstanceName}</span>
                    <ChevronDown size={12} />
                  </button>
                  <div className="card-footer">
                    <div className="card-actions">
                      <button
                        className="card-action-btn"
                        onClick={() => setProviderDetailId(provider.id)}
                        title={t("codex.modelProviders.usage.detailTitle", "服务面板")}
                      >
                        <Database size={14} />
                      </button>
                      <button
                        className="card-action-btn"
                        disabled={!primaryApiKey || usageState?.loading}
                        onClick={() =>
                          primaryApiKey &&
                          void refreshProviderUsage(provider, primaryApiKey)
                        }
                        title={t("common.shared.refreshQuota", "刷新配额")}
                      >
                        <RefreshCw
                          size={14}
                          className={usageState?.loading ? "loading-spinner" : undefined}
                        />
                      </button>
                      <button
                        className="card-action-btn success"
                        disabled={!primaryApiKey || enabling || !targetInstanceReady}
                        title={
                          targetInstanceReady
                            ? t("codex.modelProviders.enableAndStart", "启用并启动")
                            : t(
                                "codex.modelProviders.instance.uninitializedHint",
                                "目标实例尚未初始化，请先到应用多开页启动一次。",
                              )
                        }
                        onClick={() =>
                          primaryApiKey &&
                          void handleEnableProvider(
                            provider,
                            primaryApiKey,
                            targetInstanceId,
                            targetInstanceName,
                          )
                        }
                      >
                        {enabling ? (
                          <RefreshCw size={14} className="loading-spinner" />
                        ) : (
                          <Play size={14} />
                        )}
                      </button>
                      {capabilityProfile.wireApi === "chat_completions" && (
                        <button
                          className="card-action-btn"
                          disabled={!primaryApiKey || testing}
                          onClick={() =>
                            primaryApiKey &&
                            void handleTestProvider(
                              provider,
                              primaryApiKey,
                              capabilityProfile.wireApi,
                            )
                          }
                          title={t("codex.localAccess.testAction", "测试")}
                        >
                          <Activity size={14} />
                        </button>
                      )}
                      <button
                        className="card-action-btn"
                        onClick={() => openEditModal(provider)}
                        title={t("instances.actions.edit", "编辑")}
                      >
                        <Pencil size={14} />
                      </button>
                      <button
                        className="card-action-btn"
                        onClick={() => {
                          const targetUrl = normalizeApiKeyFunOfficialUrl(
                            provider.website ||
                            provider.apiKeyUrl ||
                            provider.baseUrl,
                          );
                          if (!targetUrl) return;
                          window.open(targetUrl, "_blank", "noopener,noreferrer");
                        }}
                        title={t("codex.modelProviders.website", "官网")}
                        disabled={!(provider.website || provider.apiKeyUrl || provider.baseUrl)}
                      >
                        <ExternalLink size={14} />
                      </button>
                      <button
                        className="card-action-btn danger"
                        onClick={() => void handleDeleteProvider(provider)}
                        title={t("common.delete", "删除")}
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {batchTestModalOpen && (
        <div className="modal-overlay codex-provider-batch-test-overlay">
          <div
            className={`modal codex-wakeup-modal codex-provider-batch-test-modal ${
              batchTestStep === "results" ? "codex-wakeup-results-modal" : ""
            }`}
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("codex.modelProviders.batchTest.title", "一键测试模型供应商")}</h2>
              <button
                className="modal-close"
                onClick={closeBatchTestModal}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body codex-provider-batch-test-body">
              {batchTestError && (
                <div className="add-status error">
                  <CircleAlert size={16} />
                  <span>{batchTestError}</span>
                </div>
              )}

              {batchTestStep === "select" ? (
                <>
                  <div className="codex-provider-batch-test-copy">
                    <strong>
                      {t(
                        "codex.modelProviders.batchTest.selectTitle",
                        "选择要测试的供应商",
                      )}
                    </strong>
                    <span>
                      {t(
                        "codex.modelProviders.batchTest.selectDesc",
                        "会把选中的供应商临时接入本地网关发起对话测试，并按供应商协议能力转发到上游。",
                      )}
                    </span>
                  </div>
                  <div className="form-group codex-provider-batch-test-model">
                    <label>
                      {t(
                        "codex.modelProviders.batchTest.modelLabel",
                        "测试模型",
                      )}
                    </label>
                    <SingleSelectDropdown
                      value={batchTestModelId}
                      options={batchTestModelOptions}
                      onChange={setBatchTestModelId}
                      ariaLabel={t(
                        "codex.modelProviders.batchTest.modelLabel",
                        "测试模型",
                      )}
                      placeholder={t(
                        "codex.modelProviders.batchTest.modelAuto",
                        "自动选择（按目录/探测）",
                      )}
                    />
                    {batchTestModelId === "__custom__" && (
                      <input
                        className="form-input"
                        type="text"
                        value={batchTestModelCustom}
                        onChange={(event) =>
                          setBatchTestModelCustom(event.target.value)
                        }
                        placeholder={t(
                          "codex.modelProviders.batchTest.modelCustomPlaceholder",
                          "输入模型 ID，例如 gpt-4.1-mini",
                        )}
                        style={{ marginTop: 8 }}
                      />
                    )}
                    <p className="codex-provider-batch-test-model-hint">
                      {t(
                        "codex.modelProviders.batchTest.modelHint",
                        "可选统一模型。留空则按各供应商目录或探测结果自动选择。",
                      )}
                    </p>
                  </div>
                  <div className="search-box codex-provider-batch-test-search">
                    <Search className="search-icon" size={16} />
                    <input
                      type="text"
                      placeholder={t("common.search", "搜索...")}
                      value={batchTestSearchQuery}
                      onChange={(event) => setBatchTestSearchQuery(event.target.value)}
                    />
                  </div>
                  <div className="codex-wakeup-account-selection-bar">
                    <label className="codex-overview-select-all">
                      <input
                        type="checkbox"
                        checked={isAllBatchTestProvidersSelected}
                        onChange={toggleAllVisibleBatchTestProviders}
                        disabled={providerBatchTestSelectableIds.length === 0}
                      />
                      <span>{t("common.selectAll", "全选")}</span>
                    </label>
                    <span className="codex-wakeup-account-selection-summary">
                      {t("codex.modelProviders.batchTest.selectionSummary", {
                        defaultValue: "已选 {{selected}} / 可测 {{total}}",
                        selected: batchTestSelectedCount,
                        total: providerBatchTestSelectableIds.length,
                      })}
                    </span>
                  </div>
                  <div className="codex-provider-batch-test-list">
                    {providerBatchTestVisibleProviders.map((provider) => {
                      const apiKey = getSelectedProviderApiKey(provider);
                      const wireApi = resolveProviderWireApi(provider);
                      const selected = batchTestSelectedProviderIds.has(provider.id);
                      const disabled = !apiKey;
                      return (
                        <label
                          key={provider.id}
                          className={`codex-provider-batch-test-row ${
                            selected ? "selected" : ""
                          } ${disabled ? "disabled" : ""}`}
                        >
                          <input
                            type="checkbox"
                            checked={selected}
                            disabled={disabled}
                            onChange={() => toggleBatchTestProvider(provider.id)}
                          />
                          <div className="codex-provider-batch-test-row-copy">
                            <strong>{provider.name}</strong>
                            <span>{provider.baseUrl}</span>
                            <div className="codex-provider-batch-test-row-meta">
                              <span>
                                {apiKey
                                  ? resolveProviderApiKeyLabel(
                                      apiKey,
                                      provider.name,
                                      t("codex.modelProviders.unnamedKey", "未命名 Key"),
                                    )
                                  : t(
                                      "codex.modelProviders.batchTest.noApiKey",
                                      "缺少 API Key",
                                    )}
                              </span>
                              <span>
                                {wireApi === "chat_completions"
                                  ? t(
                                      "codex.modelProviders.batchTest.gatewayProtocol",
                                      "Chat Completions · 网关",
                                    )
                                  : t(
                                      "codex.modelProviders.batchTest.directProtocol",
                                      "Responses · 网关",
                                    )}
                              </span>
                            </div>
                          </div>
                        </label>
                      );
                    })}
                    {providerBatchTestVisibleProviders.length === 0 && (
                      <p className="wakeup-hint">{t("common.none", "暂无")}</p>
                    )}
                  </div>
                </>
              ) : (
                <>
                  {batchTestSession && (
                    <>
                      <section className="codex-wakeup-results-summary-bar">
                        <div className="codex-wakeup-results-summary-copy">
                          <div className="codex-wakeup-results-summary-head">
                            <span className="codex-wakeup-results-kicker">
                              {t("codex.modelProviders.batchTest.kicker", "PROVIDER TEST")}
                            </span>
                            <h3>
                              {t(
                                "codex.modelProviders.batchTest.resultsTitle",
                                "供应商对话测试结果",
                              )}
                            </h3>
                          </div>
                          <div className="codex-wakeup-results-summary-meta">
                            <span>
                              {batchTestSession.cancelled
                                ? t("common.cancelled", "已取消")
                                : batchTestSession.running
                                ? t(
                                    "codex.modelProviders.batchTest.status.running",
                                    "测试中",
                                  )
                                : t(
                                    "codex.modelProviders.batchTest.status.completed",
                                    "已完成",
                                  )}
                            </span>
                            <span>
                              {t("codex.modelProviders.batchTest.totalCount", {
                                defaultValue: "供应商 {{count}} 个",
                                count: batchTestSession.total,
                              })}
                            </span>
                          </div>
                        </div>
                        <div className="codex-wakeup-results-summary-progress">
                          <strong>
                            {batchTestSession.completed}/{batchTestSession.total}
                          </strong>
                          <span>
                            {batchTestSession.cancelled
                              ? t("common.cancelled", "已取消")
                              : batchTestSession.running
                              ? t(
                                  "codex.modelProviders.batchTest.status.running",
                                  "测试中",
                                )
                              : t(
                                  "codex.modelProviders.batchTest.status.completed",
                                  "已完成",
                                )}
                          </span>
                        </div>
                      </section>

                      <section className="codex-wakeup-results-progress-strip">
                        <div className="codex-wakeup-results-progress-head">
                          <span>
                            {t(
                              "codex.modelProviders.batchTest.progress",
                              "测试进度",
                            )}
                          </span>
                          <strong>
                            {batchTestSession.completed}/{batchTestSession.total}
                          </strong>
                        </div>
                        <div className="codex-wakeup-results-progress-track">
                          <div
                            className="codex-wakeup-results-progress-fill"
                            style={{
                              width: `${
                                batchTestSession.total > 0
                                  ? (batchTestSession.completed /
                                      batchTestSession.total) *
                                    100
                                  : 0
                              }%`,
                            }}
                          />
                        </div>
                      </section>

                      <div className="codex-wakeup-results-filter-bar">
                        {providerBatchTestFilterOptions.map((option) => (
                          <button
                            key={option.key}
                            type="button"
                            className={`codex-wakeup-results-filter-chip ${
                              batchTestFilter === option.key ? "active" : ""
                            } tone-${option.tone}`}
                            onClick={() => setBatchTestFilter(option.key)}
                          >
                            <span>{option.label}</span>
                            <strong>{option.count}</strong>
                          </button>
                        ))}
                      </div>

                      <div className="codex-provider-batch-test-result-toolbar">
                        <label className="codex-overview-select-all">
                          <input
                            type="checkbox"
                            checked={
                              filteredProviderBatchTestRecords.length > 0 &&
                              filteredProviderBatchTestRecords.every((record) =>
                                batchTestResultSelectedProviderIds.has(record.providerId),
                              )
                            }
                            onChange={toggleAllVisibleBatchTestResults}
                            disabled={filteredProviderBatchTestRecords.length === 0}
                          />
                          <span>
                            {t("codex.modelProviders.batchTest.selectVisible", "选择当前结果")}
                          </span>
                        </label>
                        <div className="codex-provider-batch-test-result-actions">
                          <span className="codex-overview-selected-count">
                            {t("codex.modelProviders.batchTest.resultSelectedCount", {
                              defaultValue: "已选 {{count}} 个",
                              count: batchTestResultSelectedProviderIds.size,
                            })}
                          </span>
                          <button
                            type="button"
                            className="btn btn-secondary"
                            onClick={selectFailedBatchTestResults}
                            disabled={providerBatchTestCounts.error === 0}
                          >
                            {t("codex.modelProviders.batchTest.selectFailed", "选择失败")}
                          </button>
                          <button
                            type="button"
                            className="btn btn-danger"
                            onClick={() => void handleDeleteBatchTestResults()}
                            disabled={
                              batchTestResultSelectedProviderIds.size === 0 ||
                              batchTestSession.running ||
                              batchTestDeleting
                            }
                          >
                            {batchTestDeleting ? (
                              <RefreshCw size={14} className="loading-spinner" />
                            ) : (
                              <Trash2 size={14} />
                            )}
                            {t("common.delete", "删除")}
                          </button>
                        </div>
                      </div>

                      <div className="codex-wakeup-results-list">
                        {filteredProviderBatchTestRecords.map((record) => {
                          const selected = batchTestResultSelectedProviderIds.has(
                            record.providerId,
                          );
                          const recordMessage =
                            record.status === "pending"
                              ? t(
                                  "codex.modelProviders.batchTest.pendingDesc",
                                  "等待开始测试。",
                                )
                              : record.status === "running"
                                ? t(
                                    "codex.modelProviders.batchTest.runningDesc",
                                    "正在发送对话请求。",
                                  )
                              : record.status === "cancelled"
                                ? t("common.cancelled", "已取消")
                              : record.status === "success"
                                  ? record.reply ||
                                    t(
                                      "codex.modelProviders.batchTest.noReply",
                                      "上游未返回可读回复。",
                                    )
                                  : formatProviderBatchTestErrorMessage(
                                      record.error,
                                    );
                          const statusLabel =
                            record.status === "success"
                              ? t(
                                  "codex.modelProviders.batchTest.status.success",
                                  "成功",
                                )
                              : record.status === "error"
                                ? t(
                                    "codex.modelProviders.batchTest.status.error",
                                    "失败",
                                  )
                              : record.status === "cancelled"
                                ? t("common.cancelled", "已取消")
                              : record.status === "running"
                                  ? t(
                                      "codex.modelProviders.batchTest.status.running",
                                      "测试中",
                                    )
                                  : t(
                                      "codex.modelProviders.batchTest.status.pending",
                                      "等待中",
                                    );
                          return (
                            <article
                              key={record.providerId}
                              className={`codex-wakeup-execution-row codex-provider-batch-test-result-row is-${record.status}`}
                            >
                              <div className="codex-wakeup-execution-row-head">
                                <div className="codex-provider-batch-test-result-title">
                                  <label className="codex-provider-batch-test-result-check">
                                    <input
                                      type="checkbox"
                                      checked={selected}
                                      onChange={() =>
                                        toggleBatchTestResultProvider(record.providerId)
                                      }
                                      disabled={record.status === "running"}
                                    />
                                  </label>
                                  <div>
                                    <h4 className="codex-wakeup-execution-row-title">
                                      {record.providerName}
                                    </h4>
                                    <span className="codex-wakeup-execution-row-subtitle">
                                      {record.apiKeyName ||
                                        t(
                                          "codex.modelProviders.unnamedKey",
                                          "未命名 Key",
                                        )}
                                    </span>
                                  </div>
                                </div>
                                <span className={`codex-wakeup-execution-badge is-${record.status}`}>
                                  {record.status === "running" && (
                                    <RefreshCw size={14} className="loading-spinner" />
                                  )}
                                  {statusLabel}
                                </span>
                              </div>
                              <div className="codex-wakeup-execution-row-prompt">
                                {t("codex.modelProviders.batchTest.protocolSummary", {
                                  defaultValue:
                                    "{{protocol}} · {{mode}} · {{model}}",
                                  protocol:
                                    record.wireApi === "chat_completions"
                                      ? t(
                                          "codex.modelProviders.wireApi.chatCompletions",
                                          "Chat Completions",
                                        )
                                      : t(
                                          "codex.modelProviders.wireApi.responses",
                                          "Responses 原生",
                                        ),
                                  mode:
                                    record.accessMode === "gateway"
                                      ? t(
                                          "codex.modelProviders.enableMode.gatewayMode",
                                          "网关模式",
                                        )
                                      : t(
                                          "codex.modelProviders.enableMode.directMode",
                                          "直连模式",
                                        ),
                                  model:
                                    record.modelId ||
                                    t(
                                      "codex.modelProviders.batchTest.modelUnknown",
                                      "模型待定",
                                    ),
                                })}
                              </div>
                              {record.prompt && (
                                <div className="codex-wakeup-execution-row-prompt">
                                  {t("codex.modelProviders.batchTest.promptLabel", "提示词")}：
                                  {record.prompt}
                                </div>
                              )}
                              <p
                                className="codex-wakeup-execution-row-message"
                                title={
                                  record.status === "error" && record.error
                                    ? record.error
                                    : undefined
                                }
                              >
                                {recordMessage}
                              </p>
                              <div className="codex-wakeup-execution-row-meta">
                                <span>{formatDateTime(record.timestamp)}</span>
                                <span>{formatDurationMs(record.durationMs)}</span>
                              </div>
                            </article>
                          );
                        })}
                        {filteredProviderBatchTestRecords.length === 0 && (
                          <p className="wakeup-hint">{t("common.none", "暂无")}</p>
                        )}
                      </div>
                    </>
                  )}
                </>
              )}
            </div>
            <div className="modal-footer">
              {batchTestStep === "results" && !batchTestSession?.running && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setBatchTestStep("select")}
                  disabled={batchTestDeleting}
                >
                  {t("common.back", "返回")}
                </button>
              )}
              <button
                type="button"
                className="btn btn-secondary"
                onClick={closeBatchTestModal}
              >
                {t("common.close", "关闭")}
              </button>
              {batchTestStep === "results" && batchTestSession?.running && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() =>
                    void requestBatchTestCancellation(batchTestSession.runId)
                  }
                  disabled={batchTestCancelling}
                >
                  {batchTestCancelling && (
                    <RefreshCw size={14} className="loading-spinner" />
                  )}
                  {batchTestCancelling
                    ? t("common.cancelling", "正在取消...")
                    : t("common.cancel", "取消")}
                </button>
              )}
              {batchTestStep === "select" && (
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => void handleStartBatchProviderTest()}
                  disabled={batchTestSelectedCount === 0}
                >
                  <Activity size={14} />
                  {t("codex.modelProviders.batchTest.start", "开始测试")}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {showProviderCustomSortModal && (
        <div className="modal-overlay">
          <div
            className="modal codex-custom-sort-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <div>
                <h2>
                  {t(
                    "codex.modelProviders.sort.customModalTitle",
                    "自定义供应商排序",
                  )}
                </h2>
                <p className="codex-custom-sort-modal-desc">
                  {t(
                    "codex.modelProviders.sort.customModalDesc",
                    "拖动供应商或使用上下按钮调整展示顺序。",
                  )}
                </p>
              </div>
              <button
                className="modal-close"
                onClick={() => setShowProviderCustomSortModal(false)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div
                className={`codex-custom-sort-list ${
                  draggedProviderCustomSortId ? "is-sorting" : ""
                }`}
                onMouseUp={stopProviderCustomSortDragging}
                onMouseLeave={stopProviderCustomSortDragging}
              >
                {providerCustomSortProviders.map((provider, index) => {
                  const wireApi = resolveProviderWireApi(provider);
                  const rowClass = [
                    "codex-custom-sort-row",
                    draggedProviderCustomSortId === provider.id
                      ? "is-dragging"
                      : "",
                    draggedProviderCustomSortId &&
                    draggedProviderCustomSortId !== provider.id
                      ? "is-drop-candidate"
                      : "",
                    draggedProviderCustomSortId &&
                    draggedProviderCustomSortId !== provider.id &&
                    providerCustomSortDropTargetId === provider.id
                      ? "is-drop-target"
                      : "",
                  ]
                    .join(" ")
                    .trim();

                  return (
                    <div
                      key={provider.id}
                      className={rowClass}
                      onMouseEnter={() =>
                        handleProviderCustomSortDragMove(provider.id)
                      }
                    >
                      <div className="codex-custom-sort-row-main">
                        <button
                          type="button"
                          className="codex-custom-sort-drag-handle"
                          onMouseDown={(event) =>
                            handleProviderCustomSortDragStart(
                              event,
                              provider.id,
                            )
                          }
                          title={t(
                            "codex.modelProviders.sort.customDragHandle",
                            "拖拽排序",
                          )}
                          aria-label={t(
                            "codex.modelProviders.sort.customDragHandle",
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
                            <span title={provider.name}>{provider.name}</span>
                            <span className="mini-tag">
                              {wireApi === "chat_completions"
                                ? t(
                                    "codex.modelProviders.batchTest.gatewayProtocol",
                                    "Chat Completions · 网关",
                                  )
                                : t(
                                    "codex.modelProviders.batchTest.directProtocol",
                                    "Responses · 网关",
                                  )}
                            </span>
                          </div>
                          <div className="codex-custom-sort-quota-line codex-custom-sort-provider-meta">
                            <span title={provider.baseUrl}>
                              {provider.baseUrl}
                            </span>
                            <span>
                              {t("codex.modelProviders.apiKeysCount", {
                                defaultValue: "API Key {{count}} 个",
                                count: provider.apiKeys.length,
                              })}
                            </span>
                            <span>
                              {t("codex.modelProviders.referencesCount", {
                                defaultValue: "引用账号 {{count}} 个",
                                count: providerReferenceMap.get(provider.id) ?? 0,
                              })}
                            </span>
                          </div>
                        </div>
                      </div>
                      <div className="codex-custom-sort-row-actions">
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            moveProviderCustomSortProvider(provider.id, "up")
                          }
                          disabled={index === 0}
                          title={t(
                            "codex.modelProviders.sort.customMoveUp",
                            "上移",
                          )}
                          aria-label={t(
                            "codex.modelProviders.sort.customMoveUp",
                            "上移",
                          )}
                        >
                          <ArrowUp size={14} />
                        </button>
                        <button
                          type="button"
                          className="folder-icon-btn"
                          onClick={() =>
                            moveProviderCustomSortProvider(provider.id, "down")
                          }
                          disabled={
                            index === providerCustomSortProviders.length - 1
                          }
                          title={t(
                            "codex.modelProviders.sort.customMoveDown",
                            "下移",
                          )}
                          aria-label={t(
                            "codex.modelProviders.sort.customMoveDown",
                            "下移",
                          )}
                        >
                          <ArrowDown size={14} />
                        </button>
                      </div>
                    </div>
                  );
                })}
                {providerCustomSortProviders.length === 0 && (
                  <p className="wakeup-hint">{t("common.none", "暂无")}</p>
                )}
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={resetProviderCustomSortOrder}
              >
                <RotateCw size={14} />
                {t(
                  "codex.modelProviders.sort.customReset",
                  "重置自定义顺序",
                )}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => setShowProviderCustomSortModal(false)}
              >
                {t("common.confirm", "确认")}
              </button>
            </div>
          </div>
        </div>
      )}

      {showModal && (
        <div className="modal-overlay">
          <div
            className="modal codex-provider-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {form.providerId
                  ? t("codex.modelProviders.editTitle", "编辑模型供应商")
                  : t("codex.modelProviders.createTitle", "新增模型供应商")}
              </h2>
              <button
                className="modal-close"
                onClick={closeModal}
                aria-label={t("common.close", "关闭")}
                disabled={saving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="form-group">
                <label>{t("codex.api.provider.label", "供应商")}</label>
                <div className="api-provider-chip-list">
                  <button
                    className={`api-provider-chip ${selectedPresetId === CODEX_API_PROVIDER_CUSTOM_ID && !selectedSponsorTemplateId ? "active" : ""}`}
                    onClick={() =>
                      handleSelectProviderPreset(CODEX_API_PROVIDER_CUSTOM_ID)
                    }
                    type="button"
                    disabled={saving}
                  >
                    <span>{t("codex.api.provider.custom", "自定义")}</span>
                  </button>
                  {sponsorProviderTemplates.map((template) => (
                    <button
                      key={template.id}
                      className={`api-provider-chip sponsor ${selectedSponsorTemplateId === template.id ? "active" : ""}`}
                      onClick={() => handleSelectSponsorTemplate(template)}
                      type="button"
                      disabled={saving}
                    >
                      <span>{template.name}</span>
                      <Star size={12} className="api-provider-chip-badge" />
                    </button>
                  ))}
                  {CODEX_API_PROVIDER_PRESETS.filter(
                    (preset) => !preset.isService,
                  ).map((preset) => (
                    <button
                      key={preset.id}
                      className={`api-provider-chip ${selectedPresetId === preset.id ? "active" : ""}`}
                      onClick={() => handleSelectProviderPreset(preset.id)}
                      type="button"
                      disabled={saving}
                    >
                      <span>
                        {t(
                          `codex.api.providers.${preset.id}.name`,
                          preset.name,
                        )}
                      </span>
                      {preset.isPartner && (
                        <Star size={12} className="api-provider-chip-badge" />
                      )}
                    </button>
                  ))}
                </div>
              </div>
              {selectedPreset && selectedPreset.baseUrls.length > 1 && (
                <div className="form-group">
                  <label>
                    {t("codex.api.provider.endpoint", "供应商端点")}
                  </label>
                  <div className="api-provider-endpoint-list">
                    {selectedPreset.baseUrls.map((baseUrl) => (
                      <button
                        key={baseUrl}
                        className={`api-provider-endpoint-chip ${form.baseUrl === baseUrl ? "active" : ""}`}
                        onClick={() => handleSelectPresetEndpoint(baseUrl)}
                        type="button"
                        disabled={saving}
                      >
                        {baseUrl}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {selectedPreset && (
                <div className="api-provider-hint-block">
                  <p className="api-provider-hint">
                    {t(
                      "codex.api.provider.hint",
                      "已自动填写兼容 Base URL，可继续手动调整。",
                    )}
                  </p>
                  <div className="api-provider-links">
                    {selectedPreset.website && (
                      <a
                        className="btn btn-secondary"
                        href={selectedPreset.website}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} />
                        {t("codex.api.provider.website", "官网")}
                      </a>
                    )}
                    {selectedPreset.apiKeyUrl && (
                      <a
                        className="btn btn-secondary"
                        href={selectedPreset.apiKeyUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <KeyRound size={14} />
                        {t("codex.api.provider.apiKeyPage", "API Key 页面")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              {selectedSponsorTemplate && (
                <div className="api-provider-hint-block sponsor">
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.sponsorHint",
                      "已按专属中转站配置自动填写兼容服务地址。输入 API Key 后，卡片会自动查询余额和用量。",
                    )}
                  </p>
                  <div className="api-provider-links">
                    {selectedSponsorTemplate.website && (
                      <a
                        className="btn btn-secondary"
                        href={selectedSponsorTemplate.website}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} />
                        {t("codex.api.provider.website", "官网")}
                      </a>
                    )}
                    {selectedSponsorTemplate.apiKeyUrl && (
                      <a
                        className="btn btn-secondary"
                        href={selectedSponsorTemplate.apiKeyUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <KeyRound size={14} />
                        {t("codex.api.provider.apiKeyPage", "API Key 页面")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.name", "供应商名称")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.name}
                  onChange={(event) => mutateForm({ name: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.baseUrl", "Base URL")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.baseUrl}
                  onChange={(event) =>
                    mutateForm({ baseUrl: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label className="codex-provider-label-with-help">
                  <span>{t("codex.modelProviders.fields.wireApi", "协议")}</span>
                  <span
                    className="codex-provider-inline-help"
                    title={t(
                      "codex.modelProviders.wireApi.help",
                      "大多数供应商请选择 Responses；仅当供应商明确只支持 Chat Completions 时再切换。如果不确定，优先选 Responses。",
                    )}
                    aria-label={t(
                      "codex.modelProviders.wireApi.helpAria",
                      "查看协议说明",
                    )}
                  >
                    <HelpCircle size={14} />
                  </span>
                </label>
                <div className="api-provider-chip-list">
                  <button
                    type="button"
                    className={`api-provider-chip ${form.wireApi === "responses" ? "active" : ""}`}
                    onClick={() =>
                      mutateForm({
                        wireApi: "responses",
                        enableModePreference:
                          resolveEnableModePreferenceForWireApi(
                            "responses",
                            selectedPresetId,
                          ),
                      })
                    }
                    disabled={saving}
                  >
                    <span>
                      {t("codex.modelProviders.wireApi.responses", "Responses 原生")}
                    </span>
                  </button>
                  <button
                    type="button"
                    className={`api-provider-chip ${form.wireApi === "chat_completions" ? "active" : ""}`}
                    onClick={() =>
                      mutateForm({
                        wireApi: "chat_completions",
                        supportsWebsockets: false,
                        enableModePreference:
                          resolveEnableModePreferenceForWireApi(
                            "chat_completions",
                            selectedPresetId,
                          ),
                      })
                    }
                    disabled={saving}
                  >
                    <span>
                      {t(
                        "codex.modelProviders.wireApi.chatCompletions",
                        "Chat Completions 协议",
                      )}
                    </span>
                  </button>
                </div>
                {selectedPresetId === DEEPSEEK_API_PROVIDER_ID && (
                  <p className="api-provider-hint">
                    {form.wireApi === "responses"
                      ? t(
                          "codex.modelProviders.wireApi.deepseekResponsesHint",
                          "原生 Responses 直连官方 API，写入官方 models.json（工具/shell/apply_patch），默认模型 deepseek-v4-flash。",
                        )
                      : t(
                          "codex.modelProviders.wireApi.deepseekChatHint",
                          "DeepSeek Chat Completions 走本地网关协议转换，适合兼容旧链路；需要官方 Codex 工具形态时请选 Responses。",
                        )}
                  </p>
                )}
              </div>
              {form.wireApi === "responses" && (
                <div className="form-group">
                  <label>
                    {t(
                      "codex.modelProviders.fields.supportsWebsockets",
                      "WebSocket 传输",
                    )}
                  </label>
                  <label className="provider-vision-toggle">
                    <span className="provider-vision-toggle-copy">
                      <span className="provider-vision-toggle-title">
                        {t(
                          "codex.modelProviders.websockets.title",
                          "允许 Codex 使用 Responses WebSocket",
                        )}
                      </span>
                      <span className="provider-vision-toggle-desc">
                        {t(
                          "codex.modelProviders.websockets.help",
                          "仅在供应商明确支持 Responses WebSocket 时开启；连接方式可通过 Codex 或代理服务日志确认。",
                        )}
                      </span>
                    </span>
                    <span className="provider-vision-switch">
                      <input
                        type="checkbox"
                        checked={form.supportsWebsockets}
                        onChange={(event) =>
                          mutateForm({ supportsWebsockets: event.target.checked })
                        }
                        disabled={
                          saving ||
                          selectedPresetId === "openai_official" ||
                          selectedPresetId === DEEPSEEK_API_PROVIDER_ID
                        }
                      />
                      <span className="provider-vision-switch-track" />
                    </span>
                  </label>
                </div>
              )}
              {form.wireApi === "chat_completions" && (
                <>
                  <div className="form-group">
                    <label>
                      {t("codex.modelProviders.fields.modelCatalog", "模型目录")}
                    </label>
                    <textarea
                      className="form-input"
                      rows={4}
                      value={form.modelCatalogText}
                      onChange={(event) =>
                        mutateForm({ modelCatalogText: event.target.value })
                      }
                      placeholder={"deepseek-v4-flash\ndeepseek-v4-pro"}
                      disabled={saving}
                    />
                    <CodexModelContextWindowTable
                      models={parseModelCatalogText(form.modelCatalogText)}
                      drafts={form.modelContextWindowsDraft}
                      onChange={(model, value) =>
                        mutateForm({
                          modelContextWindowsDraft: {
                            ...form.modelContextWindowsDraft,
                            [model]: value,
                          },
                        })
                      }
                      disabled={saving}
                    />
                  </div>
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.fields.visionCapability",
                        "图片输入能力",
                      )}
                    </label>
                    <label className="provider-vision-toggle">
                      <span className="provider-vision-toggle-copy">
                        <span className="provider-vision-toggle-title">
                          {t(
                            "codex.modelProviders.vision.providerDefault",
                            "该供应商默认支持图片输入",
                          )}
                        </span>
                        <span className="provider-vision-toggle-desc">
                          {t(
                            "codex.modelProviders.vision.providerDefaultHint",
                            "关闭时，只有下方列出的模型会允许图片输入；其他模型会在本地网关直接提示不支持。",
                          )}
                        </span>
                      </span>
                      <span className="provider-vision-switch">
                        <input
                          type="checkbox"
                          checked={form.supportsVision}
                          onChange={(event) =>
                            mutateForm({ supportsVision: event.target.checked })
                          }
                          disabled={saving}
                        />
                        <span className="provider-vision-switch-track" />
                      </span>
                    </label>
                  </div>
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.fields.visionModels",
                        "支持图片的模型",
                      )}
                    </label>
                    <textarea
                      className="form-input"
                      rows={3}
                      value={form.visionModelText}
                      onChange={(event) =>
                        mutateForm({ visionModelText: event.target.value })
                      }
                      placeholder={"qwen-vl-plus\ngpt-4o"}
                      disabled={saving}
                    />
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.vision.modelsHint",
                      "每行一个模型名。适合同一供应商里只有部分视觉模型支持粘贴图片的情况。",
                    )}
                  </p>
                </div>
                <div className="form-group">
                  <label>
                    {t(
                      "codex.modelProviders.fields.visionRoutingModel",
                      "图片请求默认模型",
                    )}
                  </label>
                  <input
                    className="form-input"
                    value={form.visionRoutingModel}
                    onChange={(event) =>
                      mutateForm({ visionRoutingModel: event.target.value })
                    }
                    placeholder={"mimo-v2.5"}
                    disabled={saving}
                  />
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.vision.routingModelHint",
                      "当前模型不支持图片时，带图片的请求会改用该模型；留空则直接提示不支持。",
                    )}
                  </p>
                </div>
                <p className="api-provider-hint">
                  {t(
                    "codex.modelProviders.gatewayHint",
                      "第三方供应商启动时会使用本地网关隔离实例并完成协议转换；OpenAI 官方供应商保持直连。",
                    )}
                  </p>
                </>
              )}
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.website", "官网（可选）")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.website}
                  onChange={(event) =>
                    mutateForm({ website: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t(
                    "codex.modelProviders.fields.apiKeyUrl",
                    "API Key 页面（可选）",
                  )}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.apiKeyUrl}
                  onChange={(event) =>
                    mutateForm({ apiKeyUrl: event.target.value })
                  }
                  disabled={saving}
                />
              </div>

              {currentEditingProvider &&
                currentEditingProvider.apiKeys.length > 0 && (
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.existingApiKeys",
                        "现有 API Keys",
                      )}
                    </label>
                    {currentEditingProvider.apiKeys.length > 5 && (
                      <div className="search-box codex-provider-key-search">
                        <Search className="search-icon" size={16} />
                        <input
                          type="text"
                          placeholder={t(
                            "codex.modelProviders.existingApiKeysSearch",
                            "搜索已有 API Key…",
                          )}
                          value={existingApiKeySearchQuery}
                          onChange={(event) =>
                            setExistingApiKeySearchQuery(event.target.value)
                          }
                          disabled={saving}
                        />
                      </div>
                    )}
                    <div className="codex-provider-key-list inline">
                      {currentEditingProvider.apiKeys
                        .filter((item) => {
                          const query = existingApiKeySearchQuery
                            .trim()
                            .toLowerCase();
                          if (!query) return true;
                          const label = (
                            item.name ||
                            t("codex.modelProviders.unnamedKey", "未命名 Key")
                          ).toLowerCase();
                          const masked = maskApiKey(item.apiKey).toLowerCase();
                          return (
                            label.includes(query) ||
                            masked.includes(query) ||
                            item.apiKey.toLowerCase().includes(query)
                          );
                        })
                        .map((item) => {
                          const isEditing = editingApiKey?.apiKeyId === item.id;
                          if (isEditing && editingApiKey) {
                            return (
                              <div
                                className="codex-provider-key-row is-editing"
                                key={item.id}
                              >
                                <div className="codex-provider-key-edit-fields">
                                  <input
                                    className="form-input"
                                    type="text"
                                    value={editingApiKey.name}
                                    onChange={(event) =>
                                      setEditingApiKey((current) =>
                                        current
                                          ? { ...current, name: event.target.value }
                                          : current,
                                      )
                                    }
                                    placeholder={t(
                                      "codex.modelProviders.fields.newApiKeyName",
                                      "API Key name (optional)",
                                    )}
                                    disabled={saving}
                                  />
                                  <input
                                    className="form-input"
                                    type="password"
                                    value={editingApiKey.apiKey}
                                    onChange={(event) =>
                                      setEditingApiKey((current) =>
                                        current
                                          ? { ...current, apiKey: event.target.value }
                                          : current,
                                      )
                                    }
                                    placeholder={t(
                                      "codex.modelProviders.fields.apiKey",
                                      "API Key",
                                    )}
                                    autoComplete="off"
                                    disabled={saving}
                                  />
                                </div>
                                <div className="codex-provider-key-edit-actions">
                                  <button
                                    type="button"
                                    className="action-btn success"
                                    onClick={() => void handleSaveApiKeyEdit()}
                                    disabled={saving}
                                    title={t("common.save", "Save")}
                                  >
                                    <Check size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    className="action-btn"
                                    onClick={() => setEditingApiKey(null)}
                                    disabled={saving}
                                    title={t("common.cancel", "Cancel")}
                                  >
                                    <X size={12} />
                                  </button>
                                </div>
                              </div>
                            );
                          }
                          return (
                            <div className="codex-provider-key-row" key={item.id}>
                          <div className="codex-provider-key-text">
                            <span className="codex-provider-key-name">
                              {item.name ||
                                t(
                                  "codex.modelProviders.unnamedKey",
                                  "未命名 Key",
                                )}
                            </span>
                            <code>{maskApiKey(item.apiKey)}</code>
                          </div>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() =>
                              setEditingApiKey({
                                providerId: currentEditingProvider.id,
                                apiKeyId: item.id,
                                originalApiKey: item.apiKey,
                                apiKey: item.apiKey,
                                name: item.name,
                              })
                            }
                            disabled={saving}
                            title={t(
                              "codex.modelProviders.editApiKey",
                              "Edit API Key",
                            )}
                          >
                            <KeyRound size={12} />
                          </button>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() =>
                              void handleRenameApiKey(
                                currentEditingProvider,
                                item,
                              )
                            }
                            disabled={saving}
                            title={t("common.rename", "重命名")}
                          >
                            <Pencil size={12} />
                          </button>
                          <button
                            type="button"
                            className="action-btn danger"
                            onClick={() =>
                              void handleDeleteApiKey(
                                currentEditingProvider,
                                item,
                              )
                            }
                            disabled={saving}
                            title={t("common.delete", "删除")}
                          >
                            <Trash2 size={12} />
                          </button>
                            </div>
                          );
                        })}
                    </div>
                  </div>
                )}

              <div className="form-group">
                <label>
                  {t(
                    "codex.modelProviders.fields.newApiKeyName",
                    "新增 Key 名称（可选）",
                  )}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKeyName}
                  onChange={(event) =>
                    mutateForm({ newApiKeyName: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.newApiKey", "新增 API Key")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKey}
                  onChange={(event) =>
                    mutateForm({ newApiKey: event.target.value })
                  }
                  disabled={saving}
                />
              </div>

              <div className="provider-save-preview">
                <div className="provider-save-preview-header">
                  <div className="provider-save-preview-title">
                    {t("codex.modelProviders.preview.title", "保存预览")}
                  </div>
                  <span className="provider-save-preview-chip primary">
                    {t("codex.modelProviders.preview.writeNow", "会写入")}
                  </span>
                </div>
                <p className="provider-save-preview-desc">
                  {t(
                    "codex.modelProviders.preview.desc",
                    "保存供应商时会先更新供应商仓库；不会因为这次操作立刻切换官方 Codex 的当前配置。",
                  )}
                </p>
                <div className="provider-save-preview-list">
                  <div className="provider-save-preview-item primary">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          "codex.modelProviders.preview.providerStoreTitle",
                          "模型供应商仓库",
                        )}
                      </span>
                      <span className="provider-save-preview-chip primary">
                        {t("codex.modelProviders.preview.writeNow", "会写入")}
                      </span>
                    </div>
                    <code>{previewPaths.providerStorePath}</code>
                    <p>
                      {t(
                        "codex.modelProviders.preview.providerStoreDesc",
                        "保存供应商名称、Base URL、官网/API Key 页面链接，以及本弹框新增的 API Key。",
                      )}
                    </p>
                  </div>

                  <div className="provider-save-preview-item muted">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          "codex.modelProviders.preview.codexConfigTitle",
                          "当前 Codex 配置",
                        )}
                      </span>
                      <span className="provider-save-preview-chip muted">
                        {t(
                          "codex.modelProviders.preview.noImmediateChange",
                          "不会立即修改",
                        )}
                      </span>
                    </div>
                    <code>{previewPaths.codexConfigPath}</code>
                    <p>
                      {t(
                        "codex.modelProviders.preview.codexConfigDesc",
                        "不会立即改动当前 provider 或 Base URL；只有在保存或切换 Codex API Key 账号时才会更新。",
                      )}
                    </p>
                  </div>

                  <div className="provider-save-preview-item muted">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          "codex.modelProviders.preview.authFileTitle",
                          "当前 Codex 登录凭据",
                        )}
                      </span>
                      <span className="provider-save-preview-chip muted">
                        {t(
                          "codex.modelProviders.preview.noImmediateChange",
                          "不会立即修改",
                        )}
                      </span>
                    </div>
                    <code>{previewPaths.codexAuthPath}</code>
                    <p>
                      {t(
                        "codex.modelProviders.preview.authFileDesc",
                        "不会因为保存供应商而覆盖当前 auth.json 中的 OPENAI_API_KEY。",
                      )}
                    </p>
                  </div>
                </div>
              </div>

              {formError && (
                <div className="add-status error">
                  <CircleAlert size={16} />
                  <span>{formError}</span>
                </div>
              )}
            </div>

            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={closeModal}
                disabled={saving}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                className="btn btn-primary codex-provider-save-button"
                onClick={() => void handleSaveProvider()}
                disabled={saving}
              >
                <span className="codex-provider-save-button-label">
                  <span aria-hidden={saving}>
                    {t("common.save", "保存")}
                  </span>
                  <span aria-hidden={!saving}>
                    {t("common.saving", "保存中...")}
                  </span>
                </span>
              </button>
            </div>
          </div>
        </div>
      )}

      {apiKeyPickerProviderId && (() => {
        const provider = providers.find((item) => item.id === apiKeyPickerProviderId);
        if (!provider) return null;
        const unnamedKeyLabel = t(
          "codex.modelProviders.unnamedKey",
          "未命名 Key",
        );
        const filteredApiKeys = provider.apiKeys.filter((item) =>
          resolveProviderApiKeyLabel(item, provider.name, unnamedKeyLabel)
            .toLowerCase()
            .includes(pickerSearchQuery.trim().toLowerCase()),
        );
        return (
          <div className="modal-overlay">
            <div
              className="modal codex-provider-picker-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <h2>{t("codex.modelProviders.existingApiKeys", "已有 API Key")}</h2>
                <button
                  className="modal-close"
                  onClick={() => setApiKeyPickerProviderId(null)}
                  aria-label={t("common.close", "关闭")}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body codex-provider-picker-body">
                <div className="search-box codex-provider-picker-search">
                  <Search className="search-icon" size={16} />
                  <input
                    type="text"
                    placeholder={t("common.search", "搜索...")}
                    value={pickerSearchQuery}
                    onChange={(event) => setPickerSearchQuery(event.target.value)}
                  />
                </div>
                <div className="codex-provider-picker-list">
                  {filteredApiKeys.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={`codex-provider-picker-item ${selectedProviderApiKeyMap[provider.id] === item.id || (!selectedProviderApiKeyMap[provider.id] && provider.apiKeys[0]?.id === item.id) ? "active" : ""}`}
                      onClick={() => {
                        setSelectedProviderApiKeyMap((previous) => ({
                          ...previous,
                          [provider.id]: item.id,
                        }));
                        setApiKeyPickerProviderId(null);
                      }}
                    >
                      <span>{resolveProviderApiKeyLabel(item, provider.name, unnamedKeyLabel)}</span>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>
        );
      })()}

      {instancePickerProviderId && (() => {
        const provider = providers.find((item) => item.id === instancePickerProviderId);
        if (!provider) return null;
        const filteredInstances = displayInstances.filter((instance) =>
          getInstanceName(instance)
            .toLowerCase()
            .includes(pickerSearchQuery.trim().toLowerCase()),
        );
        return (
          <div className="modal-overlay">
            <div
              className="modal codex-provider-picker-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <h2>{t("codex.modelProviders.instance.shortLabel", "实例")}</h2>
                <button
                  className="modal-close"
                  onClick={() => setInstancePickerProviderId(null)}
                  aria-label={t("common.close", "关闭")}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body codex-provider-picker-body">
                <div className="search-box codex-provider-picker-search">
                  <Search className="search-icon" size={16} />
                  <input
                    type="text"
                    placeholder={t("common.search", "搜索...")}
                    value={pickerSearchQuery}
                    onChange={(event) => setPickerSearchQuery(event.target.value)}
                  />
                </div>
                <div className="codex-provider-picker-list">
                  {filteredInstances.map((instance) => (
                    <button
                      key={instance.id}
                      type="button"
                      className={`codex-provider-picker-item ${getProviderInstanceId(provider) === instance.id ? "active" : ""}`}
                      onClick={() => {
                        void handleProviderInstanceChange(provider, instance.id);
                        setInstancePickerProviderId(null);
                      }}
                    >
                      <span>
                        {getInstanceName(instance)}
                        {instance.running
                          ? ` · ${t("codex.modelProviders.instance.running", "运行中")}`
                          : ""}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>
        );
      })()}

      {providerOauthTarget && (
        <div
          className="modal-overlay"
        >
          <div
            className="modal-content codex-add-modal codex-oauth-binding-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("codex.api.oauthBinding.title", "绑定 OAuth 账号")}</h2>
              <button
                className="modal-close"
                onClick={() => !providerOauthSaving && setProviderOauthPickerId(null)}
                aria-label={t("common.close", "关闭")}
                disabled={providerOauthSaving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-section">
                <div className="codex-oauth-binding-context">
                  <p className="section-desc codex-oauth-binding-desc">
                    {t(
                      "codex.modelProviders.oauthBinding.desc",
                      "可选绑定。只要 OAuth 账号带 refresh_token 即可选择；未绑定时供应商仍按原 API Key 逻辑运行。",
                    )}
                  </p>
                  <div className="section-desc codex-oauth-binding-current-target">
                    {t("codex.modelProviders.oauthBinding.currentProvider", {
                      defaultValue: "供应商：{{name}}",
                      name: providerOauthTarget.name,
                    })}
                  </div>
                </div>
                <div className="codex-oauth-binding-picker">
                  <div className="codex-oauth-binding-picker-header">
                    <label>
                      {t("codex.api.oauthBinding.selectLabel", "选择 OAuth 账号")}
                    </label>
                  </div>
                  {providerOauthAccounts.length === 0 ? (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>
                        {t(
                          "codex.api.oauthBinding.empty",
                          "暂无 OAuth 账号，请先添加 OAuth 授权账号。",
                        )}
                      </span>
                    </div>
                  ) : providerOauthEligibleAccounts.length === 0 ? (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>
                        {t(
                          "codex.api.oauthBinding.emptyEligible",
                          "没有带 refresh_token 的 OAuth 账号，请重新 OAuth 授权或添加符合条件的 OAuth 账号。",
                        )}
                      </span>
                    </div>
                  ) : (
                    <>
                      <div className="codex-oauth-binding-toolbar">
                        <div className="search-box codex-oauth-binding-search">
                          <Search size={16} className="search-icon" />
                          <input
                            type="text"
                            placeholder={t("common.shared.search", "搜索账号...")}
                            value={providerOauthSearchQuery}
                            onChange={(event) =>
                              setProviderOauthSearchQuery(event.target.value)
                            }
                            disabled={providerOauthSaving}
                          />
                        </div>
                        <MultiSelectFilterDropdown
                          options={providerOauthTierFilterOptions}
                          selectedValues={providerOauthFilterTypes}
                          allLabel={t("common.shared.filter.all", {
                            count: providerOauthTierCounts.all,
                          })}
                          filterLabel={t("common.shared.filterLabel", "筛选")}
                          clearLabel={t("accounts.clearFilter", "清空筛选")}
                          emptyLabel={t("common.none", "暂无")}
                          ariaLabel={t("common.shared.filterLabel", "筛选")}
                          onToggleValue={toggleProviderOAuthFilterTypeValue}
                          onClear={() => setProviderOauthFilterTypes([])}
                        />
                        <AccountTagFilterDropdown
                          availableTags={providerOauthAvailableTags}
                          selectedTags={providerOauthTagFilter}
                          onToggleTag={toggleProviderOAuthTagFilterValue}
                          onClear={() => setProviderOauthTagFilter([])}
                        />
                        <SingleSelectFilterDropdown
                          value={providerOauthSortBy}
                          options={[
                            {
                              value: "last_used",
                              label: t("accounts.columns.lastUsed", "最后使用"),
                            },
                            {
                              value: "created_at",
                              label: t("common.shared.sort.createdAt", "按创建时间"),
                            },
                            {
                              value: "account",
                              label: t("common.shared.columns.account", "账号"),
                            },
                            {
                              value: "plan",
                              label: t("accounts.sort.plan", "按套餐"),
                            },
                          ]}
                          ariaLabel={t("common.shared.sortLabel", "排序")}
                          icon={<ArrowDownWideNarrow size={14} />}
                          disabled={providerOauthSaving}
                          onChange={(value) =>
                            setProviderOauthSortBy(value as OAuthBindingSortBy)
                          }
                        />
                        <button
                          type="button"
                          className="sort-direction-btn"
                          onClick={() =>
                            setProviderOauthSortDirection((prev) =>
                              prev === "desc" ? "asc" : "desc",
                            )
                          }
                          disabled={providerOauthSaving}
                          title={
                            providerOauthSortDirection === "desc"
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
                          {providerOauthSortDirection === "desc" ? (
                            <ArrowDown size={15} />
                          ) : (
                            <ArrowUp size={15} />
                          )}
                        </button>
                      </div>
                      {providerOauthFilteredAccounts.length === 0 ? (
                        <div className="group-account-empty">
                          <span>
                            {t("common.shared.noMatch.title", "没有匹配的账号")}
                          </span>
                        </div>
                      ) : (
                        <div className="codex-oauth-binding-list">
                          {providerOauthPagination.pageItems.map((account) => {
                            const presentation = resolvePresentation(account);
                            const subscriptionInfo =
                              getCodexSubscriptionPresentation(
                                account.subscription_active_until,
                                t,
                              );
                            const selected =
                              providerOauthSelectedAccountId === account.id;
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
                                aria-disabled={providerOauthSaving}
                                onClick={(event) => {
                                  if (providerOauthSaving) {
                                    event.preventDefault();
                                    return;
                                  }
                                  setProviderOauthSelectedAccountId(account.id);
                                }}
                              >
                                <input
                                  type="radio"
                                  name="codex-provider-oauth-binding-account"
                                  checked={selected}
                                  onChange={() =>
                                    setProviderOauthSelectedAccountId(account.id)
                                  }
                                  disabled={providerOauthSaving}
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
                                      {t("codex.subscription.label", "有效期")}
                                    </span>
                                    <strong>{subscriptionInfo.valueText}</strong>
                                    <span>{subscriptionInfo.detailText}</span>
                                  </span>
                                </div>
                              </label>
                            );
                          })}
                        </div>
                      )}
                      <PaginationControls
                        totalItems={providerOauthPagination.totalItems}
                        currentPage={providerOauthPagination.currentPage}
                        totalPages={providerOauthPagination.totalPages}
                        pageSize={providerOauthPagination.pageSize}
                        pageSizeOptions={providerOauthPagination.pageSizeOptions}
                        rangeStart={providerOauthPagination.rangeStart}
                        rangeEnd={providerOauthPagination.rangeEnd}
                        canGoPrevious={providerOauthPagination.canGoPrevious}
                        canGoNext={providerOauthPagination.canGoNext}
                        onPageSizeChange={providerOauthPagination.setPageSize}
                        onPreviousPage={providerOauthPagination.goToPreviousPage}
                        onNextPage={providerOauthPagination.goToNextPage}
                      />
                    </>
                  )}
                </div>
                <div className="api-key-edit-actions">
                  {providerOauthHasExistingBinding && (
                    <button
                      className="btn btn-secondary codex-oauth-binding-clear"
                      onClick={() =>
                        void handleProviderOauthBindingChange(
                          providerOauthTarget,
                          null,
                        )
                      }
                      disabled={providerOauthSaving}
                    >
                      {t("codex.api.oauthBinding.clearAction", "解除绑定")}
                    </button>
                  )}
                  <button
                    className="btn btn-secondary"
                    onClick={() => setProviderOauthPickerId(null)}
                    disabled={providerOauthSaving}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() =>
                      selectedProviderOauthAccount &&
                      void handleProviderOauthBindingChange(
                        providerOauthTarget,
                        selectedProviderOauthAccount.id,
                      )
                    }
                    disabled={
                      providerOauthSaving ||
                      !selectedProviderOauthAccount ||
                      providerOauthEligibleAccounts.length === 0
                    }
                  >
                    {providerOauthSaving
                      ? t("common.saving", "保存中...")
                      : t("common.save", "保存")}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {providerDetailId && (() => {
        const provider = providers.find((item) => item.id === providerDetailId);
        if (!provider) return null;
        const usageState = providerUsageMap[provider.id];
        const primaryApiKey = getSelectedProviderApiKey(provider);
        const usageSummary = usageState?.summary;
        const resolvedWireApi = resolveProviderWireApi(provider);
        const usageMode =
          usageSummary?.mode === "new_api" ||
          usageSummary?.mode === "sub2api" ||
          usageSummary?.mode === "deepseek" ||
          usageSummary?.mode === "token_plan"
            ? usageSummary.mode
            : provider.integrationType ?? null;
        const coreDetailKeys =
          usageMode === "new_api"
            ? new Set(["mode", "totalGranted", "totalAvailable", "expiresAt"])
            : usageMode === "sub2api"
              ? new Set(["mode", "remaining", "todayRequests", "todayTokens"])
            : usageMode === "deepseek"
                ? new Set([
                    "isAvailable",
                    "currency",
                    "totalBalance",
                    "grantedBalance",
                    "toppedUpBalance",
                  ])
                : usageMode === "token_plan"
                  ? new Set([
                      "mode",
                      "remaining",
                      "planName",
                      "expiresAt",
                    ])
                : new Set<string>();
        const detailMetrics: CodexServicePanelMetricItem[] = [
          {
            key: "wireApi",
            label: t("codex.modelProviders.fields.wireApi", "协议"),
            value:
              resolvedWireApi === "chat_completions"
                ? t(
                    "codex.modelProviders.wireApi.chatCompletions",
                    "Chat Completions 协议",
                  )
                : t(
                    "codex.modelProviders.wireApi.responses",
                    "Responses 原生",
                  ),
            rawKey: "wireApi",
          },
          {
            key: "supportsWebsockets",
            label: t(
              "codex.modelProviders.fields.supportsWebsockets",
              "WebSocket 传输",
            ),
            value:
              resolvedWireApi === "responses" && provider.supportsWebsockets
                ? t("codex.modelProviders.websockets.enabled", "已启用")
                : t("codex.modelProviders.websockets.disabled", "已停用"),
            rawKey: "supportsWebsockets",
          },
          {
            key: "oauthBinding",
            label: t("codex.api.oauthBinding.label", "OAuth 绑定"),
            value:
              resolveBoundOAuthAccount(provider)?.account_name ||
              resolveBoundOAuthAccount(provider)?.email ||
              resolveBoundOAuthAccount(provider)?.id ||
              t("codex.api.oauthBinding.unbound", "未绑定"),
            rawKey: "boundOauthAccountId",
          },
          ...(resolveCodexApiProviderPresetId(provider.baseUrl) ===
          DEEPSEEK_API_PROVIDER_ID
            ? [
                {
                  key: "enableMode",
                  label: t("codex.modelProviders.enableMode.label", "接入方式"),
                  value: t(
                    "codex.deepSeek.start.chooseAtStart",
                    "启动时选择",
                  ),
                  rawKey: "enableMode",
                },
              ]
            : resolvedWireApi === "chat_completions"
              ? [
                  {
                    key: "enableMode",
                    label: t("codex.modelProviders.enableMode.label", "接入方式"),
                    value: t(
                      "codex.modelProviders.enableMode.gatewayMode",
                      "网关模式",
                    ),
                    rawKey: "enableMode",
                  },
                ]
              : [
                  {
                    key: "enableMode",
                    label: t("codex.modelProviders.enableMode.label", "接入方式"),
                    value: t(
                      "codex.modelProviders.enableMode.directMode",
                      "直连模式",
                    ),
                    rawKey: "enableMode",
                  },
                ]),
          {
            key: "vision",
            label: t("codex.modelProviders.vision.allModels", "图片输入"),
            value: provider.supportsVision
              ? t("common.yes", "是")
              : t("common.no", "否"),
            rawKey: "supportsVision",
          },
          {
            key: "modelCatalog",
            label: t("codex.modelProviders.modelCatalog", "模型"),
            value:
              (provider.modelCatalog?.length ?? 0) > 0
                ? (provider.modelCatalog ?? []).join(", ")
                : t("codex.modelProviders.modelCatalogEmpty", "未配置模型目录"),
            rawKey: "modelCatalog",
          },
          ...((usageSummary?.details ?? [])
            .filter((item) => !coreDetailKeys.has(item.key))
            .map((item) => ({
              key: item.key,
              label: formatUsageDetailLabel(item.key, item.label),
              value: formatUsageDetailValue(item, usageSummary?.unit),
              rawKey: item.key,
            })) as CodexServicePanelMetricItem[]),
        ];

        const newApiQuota = resolveNewApiQuotaSnapshot(usageSummary);
        const coreMetrics: CodexServicePanelMetricItem[] =
          usageMode === "deepseek"
            ? [
                "isAvailable",
                "currency",
                "totalBalance",
                "grantedBalance",
                "toppedUpBalance",
              ].map((key) => {
                const item = usageSummary?.details?.find((detail) => detail.key === key);
                return {
                  key,
                  label: formatUsageDetailLabel(key, key),
                  value: item ? formatUsageDetailValue(item, usageSummary?.unit) : "-",
                };
              })
            : usageMode === "new_api"
            ? [
                {
                  key: "totalGranted",
                  label: t("codex.modelProviders.usage.fields.totalGranted", "授予额度"),
                  value: formatUsageDetailValue(
                    {
                      key: "totalGranted",
                      value: String(newApiQuota.granted ?? "-"),
                    },
                    usageSummary?.unit,
                  ),
                },
                {
                  key: "totalAvailable",
                  label: t("codex.modelProviders.usage.fields.totalAvailable", "可用额度"),
                  value: formatUsageDetailValue(
                    {
                      key: "totalAvailable",
                      value: String(newApiQuota.available ?? "-"),
                    },
                    usageSummary?.unit,
                  ),
                },
                {
                  key: "expiresAt",
                  label: t("codex.modelProviders.usage.fields.expiresAt", "过期时间"),
                  value: formatUsageDetailValue(
                    {
                      key: "expiresAt",
                      value: String(newApiQuota.expiresAt ?? "-"),
                    },
                    usageSummary?.unit,
                  ),
                },
              ]
            : usageMode === "sub2api"
              ? [
                  {
                    key: "accountBalance",
                    label: t("codex.modelProviders.usage.accountBalance", "账户余额"),
                    value: formatUsageQuotaValue(
                      usageSummary,
                      usageSummary?.remaining ??
                        usageSummary?.balance ??
                        usageSummary?.quotaRemaining,
                    ),
                  },
                  {
                    key: "todayRequests",
                    label: t("codex.modelProviders.usage.fields.todayRequests", "今日请求"),
                    value: String(usageSummary?.todayRequests ?? 0),
                  },
                  {
                    key: "todayTokens",
                    label: t("codex.modelProviders.usage.fields.todayTokens", "今日 Token"),
                    value: (usageSummary?.todayTotalTokens ?? 0).toLocaleString("en-US"),
                  },
                ]
              : [];

        const actions: CodexServicePanelActionItem[] = [
          {
            key: "refresh",
            label: t("common.shared.refreshQuota", "刷新配额"),
            variant: "secondary",
            icon: (
              <RefreshCw
                size={14}
                className={usageState?.loading ? "loading-spinner" : ""}
              />
            ),
            disabled: !primaryApiKey || usageState?.loading,
            onClick: () => {
              if (primaryApiKey) {
                void refreshProviderUsage(provider, primaryApiKey);
              }
            },
          },
          {
            key: "edit",
            label: t("instances.actions.edit", "编辑"),
            variant: "secondary",
            icon: <Pencil size={14} />,
            onClick: () => {
              setProviderDetailId(null);
              openEditModal(provider);
            },
          },
          {
            key: "oauth",
            label: t("codex.api.oauthBinding.action", "绑定 OAuth"),
            variant: "secondary",
            icon: <Link2 size={14} />,
            onClick: () => {
              setProviderDetailId(null);
              setProviderOauthPickerId(provider.id);
            },
          },
        ];

        if (provider.website || provider.apiKeyUrl || provider.baseUrl) {
          actions.push({
            key: "website",
            label: t("codex.modelProviders.website", "官网"),
            variant: "secondary",
            icon: <ExternalLink size={14} />,
            onClick: () => {
              const targetUrl = normalizeApiKeyFunOfficialUrl(
                provider.website || provider.apiKeyUrl || provider.baseUrl,
              );
              if (!targetUrl) return;
              window.open(targetUrl, "_blank", "noopener,noreferrer");
            },
          });
        }

        return (
          <CodexServicePanelModal
            open={true}
            title={t("codex.modelProviders.usage.detailTitle", "服务面板")}
            subtitle={provider.name}
            baseUrl={provider.baseUrl}
            apiKeyDisplay={primaryApiKey ? maskApiKey(primaryApiKey.apiKey) : "-"}
            rawApiKey={primaryApiKey?.apiKey}
            coreMetrics={coreMetrics}
            detailMetrics={detailMetrics}
            actions={actions}
            onClose={() => setProviderDetailId(null)}
            emptyDetailText={t("codex.cockpitApi.noStats", "暂无统计")}
          />
        );
      })()}
      {deepSeekStart.modal}
    </div>
  );
}
