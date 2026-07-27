import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Bug,
  Check,
  CircleAlert,
  Copy,
  Eye,
  EyeOff,
  ExternalLink,
  FolderPlus,
  Gauge,
  KeyRound,
  Power,
  RefreshCw,
  Search,
  Send,
  Server,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import type { CodexAccount } from "../types/codex";
import type { CodexAccountGroup } from "../services/codexAccountGroupService";
import type {
  CodexLocalAccessAddressKind,
  CodexLocalAccessAccountHealth,
  CodexLocalAccessChatMessage,
  CodexLocalAccessChatStreamEvent,
  CodexLocalAccessCustomRoutingRule,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessScope,
  CodexLocalAccessState,
  CodexLocalAccessStatsWindow,
  CodexLocalAccessUsageStats,
} from "../types/codexLocalAccess";
import { getCodexPlanFilterKey } from "../types/codex";
import { scrollElementTo } from "../utils/reducedMotion";
import {
  buildCodexAccountPresentation,
  buildQuotaPreviewLines,
} from "../presentation/platformAccountPresentation";
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
} from "../utils/accountValidityFilter";
import {
  buildCodexPlanFilterOptions,
  createCodexPlanFilterCounts,
  incrementCodexPlanFilterCount,
} from "../utils/codexAccountOverview";
import {
  formatCodexQuotaPoolPercent,
  formatCodexQuotaPoolWindowLabel,
  summarizeCodexQuotaPool,
  type CodexQuotaPoolItem,
} from "../utils/codexQuotaPool";
import {
  getCodexLocalAccessAccountIneligibleReason,
  isCodexLocalAccessEligibleAccount,
  resolveCodexLocalAccessInitialAccountIds,
} from "../utils/codexLocalAccessAccounts";
import { isBlockingCodexQuotaError } from "../utils/codexQuotaError";
import { AccountTagFilterDropdown } from "./AccountTagFilterDropdown";
import { CodexAccountPoolHealthModal } from "./CodexAccountPoolHealthModal";
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from "./MultiSelectFilterDropdown";
import { SingleSelectDropdown } from "./SingleSelectDropdown";
import { PaginationControls } from "./PaginationControls";
import { CodexStatsRangePicker } from "./CodexStatsRangePicker";
import { queryCodexLocalAccessStats } from "../services/codexLocalAccessService";
import {
  buildCodexStatsTimeRange,
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "../utils/codexStatsRange";
import { useEscClose } from "../hooks/useEscClose";
import {
  buildPaginationPageSizeStorageKey,
  usePagination,
} from "../hooks/usePagination";
import "./GroupAccountPickerModal.css";
import "./CodexLocalAccessModal.css";

const LOCAL_ACCESS_MEMBER_PAGE_SIZE_OPTIONS = [50, 100, 200] as const;

export interface CodexLocalAccessMemberViewConfig {
  accounts: CodexAccount[];
  searchQuery: string;
  filterTypes: string[];
  tagFilter: string[];
  groupFilter: string[];
  tierFilterOptions: MultiSelectFilterOption[];
  tierFilterAllLabel: string;
  availableTags: string[];
  groupFilterOptions: MultiSelectFilterOption[];
  onSearchQueryChange: (value: string) => void;
  onToggleFilterType: (value: string) => void;
  onClearFilterTypes: () => void;
  onToggleTagFilter: (value: string) => void;
  onClearTagFilter: () => void;
  onToggleGroupFilter: (value: string) => void;
  onClearGroupFilter: () => void;
}

interface CodexLocalAccessModalProps {
  isOpen: boolean;
  mode: "panel" | "members";
  state: CodexLocalAccessState | null;
  addressKind: CodexLocalAccessAddressKind;
  addressOptions: Array<{ value: string; label: string }>;
  onAddressKindChange: (value: string) => void;
  accounts: CodexAccount[];
  accountsLoaded: boolean;
  accountGroups: CodexAccountGroup[];
  memberView?: CodexLocalAccessMemberViewConfig;
  initialSelectedIds: string[];
  maskAccountText: (value?: string | null) => string;
  onClose: () => void;
  onOpenFullPage?: () => void;
  onSaveAccounts: (payload: {
    accountIds: string[];
    restrictFreeAccounts: boolean;
    backupAccountIds: string[];
    sessionAffinity: boolean;
    sessionAffinityTtlMs: number;
  }) => Promise<unknown> | unknown;
  onClearStats: () => Promise<unknown> | unknown;
  onRefreshStats: () => Promise<unknown> | unknown;
  onUpdatePort: (port: number) => Promise<unknown> | unknown;
  onUpdateRoutingStrategy: (
    strategy: CodexLocalAccessRoutingStrategy,
  ) => Promise<unknown> | unknown;
  onUpdateCustomRouting: (
    rules: CodexLocalAccessCustomRoutingRule[],
  ) => Promise<unknown> | unknown;
  onUpdateAccessScope: (
    accessScope: CodexLocalAccessScope,
  ) => Promise<unknown> | unknown;
  onUpdateUpstreamProxyConfig: (
    upstreamProxyUrl: string | null,
  ) => Promise<unknown> | unknown;
  onUpdateDebugLogs: (debugLogs: boolean) => Promise<unknown> | unknown;
  onRotateApiKey: () => Promise<unknown> | unknown;
  onKillPort: () => Promise<unknown> | unknown;
  onToggleEnabled: () => Promise<unknown> | unknown;
  onRecoverAccounts: (accountIds: string[]) => Promise<void>;
  healthActionBusy: boolean;
  onStreamTestMessage: (payload: {
    sessionId: string;
    modelId: string;
    messages: CodexLocalAccessChatMessage[];
  }) => Promise<void> | void;
  saving: boolean;
  testing: boolean;
  starting: boolean;
  portCleanupBusy: boolean;
}

type CopyableField = "apiPortUrl" | "baseUrl" | "apiKey" | "modelId";

interface AccountPoolHealthSummary {
  total: number;
  available: number;
  abnormal: number;
  cooldown: number;
  missing: number;
  authError: number;
  quotaLimited: number;
}

interface CustomRoutingDraftRule {
  priority: number;
  weight: number;
  isBackup: boolean;
}

interface TestChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  latencyMs?: number | null;
  failureTitle?: string;
  failureDetail?: string;
}

const CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY =
  "agtools.codex.local_access.stats_range.v1";
const CUSTOM_ROUTING_PRIORITY_MIN = 0;
const CUSTOM_ROUTING_PRIORITY_MAX = 100;
const CUSTOM_ROUTING_WEIGHT_MIN = 1;
const CUSTOM_ROUTING_WEIGHT_MAX = 100;
const ABNORMAL_ACCOUNT_FAILURE_CATEGORIES = new Set([
  "auth_unavailable",
  "auth_refresh_failed",
  "account_prepare_failed",
]);

function normalizeAccessScope(value: string): CodexLocalAccessScope {
  return value === "lan" ? "lan" : "localhost";
}

function normalizeStatsRangeKey(
  value: string | null | undefined,
): CodexStatsRangeKey {
  if (value === "weekly" || value === "monthly") {
    return value;
  }
  return "daily";
}

function clampInteger(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

function normalizeCustomRoutingPriority(value: number): number {
  return clampInteger(
    value,
    CUSTOM_ROUTING_PRIORITY_MIN,
    CUSTOM_ROUTING_PRIORITY_MAX,
  );
}

function normalizeCustomRoutingWeight(value: number): number {
  return clampInteger(
    value,
    CUSTOM_ROUTING_WEIGHT_MIN,
    CUSTOM_ROUTING_WEIGHT_MAX,
  );
}

function readStoredStatsRange(): CodexStatsRangeKey {
  try {
    return normalizeStatsRangeKey(
      localStorage.getItem(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY),
    );
  } catch {
    return "daily";
  }
}

function persistStatsRange(value: CodexStatsRangeKey): void {
  try {
    localStorage.setItem(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY, value);
  } catch {
    // ignore storage write failures
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

function formatUsdCost(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "$0.00";
  if (value < 0.000001) return "<$0.000001";
  if (value < 0.01) return `$${value.toFixed(6)}`;
  if (value < 1) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(2)}`;
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

function formatQuotaPoolLabel(
  baseLabel: string,
  pool: CodexQuotaPoolItem,
  weeklyLabel: string,
): string {
  const quotaText = pool.windows
    .map(
      (window) =>
        `${formatCodexQuotaPoolWindowLabel(window.label, weeklyLabel)} ${formatCodexQuotaPoolPercent(window.percentage)}`,
    )
    .join(" · ");
  return quotaText ? `${baseLabel} · ${quotaText}` : baseLabel;
}

function summarizeQuotaPoolsByPlan(
  accounts: CodexAccount[],
): Record<string, CodexQuotaPoolItem> {
  const accountsByPlan: Record<string, CodexAccount[]> = {};
  accounts.forEach((account) => {
    const planKey = getCodexPlanFilterKey(account);
    (accountsByPlan[planKey] ??= []).push(account);
  });
  return Object.fromEntries(
    Object.entries(accountsByPlan).map(([planKey, planAccounts]) => [
      planKey,
      summarizeCodexQuotaPool(planAccounts).all,
    ]),
  );
}

function areSetsEqual(left: Set<string>, right: Set<string>): boolean {
  if (left.size !== right.size) return false;
  for (const value of left) {
    if (!right.has(value)) return false;
  }
  return true;
}

function isAbnormalAccountFailure(
  health?: CodexLocalAccessAccountHealth,
): boolean {
  return Boolean(
    health &&
    ((health.schedulerAvailable === false && !health.cooldowns.length) ||
      (health.consecutiveFailures >= 3 &&
        health.lastFailureCategory &&
        ABNORMAL_ACCOUNT_FAILURE_CATEGORIES.has(health.lastFailureCategory))),
  );
}

export function CodexLocalAccessModal({
  isOpen,
  mode,
  state,
  addressKind,
  addressOptions,
  onAddressKindChange,
  accounts,
  accountsLoaded,
  accountGroups,
  memberView,
  initialSelectedIds,
  maskAccountText,
  onClose,
  onOpenFullPage,
  onSaveAccounts,
  onClearStats,
  onRefreshStats,
  onUpdatePort,
  onUpdateRoutingStrategy,
  onUpdateCustomRouting,
  onUpdateAccessScope,
  onUpdateUpstreamProxyConfig,
  onUpdateDebugLogs,
  onRotateApiKey,
  onKillPort,
  onToggleEnabled,
  onRecoverAccounts,
  healthActionBusy,
  onStreamTestMessage,
  saving,
  testing,
  starting,
  portCleanupBusy,
}: CodexLocalAccessModalProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filterTypes, setFilterTypes] = useState<string[]>([]);
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const [groupFilter, setGroupFilter] = useState<string[]>([]);
  const [restrictFreeAccounts, setRestrictFreeAccounts] = useState(true);
  const [sessionAffinity, setSessionAffinity] = useState(true);
  const [sessionAffinityTtlSeconds, setSessionAffinityTtlSeconds] =
    useState("3600");
  const [sessionAffinityTtlError, setSessionAffinityTtlError] = useState("");
  const [membersDraftDirty, setMembersDraftDirty] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [testDialogOpen, setTestDialogOpen] = useState(false);
  const [healthModalOpen, setHealthModalOpen] = useState(false);
  const [testDialogRunning, setTestDialogRunning] = useState(false);
  const [testChatMessages, setTestChatMessages] = useState<TestChatMessage[]>(
    [],
  );
  const [testChatInput, setTestChatInput] = useState("");
  const [testDialogError, setTestDialogError] = useState("");
  const [portInput, setPortInput] = useState("");
  const [upstreamProxyDraftUrl, setUpstreamProxyDraftUrl] = useState("");
  const [keyVisible, setKeyVisible] = useState(false);
  const [copiedField, setCopiedField] = useState<CopyableField | null>(null);
  const [selectedModelId, setSelectedModelId] = useState("");
  const [statsRange, setStatsRange] = useState<CodexStatsRangeKey>(() =>
    readStoredStatsRange(),
  );
  const [statsTimeRange, setStatsTimeRange] = useState<CodexStatsTimeRange>(() =>
    buildCodexStatsTimeRange(readStoredStatsRange()),
  );
  const [filteredStatsWindow, setFilteredStatsWindow] =
    useState<CodexLocalAccessStatsWindow | null>(null);
  const [statsRangeError, setStatsRangeError] = useState("");
  const statsRequestSeqRef = useRef(0);
  const [customRoutingOpen, setCustomRoutingOpen] = useState(false);
  const [customRoutingQuery, setCustomRoutingQuery] = useState("");
  const [customRoutingFilterTypes, setCustomRoutingFilterTypes] = useState<
    string[]
  >([]);
  const [customRoutingTagFilter, setCustomRoutingTagFilter] = useState<
    string[]
  >([]);
  const [customRoutingError, setCustomRoutingError] = useState("");
  const [customRoutingSelected, setCustomRoutingSelected] = useState<
    Set<string>
  >(new Set());
  const [customRoutingDraft, setCustomRoutingDraft] = useState<
    Record<string, CustomRoutingDraftRule>
  >({});
  const [backupConfirmAccountId, setBackupConfirmAccountId] = useState<
    string | null
  >(null);
  useEscClose(
    isOpen,
    backupConfirmAccountId ? () => setBackupConfirmAccountId(null) : onClose,
  );
  const [customRoutingBulkPriority, setCustomRoutingBulkPriority] =
    useState("10");
  const [customRoutingBulkWeight, setCustomRoutingBulkWeight] = useState("1");
  const selectAllCheckboxRef = useRef<HTMLInputElement | null>(null);
  const customRoutingSelectAllRef = useRef<HTMLInputElement | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const sessionAffinityTtlInputRef = useRef<HTMLInputElement | null>(null);
  const testChatScrollRef = useRef<HTMLDivElement | null>(null);

  const collection = state?.collection ?? null;
  const apiPortUrl = state?.apiPortUrl ?? "";
  const baseUrl = state?.baseUrl ?? "";
  const displayBaseUrl =
    addressKind === "lan" && state?.lanBaseUrl ? state.lanBaseUrl : baseUrl;
  const modelIds = state?.modelIds ?? [];
  const stats = state?.stats;
  const quotaPoolLabels = useMemo(
    () => ({
      weekly: t("codex.localAccess.quotaPool.weeklyShort", "周"),
      title: t("codex.localAccess.quotaPool.title", "额度池"),
    }),
    [t],
  );
  const selectedStatsWindow =
    useMemo<CodexLocalAccessStatsWindow | null>(() => {
      if (filteredStatsWindow) return filteredStatsWindow;
      if (!stats || statsRange === "custom") return null;
      return stats[statsRange];
    }, [filteredStatsWindow, stats, statsRange]);
  const selectedTotals = selectedStatsWindow?.totals;
  const routingStrategy = collection?.routingStrategy ?? "auto";
  const accessScope = collection?.accessScope ?? "localhost";
  const upstreamProxyUrl = collection?.upstreamProxyUrl ?? "";
  const accessScopeAddress = accessScope === "lan" ? "0.0.0.0" : "127.0.0.1";
  const accessScopeBadge =
    accessScope === "lan"
      ? t("codex.localAccess.accessScopeLanShort", "本机+局域网")
      : t("codex.localAccess.accessScopeLocalhostShort", "仅本机");
  const modelIdOptions = useMemo(
    () => modelIds.map((modelId) => ({ value: modelId, label: modelId })),
    [modelIds],
  );
  const avgLatencyMs =
    selectedTotals && selectedTotals.requestCount > 0
      ? selectedTotals.totalLatencyMs / selectedTotals.requestCount
      : 0;
  const successRate =
    selectedTotals && selectedTotals.requestCount > 0
      ? Math.round(
          (selectedTotals.successCount / selectedTotals.requestCount) * 100,
        )
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
  const testDialogBusy = testDialogRunning || testing;
  const actionBusy = saving || testing || starting || portCleanupBusy;
  const membersInteractionDisabled = actionBusy || !accountsLoaded;
  const summaryStats = useMemo(
    () => [
      {
        key: "requests",
        label: t("codex.localAccess.stats.requests", "总请求数"),
        value: formatCompactNumber(selectedTotals?.requestCount ?? 0),
        detail: formatRequestResultDetail(selectedTotals),
      },
      {
        key: "tokens",
        label: t("codex.localAccess.stats.tokens", "总 Token 数"),
        value: formatCompactNumber(selectedTotals?.totalTokens ?? 0),
        detail: t("codex.localAccess.stats.tokensDetail", {
          input: formatCompactNumber(selectedTotals?.inputTokens ?? 0),
          output: formatCompactNumber(selectedTotals?.outputTokens ?? 0),
          defaultValue: "输入 {{input}} / 输出 {{output}}",
        }),
      },
      {
        key: "specialTokens",
        label: t("codex.localAccess.stats.specialTokens", "缓存 / 思考"),
        value: formatCompactNumber(
          (selectedTotals?.cachedTokens ?? 0) +
            (selectedTotals?.reasoningTokens ?? 0),
        ),
        detail: t("codex.localAccess.stats.specialTokensDetail", {
          cached: formatCompactNumber(selectedTotals?.cachedTokens ?? 0),
          reasoning: formatCompactNumber(selectedTotals?.reasoningTokens ?? 0),
          defaultValue: "缓存 {{cached}} / 思考 {{reasoning}}",
        }),
      },
      {
        key: "cost",
        label: t("codex.localAccess.stats.estimatedCost", "估算价值"),
        value: formatUsdCost(selectedTotals?.estimatedCostUsd ?? 0),
        detail: t(
          "codex.localAccess.stats.estimatedCostDetail",
          "按当前请求价格快照累计",
        ),
      },
      {
        key: "latency",
        label: t("codex.localAccess.stats.avgLatency", "平均延迟"),
        value: formatLatencyMs(avgLatencyMs),
        detail: t("codex.localAccess.stats.successRate", {
          rate: successRate,
          defaultValue: "成功率 {{rate}}%",
        }),
      },
    ],
    [avgLatencyMs, selectedTotals, successRate, t],
  );

  const localAccessAccounts = useMemo(() => accounts, [accounts]);
  const quotaPoolSummary = useMemo(
    () => summarizeCodexQuotaPool(localAccessAccounts),
    [localAccessAccounts],
  );
  const currentQuotaPoolSummary = useMemo(() => {
    const accountIds = new Set(collection?.accountIds ?? []);
    return summarizeCodexQuotaPool(
      localAccessAccounts.filter((account) => accountIds.has(account.id)),
    );
  }, [collection?.accountIds, localAccessAccounts]);
  const accountPoolHealthSummary = useMemo<AccountPoolHealthSummary>(() => {
    const accountById = new Map(
      localAccessAccounts.map((account) => [account.id, account]),
    );
    const healthById = new Map(
      (state?.accountHealth ?? []).map((health) => [health.accountId, health]),
    );
    const summary: AccountPoolHealthSummary = {
      total: collection?.accountIds.length ?? 0,
      available: 0,
      abnormal: 0,
      cooldown: 0,
      missing: 0,
      authError: 0,
      quotaLimited: 0,
    };

    (collection?.accountIds ?? []).forEach((accountId) => {
      const account = accountById.get(accountId);
      const health = healthById.get(accountId);
      if (!account) {
        summary.missing += 1;
        summary.abnormal += 1;
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
      if (isAbnormalAccountFailure(health)) {
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
  }, [collection?.accountIds, localAccessAccounts, state?.accountHealth]);
  const initialRestrictFreeAccounts = collection?.restrictFreeAccounts ?? true;
  const initialSessionAffinity = collection?.sessionAffinity ?? true;
  const initialSessionAffinityTtlSeconds = Math.round(
    (collection?.sessionAffinityTtlMs ?? 60 * 60 * 1000) / 1000,
  );
  const normalizedInitialSelectedIds = useMemo(
    () =>
      resolveCodexLocalAccessInitialAccountIds(
        initialSelectedIds,
        localAccessAccounts,
        initialRestrictFreeAccounts,
        accountsLoaded,
      ),
    [
      accountsLoaded,
      initialSelectedIds,
      initialRestrictFreeAccounts,
      localAccessAccounts,
    ],
  );

  useEffect(() => {
    if (!isOpen || mode !== "members") {
      setMembersDraftDirty(false);
    }
  }, [isOpen, mode]);

  useEffect(() => {
    if (!isOpen) return;
    const shouldResetMembersDraft = mode !== "members" || !membersDraftDirty;
    if (shouldResetMembersDraft) {
      setQuery("");
      setSelected(new Set(normalizedInitialSelectedIds));
      setFilterTypes([]);
      setTagFilter([]);
      setGroupFilter([]);
      setRestrictFreeAccounts(initialRestrictFreeAccounts);
      setSessionAffinity(initialSessionAffinity);
      setSessionAffinityTtlSeconds(String(initialSessionAffinityTtlSeconds));
    }
    setSessionAffinityTtlError("");
    setError("");
    setNotice("");
    setTestDialogOpen(false);
    setTestDialogRunning(false);
    setTestChatMessages([]);
    setTestChatInput("");
    setTestDialogError("");
    setKeyVisible(false);
    setCopiedField(null);
    setPortInput(collection?.port ? String(collection.port) : "");
    setUpstreamProxyDraftUrl(collection?.upstreamProxyUrl ?? "");
    setCustomRoutingOpen(false);
    setCustomRoutingQuery("");
    setCustomRoutingFilterTypes([]);
    setCustomRoutingTagFilter([]);
    setCustomRoutingError("");
    setCustomRoutingSelected(new Set());
    setBackupConfirmAccountId(null);
    setCustomRoutingDraft(() => {
      const ruleMap = new Map(
        (collection?.customRoutingRules ?? []).map((rule) => [
          rule.accountId,
          {
            priority: normalizeCustomRoutingPriority(rule.priority),
            weight: normalizeCustomRoutingWeight(rule.weight),
            isBackup: Boolean(rule.isBackup),
          },
        ]),
      );
      const next: Record<string, CustomRoutingDraftRule> = {};
      (collection?.accountIds ?? []).forEach((accountId) => {
        next[accountId] = ruleMap.get(accountId) ?? {
          priority: CUSTOM_ROUTING_PRIORITY_MIN,
          weight: CUSTOM_ROUTING_WEIGHT_MIN,
          isBackup: false,
        };
      });
      return next;
    });
    setCustomRoutingBulkPriority("10");
    setCustomRoutingBulkWeight("1");
    if (mode === "members") {
      window.setTimeout(() => {
        searchInputRef.current?.focus();
      }, 0);
    }
  }, [
    collection?.accountIds,
    collection?.apiKeys,
    collection?.customRoutingRules,
    collection?.port,
    collection?.upstreamProxyUrl,
    initialRestrictFreeAccounts,
    initialSessionAffinity,
    initialSessionAffinityTtlSeconds,
    isOpen,
    membersDraftDirty,
    mode,
    normalizedInitialSelectedIds,
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
    persistStatsRange(statsRange);
  }, [statsRange]);

  useEffect(() => {
    if (!isOpen) return;
    const requestSeq = ++statsRequestSeqRef.current;
    setStatsRangeError("");
    void queryCodexLocalAccessStats(statsTimeRange.startAt, statsTimeRange.endAt)
      .then((window) => {
        if (requestSeq === statsRequestSeqRef.current) setFilteredStatsWindow(window);
      })
      .catch((err) => {
        if (requestSeq === statsRequestSeqRef.current) {
          setStatsRangeError(String(err).replace(/^Error:\s*/, ""));
        }
      });
  }, [isOpen, stats?.updatedAt, statsTimeRange.endAt, statsTimeRange.startAt]);

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
    if (!testDialogOpen) return;
    const node = testChatScrollRef.current;
    scrollElementTo(node, { top: node?.scrollHeight ?? 0 });
  }, [testChatMessages, testDialogOpen]);

  const normalizeTag = (value: string) => value.trim().toLowerCase();

  const availableTags = useMemo(() => {
    const next = new Set<string>();
    localAccessAccounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const trimmed = tag.trim();
        if (trimmed) next.add(trimmed);
      });
    });
    return Array.from(next).sort((left, right) => left.localeCompare(right));
  }, [localAccessAccounts]);

  const groupIdsByAccountId = useMemo(() => {
    const next = new Map<string, Set<string>>();
    accountGroups.forEach((group) => {
      group.accountIds.forEach((accountId) => {
        const current = next.get(accountId) ?? new Set<string>();
        current.add(group.id);
        next.set(accountId, current);
      });
    });
    return next;
  }, [accountGroups]);

  const groupNameByAccountId = useMemo(() => {
    const next = new Map<string, string[]>();
    accountGroups.forEach((group) => {
      group.accountIds.forEach((accountId) => {
        const current = next.get(accountId) ?? [];
        current.push(group.name);
        next.set(accountId, current);
      });
    });
    return next;
  }, [accountGroups]);

  const groupFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      accountGroups
        .map((group) => ({
          value: group.id,
          label: `${group.name} (${group.accountIds.length})`,
        }))
        .sort((left, right) => left.label.localeCompare(right.label)),
    [accountGroups],
  );

  const tierCounts = useMemo(() => {
    const counts = createCodexPlanFilterCounts(localAccessAccounts.length);
    localAccessAccounts.forEach((account) => {
      if (!isBlockingCodexQuotaError(account.quota_error)) {
        counts.VALID += 1;
      }
      const tier = getCodexPlanFilterKey(account);
      incrementCodexPlanFilterCount(counts, tier);
      if (isBlockingCodexQuotaError(account.quota_error)) {
        counts.ERROR += 1;
      }
    });
    return counts;
  }, [localAccessAccounts]);

  const quotaPoolByPlan = useMemo(
    () => summarizeQuotaPoolsByPlan(localAccessAccounts),
    [localAccessAccounts],
  );

  const allTierFilterLabel = useMemo(
    () =>
      formatQuotaPoolLabel(
        t("common.shared.filter.all", { count: tierCounts.all }),
        quotaPoolSummary.all,
        quotaPoolLabels.weekly,
      ),
    [
      quotaPoolLabels.weekly,
      quotaPoolSummary.all,
      t,
      tierCounts.all,
    ],
  );

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      buildCodexPlanFilterOptions(tierCounts, {
        includeValid: true,
        pendingLabel: t("codex.pendingAuth.badge", "待授权"),
        validOption: buildValidAccountsFilterOption(t, tierCounts.VALID),
      }).map((option) => {
        const pool = quotaPoolByPlan[option.value];
        if (!pool) return option;
        return {
          ...option,
          label: formatQuotaPoolLabel(
            option.label,
            pool,
            quotaPoolLabels.weekly,
          ),
        };
      }),
    [
      quotaPoolLabels.weekly,
      quotaPoolByPlan,
      t,
      tierCounts,
    ],
  );

  const memberSearchQuery = memberView?.searchQuery ?? query;
  const memberFilterTypes = memberView?.filterTypes ?? filterTypes;
  const memberTagFilter = memberView?.tagFilter ?? tagFilter;
  const memberGroupFilter = memberView?.groupFilter ?? groupFilter;
  const memberTierFilterOptions =
    memberView?.tierFilterOptions ?? tierFilterOptions;
  const memberTierFilterAllLabel =
    memberView?.tierFilterAllLabel ?? allTierFilterLabel;
  const memberAvailableTags = memberView?.availableTags ?? availableTags;
  const memberGroupFilterOptions =
    memberView?.groupFilterOptions ?? groupFilterOptions;

  const visibleAccounts = useMemo(() => {
    if (memberView) {
      return memberView.accounts;
    }

    const queryText = query.trim().toLowerCase();
    const sorted = [...localAccessAccounts].sort((a, b) => {
      const aName = buildCodexAccountPresentation(
        a,
        t,
      ).displayName.toLowerCase();
      const bName = buildCodexAccountPresentation(
        b,
        t,
      ).displayName.toLowerCase();
      return aName.localeCompare(bName);
    });
    const selectedTags = new Set(tagFilter.map(normalizeTag));
    const selectedGroups = new Set(groupFilter);
    const { requireValidAccounts, selectedTypes } =
      splitValidityFilterValues(filterTypes);

    return sorted.filter((account) => {
      const presentation = buildCodexAccountPresentation(account, t);
      const displayName = presentation.displayName.toLowerCase();
      const groupNames = (groupNameByAccountId.get(account.id) ?? [])
        .join(" ")
        .toLowerCase();
      const matchesQuery =
        !queryText ||
        displayName.includes(queryText) ||
        groupNames.includes(queryText);
      if (!matchesQuery) return false;

      if (selectedTags.size > 0) {
        const accountTags = (account.tags || []).map(normalizeTag);
        if (!accountTags.some((tag) => selectedTags.has(tag))) {
          return false;
        }
      }

      if (selectedGroups.size > 0) {
        const accountGroupIds = groupIdsByAccountId.get(account.id);
        if (
          !accountGroupIds ||
          !Array.from(accountGroupIds).some((id) => selectedGroups.has(id))
        ) {
          return false;
        }
      }

      if (
        requireValidAccounts &&
        isBlockingCodexQuotaError(account.quota_error)
      ) {
        return false;
      }

      if (selectedTypes.size > 0) {
        const planKey = getCodexPlanFilterKey(account);
        const matchesType = Array.from(selectedTypes).some((type) => {
          if (type === "ERROR") {
            return isBlockingCodexQuotaError(account.quota_error);
          }
          return type === planKey;
        });
        if (!matchesType) {
          return false;
        }
      }

      return true;
    });
  }, [
    filterTypes,
    groupFilter,
    groupIdsByAccountId,
    groupNameByAccountId,
    localAccessAccounts,
    memberView,
    query,
    t,
    tagFilter,
  ]);

  const visibleSelectableAccounts = useMemo(
    () =>
      visibleAccounts.filter((account) => {
        const ineligibleReason = getCodexLocalAccessAccountIneligibleReason(
          account,
          restrictFreeAccounts,
        );
        // Keep unsupported accounts visible so users know why they cannot join.
        if (
          ineligibleReason === "chat_completions_api_key" ||
          ineligibleReason === "pending_oauth"
        ) {
          return true;
        }
        if (isCodexLocalAccessEligibleAccount(account, restrictFreeAccounts)) {
          return true;
        }
        return selected.has(account.id);
      }),
    [restrictFreeAccounts, selected, visibleAccounts],
  );
  const memberPagination = usePagination({
    items: visibleSelectableAccounts,
    storageKey: buildPaginationPageSizeStorageKey("CodexLocalAccessMembers"),
    pageSizeOptions: LOCAL_ACCESS_MEMBER_PAGE_SIZE_OPTIONS,
    defaultPageSize: 50,
  });
  const paginatedVisibleSelectableAccounts = memberPagination.pageItems;

  useEffect(() => {
    memberPagination.setCurrentPage(1);
  }, [
    memberFilterTypes,
    memberGroupFilter,
    memberPagination.setCurrentPage,
    memberSearchQuery,
    restrictFreeAccounts,
    memberTagFilter,
  ]);

  const visibleEnabledAccounts = useMemo(
    () =>
      visibleSelectableAccounts.filter((account) =>
        isCodexLocalAccessEligibleAccount(account, restrictFreeAccounts),
      ),
    [restrictFreeAccounts, visibleSelectableAccounts],
  );

  const selectedVisibleCount = useMemo(
    () =>
      visibleEnabledAccounts.reduce(
        (count, account) => count + (selected.has(account.id) ? 1 : 0),
        0,
      ),
    [selected, visibleEnabledAccounts],
  );

  const allVisibleSelected =
    visibleEnabledAccounts.length > 0 &&
    selectedVisibleCount === visibleEnabledAccounts.length;

  useEffect(() => {
    if (!selectAllCheckboxRef.current) return;
    selectAllCheckboxRef.current.indeterminate =
      selectedVisibleCount > 0 && !allVisibleSelected;
  }, [allVisibleSelected, selectedVisibleCount]);

  const initialBackupAccountIds = useMemo(() => {
    const selectedSet = new Set(normalizedInitialSelectedIds);
    return new Set(
      (collection?.customRoutingRules ?? [])
        .filter(
          (rule) => rule.isBackup && selectedSet.has(rule.accountId),
        )
        .map((rule) => rule.accountId),
    );
  }, [collection?.customRoutingRules, normalizedInitialSelectedIds]);

  const currentBackupAccountIds = useMemo(() => {
    const ruleBackupById = new Map(
      (collection?.customRoutingRules ?? []).map((rule) => [
        rule.accountId,
        Boolean(rule.isBackup),
      ]),
    );
    const next = new Set<string>();
    selected.forEach((accountId) => {
      const isBackup =
        customRoutingDraft[accountId]?.isBackup ??
        ruleBackupById.get(accountId) ??
        false;
      if (isBackup) {
        next.add(accountId);
      }
    });
    return next;
  }, [collection?.customRoutingRules, customRoutingDraft, selected]);

  const selectionDirty = useMemo(
    () =>
      !areSetsEqual(selected, new Set(normalizedInitialSelectedIds)) ||
      restrictFreeAccounts !== (collection?.restrictFreeAccounts ?? true) ||
      sessionAffinity !== initialSessionAffinity ||
      sessionAffinityTtlSeconds !== String(initialSessionAffinityTtlSeconds) ||
      !areSetsEqual(currentBackupAccountIds, initialBackupAccountIds),
    [
      collection?.restrictFreeAccounts,
      currentBackupAccountIds,
      initialSessionAffinity,
      initialSessionAffinityTtlSeconds,
      initialBackupAccountIds,
      normalizedInitialSelectedIds,
      restrictFreeAccounts,
      sessionAffinity,
      sessionAffinityTtlSeconds,
      selected,
    ],
  );

  const allStatsByAccountId = useMemo(() => {
    const next = new Map<
      string,
      NonNullable<CodexLocalAccessState["stats"]>["accounts"][number]
    >();
    stats?.accounts.forEach((item) => next.set(item.accountId, item));
    return next;
  }, [stats?.accounts]);

  const windowStatsByAccountId = useMemo(() => {
    const next = new Map<
      string,
      NonNullable<CodexLocalAccessState["stats"]>["accounts"][number]
    >();
    selectedStatsWindow?.accounts.forEach((item) =>
      next.set(item.accountId, item),
    );
    return next;
  }, [selectedStatsWindow?.accounts]);

  const currentMemberStats = useMemo(() => {
    const currentIds = collection?.accountIds ?? [];
    return currentIds
      .map((accountId) => {
        const account = localAccessAccounts.find(
          (item) => item.id === accountId,
        );
        if (!account) return null;
        const presentation = buildCodexAccountPresentation(account, t);
        const accountStats = windowStatsByAccountId.get(account.id);
        return {
          account,
          presentation,
          stats: accountStats?.usage ?? null,
        };
      })
      .filter((item): item is NonNullable<typeof item> => Boolean(item))
      .sort((left, right) => {
        const rightCount = right.stats?.requestCount ?? 0;
        const leftCount = left.stats?.requestCount ?? 0;
        return rightCount - leftCount;
      });
  }, [collection?.accountIds, localAccessAccounts, t, windowStatsByAccountId]);

  const routingStrategyOptions = useMemo(
    () =>
      [
        {
          value: "auto",
          label: t("codex.localAccess.routingStrategy.auto", "自动（推荐）"),
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
          label: t(
            "codex.localAccess.routingStrategy.quotaLowFirst",
            "优先低配额",
          ),
        },
        {
          value: "plan_high_first",
          label: t(
            "codex.localAccess.routingStrategy.planHighFirst",
            "优先高订阅",
          ),
        },
        {
          value: "plan_low_first",
          label: t(
            "codex.localAccess.routingStrategy.planLowFirst",
            "优先低订阅",
          ),
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
      ] satisfies Array<{
        value: CodexLocalAccessRoutingStrategy;
        label: string;
      }>,
    [t],
  );
  const accessScopeOptions = useMemo(
    () => [
      {
        value: "localhost",
        label: t("codex.localAccess.accessScopeLocalhost", "仅本机"),
      },
      {
        value: "lan",
        label: t("codex.localAccess.accessScopeLan", "局域网"),
      },
    ],
    [t],
  );
  const renderQuotaPreview = (
    presentation: ReturnType<typeof buildCodexAccountPresentation>,
    limit = 2,
  ) => {
    const quotaLines = buildQuotaPreviewLines(presentation.quotaItems, limit);
    if (quotaLines.length === 0) {
      return null;
    }

    return (
      <div className="codex-local-access-quota-line">
        {quotaLines.map((line) => (
          <span
            key={line.key}
            className={`codex-local-access-quota-chip ${line.quotaClass}`}
            title={line.title}
          >
            <span className="codex-local-access-quota-dot" />
            <span>{line.text}</span>
          </span>
        ))}
      </div>
    );
  };

  const localAccessAccountById = useMemo(
    () => new Map(localAccessAccounts.map((account) => [account.id, account])),
    [localAccessAccounts],
  );

  const customRoutingRuleByAccountId = useMemo(() => {
    const next = new Map<string, CustomRoutingDraftRule>();
    collection?.customRoutingRules?.forEach((rule) => {
      next.set(rule.accountId, {
        priority: normalizeCustomRoutingPriority(rule.priority),
        weight: normalizeCustomRoutingWeight(rule.weight),
        isBackup: Boolean(rule.isBackup),
      });
    });
    return next;
  }, [collection?.customRoutingRules]);

  const customRoutingAccounts = useMemo(() => {
    const currentIds = collection?.accountIds ?? [];
    return currentIds
      .map((accountId) => localAccessAccountById.get(accountId))
      .filter((account): account is CodexAccount => Boolean(account));
  }, [collection?.accountIds, localAccessAccountById]);

  const customRoutingAvailableTags = useMemo(() => {
    const next = new Set<string>();
    customRoutingAccounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const trimmed = tag.trim();
        if (trimmed) next.add(trimmed);
      });
    });
    return Array.from(next).sort((left, right) => left.localeCompare(right));
  }, [customRoutingAccounts]);

  const customRoutingQuotaPoolSummary = useMemo(
    () => summarizeCodexQuotaPool(customRoutingAccounts),
    [customRoutingAccounts],
  );

  const customRoutingTierCounts = useMemo(() => {
    const counts = createCodexPlanFilterCounts(customRoutingAccounts.length);
    customRoutingAccounts.forEach((account) => {
      if (!isBlockingCodexQuotaError(account.quota_error)) {
        counts.VALID += 1;
      }
      const tier = getCodexPlanFilterKey(account);
      incrementCodexPlanFilterCount(counts, tier);
      if (isBlockingCodexQuotaError(account.quota_error)) {
        counts.ERROR += 1;
      }
    });
    return counts;
  }, [customRoutingAccounts]);

  const customRoutingQuotaPoolByPlan = useMemo(() => {
    return summarizeQuotaPoolsByPlan(customRoutingAccounts);
  }, [customRoutingAccounts]);

  const customRoutingAllTierFilterLabel = useMemo(
    () =>
      formatQuotaPoolLabel(
        t("common.shared.filter.all", { count: customRoutingTierCounts.all }),
        customRoutingQuotaPoolSummary.all,
        quotaPoolLabels.weekly,
      ),
    [
      customRoutingQuotaPoolSummary.all,
      customRoutingTierCounts.all,
      quotaPoolLabels.weekly,
      t,
    ],
  );

  const customRoutingTierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      buildCodexPlanFilterOptions(customRoutingTierCounts, {
        includeValid: true,
        pendingLabel: t("codex.pendingAuth.badge", "待授权"),
        validOption: buildValidAccountsFilterOption(
          t,
          customRoutingTierCounts.VALID,
        ),
      }).map((option) => {
        const pool = customRoutingQuotaPoolByPlan[option.value];
        if (!pool) return option;
        return {
          ...option,
          label: formatQuotaPoolLabel(
            option.label,
            pool,
            quotaPoolLabels.weekly,
          ),
        };
      }),
    [
      customRoutingQuotaPoolByPlan,
      customRoutingTierCounts,
      quotaPoolLabels.weekly,
      t,
    ],
  );

  const visibleCustomRoutingAccounts = useMemo(() => {
    const normalizedQuery = customRoutingQuery.trim().toLowerCase();
    const selectedTags = new Set(customRoutingTagFilter.map(normalizeTag));
    const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(
      customRoutingFilterTypes,
    );

    return customRoutingAccounts.filter((account) => {
      const presentation = buildCodexAccountPresentation(account, t);
      const matchesQuery =
        !normalizedQuery ||
        presentation.displayName.toLowerCase().includes(normalizedQuery) ||
        account.id.toLowerCase().includes(normalizedQuery) ||
        presentation.planLabel.toLowerCase().includes(normalizedQuery);
      if (!matchesQuery) return false;

      if (selectedTags.size > 0) {
        const accountTags = (account.tags || []).map(normalizeTag);
        if (!accountTags.some((tag) => selectedTags.has(tag))) {
          return false;
        }
      }

      if (
        requireValidAccounts &&
        isBlockingCodexQuotaError(account.quota_error)
      ) {
        return false;
      }

      if (selectedTypes.size > 0) {
        const planKey = getCodexPlanFilterKey(account);
        const matchesType = Array.from(selectedTypes).some((type) => {
          if (type === "ERROR") {
            return isBlockingCodexQuotaError(account.quota_error);
          }
          return type === planKey;
        });
        if (!matchesType) {
          return false;
        }
      }

      return true;
    });
  }, [
    customRoutingAccounts,
    customRoutingFilterTypes,
    customRoutingQuery,
    customRoutingTagFilter,
    t,
  ]);

  const selectedVisibleCustomRoutingCount = useMemo(
    () =>
      visibleCustomRoutingAccounts.filter((account) =>
        customRoutingSelected.has(account.id),
      ).length,
    [customRoutingSelected, visibleCustomRoutingAccounts],
  );
  const allVisibleCustomRoutingSelected =
    visibleCustomRoutingAccounts.length > 0 &&
    selectedVisibleCustomRoutingCount === visibleCustomRoutingAccounts.length;

  useEffect(() => {
    if (!customRoutingSelectAllRef.current) return;
    customRoutingSelectAllRef.current.indeterminate =
      selectedVisibleCustomRoutingCount > 0 && !allVisibleCustomRoutingSelected;
  }, [allVisibleCustomRoutingSelected, selectedVisibleCustomRoutingCount]);

  const handleCopy = async (field: CopyableField, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      window.setTimeout(
        () => setCopiedField((current) => (current === field ? null : current)),
        1200,
      );
    } catch (err) {
      setError(t("common.shared.export.copyFailed", "复制失败，请手动复制"));
      console.error("Failed to copy local access value:", err);
    }
  };

  const runAction = async (task: () => Promise<void>, successText: string) => {
    setError("");
    setNotice("");
    try {
      await task();
      setNotice(successText);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const toggleSelectAllVisible = () => {
    if (membersInteractionDisabled || visibleEnabledAccounts.length === 0) {
      return;
    }
    setMembersDraftDirty(true);
    setSelected((prev) => {
      const next = new Set(prev);
      if (allVisibleSelected) {
        for (const account of visibleEnabledAccounts) {
          next.delete(account.id);
        }
      } else {
        for (const account of visibleEnabledAccounts) {
          next.add(account.id);
        }
      }
      return next;
    });
  };

  const handleToggleRestrictFreeAccounts = async () => {
    if (membersInteractionDisabled) return;
    setMembersDraftDirty(true);
    setRestrictFreeAccounts((prev) => !prev);
  };

  const handleToggleSessionAffinity = async () => {
    if (membersInteractionDisabled) return;
    setSessionAffinityTtlError("");
    if (sessionAffinity) {
      setMembersDraftDirty(true);
      setSessionAffinity(false);
      return;
    }
    const confirmed = await confirmDialog(
      t(
        "codex.localAccess.modal.sessionAffinityDescription",
        "开启后，同一个会话会尽量持续使用同一个账号进行对话；账号不可用时仍会按调度策略切换，超过过期时间后会重新选择账号。",
      ),
      {
        title: t(
          "codex.localAccess.modal.sessionAffinityConfirmTitle",
          "开启会话亲和？",
        ),
      },
    );
    if (!confirmed) return;
    setMembersDraftDirty(true);
    setSessionAffinity(true);
  };

  const toggleSelect = (accountId: string) => {
    if (membersInteractionDisabled) return;
    const account = localAccessAccountById.get(accountId);
    if (!account) return;
    const isSelectionBlocked =
      !isCodexLocalAccessEligibleAccount(account, restrictFreeAccounts) &&
      !selected.has(accountId);
    if (isSelectionBlocked) {
      return;
    }
    setMembersDraftDirty(true);
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  };

  const handleSaveMembers = async () => {
    if (!accountsLoaded) return;
    setError("");
    setNotice("");
    setSessionAffinityTtlError("");
    const parsedSessionAffinityTtlSeconds = Number(
      sessionAffinityTtlSeconds.trim(),
    );
    if (
      !Number.isInteger(parsedSessionAffinityTtlSeconds) ||
      parsedSessionAffinityTtlSeconds < 60 ||
      parsedSessionAffinityTtlSeconds > 86400
    ) {
      const message = t("codex.apiService.validation.numberRange", {
        min: 60,
        max: 86400,
        defaultValue: "请输入 {{min}} 到 {{max}} 之间的数字",
      });
      setSessionAffinityTtlError(message);
      requestAnimationFrame(() => sessionAffinityTtlInputRef.current?.focus());
      return;
    }
    try {
      const filtered = Array.from(selected).filter((accountId) => {
        const account = localAccessAccountById.get(accountId);
        if (!account) return false;
        return isCodexLocalAccessEligibleAccount(account, restrictFreeAccounts);
      });
      const backupAccountIds = filtered.filter((accountId) => {
        const isBackup =
          customRoutingDraft[accountId]?.isBackup ??
          customRoutingRuleByAccountId.get(accountId)?.isBackup ??
          false;
        return isBackup;
      });
      await onSaveAccounts({
        accountIds: filtered,
        restrictFreeAccounts,
        backupAccountIds,
        sessionAffinity,
        sessionAffinityTtlMs: parsedSessionAffinityTtlSeconds * 1000,
      });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
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
        await onUpdatePort(nextPort);
      },
      t("codex.localAccess.portSaveSuccess", "API 服务端口已更新"),
    );
  };

  const openCustomRoutingDialog = () => {
    if (!collection) return;
    setError("");
    setNotice("");
    setCustomRoutingError("");
    setCustomRoutingOpen(true);
  };

  const handleChangeRoutingStrategy = async (nextStrategy: string) => {
    if (!collection) return;
    if (nextStrategy === "custom") {
      openCustomRoutingDialog();
      return;
    }
    if (nextStrategy === routingStrategy) return;

    await runAction(
      async () => {
        await onUpdateRoutingStrategy(
          nextStrategy as CodexLocalAccessRoutingStrategy,
        );
      },
      t("codex.localAccess.routingSaveSuccess", "API 服务调度策略已更新"),
    );
  };

  const closeCustomRoutingDialog = () => {
    if (saving) return;
    setBackupConfirmAccountId(null);
    setCustomRoutingOpen(false);
    setCustomRoutingError("");
    setCustomRoutingSelected(new Set());
  };

  const toggleCustomRoutingSelect = (accountId: string) => {
    if (saving) return;
    setCustomRoutingSelected((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  };

  const toggleCustomRoutingSelectAllVisible = () => {
    if (saving || visibleCustomRoutingAccounts.length === 0) return;
    setCustomRoutingSelected((prev) => {
      const next = new Set(prev);
      if (allVisibleCustomRoutingSelected) {
        visibleCustomRoutingAccounts.forEach((account) =>
          next.delete(account.id),
        );
      } else {
        visibleCustomRoutingAccounts.forEach((account) => next.add(account.id));
      }
      return next;
    });
  };

  const updateCustomRoutingRule = (
    accountId: string,
    field: "priority" | "weight",
    rawValue: string,
  ) => {
    const parsed = Number.parseInt(rawValue, 10);
    setCustomRoutingDraft((prev) => {
      const current = prev[accountId] ?? {
        priority: CUSTOM_ROUTING_PRIORITY_MIN,
        weight: CUSTOM_ROUTING_WEIGHT_MIN,
        isBackup: false,
      };
      return {
        ...prev,
        [accountId]: {
          ...current,
          [field]:
            field === "priority"
              ? normalizeCustomRoutingPriority(parsed)
              : normalizeCustomRoutingWeight(parsed),
        },
      };
    });
  };

  const toggleMemberBackup = (accountId: string) => {
    if (membersInteractionDisabled) return;
    if (!selected.has(accountId)) {
      setMembersDraftDirty(true);
      setSelected((prev) => {
        const next = new Set(prev);
        next.add(accountId);
        return next;
      });
    }
    const current = customRoutingDraft[accountId] ??
      customRoutingRuleByAccountId.get(accountId) ?? {
        priority: CUSTOM_ROUTING_PRIORITY_MIN,
        weight: CUSTOM_ROUTING_WEIGHT_MIN,
        isBackup: false,
      };
    if (!current.isBackup) {
      setBackupConfirmAccountId(accountId);
      return;
    }
    setMembersDraftDirty(true);
    setCustomRoutingDraft((prev) => ({
      ...prev,
      [accountId]: {
        ...current,
        isBackup: false,
      },
    }));
  };

  const confirmMemberBackup = () => {
    const accountId = backupConfirmAccountId;
    if (!accountId || membersInteractionDisabled) return;
    setMembersDraftDirty(true);
    setCustomRoutingDraft((prev) => {
      const current = prev[accountId] ??
        customRoutingRuleByAccountId.get(accountId) ?? {
          priority: CUSTOM_ROUTING_PRIORITY_MIN,
          weight: CUSTOM_ROUTING_WEIGHT_MIN,
          isBackup: false,
        };
      return {
        ...prev,
        [accountId]: {
          ...current,
          isBackup: true,
        },
      };
    });
    setBackupConfirmAccountId(null);
  };

  const applyCustomRoutingBatch = () => {
    if (saving || customRoutingSelected.size === 0) return;
    const priority = normalizeCustomRoutingPriority(
      Number.parseInt(customRoutingBulkPriority, 10),
    );
    const weight = normalizeCustomRoutingWeight(
      Number.parseInt(customRoutingBulkWeight, 10),
    );
    setCustomRoutingBulkPriority(String(priority));
    setCustomRoutingBulkWeight(String(weight));
    setCustomRoutingDraft((prev) => {
      const next = { ...prev };
      customRoutingSelected.forEach((accountId) => {
        next[accountId] = {
          priority,
          weight,
          isBackup:
            next[accountId]?.isBackup ??
            customRoutingRuleByAccountId.get(accountId)?.isBackup ??
            false,
        };
      });
      return next;
    });
  };

  const resetCustomRoutingDraft = () => {
    if (!collection || saving) return;
    const next: Record<string, CustomRoutingDraftRule> = {};
    collection.accountIds.forEach((accountId) => {
      next[accountId] = {
        priority: CUSTOM_ROUTING_PRIORITY_MIN,
        weight: CUSTOM_ROUTING_WEIGHT_MIN,
        isBackup: false,
      };
    });
    setCustomRoutingDraft(next);
    setCustomRoutingSelected(new Set());
  };

  const handleSaveCustomRouting = async () => {
    if (!collection) return;
    setCustomRoutingError("");
    setNotice("");
    try {
      const rules = collection.accountIds.map((accountId) => {
        const rule = customRoutingDraft[accountId] ??
          customRoutingRuleByAccountId.get(accountId) ?? {
            priority: CUSTOM_ROUTING_PRIORITY_MIN,
            weight: CUSTOM_ROUTING_WEIGHT_MIN,
            isBackup: false,
          };
        return {
          accountId,
          priority: normalizeCustomRoutingPriority(rule.priority),
          weight: normalizeCustomRoutingWeight(rule.weight),
          isBackup: rule.isBackup,
        };
      });
      await onUpdateCustomRouting(rules);
      setNotice(
        t(
          "codex.localAccess.customRoutingSaveSuccess",
          "API 服务自定义调度已更新",
        ),
      );
      setCustomRoutingOpen(false);
      setCustomRoutingSelected(new Set());
    } catch (err) {
      setCustomRoutingError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleChangeAccessScope = async (nextValue: string) => {
    if (!collection) return;
    const nextAccessScope = normalizeAccessScope(nextValue);
    if (nextAccessScope === accessScope) return;

    await runAction(
      async () => {
        await onUpdateAccessScope(nextAccessScope);
      },
      t("codex.localAccess.accessScopeSaveSuccess", "API 服务访问范围已更新"),
    );
  };

  const handleSaveUpstreamProxyConfig = async () => {
    if (!collection) return;
    const upstreamProxyUrlDraft = upstreamProxyDraftUrl.trim();
    if (upstreamProxyUrlDraft === upstreamProxyUrl.trim()) {
      setUpstreamProxyDraftUrl(upstreamProxyUrlDraft);
      return;
    }

    await runAction(
      async () => {
        await onUpdateUpstreamProxyConfig(upstreamProxyUrlDraft || null);
      },
      t("codex.localAccess.upstreamProxySaveSuccess", "API 代理地址已更新"),
    );
  };

  const handleToggleDebugLogs = async () => {
    if (!collection) return;
    const nextDebugLogs = !collection.debugLogs;
    const confirmed = await confirmDialog(
      nextDebugLogs
        ? t(
            "codex.localAccess.debugLogsEnableConfirmMessage",
            "打开后会输出 API 服务调试日志，用于定位网关、代理、上游请求和流式响应问题。高并发或长时间流式请求时可能带来少量性能开销，建议排查完成后关闭。确认打开吗？",
          )
        : t(
            "codex.localAccess.debugLogsDisableConfirmMessage",
            "关闭后会停止输出 API 服务调试日志，减少日志噪声和额外开销；后续排查网关、代理或流式响应问题时可再次打开。确认关闭吗？",
          ),
      {
        title: nextDebugLogs
          ? t(
              "codex.localAccess.debugLogsEnableConfirmTitle",
              "是否打开日志调试模式？",
            )
          : t(
              "codex.localAccess.debugLogsDisableConfirmTitle",
              "是否关闭日志调试模式？",
            ),
        kind: "info",
        okLabel: nextDebugLogs
          ? t("codex.localAccess.debugLogsEnableConfirmAction", "打开")
          : t("codex.localAccess.debugLogsDisableConfirmAction", "关闭"),
        cancelLabel: t("common.cancel"),
      },
    );

    if (!confirmed) {
      return;
    }

    setError("");
    setNotice("");
    try {
      await onUpdateDebugLogs(nextDebugLogs);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleResetKey = async () => {
    const confirmed = await confirmDialog(
      t(
        "codex.localAccess.rotateConfirmMessage",
        "重置后当前 API 服务密钥会立即失效，正在进行中的请求可能不可用。确认继续吗？",
      ),
      {
        title: t("codex.localAccess.rotateKey", "重置密钥"),
        kind: "warning",
        okLabel: t("common.confirm"),
        cancelLabel: t("common.cancel"),
      },
    );

    if (!confirmed) {
      return;
    }

    await runAction(
      async () => {
        await onRotateApiKey();
        setKeyVisible(true);
      },
      t("codex.localAccess.rotateSuccess", "API 服务密钥已重置"),
    );
  };

  const handleClearStats = async () => {
    const confirmed = await confirmDialog(
      t("codex.localAccess.clearStatsConfirm", "确定要清空 API 服务统计吗？"),
      {
        title: t("codex.localAccess.clearStats", "清除统计"),
        kind: "warning",
        okLabel: t("common.confirm"),
        cancelLabel: t("common.cancel"),
      },
    );

    if (!confirmed) {
      return;
    }

    await runAction(
      async () => {
        await onClearStats();
      },
      t("codex.localAccess.clearStatsSuccess", "API 服务统计已清空"),
    );
  };

  const handleKillPort = async () => {
    await runAction(
      async () => {
        await onKillPort();
      },
      t("codex.localAccess.killPortSuccessUnknown", "API 服务端口已清理"),
    );
  };

  const handleRefreshStats = async () => {
    setError("");
    setNotice("");
    try {
      await onRefreshStats();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleToggleEnabled = async () => {
    await runAction(
      async () => {
        await onToggleEnabled();
      },
      collection?.enabled
        ? t("codex.localAccess.disabledSuccess", "API 服务已停用")
        : t("codex.localAccess.enabledSuccess", "API 服务已启用"),
    );
  };

  const closeTestDialog = () => {
    if (testDialogBusy) return;
    setTestDialogOpen(false);
  };

  const handleTest = () => {
    setTestDialogOpen(true);
    setTestDialogError("");
  };

  const clearTestChat = () => {
    if (testDialogBusy) return;
    setTestChatMessages([]);
    setTestDialogError("");
  };

  const handleSendTestChatMessage = async () => {
    if (testDialogBusy) return;
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
    const sessionId = `local-access-test-${Date.now()}-${Math.random()
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
      await onStreamTestMessage({
        sessionId,
        modelId: selectedModelId,
        messages: apiMessages,
      });
    } catch (err) {
      setTestDialogError(err instanceof Error ? err.message : String(err));
      setTestChatMessages((current) =>
        current.filter((message) => message.id !== assistantMessage.id),
      );
    } finally {
      unlisten?.();
      setTestDialogRunning(false);
    }
  };

  if (!isOpen) return null;
  const isMembersMode = mode === "members";

  return (
    <>
      <div
        className={`modal-overlay codex-local-access-modal-overlay${
          isMembersMode ? "" : " codex-local-access-modal-overlay-panel"
        }`}
      >
        <div
          className={`modal codex-local-access-modal${
            isMembersMode
              ? " codex-local-access-modal-members group-account-picker-modal"
              : " codex-local-access-modal-panel"
          }`}
          onClick={(event) => event.stopPropagation()}
        >
          <div className="modal-header codex-local-access-modal-header">
            <div className="codex-local-access-header-main">
              <h2 className="group-account-picker-title">
                <Server size={18} />
                <span>
                  {isMembersMode
                    ? t("codex.localAccess.entryAction", "添加至 API 服务")
                    : t("codex.localAccess.title", "API 服务")}
                </span>
              </h2>
              {!isMembersMode && (
                <div className="codex-local-access-header-meta">
                  <div className="codex-local-access-header-badges">
                    <span
                      className={`codex-local-access-status ${
                        state?.running ? "running" : "stopped"
                      }`}
                    >
                      {collection?.enabled
                        ? state?.running
                          ? t("codex.localAccess.statusRunning", "运行中")
                          : t("codex.localAccess.statusStopped", "未运行")
                        : t("codex.localAccess.statusDisabled", "已停用")}
                    </span>
                    <span className="codex-local-access-subtle-badge">
                      {accessScopeBadge}
                    </span>
                    <button
                      type="button"
                      className="codex-local-access-test-pill"
                      onClick={() => void handleTest()}
                      disabled={!collection || testDialogBusy || saving}
                      title={t(
                        "codex.localAccess.testDialogTitle",
                        "测试 API 服务",
                      )}
                      aria-label={t(
                        "codex.localAccess.testDialogTitle",
                        "测试 API 服务",
                      )}
                    >
                      <ShieldCheck
                        size={13}
                        className={testDialogBusy ? "loading-spinner" : ""}
                      />
                      <span>{t("codex.localAccess.testAction", "测试")}</span>
                    </button>
                    {collection && (
                      <div className="codex-local-access-header-upstream">
                        <div className="codex-local-access-header-upstream-url">
                          <input
                            type="text"
                            value={upstreamProxyDraftUrl}
                            onChange={(event) =>
                              setUpstreamProxyDraftUrl(event.target.value)
                            }
                            onKeyDown={(event) => {
                              if (event.key === "Enter") {
                                event.preventDefault();
                                void handleSaveUpstreamProxyConfig();
                              }
                            }}
                            disabled={saving || testing || starting}
                            placeholder={t(
                              "codex.localAccess.upstreamProxyUrlPlaceholder",
                              "留空使用全局代理",
                            )}
                            aria-label={t(
                              "codex.localAccess.upstreamProxyLabel",
                              "API 代理地址",
                            )}
                          />
                          <button
                            type="button"
                            className="codex-local-access-upstream-save-btn"
                            onClick={() => void handleSaveUpstreamProxyConfig()}
                            disabled={saving || testing || starting}
                            title={t(
                              "codex.localAccess.upstreamProxySaveAction",
                              "保存代理",
                            )}
                            aria-label={t(
                              "codex.localAccess.upstreamProxySaveAction",
                              "保存代理",
                            )}
                          >
                            <Check size={13} />
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                  <div className="codex-local-access-header-tools">
                    {onOpenFullPage && (
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm codex-local-access-full-page-btn"
                        onClick={onOpenFullPage}
                        title={t(
                          "codex.apiService.openFullPage",
                          "查看全部功能",
                        )}
                        aria-label={t(
                          "codex.apiService.openFullPage",
                          "查看全部功能",
                        )}
                      >
                        <ExternalLink size={14} />
                        <span>
                          {t("codex.apiService.openFullPage", "查看全部功能")}
                        </span>
                      </button>
                    )}
                    <button
                      type="button"
                      className="folder-icon-btn codex-local-access-toolbar-btn"
                      onClick={() => void handleRefreshStats()}
                      disabled={!collection || actionBusy}
                      title={t("codex.localAccess.refreshStats", "刷新统计")}
                      aria-label={t(
                        "codex.localAccess.refreshStats",
                        "刷新统计",
                      )}
                    >
                      <RefreshCw
                        size={14}
                        className={saving ? "loading-spinner" : ""}
                      />
                    </button>
                    {collection && (
                      <>
                        <div className="codex-local-access-header-routing">
                          <SingleSelectDropdown
                            value={routingStrategy}
                            options={routingStrategyOptions}
                            onChange={(value) =>
                              void handleChangeRoutingStrategy(value)
                            }
                            disabled={saving || testing || starting}
                            ariaLabel={t(
                              "codex.localAccess.routingLabel",
                              "调度策略",
                            )}
                          />
                        </div>
                        {routingStrategy === "custom" && (
                          <button
                            type="button"
                            className="folder-icon-btn codex-local-access-toolbar-btn"
                            onClick={openCustomRoutingDialog}
                            disabled={saving || testing || starting}
                            title={t(
                              "codex.localAccess.customRoutingEdit",
                              "编辑自定义调度",
                            )}
                            aria-label={t(
                              "codex.localAccess.customRoutingEdit",
                              "编辑自定义调度",
                            )}
                          >
                            <SlidersHorizontal size={14} />
                          </button>
                        )}
                      </>
                    )}
                    {collection && (
                      <button
                        type="button"
                        className={`folder-icon-btn codex-local-access-toolbar-btn codex-local-access-debug-toggle${
                          collection.debugLogs ? " is-active" : ""
                        }`}
                        onClick={() => void handleToggleDebugLogs()}
                        disabled={saving || testing || starting}
                        title={
                          collection.debugLogs
                            ? t(
                                "codex.localAccess.debugLogsEnabledSuccess",
                                "调试日志已开启",
                              )
                            : t(
                                "codex.localAccess.debugLogsDisabledSuccess",
                                "调试日志已关闭",
                              )
                        }
                        aria-label={
                          collection.debugLogs
                            ? t(
                                "codex.localAccess.debugLogsEnabledSuccess",
                                "调试日志已开启",
                              )
                            : t(
                                "codex.localAccess.debugLogsDisabledSuccess",
                                "调试日志已关闭",
                              )
                        }
                        aria-pressed={collection.debugLogs}
                      >
                        <Bug size={14} />
                      </button>
                    )}
                    <button
                      type="button"
                      className={`folder-icon-btn codex-local-access-toolbar-btn ${
                        collection?.enabled ? "is-danger" : "is-primary"
                      }`}
                      onClick={() => void handleToggleEnabled()}
                      disabled={!collection || saving || testing || starting}
                      title={
                        collection?.enabled
                          ? t("codex.localAccess.disableService", "停用服务")
                          : t("codex.localAccess.enableService", "启用服务")
                      }
                      aria-label={
                        collection?.enabled
                          ? t("codex.localAccess.disableService", "停用服务")
                          : t("codex.localAccess.enableService", "启用服务")
                      }
                    >
                      <Power size={14} />
                    </button>
                  </div>
                </div>
              )}
            </div>
            <button
              className="modal-close codex-local-access-close"
              onClick={onClose}
              aria-label={t("common.close")}
            >
              <X size={18} />
            </button>
          </div>

          <div className="modal-body codex-local-access-modal-body">
            {state?.lastError && (
              <div className="codex-local-access-inline-error codex-local-access-inline-error-with-action">
                <CircleAlert size={14} />
                <span>{state.lastError}</span>
                {collection && (
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm codex-local-access-inline-action"
                    onClick={() => void handleKillPort()}
                    disabled={actionBusy}
                  >
                    {portCleanupBusy ? (
                      <RefreshCw size={14} className="loading-spinner" />
                    ) : (
                      <Wrench size={14} />
                    )}
                    {t("codex.localAccess.killPortAction", "清理端口")}
                  </button>
                )}
              </div>
            )}

            {error && (
              <div className="codex-local-access-inline-error">
                <CircleAlert size={14} />
                <span>{error}</span>
              </div>
            )}

            {notice && (
              <div className="codex-local-access-inline-success">
                <Check size={14} />
                <span>{notice}</span>
              </div>
            )}

            {!isMembersMode && (
              <section className="codex-local-access-section codex-local-access-section-surface codex-local-access-summary-block">
                <div className="codex-local-access-summary-head">
                  <div className="codex-local-access-section-title">
                    <Activity size={16} />
                    <span>{t("codex.localAccess.statsTitle", "总量统计")}</span>
                  </div>
                  <div className="codex-local-access-summary-actions">
                    <CodexStatsRangePicker
                      value={statsRange}
                      range={statsTimeRange}
                      onPresetChange={handleStatsPresetChange}
                      onCustomApply={handleCustomStatsRangeApply}
                      disabled={actionBusy}
                      error={statsRangeError}
                      compact
                    />
                    <button
                      type="button"
                      className="btn btn-danger btn-sm"
                      onClick={() => void handleClearStats()}
                      disabled={!collection || actionBusy}
                      title={t("codex.localAccess.clearStats", "清除统计")}
                      aria-label={t("codex.localAccess.clearStats", "清除统计")}
                    >
                      <Trash2 size={14} />
                      {t("codex.localAccess.clearStats", "清除统计")}
                    </button>
                  </div>
                </div>
                <div className="codex-local-access-stats-grid">
                  {summaryStats.map((item) => (
                    <div
                      key={item.key}
                      className={`codex-local-access-stat-card codex-local-access-stat-card-${item.key}`}
                    >
                      <span className="codex-local-access-stat-label">
                        {item.label}
                      </span>
                      <strong>{item.value}</strong>
                      <span className="codex-local-access-stat-sub">
                        {item.detail}
                      </span>
                    </div>
                  ))}
                </div>
                {currentQuotaPoolSummary.visiblePlans.length > 0 && (
                  <div
                    className="codex-local-access-quota-pool-grid"
                    aria-label={quotaPoolLabels.title}
                  >
                    {accountPoolHealthSummary.total > 0 && (
                      <button
                        type="button"
                        className={`codex-local-access-quota-pool-card codex-local-access-health-pool-card${
                          accountPoolHealthSummary.available <
                            accountPoolHealthSummary.total ||
                          accountPoolHealthSummary.abnormal > 0 ||
                          accountPoolHealthSummary.cooldown > 0
                            ? " has-issue"
                            : ""
                        }`}
                        title={t("codex.localAccess.accountPoolHealth.detail", {
                          available: accountPoolHealthSummary.available,
                          total: accountPoolHealthSummary.total,
                          abnormal: accountPoolHealthSummary.abnormal,
                          cooldown: accountPoolHealthSummary.cooldown,
                          missing: accountPoolHealthSummary.missing,
                          authError: accountPoolHealthSummary.authError,
                          quotaLimited: accountPoolHealthSummary.quotaLimited,
                          defaultValue:
                            "可用 {{available}}/{{total}}，异常 {{abnormal}}，冷却 {{cooldown}}，缺失 {{missing}}，鉴权 {{authError}}，额度 {{quotaLimited}}",
                        })}
                        onClick={() => setHealthModalOpen(true)}
                        aria-label={t(
                          "codex.localAccess.accountPoolHealth.openDetails",
                          "查看异常账号详情",
                        )}
                      >
                        <span className="codex-local-access-quota-pool-plan">
                          {t(
                            "codex.localAccess.accountPoolHealth.title",
                            "账号池",
                          )}
                        </span>
                        <span className="codex-local-access-quota-pool-value">
                          {accountPoolHealthSummary.available ===
                            accountPoolHealthSummary.total &&
                          accountPoolHealthSummary.abnormal === 0 &&
                          accountPoolHealthSummary.cooldown === 0
                            ? t(
                                "codex.localAccess.accountPoolHealth.allAvailable",
                                {
                                  count: accountPoolHealthSummary.total,
                                  defaultValue: "全部可用 {{count}}",
                                },
                              )
                            : t(
                                "codex.localAccess.accountPoolHealth.availableRatio",
                                {
                                  available: accountPoolHealthSummary.available,
                                  total: accountPoolHealthSummary.total,
                                  defaultValue: "可用 {{available}}/{{total}}",
                                },
                              )}
                        </span>
                        {(accountPoolHealthSummary.abnormal > 0 ||
                          accountPoolHealthSummary.cooldown > 0) && (
                          <span className="codex-local-access-quota-pool-value codex-local-access-health-issue">
                            {t(
                              "codex.localAccess.accountPoolHealth.issueSummary",
                              {
                                abnormal: accountPoolHealthSummary.abnormal,
                                cooldown: accountPoolHealthSummary.cooldown,
                                defaultValue:
                                  "异常 {{abnormal}} · 冷却 {{cooldown}}",
                              },
                            )}
                          </span>
                        )}
                      </button>
                    )}
                    {currentQuotaPoolSummary.visiblePlans.map((item) => (
                      <div
                        key={item.key}
                        className="codex-local-access-quota-pool-card"
                      >
                        <span className="codex-local-access-quota-pool-plan">
                          {item.key} ({item.count})
                        </span>
                        {item.windows.map((window) => (
                          <span
                            key={window.key}
                            className="codex-local-access-quota-pool-value"
                          >
                            {formatCodexQuotaPoolWindowLabel(
                              window.label,
                              quotaPoolLabels.weekly,
                            )}{" "}
                            {formatCodexQuotaPoolPercent(window.percentage)}
                          </span>
                        ))}
                      </div>
                    ))}
                  </div>
                )}
              </section>
            )}

            {!isMembersMode && (
              <div className="codex-local-access-panel-grid">
                <section className="codex-local-access-section codex-local-access-section-surface codex-local-access-config-section">
                  <div className="codex-local-access-section-title">
                    <KeyRound size={16} />
                    <span>
                      {t("codex.localAccess.configTitle", "服务配置")}
                    </span>
                  </div>
                  {collection ? (
                    <div className="codex-local-access-config-grid">
                      <div className="codex-local-access-config-card codex-local-access-config-card-base">
                        <div className="codex-local-access-config-head">
                          <div className="codex-local-access-config-label codex-local-access-address-select">
                            <SingleSelectDropdown
                              value={addressKind}
                              options={addressOptions}
                              onChange={onAddressKindChange}
                              menuClassName="codex-local-access-address-menu"
                              menuWidth={92}
                              menuMaxHeight={120}
                              disabled={addressOptions.length < 2}
                              ariaLabel={t(
                                "codex.localAccess.addressKind",
                                "地址类型",
                              )}
                            />
                          </div>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() =>
                                void handleCopy("baseUrl", displayBaseUrl)
                              }
                              title={t("common.copy", "复制")}
                            >
                              {copiedField === "baseUrl" ? (
                                <Check size={14} />
                              ) : (
                                <Copy size={14} />
                              )}
                            </button>
                          </div>
                        </div>
                        <code
                          className="codex-local-access-code"
                          title={displayBaseUrl}
                        >
                          {displayBaseUrl}
                        </code>
                      </div>

                      <div className="codex-local-access-config-card codex-local-access-config-card-key">
                        <div className="codex-local-access-config-head">
                          <span className="codex-local-access-config-label">
                            {t("codex.localAccess.apiKey", "密钥")}
                          </span>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() => setKeyVisible((prev) => !prev)}
                              title={
                                keyVisible
                                  ? t("codex.localAccess.hideKey", "隐藏密钥")
                                  : t("codex.localAccess.showKey", "显示密钥")
                              }
                            >
                              {keyVisible ? (
                                <EyeOff size={14} />
                              ) : (
                                <Eye size={14} />
                              )}
                            </button>
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() =>
                                void handleCopy("apiKey", collection.apiKey)
                              }
                              title={t("common.copy", "复制")}
                            >
                              {copiedField === "apiKey" ? (
                                <Check size={14} />
                              ) : (
                                <Copy size={14} />
                              )}
                            </button>
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              onClick={() => void handleResetKey()}
                              disabled={saving || testing || starting}
                            >
                              {saving ? (
                                <RefreshCw
                                  size={14}
                                  className="loading-spinner"
                                />
                              ) : (
                                <RefreshCw size={14} />
                              )}
                              {t("codex.localAccess.rotateKey", "重置密钥")}
                            </button>
                          </div>
                        </div>
                        <code
                          className="codex-local-access-code"
                          title={collection.apiKey}
                        >
                          {keyVisible
                            ? collection.apiKey
                            : `${collection.apiKey.slice(0, 10)}••••••••••••`}
                        </code>
                      </div>

                      <div className="codex-local-access-config-card codex-local-access-config-card-port codex-local-access-port-card">
                        <div className="codex-local-access-config-head">
                          <label
                            className="codex-local-access-config-label"
                            htmlFor="codex-local-access-port"
                          >
                            {t("codex.localAccess.portLabel", "服务端口")}
                          </label>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              onClick={() => void handleSavePort()}
                              disabled={saving || testing || starting}
                            >
                              {saving ? (
                                <RefreshCw
                                  size={14}
                                  className="loading-spinner"
                                />
                              ) : (
                                <Gauge size={14} />
                              )}
                              {t("codex.localAccess.portSave", "保存端口")}
                            </button>
                          </div>
                        </div>
                        <div className="codex-local-access-port-row">
                          <input
                            id="codex-local-access-port"
                            type="number"
                            min={1}
                            max={65535}
                            value={portInput}
                            onChange={(event) =>
                              setPortInput(event.target.value)
                            }
                            disabled={saving || testing || starting}
                          />
                        </div>
                      </div>

                      <div className="codex-local-access-config-card codex-local-access-config-card-scope">
                        <div className="codex-local-access-config-head">
                          <span className="codex-local-access-config-label">
                            {t(
                              "codex.localAccess.accessScopeLabel",
                              "访问范围",
                            )}
                          </span>
                          <div className="codex-local-access-config-actions">
                            <SingleSelectDropdown
                              value={accessScope}
                              options={accessScopeOptions}
                              onChange={(value) =>
                                void handleChangeAccessScope(value)
                              }
                              menuClassName="codex-local-access-scope-menu"
                              menuWidth={132}
                              menuMaxHeight={120}
                              disabled={saving || testing || starting}
                              ariaLabel={t(
                                "codex.localAccess.accessScopeLabel",
                                "访问范围",
                              )}
                            />
                          </div>
                        </div>
                        <code
                          className="codex-local-access-code"
                          title={accessScopeAddress}
                        >
                          {accessScopeAddress}
                        </code>
                      </div>
                    </div>
                  ) : (
                    <div className="group-account-empty">
                      {t(
                        "codex.localAccess.configEmpty",
                        "先把账号保存到 API 服务集合，随后会自动生成地址、密钥和端口。",
                      )}
                    </div>
                  )}
                  {collection || modelIdOptions.length > 0 ? (
                    <div className="codex-local-access-config-extra-grid">
                      {collection ? (
                        <div className="codex-local-access-config-card codex-local-access-config-card-root">
                          <div className="codex-local-access-config-head">
                            <span className="codex-local-access-config-label">
                              {t("codex.localAccess.apiPortUrl", "API端口URL")}
                            </span>
                            <div className="codex-local-access-config-actions">
                              <button
                                type="button"
                                className="folder-icon-btn"
                                onClick={() =>
                                  void handleCopy("apiPortUrl", apiPortUrl)
                                }
                                title={t("common.copy", "复制")}
                              >
                                {copiedField === "apiPortUrl" ? (
                                  <Check size={14} />
                                ) : (
                                  <Copy size={14} />
                                )}
                              </button>
                            </div>
                          </div>
                          <code
                            className="codex-local-access-code"
                            title={apiPortUrl}
                          >
                            {apiPortUrl}
                          </code>
                        </div>
                      ) : null}

                      {modelIdOptions.length > 0 ? (
                        <div className="codex-local-access-config-card codex-local-access-config-card-model">
                          <div className="codex-local-access-config-head">
                            <span className="codex-local-access-config-label">
                              {t("codex.localAccess.modelId", "模型 ID")}
                            </span>
                            <span className="codex-local-access-view-only-badge">
                              {t(
                                "codex.localAccess.modelIdViewOnly",
                                "仅查看使用，无切换功能",
                              )}
                            </span>
                            <div className="codex-local-access-config-actions">
                              <button
                                type="button"
                                className="folder-icon-btn"
                                onClick={() =>
                                  void handleCopy("modelId", selectedModelId)
                                }
                                title={t("common.copy", "复制")}
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
                          <div className="codex-local-access-model-row">
                            <SingleSelectDropdown
                              value={selectedModelId}
                              options={modelIdOptions}
                              onChange={setSelectedModelId}
                              disabled={modelIdOptions.length === 0}
                              ariaLabel={t(
                                "codex.localAccess.modelId",
                                "模型 ID",
                              )}
                              placeholder={t(
                                "codex.localAccess.modelIdPlaceholder",
                                "选择模型 ID",
                              )}
                              menuPlacement="up"
                              menuMaxHeight={240}
                            />
                          </div>
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                </section>

                <section className="codex-local-access-section codex-local-access-section-surface codex-local-access-account-stats-section">
                  <div className="codex-local-access-section-title">
                    <Server size={16} />
                    <span>
                      {t("codex.localAccess.accountStatsTitle", "按账号统计")}
                    </span>
                  </div>
                  <div className="codex-local-access-account-stats">
                    {currentMemberStats.length === 0 ? (
                      <div className="group-account-empty">
                        {t(
                          "codex.localAccess.statsEmpty",
                          "当前还没有统计数据",
                        )}
                      </div>
                    ) : (
                      currentMemberStats.map(
                        ({ account, presentation, stats: accountStats }) => (
                          <div
                            key={account.id}
                            className="codex-local-access-account-stat-row"
                          >
                            <div className="codex-local-access-account-stat-top">
                              <div className="codex-local-access-account-stat-main">
                                <span
                                  className="group-account-email"
                                  title={maskAccountText(
                                    presentation.displayName,
                                  )}
                                >
                                  {maskAccountText(presentation.displayName)}
                                </span>
                                <span
                                  className={`tier-badge ${presentation.planClass}`}
                                >
                                  {presentation.planLabel}
                                </span>
                              </div>
                              <div className="codex-local-access-account-stat-block codex-local-access-account-stat-block-quota">
                                {renderQuotaPreview(presentation, 3)}
                              </div>
                              <div className="codex-local-access-account-stat-block codex-local-access-account-stat-block-metrics">
                                <div className="codex-local-access-account-stat-metrics">
                                  <span className="codex-local-access-account-stat-pill">
                                    {formatRequestResultDetail(accountStats)}
                                  </span>
                                  <span className="codex-local-access-account-stat-pill">
                                    {(accountStats?.totalTokens ?? 0) === 0
                                      ? t(
                                          "codex.localAccess.stats.accountTokens",
                                          {
                                            count: 0,
                                            defaultValue: "0 Tokens",
                                          },
                                        )
                                      : t(
                                          "codex.localAccess.stats.accountTokensCompact",
                                          {
                                            value: formatCompactNumber(
                                              accountStats?.totalTokens ?? 0,
                                            ),
                                            defaultValue: "{{value}}",
                                          },
                                        )}
                                  </span>
                                </div>
                              </div>
                            </div>
                          </div>
                        ),
                      )
                    )}
                  </div>
                </section>
              </div>
            )}

            {isMembersMode && (
              <section className="codex-local-access-section codex-local-access-section-surface codex-local-access-member-section">
                <div className="codex-local-access-section-head">
                  <div className="codex-local-access-section-title">
                    <FolderPlus size={16} />
                    <span>
                      {t("codex.localAccess.memberTitle", "集合成员")}
                    </span>
                  </div>
                  <div className="codex-local-access-member-head-actions">
                    <label className="codex-local-access-free-toggle">
                      <input
                        type="checkbox"
                        checked={restrictFreeAccounts}
                        onChange={() => void handleToggleRestrictFreeAccounts()}
                        disabled={membersInteractionDisabled}
                      />
                      <span>
                        {t(
                          "codex.localAccess.modal.restrictFreeToggle",
                          "限制 Free 账号使用",
                        )}
                      </span>
                    </label>
                    <div className="codex-local-access-session-affinity-settings">
                      <div className="codex-local-access-session-affinity-row">
                        <label className="codex-local-access-session-affinity-toggle">
                          <input
                            type="checkbox"
                            checked={sessionAffinity}
                            onChange={() => void handleToggleSessionAffinity()}
                            disabled={membersInteractionDisabled}
                          />
                          <span>
                            {t(
                              "codex.apiService.routing.sessionAffinity",
                              "会话亲和",
                            )}
                          </span>
                        </label>
                        <label className="codex-local-access-session-affinity-expiry">
                          <span>
                            {t(
                              "codex.apiService.routing.sessionAffinityTtl",
                              "过期时间（秒）",
                            )}
                          </span>
                          <input
                            ref={sessionAffinityTtlInputRef}
                            type="number"
                            min={60}
                            max={86400}
                            value={sessionAffinityTtlSeconds}
                            aria-invalid={Boolean(sessionAffinityTtlError)}
                            onChange={(event) => {
                              setMembersDraftDirty(true);
                              setSessionAffinityTtlError("");
                              setSessionAffinityTtlSeconds(event.target.value);
                            }}
                            disabled={membersInteractionDisabled}
                          />
                        </label>
                      </div>
                      {sessionAffinityTtlError && (
                        <small className="codex-local-access-session-affinity-error">
                          {sessionAffinityTtlError}
                        </small>
                      )}
                    </div>
                    {collection && (
                      <>
                        <div className="codex-local-access-member-routing">
                          <SingleSelectDropdown
                            value={routingStrategy}
                            options={routingStrategyOptions}
                            onChange={(value) =>
                              void handleChangeRoutingStrategy(value)
                            }
                            disabled={saving || testing || starting}
                            ariaLabel={t(
                              "codex.localAccess.routingLabel",
                              "调度策略",
                            )}
                          />
                        </div>
                        {routingStrategy === "custom" && (
                          <button
                            type="button"
                            className="folder-icon-btn codex-local-access-toolbar-btn"
                            onClick={openCustomRoutingDialog}
                            disabled={saving || testing || starting}
                            title={t(
                              "codex.localAccess.customRoutingEdit",
                              "编辑自定义调度",
                            )}
                            aria-label={t(
                              "codex.localAccess.customRoutingEdit",
                              "编辑自定义调度",
                            )}
                          >
                            <SlidersHorizontal size={14} />
                          </button>
                        )}
                      </>
                    )}
                  </div>
                </div>

                <div className="group-account-toolbar">
                  <div className="group-account-search">
                    <Search size={16} className="group-account-search-icon" />
                    <input
                      ref={searchInputRef}
                      type="text"
                      value={memberSearchQuery}
                      onChange={(event) => {
                        if (memberView) {
                          memberView.onSearchQueryChange(event.target.value);
                          return;
                        }
                        setQuery(event.target.value);
                      }}
                      placeholder={t("accounts.search")}
                    />
                  </div>
                  <div className="group-account-picker-filters">
                    <MultiSelectFilterDropdown
                      options={memberTierFilterOptions}
                      selectedValues={memberFilterTypes}
                      allLabel={memberTierFilterAllLabel}
                      filterLabel={t("common.shared.filterLabel", "筛选")}
                      clearLabel={t("accounts.clearFilter", "清空筛选")}
                      emptyLabel={t("common.none", "暂无")}
                      ariaLabel={t("common.shared.filterLabel", "筛选")}
                      onToggleValue={(value) => {
                        if (memberView) {
                          memberView.onToggleFilterType(value);
                          return;
                        }
                        setFilterTypes((prev) =>
                          prev.includes(value)
                            ? prev.filter((item) => item !== value)
                            : [...prev, value],
                        );
                      }}
                      onClear={() => {
                        if (memberView) {
                          memberView.onClearFilterTypes();
                          return;
                        }
                        setFilterTypes([]);
                      }}
                    />
                    <AccountTagFilterDropdown
                      availableTags={memberAvailableTags}
                      selectedTags={memberTagFilter}
                      onToggleTag={(value) => {
                        if (memberView) {
                          memberView.onToggleTagFilter(value);
                          return;
                        }
                        setTagFilter((prev) =>
                          prev.includes(value)
                            ? prev.filter((item) => item !== value)
                            : [...prev, value],
                        );
                      }}
                      onClear={() => {
                        if (memberView) {
                          memberView.onClearTagFilter();
                          return;
                        }
                        setTagFilter([]);
                      }}
                    />
                    <MultiSelectFilterDropdown
                      options={memberGroupFilterOptions}
                      selectedValues={memberGroupFilter}
                      allLabel={t("accounts.groups.allGroups", "全部分组")}
                      filterLabel={t("accounts.groups.manageTitle", "分组管理")}
                      clearLabel={t("accounts.clearFilter", "清空筛选")}
                      emptyLabel={t("common.none", "暂无")}
                      ariaLabel={t("accounts.groups.manageTitle", "分组管理")}
                      onToggleValue={(value) => {
                        if (memberView) {
                          memberView.onToggleGroupFilter(value);
                          return;
                        }
                        setGroupFilter((prev) =>
                          prev.includes(value)
                            ? prev.filter((item) => item !== value)
                            : [...prev, value],
                        );
                      }}
                      onClear={() => {
                        if (memberView) {
                          memberView.onClearGroupFilter();
                          return;
                        }
                        setGroupFilter([]);
                      }}
                    />
                  </div>
                </div>

                <div className="group-account-item group-account-item-header">
                  <input
                    ref={selectAllCheckboxRef}
                    type="checkbox"
                    checked={allVisibleSelected}
                    onChange={toggleSelectAllVisible}
                    disabled={
                      membersInteractionDisabled ||
                      visibleEnabledAccounts.length === 0
                    }
                  />
                  <div className="group-account-main" />
                </div>

                <div className="group-account-list codex-local-access-member-list">
                  {!accountsLoaded ? (
                    <div className="group-account-empty">
                      {t("common.loading", "加载中...")}
                    </div>
                  ) : localAccessAccounts.length === 0 ? (
                    <div className="group-account-empty">
                      {t(
                        "codex.localAccess.modal.empty",
                        "暂无可加入的 Codex 账号",
                      )}
                    </div>
                  ) : visibleSelectableAccounts.length === 0 ? (
                    <div className="group-account-empty">
                      {t("common.shared.noMatch.title", "没有匹配的账号")}
                    </div>
                  ) : (
                    paginatedVisibleSelectableAccounts.map((account) => {
                      const presentation = buildCodexAccountPresentation(
                        account,
                        t,
                      );
                      const ineligibleReason =
                        getCodexLocalAccessAccountIneligibleReason(
                          account,
                          restrictFreeAccounts,
                        );
                      const isChatCompletionsApiKeyUnsupported =
                        ineligibleReason === "chat_completions_api_key";
                      const isPendingOauthUnsupported =
                        ineligibleReason === "pending_oauth";
                      const isJoinUnsupported =
                        isChatCompletionsApiKeyUnsupported ||
                        isPendingOauthUnsupported;
                      const isChecked =
                        !isJoinUnsupported && selected.has(account.id);
                      const isBackup =
                        customRoutingDraft[account.id]?.isBackup ??
                        customRoutingRuleByAccountId.get(account.id)
                          ?.isBackup ??
                        false;
                      const accountStats = allStatsByAccountId.get(
                        account.id,
                      )?.usage;

                      return (
                        <div
                          key={account.id}
                          className={`group-account-item${isChecked ? " is-current" : ""}${isJoinUnsupported ? " is-disabled" : ""}`}
                        >
                          <input
                            type="checkbox"
                            className="codex-local-access-member-select"
                            checked={isChecked}
                            disabled={
                              membersInteractionDisabled || isJoinUnsupported
                            }
                            onChange={() => toggleSelect(account.id)}
                            aria-label={maskAccountText(
                              presentation.displayName,
                            )}
                          />
                          <div className="group-account-main">
                            <div className="codex-local-access-member-mainline">
                              <button
                                type="button"
                                className="group-account-email"
                                title={maskAccountText(
                                  presentation.displayName,
                                )}
                                disabled={
                                  membersInteractionDisabled || isJoinUnsupported
                                }
                                onClick={() => toggleSelect(account.id)}
                              >
                                {maskAccountText(presentation.displayName)}
                              </button>
                              <span className="codex-local-access-member-backup-slot">
                                {!isJoinUnsupported ? (
                                  <button
                                    type="button"
                                    className={`codex-local-access-member-backup-field${
                                      isBackup ? " is-on" : ""
                                    }`}
                                    title={t(
                                      "codex.localAccess.customRoutingBackupDesc",
                                      "仅在所有普通账号不可用时使用；普通账号恢复后，新请求会自动切回。",
                                    )}
                                    aria-pressed={isBackup}
                                    aria-label={t(
                                      "codex.localAccess.customRoutingBackupTitle",
                                      "备用账号",
                                    )}
                                    disabled={membersInteractionDisabled}
                                    onClick={(event) => {
                                      event.preventDefault();
                                      event.stopPropagation();
                                      toggleMemberBackup(account.id);
                                    }}
                                  >
                                    <span className="codex-local-access-member-backup-label">
                                      {t(
                                        "codex.localAccess.customRoutingBackupShort",
                                        "备用",
                                      )}
                                    </span>
                                    <span
                                      className="codex-local-access-member-backup-switch"
                                      aria-hidden="true"
                                    >
                                      <span className="codex-local-access-member-backup-switch-track" />
                                    </span>
                                  </button>
                                ) : null}
                              </span>
                              <span className="codex-local-access-member-plan">
                                <span
                                  className={`tier-badge ${presentation.planClass}`}
                                >
                                  {presentation.planLabel}
                                </span>
                              </span>
                              <span className="codex-local-access-member-metric">
                                {t("codex.localAccess.stats.accountRequests", {
                                  count: accountStats?.requestCount ?? 0,
                                  defaultValue: "{{count}} 次请求",
                                })}
                              </span>
                              <span className="codex-local-access-member-trailing">
                                {isChatCompletionsApiKeyUnsupported && (
                                  <span className="codex-local-access-member-unsupported">
                                    {t(
                                      "codex.localAccess.modal.chatApiKeyUnsupported",
                                      "Chat Completions 协议不支持加入 API 服务",
                                    )}
                                  </span>
                                )}
                                {isPendingOauthUnsupported && (
                                  <span className="codex-local-access-member-unsupported">
                                    {t(
                                      "codex.localAccess.modal.pendingOauthUnsupported",
                                      "待授权账号不可加入 API 服务",
                                    )}
                                  </span>
                                )}
                                {renderQuotaPreview(presentation, 2)}
                              </span>
                            </div>
                          </div>
                        </div>
                      );
                    })
                  )}
                </div>
                {visibleSelectableAccounts.length > 0 && (
                  <PaginationControls
                    totalItems={memberPagination.totalItems}
                    currentPage={memberPagination.currentPage}
                    totalPages={memberPagination.totalPages}
                    pageSize={memberPagination.pageSize}
                    pageSizeOptions={memberPagination.pageSizeOptions}
                    rangeStart={memberPagination.rangeStart}
                    rangeEnd={memberPagination.rangeEnd}
                    canGoPrevious={memberPagination.canGoPrevious}
                    canGoNext={memberPagination.canGoNext}
                    onPageSizeChange={memberPagination.setPageSize}
                    onPreviousPage={memberPagination.goToPreviousPage}
                    onNextPage={memberPagination.goToNextPage}
                  />
                )}
              </section>
            )}
          </div>

          <div className="modal-footer group-account-picker-footer codex-local-access-modal-footer">
            {isMembersMode ? (
              <>
                <button
                  className="btn btn-secondary"
                  onClick={onClose}
                >
                  {t("common.cancel")}
                </button>
                <button
                  className="btn btn-primary"
                  onClick={() => void handleSaveMembers()}
                  disabled={membersInteractionDisabled || !selectionDirty}
                >
                  {saving
                    ? t("common.saving")
                    : t("codex.localAccess.modal.save", "保存集合")}
                </button>
              </>
            ) : (
              <button
                className="btn btn-secondary"
                onClick={onClose}
              >
                {t("common.close")}
              </button>
            )}
          </div>
        </div>
      </div>

      {customRoutingOpen && collection && (
        <div className="modal-overlay codex-local-access-custom-routing-overlay">
          <div
            className="modal codex-local-access-custom-routing-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-local-access-custom-routing-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header codex-local-access-custom-routing-header">
              <div>
                <h3 id="codex-local-access-custom-routing-title">
                  <SlidersHorizontal size={18} />
                  <span>
                    {t("codex.localAccess.customRoutingTitle", "自定义调度")}
                  </span>
                </h3>
                <p>
                  {t(
                    "codex.localAccess.customRoutingDesc",
                    "设置账号的优先级和权重，控制账号选择顺序与负载分配。",
                  )}
                </p>
              </div>
              <button
                className="modal-close codex-local-access-custom-routing-close"
                onClick={closeCustomRoutingDialog}
                disabled={saving}
                aria-label={t("common.close")}
              >
                <X size={18} />
              </button>
            </div>

            <div className="modal-body codex-local-access-custom-routing-body">
              {customRoutingError && (
                <div
                  className="codex-local-access-inline-error"
                  aria-live="assertive"
                >
                  <CircleAlert size={14} />
                  <span>{customRoutingError}</span>
                </div>
              )}

              <div className="codex-local-access-custom-routing-guide">
                <div className="codex-local-access-custom-routing-guide-card">
                  <strong>
                    {t(
                      "codex.localAccess.customRoutingPriorityTitle",
                      "优先级",
                    )}
                  </strong>
                  <span>
                    {t(
                      "codex.localAccess.customRoutingPriorityDesc",
                      "数值越高越先被选中；高优先级账号不可用时，才会继续尝试低优先级账号。",
                    )}
                  </span>
                </div>
                <div className="codex-local-access-custom-routing-guide-card">
                  <strong>
                    {t("codex.localAccess.customRoutingWeightTitle", "权重")}
                  </strong>
                  <span>
                    {t(
                      "codex.localAccess.customRoutingWeightDesc",
                      "相同优先级内用于负载均衡；权重越高，分到的请求越多。",
                    )}
                  </span>
                </div>
              </div>

              <div className="codex-local-access-custom-routing-toolbar">
                <div className="group-account-search codex-local-access-custom-routing-search">
                  <Search size={16} className="group-account-search-icon" />
                  <input
                    type="text"
                    value={customRoutingQuery}
                    onChange={(event) =>
                      setCustomRoutingQuery(event.target.value)
                    }
                    placeholder={t(
                      "codex.localAccess.customRoutingSearch",
                      "搜索邮箱、订阅或账号 ID",
                    )}
                  />
                </div>
                <div className="group-account-picker-filters codex-local-access-custom-routing-filters">
                  <MultiSelectFilterDropdown
                    options={customRoutingTierFilterOptions}
                    selectedValues={customRoutingFilterTypes}
                    allLabel={customRoutingAllTierFilterLabel}
                    filterLabel={t("common.shared.filterLabel", "筛选")}
                    clearLabel={t("accounts.clearFilter", "清空筛选")}
                    emptyLabel={t("common.none", "暂无")}
                    ariaLabel={t("common.shared.filterLabel", "筛选")}
                    onToggleValue={(value) =>
                      setCustomRoutingFilterTypes((prev) =>
                        prev.includes(value)
                          ? prev.filter((item) => item !== value)
                          : [...prev, value],
                      )
                    }
                    onClear={() => setCustomRoutingFilterTypes([])}
                  />
                  <AccountTagFilterDropdown
                    availableTags={customRoutingAvailableTags}
                    selectedTags={customRoutingTagFilter}
                    onToggleTag={(value) =>
                      setCustomRoutingTagFilter((prev) =>
                        prev.includes(value)
                          ? prev.filter((item) => item !== value)
                          : [...prev, value],
                      )
                    }
                    onClear={() => setCustomRoutingTagFilter([])}
                  />
                </div>
                <div className="codex-local-access-custom-routing-bulk">
                  <span className="codex-local-access-custom-routing-selected-count">
                    {t("codex.localAccess.customRoutingSelected", {
                      count: customRoutingSelected.size,
                      defaultValue: "已选 {{count}}",
                    })}
                  </span>
                  <label>
                    <span>
                      {t(
                        "codex.localAccess.customRoutingPriorityShort",
                        "优先级",
                      )}
                    </span>
                    <input
                      type="number"
                      min={CUSTOM_ROUTING_PRIORITY_MIN}
                      max={CUSTOM_ROUTING_PRIORITY_MAX}
                      value={customRoutingBulkPriority}
                      onChange={(event) =>
                        setCustomRoutingBulkPriority(event.target.value)
                      }
                      disabled={saving}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.localAccess.customRoutingWeightShort", "权重")}
                    </span>
                    <input
                      type="number"
                      min={CUSTOM_ROUTING_WEIGHT_MIN}
                      max={CUSTOM_ROUTING_WEIGHT_MAX}
                      value={customRoutingBulkWeight}
                      onChange={(event) =>
                        setCustomRoutingBulkWeight(event.target.value)
                      }
                      disabled={saving}
                    />
                  </label>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={applyCustomRoutingBatch}
                    disabled={saving || customRoutingSelected.size === 0}
                  >
                    {t(
                      "codex.localAccess.customRoutingApplyBatch",
                      "应用到已选",
                    )}
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={resetCustomRoutingDraft}
                    disabled={saving || customRoutingAccounts.length === 0}
                  >
                    {t("codex.localAccess.customRoutingReset", "重置")}
                  </button>
                </div>
              </div>

              <div className="codex-local-access-custom-routing-list-shell">
                <div className="codex-local-access-custom-routing-row codex-local-access-custom-routing-row-head">
                  <input
                    ref={customRoutingSelectAllRef}
                    type="checkbox"
                    checked={allVisibleCustomRoutingSelected}
                    onChange={toggleCustomRoutingSelectAllVisible}
                    disabled={
                      saving || visibleCustomRoutingAccounts.length === 0
                    }
                    aria-label={t("common.selectAll", "全选")}
                  />
                  <span>
                    {t("codex.localAccess.customRoutingAccountColumn", "账号")}
                  </span>
                  <span>
                    {t("codex.localAccess.customRoutingQuotaColumn", "额度")}
                  </span>
                  <span>
                    {t(
                      "codex.localAccess.customRoutingPriorityShort",
                      "优先级",
                    )}
                  </span>
                  <span>
                    {t("codex.localAccess.customRoutingWeightShort", "权重")}
                  </span>
                </div>

                <div className="codex-local-access-custom-routing-list">
                  {customRoutingAccounts.length === 0 ? (
                    <div className="group-account-empty">
                      {t(
                        "codex.localAccess.customRoutingEmpty",
                        "当前 API 服务集合没有可配置的账号",
                      )}
                    </div>
                  ) : visibleCustomRoutingAccounts.length === 0 ? (
                    <div className="group-account-empty">
                      {t("common.shared.noMatch.title", "没有匹配的账号")}
                    </div>
                  ) : (
                    visibleCustomRoutingAccounts.map((account) => {
                      const presentation = buildCodexAccountPresentation(
                        account,
                        t,
                      );
                      const draftRule = customRoutingDraft[account.id] ?? {
                        priority: CUSTOM_ROUTING_PRIORITY_MIN,
                        weight: CUSTOM_ROUTING_WEIGHT_MIN,
                        isBackup: false,
                      };
                      const checked = customRoutingSelected.has(account.id);

                      return (
                        <div
                          key={account.id}
                          className={`codex-local-access-custom-routing-row${
                            checked ? " is-selected" : ""
                          }`}
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() =>
                              toggleCustomRoutingSelect(account.id)
                            }
                            disabled={saving}
                          />
                          <div className="codex-local-access-custom-routing-account">
                            <span
                              className="group-account-email"
                              title={maskAccountText(presentation.displayName)}
                            >
                              {maskAccountText(presentation.displayName)}
                            </span>
                            <span
                              className={`tier-badge ${presentation.planClass}`}
                            >
                              {presentation.planLabel}
                            </span>
                            {draftRule.isBackup && (
                              <span
                                className="codex-local-access-custom-routing-backup-badge"
                                title={t(
                                  "codex.localAccess.customRoutingBackupDesc",
                                  "仅在所有普通账号不可用时使用；普通账号恢复后，新请求会自动切回。",
                                )}
                              >
                                {t(
                                  "codex.localAccess.customRoutingBackupShort",
                                  "备用",
                                )}
                              </span>
                            )}
                          </div>
                          <div className="codex-local-access-custom-routing-quota">
                            {renderQuotaPreview(presentation, 2) ?? (
                              <span className="codex-local-access-custom-routing-muted">
                                {t("common.none", "暂无")}
                              </span>
                            )}
                          </div>
                          <label className="codex-local-access-custom-routing-number-field is-priority">
                            <span>
                              {t(
                                "codex.localAccess.customRoutingPriorityShort",
                                "优先级",
                              )}
                            </span>
                            <input
                              className="codex-local-access-custom-routing-number"
                              type="number"
                              min={CUSTOM_ROUTING_PRIORITY_MIN}
                              max={CUSTOM_ROUTING_PRIORITY_MAX}
                              value={draftRule.priority}
                              onChange={(event) =>
                                updateCustomRoutingRule(
                                  account.id,
                                  "priority",
                                  event.target.value,
                                )
                              }
                              disabled={saving}
                              aria-label={t(
                                "codex.localAccess.customRoutingPriorityShort",
                                "优先级",
                              )}
                            />
                          </label>
                          <label className="codex-local-access-custom-routing-number-field is-weight">
                            <span>
                              {t(
                                "codex.localAccess.customRoutingWeightShort",
                                "权重",
                              )}
                            </span>
                            <input
                              className="codex-local-access-custom-routing-number"
                              type="number"
                              min={CUSTOM_ROUTING_WEIGHT_MIN}
                              max={CUSTOM_ROUTING_WEIGHT_MAX}
                              value={draftRule.weight}
                              onChange={(event) =>
                                updateCustomRoutingRule(
                                  account.id,
                                  "weight",
                                  event.target.value,
                                )
                              }
                              disabled={saving}
                              aria-label={t(
                                "codex.localAccess.customRoutingWeightShort",
                                "权重",
                              )}
                            />
                          </label>
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            </div>

            <div className="modal-footer codex-local-access-custom-routing-footer">
              <button
                className="btn btn-secondary"
                onClick={closeCustomRoutingDialog}
                disabled={saving}
              >
                {t("common.cancel")}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleSaveCustomRouting()}
                disabled={saving || customRoutingAccounts.length === 0}
              >
                {saving
                  ? t("common.saving")
                  : t("codex.localAccess.customRoutingSave", "保存自定义调度")}
              </button>
            </div>
          </div>
        </div>
      )}

      {backupConfirmAccountId && (
        <div className="modal-overlay codex-local-access-backup-confirm-overlay">
          <div
            className="modal codex-local-access-backup-confirm-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-local-access-backup-confirm-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header codex-local-access-backup-confirm-header">
              <h3 id="codex-local-access-backup-confirm-title">
                {t("codex.localAccess.customRoutingBackupTitle", "备用账号")}
              </h3>
              <button
                type="button"
                className="modal-close"
                onClick={() => setBackupConfirmAccountId(null)}
                disabled={actionBusy}
                aria-label={t("common.close")}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body codex-local-access-backup-confirm-body">
              <strong>
                {maskAccountText(
                  localAccessAccountById.get(backupConfirmAccountId)
                    ?.account_name ||
                    localAccessAccountById.get(backupConfirmAccountId)?.email ||
                    backupConfirmAccountId,
                )}
              </strong>
              <p>
                {t(
                  "codex.localAccess.customRoutingBackupDesc",
                  "仅在所有普通账号不可用时使用；普通账号恢复后，新请求会自动切回。",
                )}
              </p>
            </div>
            <div className="modal-footer codex-local-access-backup-confirm-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setBackupConfirmAccountId(null)}
                disabled={actionBusy}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={confirmMemberBackup}
                disabled={actionBusy}
              >
                {t("common.confirm")}
              </button>
            </div>
          </div>
        </div>
      )}

      {testDialogOpen && (
        <div className="modal-overlay codex-local-access-test-dialog-overlay">
          <div
            className="modal codex-local-access-test-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-local-access-test-dialog-title"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header codex-local-access-test-dialog-header">
              <div>
                <h3 id="codex-local-access-test-dialog-title">
                  <ShieldCheck size={18} />
                  <span>
                    {t("codex.localAccess.testDialogTitle", "测试 API 服务")}
                  </span>
                </h3>
                <p>
                  {t(
                    "codex.localAccess.testDialogDesc",
                    "通过本地 API 服务发起一次真实对话，验证本地服务、密钥和上游响应。",
                  )}
                </p>
              </div>
              <button
                className="modal-close codex-local-access-test-dialog-close"
                onClick={closeTestDialog}
                disabled={testDialogBusy}
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
                    options={modelIdOptions}
                    onChange={setSelectedModelId}
                    disabled={modelIdOptions.length === 0 || testDialogBusy}
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
                  disabled={testDialogBusy || testChatMessages.length === 0}
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
                disabled={testDialogBusy}
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
                  testDialogBusy || !testChatInput.trim() || !selectedModelId
                }
              >
                <Send size={15} />
                {t("codex.localAccess.testChatSend", "发送")}
              </button>
              <button
                className="btn btn-secondary"
                onClick={closeTestDialog}
                disabled={testDialogBusy}
              >
                {t("common.close")}
              </button>
            </div>
          </div>
        </div>
      )}
      <CodexAccountPoolHealthModal
        isOpen={healthModalOpen}
        accountIds={collection?.accountIds ?? []}
        accounts={accounts}
        accountHealth={state?.accountHealth ?? []}
        actionBusy={healthActionBusy}
        maskAccountText={(value) => maskAccountText(value)}
        onClose={() => setHealthModalOpen(false)}
        onRecover={(accountId) => onRecoverAccounts([accountId])}
        onRecoverAll={onRecoverAccounts}
      />
    </>
  );
}

export default CodexLocalAccessModal;
