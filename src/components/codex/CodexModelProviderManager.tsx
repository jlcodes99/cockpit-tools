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
import type { MultiSelectFilterOption } from "../MultiSelectFilterDropdown";
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
  type CodexModelProviderIntegrationType,
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
  formatCockpitToolsApiKeyBalance,
  formatModelProviderUsageMoney,
} from "../../services/modelProviderUsageService";
import { useSponsorStore } from "../../stores/useSponsorStore";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import type { Sponsor } from "../../types/sponsor";
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from "../../utils/codexProviderPresets";
import {
  normalizeApiKeyFunOfficialUrl,
  resolveApiKeyFunWireApi,
} from "../../utils/apikeyFunLinks";
import {
  getCodexPlanFilterKey,
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
import { CodexModelProviderManagerView } from "./CodexModelProviderManagerView";


const DEFAULT_INSTANCE_ID = "__default__";
const OAUTH_BINDING_PAGE_SIZE_OPTIONS = [10, 20, 50] as const;
export type OAuthBindingSortBy = "account" | "created_at" | "last_used" | "plan";
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
  integrationType: CodexModelProviderIntegrationType | "";
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
  integrationType?: CodexModelProviderIntegrationType | null;
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

export function useCodexModelProviderManagerController({
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
        visionModelText: (preset.visionModelCatalog ?? []).join("\n"),
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
        apiKeyBalance: t(
          "codex.modelProviders.usage.fields.apiKeyBalance",
          "API_KEY 余额",
        ),
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
      if (Number.isFinite(numeric) && item.key === "apiKeyBalance") {
        return formatCockpitToolsApiKeyBalance(numeric, unit ?? undefined);
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

  return {
    apiKeyPickerProviderId,
    batchTestCancelling,
    batchTestDeleting,
    batchTestError,
    batchTestFilter,
    batchTestModalOpen,
    batchTestModelCustom,
    batchTestModelId,
    batchTestModelOptions,
    batchTestResultSelectedProviderIds,
    batchTestSearchQuery,
    batchTestSelectedCount,
    batchTestSelectedProviderIds,
    batchTestSession,
    batchTestStep,
    closeBatchTestModal,
    closeModal,
    currentEditingProvider,
    deepSeekStart,
    displayInstances,
    draggedProviderCustomSortId,
    editingApiKey,
    enablingProviderId,
    error,
    existingApiKeySearchQuery,
    filteredProviderBatchTestRecords,
    filteredProviderIds,
    filteredProviders,
    form,
    formatDateTime,
    formatDurationMs,
    formatProviderBatchTestErrorMessage,
    formatUsageDetailLabel,
    formatUsageDetailValue,
    formatUsageMoney,
    formatUsageQuotaValue,
    formError,
    getInstanceName,
    getProviderInstanceId,
    getSelectedProviderApiKey,
    handleBatchDeleteProviders,
    handleDeleteApiKey,
    handleDeleteBatchTestResults,
    handleDeleteProvider,
    handleEnableProvider,
    handleProviderCustomSortDragMove,
    handleProviderCustomSortDragStart,
    handleProviderInstanceChange,
    handleProviderOauthBindingChange,
    handleProviderSortByChange,
    handleRenameApiKey,
    handleSaveApiKeyEdit,
    handleSaveProvider,
    handleSelectPresetEndpoint,
    handleSelectProviderPreset,
    handleSelectSponsorTemplate,
    handleStartBatchProviderTest,
    handleTestProvider,
    instancePickerProviderId,
    isAllBatchTestProvidersSelected,
    isAllProvidersSelected,
    isCurrentProviderActive,
    isInstanceReady,
    isProviderCustomSortActive,
    isSponsorProvider,
    loading,
    maskAccountText,
    maskApiKey,
    moveProviderCustomSortProvider,
    mutateForm,
    notice,
    openBatchTestModal,
    openCreateModal,
    openEditModal,
    parseModelCatalogText,
    pickerSearchQuery,
    previewPaths,
    providerBatchTestCounts,
    providerBatchTestFilterOptions,
    providerBatchTestSelectableIds,
    providerBatchTestVisibleProviders,
    providerCustomSortDropTargetId,
    providerCustomSortProviders,
    providerDetailId,
    providerFilterOptions,
    providerNameFilter,
    providerOauthAccounts,
    providerOauthAvailableTags,
    providerOauthEligibleAccounts,
    providerOauthFilteredAccounts,
    providerOauthFilterTypes,
    providerOauthHasExistingBinding,
    providerOauthPagination,
    providerOauthSaving,
    providerOauthSearchQuery,
    providerOauthSelectedAccountId,
    providerOauthSortBy,
    providerOauthSortDirection,
    providerOauthTagFilter,
    providerOauthTarget,
    providerOauthTierCounts,
    providerOauthTierFilterOptions,
    providerReferenceMap,
    providers,
    providerSortBy,
    providerSortDirection,
    providerUsageMap,
    providerUsageRefreshingAll,
    providerViewMode,
    refreshAllProviderUsage,
    refreshProviderUsage,
    requestBatchTestCancellation,
    resetProviderCustomSortOrder,
    resolveBoundOAuthAccount,
    resolveEnableModePreferenceForWireApi,
    resolveInstanceById,
    resolvePresentation,
    resolveProviderApiKeyLabel,
    resolveProviderWireApi,
    saving,
    searchQuery,
    selectedPreset,
    selectedPresetId,
    selectedProviderApiKeyMap,
    selectedProviderIds,
    selectedProviderOauthAccount,
    selectedSponsorTemplate,
    selectedSponsorTemplateId,
    selectFailedBatchTestResults,
    setApiKeyPickerProviderId,
    setBatchTestFilter,
    setBatchTestModelCustom,
    setBatchTestModelId,
    setBatchTestSearchQuery,
    setBatchTestStep,
    setEditingApiKey,
    setExistingApiKeySearchQuery,
    setInstancePickerProviderId,
    setNotice,
    setPickerSearchQuery,
    setProviderDetailId,
    setProviderNameFilter,
    setProviderOauthFilterTypes,
    setProviderOauthPickerId,
    setProviderOauthSearchQuery,
    setProviderOauthSelectedAccountId,
    setProviderOauthSortBy,
    setProviderOauthSortDirection,
    setProviderOauthTagFilter,
    setProviderSortDirection,
    setProviderViewMode,
    setSearchQuery,
    setSelectedProviderApiKeyMap,
    setShowProviderCustomSortModal,
    setShowQuickConfigModal,
    showModal,
    showProviderCustomSortModal,
    showQuickConfigModal,
    sponsorProviderTemplates,
    stopProviderCustomSortDragging,
    t,
    testingProviderId,
    toggleAllVisibleBatchTestProviders,
    toggleAllVisibleBatchTestResults,
    toggleBatchTestProvider,
    toggleBatchTestResultProvider,
    toggleProviderOAuthFilterTypeValue,
    toggleProviderOAuthTagFilterValue,
    toggleProviderSelected,
    toggleSelectAllProviders,
  };
}

/** 组合业务 Controller 与独立 View，保持原组件公开调用入口不变。 */
export function CodexModelProviderManager(props: CodexModelProviderManagerProps) {
  const controller = useCodexModelProviderManagerController(props);
  return <CodexModelProviderManagerView {...controller} />;
}
