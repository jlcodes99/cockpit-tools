import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import {
  isGrokApiKeyAccount,
  type GrokAccount,
} from "../types/grok";
import { listGrokAccounts } from "../services/grokService";
import type { CodexAccountGroup } from "../services/codexAccountGroupService";
import type {
  CodexLocalAccessAddressKind,
  CodexLocalAccessChatMessage,
  CodexLocalAccessChatStreamEvent,
  CodexLocalAccessCustomRoutingRule,
  CodexLocalAccessImageGenerationPolicy,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessScope,
  CodexLocalAccessState,
  CodexLocalAccessStatsWindow,
  CodexLocalAccessUsageStats,
} from "../types/codexLocalAccess";
import { getCodexPlanFilterKey, isCodexApiKeyAccount } from "../types/codex";
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
} from "../utils/codexQuotaPool";
import {
  getCodexLocalAccessAccountIneligibleReason,
  isCodexLocalAccessEligibleAccount,
  resolveCodexLocalAccessInitialAccountIds,
} from "../utils/codexLocalAccessAccounts";
import { isBlockingCodexAccountQuotaError } from "../utils/codexQuotaError";
import { AccountTagFilterDropdown } from "./AccountTagFilterDropdown";
import { CodexAccountPoolHealthModal } from "./CodexAccountPoolHealthModal";
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from "./MultiSelectFilterDropdown";
import { SingleSelectDropdown } from "./SingleSelectDropdown";
import { CodexImageModelSelect } from "./CodexImageModelConfig";
import { CodexImageForwardConfig } from "./CodexImageForwardConfig";
import { buildGrokMemberRowAccounts, CodexGrokBuildQuotaChip, selectCodexLocalAccessMemberRows } from "./codex/codexGrokMemberRows";
import { PaginationControls } from "./PaginationControls";
import { CodexStatsRangePicker } from "./CodexStatsRangePicker";
import { queryCodexLocalAccessStats } from "../services/codexLocalAccessService";
import {
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "../utils/codexStatsRange";
import {
  persistCodexStatsRangeSelection,
  readCodexStatsRangeSelection,
} from "../utils/codexStatsRangePreference";
import { useEscClose } from "../hooks/useEscClose";
import {
  buildPaginationPageSizeStorageKey,
  usePagination,
} from "../hooks/usePagination";
import {
  CUSTOM_ROUTING_PRIORITY_MIN,
  CUSTOM_ROUTING_PRIORITY_MAX,
  CUSTOM_ROUTING_WEIGHT_MIN,
  CUSTOM_ROUTING_WEIGHT_MAX,
  areSetsEqual,
  buildLocalAccessSummaryStats,
  buildLocalAccessRoutingStrategyOptions,
  createTestChatMessage,
  formatCompactNumber,
  formatLatencyMs,
  formatQuotaPoolLabel,
  formatRequestResultDetail as formatLocalAccessRequestResultDetail,
  normalizeAccessScope,
  normalizeCustomRoutingPriority,
  normalizeCustomRoutingWeight,
  resolveAccountUsagePriority,
  summarizeAccountPoolHealth,
  summarizeQuotaPoolsByPlan,
  type AccountUsagePriority,
  type CustomRoutingDraftRule,
  type TestChatMessage,
} from "./codex/codexLocalAccessModalModel";
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
  /**
   * 把 Grok 平台（已登录）账号加入 API 服务集合。
   *
   * 宿主负责创建/复用绑定的供应商账号并刷新账号列表，返回对应的 Codex 账号 ID；
   * 未提供该回调时成员弹框不展示 Grok 平台分组。
   */
  onAddGrokMember?: (grokAccountId: string) => Promise<CodexAccount | null>;
  memberView?: CodexLocalAccessMemberViewConfig;
  initialSelectedIds: string[];
  maskAccountText: (value?: string | null) => string;
  onClose: () => void;
  onOpenFullPage?: () => void;
  onSaveAccounts: (payload: {
    accountIds: string[];
    restrictFreeAccounts: boolean;
    backupAccountIds: string[];
    preferredAccountIds: string[];
    sessionAffinity: boolean;
    sessionAffinityTtlMs: number;
    imageGenerationAccountPolicies: Record<string, CodexLocalAccessImageGenerationPolicy>;
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
  onUpdateImageGenerationModel: (model: string) => Promise<unknown> | unknown;
  /** 保存 API 服务的生图转发账号池（生图请求交给这些 OAuth 账号）。 */
  onUpdateImageGenerationAccounts: (
    accountIds: string[],
  ) => Promise<unknown> | unknown;
  onRotateApiKey: () => Promise<unknown> | unknown;
  onRestartSidecar: () => Promise<unknown> | unknown;
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
  sidecarRestarting: boolean;
}

type CopyableField = "apiPortUrl" | "baseUrl" | "apiKey" | "modelId";

const CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY =
  "agtools.codex.local_access.stats_range.v1";
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
  onAddGrokMember,
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
  onUpdateImageGenerationModel,
  onUpdateImageGenerationAccounts,
  onRotateApiKey,
  onRestartSidecar,
  onKillPort,
  onToggleEnabled,
  onRecoverAccounts,
  healthActionBusy,
  onStreamTestMessage,
  saving,
  testing,
  starting,
  portCleanupBusy,
  sidecarRestarting,
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
  const [imageGenerationPolicies, setImageGenerationPolicies] = useState<Record<string, CodexLocalAccessImageGenerationPolicy>>({});
  const [membersDraftDirty, setMembersDraftDirty] = useState(false);
  // 跨平台成员：Grok 平台已登录账号（默认与 Codex 一起展示）。
  const [grokMemberAccounts, setGrokMemberAccounts] = useState<GrokAccount[]>([]);
  const [grokMemberLoading, setGrokMemberLoading] = useState(false);
  const [grokMemberError, setGrokMemberError] = useState("");
  const [grokMemberBusyId, setGrokMemberBusyId] = useState("");
  const [platformFilter, setPlatformFilter] = useState<string[]>([]);
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
  const [statsSelection, setStatsSelection] = useState(() =>
    readCodexStatsRangeSelection(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY),
  );
  const { key: statsRange, range: statsTimeRange } = statsSelection;
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
  useEscClose(isOpen, onClose);
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
      if (!stats || statsRange === "custom" || statsRange === "rolling7d") {
        return filteredStatsWindow;
      }
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
    selectedTotals && selectedTotals.successCount > 0
      ? selectedTotals.totalLatencyMs / selectedTotals.successCount
      : 0;
  const successRate =
    selectedTotals && selectedTotals.requestCount > 0
      ? Math.round(
          (selectedTotals.successCount / selectedTotals.requestCount) * 100,
        )
      : 0;
  const formatRequestResultDetail = (usage?: CodexLocalAccessUsageStats | null) =>
    formatLocalAccessRequestResultDetail(t, usage);
  const testDialogBusy = testDialogRunning || testing;
  const actionBusy = saving || testing || starting || portCleanupBusy;
  const membersInteractionDisabled =
    actionBusy || !accountsLoaded || Boolean(grokMemberBusyId);
  const summaryStats = useMemo(
    () => buildLocalAccessSummaryStats(t, selectedTotals, avgLatencyMs, successRate),
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
  const accountPoolHealthSummary = useMemo(
    () => summarizeAccountPoolHealth(localAccessAccounts, collection?.accountIds ?? [], state),
    [
      collection?.accountIds,
      localAccessAccounts,
      state?.accountHealth,
      state?.accountPoolHealth,
      state?.recoverySuppressedAccountIds,
    ],
  );
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
      setImageGenerationPolicies(collection?.imageGenerationAccountPolicies ?? {});
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
    if (shouldResetMembersDraft) {
      setCustomRoutingDraft(() => {
        const ruleMap = new Map(
          (collection?.customRoutingRules ?? []).map((rule) => [
            rule.accountId,
            {
              priority: normalizeCustomRoutingPriority(rule.priority),
              weight: normalizeCustomRoutingWeight(rule.weight),
              isBackup: Boolean(rule.isBackup),
              isPreferred: Boolean(rule.isPreferred),
            },
          ]),
        );
        const next: Record<string, CustomRoutingDraftRule> = {};
        (collection?.accountIds ?? []).forEach((accountId) => {
          next[accountId] = ruleMap.get(accountId) ?? {
            priority: CUSTOM_ROUTING_PRIORITY_MIN,
            weight: CUSTOM_ROUTING_WEIGHT_MIN,
            isBackup: false,
            isPreferred: false,
          };
        });
        return next;
      });
    }
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
    if (isOpen) {
      setStatsSelection(readCodexStatsRangeSelection(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY));
    }
  }, [isOpen]);

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
    const selection = { key, range };
    setStatsSelection(selection);
    persistCodexStatsRangeSelection(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY, selection);
  };

  const handleCustomStatsRangeApply = (range: CodexStatsTimeRange) => {
    const selection = { key: "custom" as const, range };
    setStatsSelection(selection);
    persistCodexStatsRangeSelection(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY, selection);
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
      if (!isBlockingCodexAccountQuotaError(account)) {
        counts.VALID += 1;
      }
      const tier = getCodexPlanFilterKey(account);
      incrementCodexPlanFilterCount(counts, tier);
      if (isBlockingCodexAccountQuotaError(account)) {
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

  // 平台筛选：默认全部平台（Codex + Grok），后续可继续追加平台。
  const platformFilterActive =
    platformFilter.length > 0 && platformFilter.length < 2;
  const showCodexPlatform =
    !platformFilterActive || platformFilter.includes("codex");
  const showGrokPlatform =
    !platformFilterActive || platformFilter.includes("grok");

  // Grok 平台账号：仅 OAuth(订阅) 账号可作为 API 服务上游，API Key 账号不在此处展示。
  const grokMemberCandidates = useMemo(
    () => grokMemberAccounts.filter((account) => !isGrokApiKeyAccount(account)),
    [grokMemberAccounts],
  );
  // grok account id -> 绑定它的 Codex 供应商账号 ID（已加入集合时用于勾选反查）。
  const grokMemberBindingByAccountId = useMemo(() => {
    const map = new Map<string, string>();
    accounts.forEach((account) => {
      const grokAccountId = account.upstream_grok_account_id?.trim();
      if (grokAccountId && !map.has(grokAccountId)) {
        map.set(grokAccountId, account.id);
      }
    });
    return map;
  }, [accounts]);

  /**
   * Grok 平台账号在成员列表里的行数据（与 Codex 账号同一列表、同一套交互）。
   *
   * 行 ID 使用 `grok:` 前缀与真实 Codex 账号区分；勾选后由宿主创建绑定的
   * 供应商账号，选中的始终是真实账号 ID。
   */
  const grokMemberRowAccounts = useMemo(() => {
    if (mode !== "members" || !onAddGrokMember || !showGrokPlatform) {
      return [] as CodexAccount[];
    }
    const { requireValidAccounts, selectedTypes } =
      splitValidityFilterValues(memberFilterTypes);
    return buildGrokMemberRowAccounts({
      accounts: grokMemberCandidates,
      searchQuery: memberSearchQuery,
      tagFilterActive: memberTagFilter.length > 0,
      groupFilterActive: memberGroupFilter.length > 0,
      requireValidAccounts,
      selectedTypes,
    });
  }, [
    grokMemberCandidates,
    memberFilterTypes,
    memberGroupFilter,
    memberSearchQuery,
    memberTagFilter,
    mode,
    onAddGrokMember,
    showGrokPlatform,
  ]);

  /** 行 ID -> Grok 平台账号（成员列表里 Grok 行与 Codex 行共用同一张表）。 */
  const grokMemberRowAccountById = useMemo(() => {
    const map = new Map<string, GrokAccount>();
    grokMemberRowAccounts.forEach((row) => {
      const grokAccountId = row.upstream_grok_account_id?.trim();
      if (!grokAccountId) {
        return;
      }
      const source = grokMemberCandidates.find(
        (candidate) => candidate.id === grokAccountId,
      );
      if (source) {
        map.set(row.id, source);
      }
    });
    return map;
  }, [grokMemberCandidates, grokMemberRowAccounts]);

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
        isBlockingCodexAccountQuotaError(account)
      ) {
        return false;
      }

      if (selectedTypes.size > 0) {
        const planKey = getCodexPlanFilterKey(account);
        const matchesType = Array.from(selectedTypes).some((type) => {
          if (type === "ERROR") {
            return isBlockingCodexAccountQuotaError(account);
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

  const visibleSelectableAccounts = selectCodexLocalAccessMemberRows({
    codexAccounts: visibleAccounts,
    grokRows: grokMemberRowAccounts,
    grokAccounts: grokMemberCandidates,
    manageGrok: Boolean(onAddGrokMember),
    showCodexPlatform,
    showGrokPlatform,
    restrictFreeAccounts,
    selected,
  });
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

  /** 行是否已被选中：Grok 行按绑定的供应商账号 ID 判断。 */
  const isMemberRowSelected = useCallback(
    (account: CodexAccount) => {
      const grokRowAccount = grokMemberRowAccountById.get(account.id);
      if (!grokRowAccount) {
        return selected.has(account.id);
      }
      const boundAccountId = grokMemberBindingByAccountId.get(grokRowAccount.id);
      return Boolean(boundAccountId && selected.has(boundAccountId));
    },
    [grokMemberBindingByAccountId, grokMemberRowAccountById, selected],
  );

  const selectedVisibleCount = useMemo(
    () =>
      visibleEnabledAccounts.reduce(
        (count, account) => count + (isMemberRowSelected(account) ? 1 : 0),
        0,
      ),
    [isMemberRowSelected, visibleEnabledAccounts],
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

  const initialPreferredAccountIds = useMemo(() => {
    const selectedSet = new Set(normalizedInitialSelectedIds);
    return new Set(
      (collection?.customRoutingRules ?? [])
        .filter(
          (rule) => rule.isPreferred && selectedSet.has(rule.accountId),
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

  const currentPreferredAccountIds = useMemo(() => {
    const rulePreferredById = new Map(
      (collection?.customRoutingRules ?? []).map((rule) => [
        rule.accountId,
        Boolean(rule.isPreferred),
      ]),
    );
    const next = new Set<string>();
    selected.forEach((accountId) => {
      const isPreferred =
        customRoutingDraft[accountId]?.isPreferred ??
        rulePreferredById.get(accountId) ??
        false;
      if (isPreferred) {
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
      !areSetsEqual(currentBackupAccountIds, initialBackupAccountIds) ||
      !areSetsEqual(currentPreferredAccountIds, initialPreferredAccountIds) ||
      JSON.stringify(imageGenerationPolicies) !==
        JSON.stringify(collection?.imageGenerationAccountPolicies ?? {}),
    [
      collection?.restrictFreeAccounts,
      currentBackupAccountIds,
      currentPreferredAccountIds,
      initialSessionAffinity,
      initialSessionAffinityTtlSeconds,
      initialBackupAccountIds,
      initialPreferredAccountIds,
      normalizedInitialSelectedIds,
      restrictFreeAccounts,
      sessionAffinity,
      sessionAffinityTtlSeconds,
      selected,
      imageGenerationPolicies,
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
    () => buildLocalAccessRoutingStrategyOptions(t),
    [t],
  );
  const memberPriorityOptions = useMemo(
    () => [
      {
        value: "lowest",
        label: t("codex.localAccess.memberPriorityLowest", "最低"),
      },
      {
        value: "normal",
        label: t("codex.localAccess.memberPriorityNormal", "正常"),
      },
      {
        value: "highest",
        label: t("codex.localAccess.memberPriorityHighest", "最高"),
      },
    ],
    [t],
  );
  const imageGenerationPolicyOptions = useMemo(
    () => [
      { value: "inherit", label: t("codex.localAccess.imagePolicy.inherit", "生图：自动") },
      { value: "enabled", label: t("codex.localAccess.imagePolicy.enabled", "生图：启用") },
      { value: "disabled", label: t("codex.localAccess.imagePolicy.disabled", "生图：禁用") },
    ],
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

  useEffect(() => {
    if (!isOpen || mode !== "members" || !onAddGrokMember) {
      return;
    }
    let cancelled = false;
    setGrokMemberLoading(true);
    setGrokMemberError("");
    void listGrokAccounts()
      .then((list) => {
        if (!cancelled) {
          setGrokMemberAccounts(list);
        }
      })
      .catch((loadError) => {
        if (!cancelled) {
          setGrokMemberError(String(loadError).replace(/^Error:\s*/, ""));
          setGrokMemberAccounts([]);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setGrokMemberLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [isOpen, mode, onAddGrokMember]);

  const handleToggleGrokMember = async (
    grokAccount: GrokAccount,
    nextChecked: boolean,
  ) => {
    const boundAccountId = grokMemberBindingByAccountId.get(grokAccount.id);
    if (!nextChecked) {
      if (boundAccountId) {
        setSelected((previous) => {
          const next = new Set(previous);
          next.delete(boundAccountId);
          return next;
        });
      }
      setMembersDraftDirty(true);
      return;
    }
    if (boundAccountId) {
      setSelected((previous) => new Set(previous).add(boundAccountId));
      setMembersDraftDirty(true);
      return;
    }
    if (!onAddGrokMember) {
      return;
    }
    setGrokMemberBusyId(grokAccount.id);
    setGrokMemberError("");
    try {
      const bound = await onAddGrokMember(grokAccount.id);
      if (bound) {
        setSelected((previous) => new Set(previous).add(bound.id));
        setMembersDraftDirty(true);
      }
    } catch (addError) {
      setGrokMemberError(String(addError).replace(/^Error:\s*/, ""));
    } finally {
      setGrokMemberBusyId("");
    }
  };

  const customRoutingRuleByAccountId = useMemo(() => {
    const next = new Map<string, CustomRoutingDraftRule>();
    collection?.customRoutingRules?.forEach((rule) => {
      next.set(rule.accountId, {
        priority: normalizeCustomRoutingPriority(rule.priority),
        weight: normalizeCustomRoutingWeight(rule.weight),
        isBackup: Boolean(rule.isBackup),
        isPreferred: Boolean(rule.isPreferred),
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
      if (!isBlockingCodexAccountQuotaError(account)) {
        counts.VALID += 1;
      }
      const tier = getCodexPlanFilterKey(account);
      incrementCodexPlanFilterCount(counts, tier);
      if (isBlockingCodexAccountQuotaError(account)) {
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
        isBlockingCodexAccountQuotaError(account)
      ) {
        return false;
      }

      if (selectedTypes.size > 0) {
        const planKey = getCodexPlanFilterKey(account);
        const matchesType = Array.from(selectedTypes).some((type) => {
          if (type === "ERROR") {
            return isBlockingCodexAccountQuotaError(account);
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

  const toggleSelectAllVisible = async () => {
    if (membersInteractionDisabled || visibleEnabledAccounts.length === 0) {
      return;
    }
    const codexRows: CodexAccount[] = [];
    const grokRows: GrokAccount[] = [];
    for (const account of visibleEnabledAccounts) {
      const grokRowAccount = grokMemberRowAccountById.get(account.id);
      if (grokRowAccount) {
        grokRows.push(grokRowAccount);
      } else {
        codexRows.push(account);
      }
    }
    if (allVisibleSelected) {
      setMembersDraftDirty(true);
      setSelected((prev) => {
        const next = new Set(prev);
        codexRows.forEach((account) => next.delete(account.id));
        grokRows.forEach((grokAccount) => {
          const boundAccountId =
            grokMemberBindingByAccountId.get(grokAccount.id);
          if (boundAccountId) {
            next.delete(boundAccountId);
          }
        });
        return next;
      });
      return;
    }
    setMembersDraftDirty(true);
    setSelected((prev) => {
      const next = new Set(prev);
      codexRows.forEach((account) => next.add(account.id));
      return next;
    });
    // Grok 账号需要先接入（创建绑定账号）才能加入集合，逐个串行执行。
    void (async () => {
      for (const grokAccount of grokRows) {
        if (membersInteractionDisabled) {
          return;
        }
        const boundAccountId = grokMemberBindingByAccountId.get(grokAccount.id);
        if (boundAccountId) {
          setSelected((prev) => new Set(prev).add(boundAccountId));
          continue;
        }
        await handleToggleGrokMember(grokAccount, true);
      }
    })();
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

  const toggleSelect = async (accountId: string) => {
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
      const preferredAccountIds = filtered.filter((accountId) => {
        const isPreferred =
          customRoutingDraft[accountId]?.isPreferred ??
          customRoutingRuleByAccountId.get(accountId)?.isPreferred ??
          false;
        return isPreferred;
      });
      await onSaveAccounts({
        accountIds: filtered,
        restrictFreeAccounts,
        backupAccountIds,
        preferredAccountIds,
        sessionAffinity,
        sessionAffinityTtlMs: parsedSessionAffinityTtlSeconds * 1000,
        imageGenerationAccountPolicies: Object.fromEntries(
          filtered.map((accountId) => [
            accountId,
            imageGenerationPolicies[accountId] ??
              (isCodexApiKeyAccount(
                localAccessAccountById.get(accountId) as CodexAccount,
              )
                ? "disabled"
                : "inherit"),
          ]),
        ),
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
        isPreferred: false,
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

  const updateMemberUsagePriority = (
    accountId: string,
    usagePriority: AccountUsagePriority,
  ) => {
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
        isPreferred: false,
      };
    setMembersDraftDirty(true);
    setCustomRoutingDraft((prev) => ({
      ...prev,
      [accountId]: {
        ...current,
        isBackup: usagePriority === "lowest",
        isPreferred: usagePriority === "highest",
      },
    }));
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
          isPreferred:
            next[accountId]?.isPreferred ??
            customRoutingRuleByAccountId.get(accountId)?.isPreferred ??
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
        isPreferred: false,
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
            isPreferred: false,
          };
        return {
          accountId,
          priority: normalizeCustomRoutingPriority(rule.priority),
          weight: normalizeCustomRoutingWeight(rule.weight),
          isBackup: rule.isBackup,
          isPreferred: rule.isPreferred,
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
    await runAction(
      async () => {
        await onRestartSidecar();
      },
      t("codex.localAccess.restartSuccess", "API 服务 Sidecar 已重启"),
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
              ? " codex-local-access-modal-members"
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
                        collection?.enabled && state?.running ? "running" : "stopped"
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
                  <>
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm codex-local-access-inline-action"
                      onClick={() => void handleRestartSidecar()}
                      disabled={actionBusy || sidecarRestarting}
                    >
                      <RefreshCw
                        size={14}
                        className={sidecarRestarting ? "loading-spinner" : ""}
                      />
                      {t("codex.localAccess.restartAction", "重启 Sidecar")}
                    </button>
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
                  </>
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
                            "账号状态",
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

                      {collection ? (
                        <CodexImageForwardConfig
                          accounts={accounts}
                          accountIds={collection.imageGenerationAccountIds}
                          disabled={saving || testing || starting}
                          maskAccountText={maskAccountText}
                          onSave={onUpdateImageGenerationAccounts}
                          imageModelControl={
                            <CodexImageModelSelect
                              model={collection.imageGenerationModel}
                              disabled={saving || testing || starting}
                              onSave={onUpdateImageGenerationModel}
                            />
                          }
                        />
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
                    {onAddGrokMember && (
                      <MultiSelectFilterDropdown
                        options={[
                          { value: "codex", label: "Codex" },
                          { value: "grok", label: "Grok" },
                        ]}
                        selectedValues={platformFilter}
                        allLabel={t(
                          "codex.localAccess.memberPlatformAll",
                          "全部平台",
                        )}
                        filterLabel={t(
                          "codex.localAccess.memberPlatformLabel",
                          "平台",
                        )}
                        clearLabel={t("accounts.clearFilter", "清空筛选")}
                        emptyLabel={t("common.none", "暂无")}
                        ariaLabel={t(
                          "codex.localAccess.memberPlatformLabel",
                          "平台",
                        )}
                        onToggleValue={(value) =>
                          setPlatformFilter((prev) =>
                            prev.includes(value)
                              ? prev.filter((item) => item !== value)
                              : [...prev, value],
                          )
                        }
                        onClear={() => setPlatformFilter([])}
                      />
                    )}
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
                  ) : localAccessAccounts.length === 0 &&
                    grokMemberRowAccounts.length === 0 &&
                    !grokMemberLoading ? (
                    <div className="group-account-empty">
                      {t(
                        "codex.localAccess.modal.empty",
                        "暂无可加入的 Codex 账号",
                      )}
                    </div>
                  ) : visibleSelectableAccounts.length === 0 &&
                    grokMemberRowAccounts.length === 0 &&
                    !grokMemberLoading ? (
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
                      const isPendingOauthUnsupported =
                        ineligibleReason === "pending_oauth";
                      const isWebSessionUnsupported =
                        ineligibleReason === "web_session_quota_only";
                      const isJoinUnsupported =
                        isPendingOauthUnsupported || isWebSessionUnsupported;
                      const grokRowAccount = grokMemberRowAccountById.get(
                        account.id,
                      );
                      const grokBoundAccountId = grokRowAccount
                        ? grokMemberBindingByAccountId.get(grokRowAccount.id)
                        : undefined;
                      const rowAccountId = grokBoundAccountId ?? account.id;
                      const isChecked =
                        !isJoinUnsupported && selected.has(rowAccountId);
                      const grokRowBusy =
                        Boolean(grokRowAccount) &&
                        grokMemberBusyId === grokRowAccount?.id;
                      const usagePriority = resolveAccountUsagePriority(
                        customRoutingDraft[rowAccountId] ??
                          customRoutingRuleByAccountId.get(rowAccountId),
                      );
                      const accountStats = allStatsByAccountId.get(
                        rowAccountId,
                      )?.usage;
                      const grokQuotaChip = grokRowAccount ? (
                        <CodexGrokBuildQuotaChip t={t} account={grokRowAccount} />
                      ) : null;

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
                              membersInteractionDisabled ||
                              isJoinUnsupported ||
                              grokRowBusy
                            }
                            onChange={() =>
                              grokRowAccount
                                ? void handleToggleGrokMember(
                                    grokRowAccount,
                                    !isChecked,
                                  )
                                : toggleSelect(account.id)
                            }
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
                                  membersInteractionDisabled ||
                                  isJoinUnsupported ||
                                  grokRowBusy
                                }
                                onClick={() =>
                                  grokRowAccount
                                    ? void handleToggleGrokMember(
                                        grokRowAccount,
                                        !isChecked,
                                      )
                                    : toggleSelect(account.id)
                                }
                              >
                                {maskAccountText(presentation.displayName)}
                              </button>
                              <span
                                className="codex-local-access-member-priority-slot"
                                title={t(
                                  "codex.localAccess.memberPriorityDesc",
                                  "最高优先使用，正常按当前规则调度，最低仅在其他账号不可用时使用。",
                                )}
                              >
                                {!isJoinUnsupported ? (
                                  <>
                                    <span className="codex-local-access-member-priority-label">
                                      {t(
                                        "codex.localAccess.memberPriorityLabel",
                                        "优先级",
                                      )}
                                    </span>
                                    <SingleSelectDropdown
                                      value={usagePriority}
                                      options={memberPriorityOptions}
                                      className="codex-local-access-member-priority-dropdown"
                                      menuClassName="codex-local-access-member-priority-menu"
                                      menuWidth={112}
                                      menuMaxHeight={180}
                                      ariaLabel={t(
                                        "codex.localAccess.memberPriorityLabel",
                                        "优先级",
                                      )}
                                      disabled={
                                        membersInteractionDisabled ||
                                        grokRowBusy ||
                                        (Boolean(grokRowAccount) &&
                                          !grokBoundAccountId)
                                      }
                                      onChange={(value) =>
                                        updateMemberUsagePriority(
                                          rowAccountId,
                                          value as AccountUsagePriority,
                                        )
                                      }
                                    />
                                  </>
                                ) : null}
                              </span>
                              <span className="codex-local-access-member-plan">
                                {!isJoinUnsupported && (
                                  <SingleSelectDropdown
                                    value={
                                      imageGenerationPolicies[rowAccountId] ??
                                      (isCodexApiKeyAccount(account)
                                        ? "disabled"
                                        : "inherit")
                                    }
                                    options={imageGenerationPolicyOptions}
                                    className="codex-local-access-member-image-policy-dropdown"
                                    menuClassName="codex-local-access-member-image-policy-menu"
                                    menuWidth={120}
                                    ariaLabel={t(
                                      "codex.localAccess.imagePolicy.label",
                                      "生图策略",
                                    )}
                                    disabled={
                                      membersInteractionDisabled ||
                                      grokRowBusy ||
                                      (Boolean(grokRowAccount) && !grokBoundAccountId)
                                    }
                                    onChange={(value) =>
                                      setImageGenerationPolicies((prev) => ({
                                        ...prev,
                                        [rowAccountId]:
                                          value as CodexLocalAccessImageGenerationPolicy,
                                      }))
                                    }
                                  />
                                )}
                                <span
                                  className={`tier-badge ${presentation.planClass}`}
                                >
                                  {grokRowAccount
                                    ? t("codex.grokAdd.tab", "Grok 账号")
                                    : presentation.planLabel}
                                </span>
                              </span>
                              <span className="codex-local-access-member-metric">
                                {t("codex.localAccess.stats.accountRequests", {
                                  count: accountStats?.requestCount ?? 0,
                                  defaultValue: "{{count}} 次请求",
                                })}
                              </span>
                              <span className="codex-local-access-member-trailing">
                                {isPendingOauthUnsupported && (
                                  <span className="codex-local-access-member-unsupported">
                                    {t(
                                      "codex.localAccess.modal.pendingOauthUnsupported",
                                      "待授权账号不可加入 API 服务",
                                    )}
                                  </span>
                                )}
                                {isWebSessionUnsupported && (
                                  <span className="codex-local-access-member-unsupported">
                                    {t(
                                      "codex.webSessionImport.apiIneligible",
                                      "Web Session 仅支持查看额度，不能加入 API 服务",
                                    )}
                                  </span>
                                )}
                                {grokQuotaChip ??
                                  renderQuotaPreview(presentation, 2)}
                              </span>
                            </div>
                          </div>
                        </div>
                      );
                    })
                  )}
                </div>
                {grokMemberError && (
                  <div className="group-account-empty codex-local-access-grok-member-error">
                    {grokMemberError}
                  </div>
                )}
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
                        isPreferred: false,
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
                            {(draftRule.isBackup || draftRule.isPreferred) && (
                              <span
                                className="codex-local-access-custom-routing-usage-priority-badge"
                                title={t(
                                  "codex.localAccess.memberPriorityDesc",
                                  "最高优先使用，正常按当前规则调度，最低仅在其他账号不可用时使用。",
                                )}
                              >
                                {t(
                                  draftRule.isPreferred
                                    ? "codex.localAccess.memberPriorityHighest"
                                    : "codex.localAccess.memberPriorityLowest",
                                  draftRule.isPreferred ? "最高" : "最低",
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
        accountPoolHealth={state?.accountPoolHealth ?? []}
        recoverySuppressedAccountIds={
          state?.recoverySuppressedAccountIds ?? []
        }
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
