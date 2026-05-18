import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Activity,
  Check,
  CircleAlert,
  Copy,
  Eye,
  EyeOff,
  FolderPlus,
  Gauge,
  KeyRound,
  Power,
  RefreshCw,
  Search,
  Server,
  ShieldCheck,
  Trash2,
  Wrench,
  X,
} from 'lucide-react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../types/codex';
import type { CodexAccountGroup } from '../services/codexAccountGroupService';
import type {
  CodexLocalAccessAddressKind,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessScope,
  CodexLocalAccessState,
  CodexLocalAccessStatsWindow,
  CodexLocalAccessTestResult,
} from '../types/codexLocalAccess';
import {
  getCodexPlanFilterKey,
  isCodexApiKeyAccount,
  isCodexExplicitFreePlanType,
} from '../types/codex';
import {
  buildCodexAccountPresentation,
  buildQuotaPreviewLines,
} from '../presentation/platformAccountPresentation';
import { buildValidAccountsFilterOption, splitValidityFilterValues } from '../utils/accountValidityFilter';
import {
  formatCodexQuotaPoolPercent,
  summarizeCodexQuotaPool,
  type CodexQuotaPoolItem,
} from '../utils/codexQuotaPool';
import { AccountTagFilterDropdown } from './AccountTagFilterDropdown';
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from './MultiSelectFilterDropdown';
import { SingleSelectDropdown } from './SingleSelectDropdown';
import { useEscClose } from '../hooks/useEscClose';
import './GroupAccountPickerModal.css';
import './CodexLocalAccessModal.css';

interface CodexLocalAccessModalProps {
  isOpen: boolean;
  mode: 'panel' | 'members';
  state: CodexLocalAccessState | null;
  addressKind: CodexLocalAccessAddressKind;
  addressOptions: Array<{ value: string; label: string }>;
  onAddressKindChange: (value: string) => void;
  accounts: CodexAccount[];
  accountGroups: CodexAccountGroup[];
  initialSelectedIds: string[];
  maskAccountText: (value?: string | null) => string;
  onClose: () => void;
  onSaveAccounts: (payload: {
    accountIds: string[];
    restrictFreeAccounts: boolean;
  }) => Promise<unknown> | unknown;
  onClearStats: () => Promise<unknown> | unknown;
  onRefreshStats: () => Promise<unknown> | unknown;
  onUpdatePort: (port: number) => Promise<unknown> | unknown;
  onUpdateRoutingStrategy: (
    strategy: CodexLocalAccessRoutingStrategy,
  ) => Promise<unknown> | unknown;
  onUpdateAccessScope: (
    accessScope: CodexLocalAccessScope,
  ) => Promise<unknown> | unknown;
  onRotateApiKey: () => Promise<unknown> | unknown;
  onKillPort: () => Promise<unknown> | unknown;
  onToggleEnabled: () => Promise<unknown> | unknown;
  onTest: () => Promise<CodexLocalAccessTestResult> | CodexLocalAccessTestResult;
  saving: boolean;
  testing: boolean;
  starting: boolean;
  portCleanupBusy: boolean;
}

type StatsRangeKey = 'daily' | 'weekly' | 'monthly';
type CopyableField = 'apiPortUrl' | 'baseUrl' | 'apiKey' | 'modelId';
const CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY =
  'agtools.codex.local_access.stats_range.v1';

function normalizeAccessScope(value: string): CodexLocalAccessScope {
  return value === 'lan' ? 'lan' : 'localhost';
}

function normalizeStatsRangeKey(value: string | null | undefined): StatsRangeKey {
  if (value === 'weekly' || value === 'monthly') {
    return value;
  }
  return 'daily';
}

function readStoredStatsRange(): StatsRangeKey {
  try {
    return normalizeStatsRangeKey(localStorage.getItem(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY));
  } catch {
    return 'daily';
  }
}

function persistStatsRange(value: StatsRangeKey): void {
  try {
    localStorage.setItem(CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY, value);
  } catch {
    // ignore storage write failures
  }
}

function formatCompactNumber(value: number): string {
  return new Intl.NumberFormat('en', {
    notation: value >= 1000 ? 'compact' : 'standard',
    maximumFractionDigits: value >= 1000 ? 1 : 0,
  }).format(value || 0);
}

function formatLatencyMs(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '--';
  if (value >= 1000) return `${(value / 1000).toFixed(2)}s`;
  return `${Math.round(value)}ms`;
}

function formatQuotaPoolLabel(
  baseLabel: string,
  pool: CodexQuotaPoolItem,
  hourlyLabel: string,
  weeklyLabel: string,
): string {
  return `${baseLabel} · ${hourlyLabel} ${formatCodexQuotaPoolPercent(pool.hourly)} · ${weeklyLabel} ${formatCodexQuotaPoolPercent(pool.weekly)}`;
}

function areSetsEqual(left: Set<string>, right: Set<string>): boolean {
  if (left.size !== right.size) return false;
  for (const value of left) {
    if (!right.has(value)) return false;
  }
  return true;
}

export function CodexLocalAccessModal({
  isOpen,
  mode,
  state,
  addressKind,
  addressOptions,
  onAddressKindChange,
  accounts,
  accountGroups,
  initialSelectedIds,
  maskAccountText,
  onClose,
  onSaveAccounts,
  onClearStats,
  onRefreshStats,
  onUpdatePort,
  onUpdateRoutingStrategy,
  onUpdateAccessScope,
  onRotateApiKey,
  onKillPort,
  onToggleEnabled,
  onTest,
  saving,
  testing,
  starting,
  portCleanupBusy,
}: CodexLocalAccessModalProps) {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filterTypes, setFilterTypes] = useState<string[]>([]);
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const [groupFilter, setGroupFilter] = useState<string[]>([]);
  const [restrictFreeAccounts, setRestrictFreeAccounts] = useState(true);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [testDialogOpen, setTestDialogOpen] = useState(false);
  const [testDialogRunning, setTestDialogRunning] = useState(false);
  const [testDialogResult, setTestDialogResult] =
    useState<CodexLocalAccessTestResult | null>(null);
  const [testDialogError, setTestDialogError] = useState('');
  const [portInput, setPortInput] = useState('');
  const [keyVisible, setKeyVisible] = useState(false);
  const [copiedField, setCopiedField] = useState<CopyableField | null>(null);
  const [selectedModelId, setSelectedModelId] = useState('');
  const [statsRange, setStatsRange] = useState<StatsRangeKey>(() => readStoredStatsRange());
  const selectAllCheckboxRef = useRef<HTMLInputElement | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  const collection = state?.collection ?? null;
  const apiPortUrl = state?.apiPortUrl ?? '';
  const baseUrl = state?.baseUrl ?? '';
  const displayBaseUrl =
    addressKind === 'lan' && state?.lanBaseUrl ? state.lanBaseUrl : baseUrl;
  const modelIds = state?.modelIds ?? [];
  const stats = state?.stats;
  const statsRangeOptions = useMemo(
    () =>
      [
        { key: 'daily', label: t('codex.localAccess.statsRange.daily', '日') },
        { key: 'weekly', label: t('codex.localAccess.statsRange.weekly', '周') },
        { key: 'monthly', label: t('codex.localAccess.statsRange.monthly', '月') },
      ] satisfies Array<{ key: StatsRangeKey; label: string }>,
    [t],
  );
  const quotaPoolLabels = useMemo(
    () => ({
      hourly: t('codex.localAccess.quotaPool.hourlyShort', '5h'),
      weekly: t('codex.localAccess.quotaPool.weeklyShort', '周'),
      title: t('codex.localAccess.quotaPool.title', '额度池'),
    }),
    [t],
  );
  const selectedStatsWindow = useMemo<CodexLocalAccessStatsWindow | null>(() => {
    if (!stats) return null;
    return stats[statsRange];
  }, [stats, statsRange]);
  const selectedTotals = selectedStatsWindow?.totals;
  const routingStrategy = collection?.routingStrategy ?? 'auto';
  const accessScope = collection?.accessScope ?? 'localhost';
  const accessScopeAddress =
    accessScope === 'lan' ? '0.0.0.0' : '127.0.0.1';
  const accessScopeBadge =
    accessScope === 'lan'
      ? t('codex.localAccess.accessScopeLanShort', '本机+局域网')
      : t('codex.localAccess.accessScopeLocalhostShort', '仅本机');
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
      ? Math.round((selectedTotals.successCount / selectedTotals.requestCount) * 100)
      : 0;
  const testDialogBusy = testDialogRunning || testing;
  const actionBusy = saving || testing || starting || portCleanupBusy;
  const summaryStats = useMemo(
    () => [
      {
        key: 'requests',
        label: t('codex.localAccess.stats.requests', '总请求数'),
        value: formatCompactNumber(selectedTotals?.requestCount ?? 0),
        detail: t('codex.localAccess.stats.requestsDetail', {
          success: formatCompactNumber(selectedTotals?.successCount ?? 0),
          failed: formatCompactNumber(selectedTotals?.failureCount ?? 0),
          defaultValue: '成功 {{success}} / 失败 {{failed}}',
        }),
      },
      {
        key: 'tokens',
        label: t('codex.localAccess.stats.tokens', '总 Token 数'),
        value: formatCompactNumber(selectedTotals?.totalTokens ?? 0),
        detail: t('codex.localAccess.stats.tokensDetail', {
          input: formatCompactNumber(selectedTotals?.inputTokens ?? 0),
          output: formatCompactNumber(selectedTotals?.outputTokens ?? 0),
          defaultValue: '输入 {{input}} / 输出 {{output}}',
        }),
      },
      {
        key: 'specialTokens',
        label: t('codex.localAccess.stats.specialTokens', '缓存 / 思考'),
        value: formatCompactNumber(
          (selectedTotals?.cachedTokens ?? 0) + (selectedTotals?.reasoningTokens ?? 0),
        ),
        detail: t('codex.localAccess.stats.specialTokensDetail', {
          cached: formatCompactNumber(selectedTotals?.cachedTokens ?? 0),
          reasoning: formatCompactNumber(selectedTotals?.reasoningTokens ?? 0),
          defaultValue: '缓存 {{cached}} / 思考 {{reasoning}}',
        }),
      },
      {
        key: 'latency',
        label: t('codex.localAccess.stats.avgLatency', '平均延迟'),
        value: formatLatencyMs(avgLatencyMs),
        detail: t('codex.localAccess.stats.successRate', {
          rate: successRate,
          defaultValue: '成功率 {{rate}}%',
        }),
      },
    ],
    [avgLatencyMs, selectedTotals, successRate, t],
  );

  const oauthAccounts = useMemo(
    () => accounts.filter((account) => !isCodexApiKeyAccount(account)),
    [accounts],
  );
  const quotaPoolSummary = useMemo(
    () => summarizeCodexQuotaPool(oauthAccounts),
    [oauthAccounts],
  );
  const currentQuotaPoolSummary = useMemo(() => {
    const accountIds = new Set(collection?.accountIds ?? []);
    return summarizeCodexQuotaPool(oauthAccounts.filter((account) => accountIds.has(account.id)));
  }, [collection?.accountIds, oauthAccounts]);
  const oauthAccountIdSet = useMemo(
    () => new Set(oauthAccounts.map((account) => account.id)),
    [oauthAccounts],
  );
  const normalizedInitialSelectedIds = useMemo(
    () => initialSelectedIds.filter((accountId) => oauthAccountIdSet.has(accountId)),
    [initialSelectedIds, oauthAccountIdSet],
  );

  useEffect(() => {
    if (!isOpen) return;
    setQuery('');
    setSelected(new Set(normalizedInitialSelectedIds));
    setFilterTypes([]);
    setTagFilter([]);
    setGroupFilter([]);
    setRestrictFreeAccounts(collection?.restrictFreeAccounts ?? true);
    setError('');
    setNotice('');
    setTestDialogOpen(false);
    setTestDialogRunning(false);
    setTestDialogResult(null);
    setTestDialogError('');
    setKeyVisible(false);
    setCopiedField(null);
    setPortInput(collection?.port ? String(collection.port) : '');
    if (mode === 'members') {
      window.setTimeout(() => {
        searchInputRef.current?.focus();
      }, 0);
    }
  }, [collection?.port, collection?.restrictFreeAccounts, isOpen, mode, normalizedInitialSelectedIds]);

  useEffect(() => {
    if (modelIds.length === 0) {
      setSelectedModelId('');
      return;
    }
    setSelectedModelId((current) => (modelIds.includes(current) ? current : modelIds[0]));
  }, [modelIds]);

  useEffect(() => {
    persistStatsRange(statsRange);
  }, [statsRange]);

  const normalizeTag = (value: string) => value.trim().toLowerCase();

  const availableTags = useMemo(() => {
    const next = new Set<string>();
    oauthAccounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const trimmed = tag.trim();
        if (trimmed) next.add(trimmed);
      });
    });
    return Array.from(next).sort((left, right) => left.localeCompare(right));
  }, [oauthAccounts]);

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
    const counts = { all: oauthAccounts.length, VALID: 0, FREE: 0, PLUS: 0, PRO: 0, TEAM: 0, ENTERPRISE: 0, ERROR: 0 };
    oauthAccounts.forEach((account) => {
      if (!account.quota_error) {
        counts.VALID += 1;
      }
      const tier = getCodexPlanFilterKey(account);
      if (tier in counts) {
        counts[tier as keyof typeof counts] += 1;
      }
      if (account.quota_error) {
        counts.ERROR += 1;
      }
    });
    return counts;
  }, [oauthAccounts]);

  const allTierFilterLabel = useMemo(
    () =>
      formatQuotaPoolLabel(
        t('common.shared.filter.all', { count: tierCounts.all }),
        quotaPoolSummary.all,
        quotaPoolLabels.hourly,
        quotaPoolLabels.weekly,
      ),
    [quotaPoolLabels.hourly, quotaPoolLabels.weekly, quotaPoolSummary.all, t, tierCounts.all],
  );

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => [
      {
        value: 'FREE',
        label: formatQuotaPoolLabel(
          `FREE (${tierCounts.FREE})`,
          quotaPoolSummary.byPlan.FREE,
          quotaPoolLabels.hourly,
          quotaPoolLabels.weekly,
        ),
      },
      {
        value: 'PLUS',
        label: formatQuotaPoolLabel(
          `PLUS (${tierCounts.PLUS})`,
          quotaPoolSummary.byPlan.PLUS,
          quotaPoolLabels.hourly,
          quotaPoolLabels.weekly,
        ),
      },
      {
        value: 'PRO',
        label: formatQuotaPoolLabel(
          `PRO (${tierCounts.PRO})`,
          quotaPoolSummary.byPlan.PRO,
          quotaPoolLabels.hourly,
          quotaPoolLabels.weekly,
        ),
      },
      {
        value: 'TEAM',
        label: formatQuotaPoolLabel(
          `TEAM (${tierCounts.TEAM})`,
          quotaPoolSummary.byPlan.TEAM,
          quotaPoolLabels.hourly,
          quotaPoolLabels.weekly,
        ),
      },
      {
        value: 'ENTERPRISE',
        label: formatQuotaPoolLabel(
          `ENTERPRISE (${tierCounts.ENTERPRISE})`,
          quotaPoolSummary.byPlan.ENTERPRISE,
          quotaPoolLabels.hourly,
          quotaPoolLabels.weekly,
        ),
      },
      { value: 'ERROR', label: `ERROR (${tierCounts.ERROR})` },
      buildValidAccountsFilterOption(t, tierCounts.VALID),
    ],
    [quotaPoolLabels.hourly, quotaPoolLabels.weekly, quotaPoolSummary.byPlan, t, tierCounts],
  );

  const visibleAccounts = useMemo(() => {
    const queryText = query.trim().toLowerCase();
    const sorted = [...oauthAccounts].sort((a, b) => {
      const aName = buildCodexAccountPresentation(a, t).displayName.toLowerCase();
      const bName = buildCodexAccountPresentation(b, t).displayName.toLowerCase();
      return aName.localeCompare(bName);
    });
    const selectedTags = new Set(tagFilter.map(normalizeTag));
    const selectedGroups = new Set(groupFilter);
    const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);

    return sorted.filter((account) => {
      const presentation = buildCodexAccountPresentation(account, t);
      const displayName = presentation.displayName.toLowerCase();
      const groupNames = (groupNameByAccountId.get(account.id) ?? []).join(' ').toLowerCase();
      const matchesQuery =
        !queryText || displayName.includes(queryText) || groupNames.includes(queryText);
      if (!matchesQuery) return false;

      if (selectedTags.size > 0) {
        const accountTags = (account.tags || []).map(normalizeTag);
        if (!accountTags.some((tag) => selectedTags.has(tag))) {
          return false;
        }
      }

      if (selectedGroups.size > 0) {
        const accountGroupIds = groupIdsByAccountId.get(account.id);
        if (!accountGroupIds || !Array.from(accountGroupIds).some((id) => selectedGroups.has(id))) {
          return false;
        }
      }

      if (requireValidAccounts && account.quota_error) {
        return false;
      }

      if (selectedTypes.size > 0) {
        const planKey = getCodexPlanFilterKey(account);
        const matchesType = Array.from(selectedTypes).some((type) => {
          if (type === 'ERROR') return Boolean(account.quota_error);
          return type === planKey;
        });
        if (!matchesType) {
          return false;
        }
      }

      return true;
    });
  }, [filterTypes, groupFilter, groupIdsByAccountId, groupNameByAccountId, oauthAccounts, query, t, tagFilter]);

  const visibleSelectableAccounts = useMemo(
    () =>
      visibleAccounts.filter((account) => {
        if (!restrictFreeAccounts) return true;
        if (!isCodexExplicitFreePlanType(account.plan_type)) return true;
        return selected.has(account.id);
      }),
    [restrictFreeAccounts, selected, visibleAccounts],
  );

  const selectedVisibleCount = useMemo(
    () =>
      visibleSelectableAccounts.reduce(
        (count, account) => count + (selected.has(account.id) ? 1 : 0),
        0,
      ),
    [selected, visibleSelectableAccounts],
  );

  const allVisibleSelected =
    visibleSelectableAccounts.length > 0 &&
    selectedVisibleCount === visibleSelectableAccounts.length;

  useEffect(() => {
    if (!selectAllCheckboxRef.current) return;
    selectAllCheckboxRef.current.indeterminate =
      selectedVisibleCount > 0 && !allVisibleSelected;
  }, [allVisibleSelected, selectedVisibleCount]);

  const selectionDirty = useMemo(
    () =>
      !areSetsEqual(selected, new Set(normalizedInitialSelectedIds)) ||
      restrictFreeAccounts !== (collection?.restrictFreeAccounts ?? true),
    [collection?.restrictFreeAccounts, normalizedInitialSelectedIds, restrictFreeAccounts, selected],
  );

  const allStatsByAccountId = useMemo(() => {
    const next = new Map<string, NonNullable<CodexLocalAccessState['stats']>['accounts'][number]>();
    stats?.accounts.forEach((item) => next.set(item.accountId, item));
    return next;
  }, [stats?.accounts]);

  const windowStatsByAccountId = useMemo(() => {
    const next = new Map<string, NonNullable<CodexLocalAccessState['stats']>['accounts'][number]>();
    selectedStatsWindow?.accounts.forEach((item) => next.set(item.accountId, item));
    return next;
  }, [selectedStatsWindow?.accounts]);

  const currentMemberStats = useMemo(() => {
    const currentIds = collection?.accountIds ?? [];
    return currentIds
      .map((accountId) => {
        const account = oauthAccounts.find((item) => item.id === accountId);
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
  }, [collection?.accountIds, oauthAccounts, t, windowStatsByAccountId]);

  const routingStrategyOptions = useMemo(
    () => [
      {
        value: 'auto',
        label: t('codex.localAccess.routingStrategy.auto', '自动（推荐）'),
      },
      {
        value: 'quota_high_first',
        label: t('codex.localAccess.routingStrategy.quotaHighFirst', '优先高配额'),
      },
      {
        value: 'quota_low_first',
        label: t('codex.localAccess.routingStrategy.quotaLowFirst', '优先低配额'),
      },
      {
        value: 'plan_high_first',
        label: t('codex.localAccess.routingStrategy.planHighFirst', '优先高订阅'),
      },
      {
        value: 'plan_low_first',
        label: t('codex.localAccess.routingStrategy.planLowFirst', '优先低订阅'),
      },
      {
        value: 'expiry_soon_first',
        label: t('codex.localAccess.routingStrategy.expirySoonFirst', '优先近到期'),
      },
    ] satisfies Array<{ value: CodexLocalAccessRoutingStrategy; label: string }>,
    [t],
  );
  const accessScopeOptions = useMemo(
    () => [
      {
        value: 'localhost',
        label: t('codex.localAccess.accessScopeLocalhost', '仅本机'),
      },
      {
        value: 'lan',
        label: t('codex.localAccess.accessScopeLan', '局域网'),
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

  const oauthAccountById = useMemo(
    () => new Map(oauthAccounts.map((account) => [account.id, account])),
    [oauthAccounts],
  );

  const handleCopy = async (field: CopyableField, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      window.setTimeout(
        () => setCopiedField((current) => (current === field ? null : current)),
        1200,
      );
    } catch (err) {
      setError(t('common.shared.export.copyFailed', '复制失败，请手动复制'));
      console.error('Failed to copy local access value:', err);
    }
  };

  const runAction = async (task: () => Promise<void>, successText: string) => {
    setError('');
    setNotice('');
    try {
      await task();
      setNotice(successText);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const toggleSelectAllVisible = () => {
    if (actionBusy || visibleSelectableAccounts.length === 0) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (allVisibleSelected) {
        for (const account of visibleSelectableAccounts) {
          next.delete(account.id);
        }
      } else {
        for (const account of visibleSelectableAccounts) {
          next.add(account.id);
        }
      }
      return next;
    });
  };

  const handleToggleRestrictFreeAccounts = async () => {
    if (actionBusy) return;
    setRestrictFreeAccounts((prev) => !prev);
  };

  const toggleSelect = (accountId: string) => {
    if (actionBusy) return;
    const account = oauthAccountById.get(accountId);
    if (!account) return;
    setSelected((prev) => {
      const isFreeAccount = isCodexExplicitFreePlanType(account.plan_type);
      if (isFreeAccount && restrictFreeAccounts && !prev.has(accountId)) {
        return prev;
      }
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
    setError('');
    setNotice('');
    try {
      const filtered = Array.from(selected).filter((accountId) => {
        const account = oauthAccountById.get(accountId);
        if (!account) return false;
        if (restrictFreeAccounts && isCodexExplicitFreePlanType(account.plan_type)) {
          return false;
        }
        return true;
      });
      await onSaveAccounts({
        accountIds: filtered,
        restrictFreeAccounts,
      });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleSavePort = async () => {
    const nextPort = Number(portInput.trim());
    if (!Number.isInteger(nextPort) || nextPort <= 0 || nextPort > 65535) {
      setError(t('codex.localAccess.portInvalid', '请输入 1 到 65535 之间的端口'));
      return;
    }

    await runAction(
      async () => {
        await onUpdatePort(nextPort);
      },
      t('codex.localAccess.portSaveSuccess', 'API 服务端口已更新'),
    );
  };

  const handleChangeRoutingStrategy = async (nextStrategy: string) => {
    if (!collection) return;
    if (nextStrategy === routingStrategy) return;

    await runAction(
      async () => {
        await onUpdateRoutingStrategy(nextStrategy as CodexLocalAccessRoutingStrategy);
      },
      t('codex.localAccess.routingSaveSuccess', 'API 服务调度策略已更新'),
    );
  };

  const handleChangeAccessScope = async (nextValue: string) => {
    if (!collection) return;
    const nextAccessScope = normalizeAccessScope(nextValue);
    if (nextAccessScope === accessScope) return;

    await runAction(
      async () => {
        await onUpdateAccessScope(nextAccessScope);
      },
      t('codex.localAccess.accessScopeSaveSuccess', 'API 服务访问范围已更新'),
    );
  };

  const handleResetKey = async () => {
    const confirmed = await confirmDialog(
      t(
        'codex.localAccess.rotateConfirmMessage',
        '重置后当前 API 服务密钥会立即失效，正在进行中的请求可能不可用。确认继续吗？',
      ),
      {
        title: t('codex.localAccess.rotateKey', '重置密钥'),
        kind: 'warning',
        okLabel: t('common.confirm'),
        cancelLabel: t('common.cancel'),
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
      t('codex.localAccess.rotateSuccess', 'API 服务密钥已重置'),
    );
  };

  const handleClearStats = async () => {
    const confirmed = await confirmDialog(
      t('codex.localAccess.clearStatsConfirm', '确定要清空 API 服务统计吗？'),
      {
        title: t('codex.localAccess.clearStats', '清除统计'),
        kind: 'warning',
        okLabel: t('common.confirm'),
        cancelLabel: t('common.cancel'),
      },
    );

    if (!confirmed) {
      return;
    }

    await runAction(async () => {
      await onClearStats();
    }, t('codex.localAccess.clearStatsSuccess', 'API 服务统计已清空'));
  };

  const handleKillPort = async () => {
    await runAction(
      async () => {
        await onKillPort();
      },
      t('codex.localAccess.killPortSuccessUnknown', 'API 服务端口已清理'),
    );
  };

  const handleRefreshStats = async () => {
    setError('');
    setNotice('');
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
        ? t('codex.localAccess.disabledSuccess', 'API 服务已停用')
        : t('codex.localAccess.enabledSuccess', 'API 服务已启用'),
    );
  };

  const closeTestDialog = () => {
    if (testDialogBusy) return;
    setTestDialogOpen(false);
  };

  const handleTest = async () => {
    setTestDialogOpen(true);
    setTestDialogRunning(true);
    setTestDialogResult(null);
    setTestDialogError('');
    try {
      const result = await onTest();
      setTestDialogResult(result);
    } catch (err) {
      setTestDialogError(err instanceof Error ? err.message : String(err));
    } finally {
      setTestDialogRunning(false);
    }
  };

  if (!isOpen) return null;
  const testFailure = testDialogResult?.failure ?? null;
  const testSuccessResult = testDialogResult && !testFailure ? testDialogResult : null;
  const isMembersMode = mode === 'members';

  return (
    <>
      <div
        className={`modal-overlay codex-local-access-modal-overlay${
          isMembersMode ? '' : ' codex-local-access-modal-overlay-panel'
        }`}
        onClick={onClose}
      >
      <div
        className={`modal codex-local-access-modal${
          isMembersMode
            ? ' codex-local-access-modal-members group-account-picker-modal'
            : ' codex-local-access-modal-panel'
        }`}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-header codex-local-access-modal-header">
          <div className="codex-local-access-header-main">
            <h2 className="group-account-picker-title">
              <Server size={18} />
              <span>
                {isMembersMode
                  ? t('codex.localAccess.entryAction', '添加至 API 服务')
                  : t('codex.localAccess.title', 'API 服务')}
              </span>
            </h2>
            {!isMembersMode && (
              <div className="codex-local-access-header-meta">
                <div className="codex-local-access-header-badges">
                  <span
                    className={`codex-local-access-status ${
                      state?.running ? 'running' : 'stopped'
                    }`}
                  >
                    {collection?.enabled
                      ? state?.running
                        ? t('codex.localAccess.statusRunning', '运行中')
                        : t('codex.localAccess.statusStopped', '未运行')
                      : t('codex.localAccess.statusDisabled', '已停用')}
                  </span>
                  <span className="codex-local-access-subtle-badge">
                    {accessScopeBadge}
                  </span>
                  <button
                    type="button"
                    className="codex-local-access-test-pill"
                    onClick={() => void handleTest()}
                    disabled={!collection || testDialogBusy || saving}
                    title={t('codex.localAccess.testDialogTitle', '测试 API 服务')}
                    aria-label={t('codex.localAccess.testDialogTitle', '测试 API 服务')}
                  >
                    <ShieldCheck size={13} className={testDialogBusy ? 'loading-spinner' : ''} />
                    <span>{t('codex.localAccess.testAction', '测试')}</span>
                  </button>
                </div>
                <div className="codex-local-access-header-tools">
                  <button
                    type="button"
                    className="folder-icon-btn codex-local-access-toolbar-btn"
                    onClick={() => void handleRefreshStats()}
                    disabled={!collection || actionBusy}
                    title={t('codex.localAccess.refreshStats', '刷新统计')}
                    aria-label={t('codex.localAccess.refreshStats', '刷新统计')}
                  >
                    <RefreshCw size={14} className={saving ? 'loading-spinner' : ''} />
                  </button>
                  {collection && (
                    <div className="codex-local-access-header-routing">
                      <SingleSelectDropdown
                        value={routingStrategy}
                        options={routingStrategyOptions}
                        onChange={(value) => void handleChangeRoutingStrategy(value)}
                        disabled={saving || testing || starting}
                        ariaLabel={t('codex.localAccess.routingLabel', '调度策略')}
                      />
                    </div>
                  )}
                  <button
                    type="button"
                    className={`folder-icon-btn codex-local-access-toolbar-btn ${
                      collection?.enabled ? 'is-danger' : 'is-primary'
                    }`}
                    onClick={() => void handleToggleEnabled()}
                    disabled={!collection || saving || testing || starting}
                    title={
                      collection?.enabled
                        ? t('codex.localAccess.disableService', '停用服务')
                        : t('codex.localAccess.enableService', '启用服务')
                    }
                    aria-label={
                      collection?.enabled
                        ? t('codex.localAccess.disableService', '停用服务')
                        : t('codex.localAccess.enableService', '启用服务')
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
            aria-label={t('common.close')}
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
                  {t('codex.localAccess.killPortAction', '清理端口')}
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
                  <span>{t('codex.localAccess.statsTitle', '总量统计')}</span>
                </div>
                <div className="codex-local-access-summary-actions">
                  <div
                    className="codex-local-access-stats-range-tabs"
                    role="tablist"
                    aria-label={t('codex.localAccess.statsRange.label', '统计范围')}
                  >
                    {statsRangeOptions.map((option) => (
                      <button
                        key={option.key}
                        type="button"
                        role="tab"
                        className={`codex-local-access-stats-range-tab${
                          statsRange === option.key ? ' is-active' : ''
                        }`}
                        aria-selected={statsRange === option.key}
                        onClick={() => setStatsRange(option.key)}
                        disabled={actionBusy}
                      >
                        {option.label}
                      </button>
                    ))}
                  </div>
                  <button
                    type="button"
                    className="btn btn-danger btn-sm"
                    onClick={() => void handleClearStats()}
                    disabled={!collection || actionBusy}
                    title={t('codex.localAccess.clearStats', '清除统计')}
                    aria-label={t('codex.localAccess.clearStats', '清除统计')}
                  >
                    <Trash2 size={14} />
                    {t('codex.localAccess.clearStats', '清除统计')}
                  </button>
                </div>
              </div>
              <div className="codex-local-access-stats-grid">
                {summaryStats.map((item) => (
                  <div
                    key={item.key}
                    className={`codex-local-access-stat-card codex-local-access-stat-card-${item.key}`}
                  >
                    <span className="codex-local-access-stat-label">{item.label}</span>
                    <strong>{item.value}</strong>
                    <span className="codex-local-access-stat-sub">{item.detail}</span>
                  </div>
                ))}
              </div>
              {currentQuotaPoolSummary.visiblePlans.length > 0 && (
                <div
                  className="codex-local-access-quota-pool-grid"
                  aria-label={quotaPoolLabels.title}
                >
                  {currentQuotaPoolSummary.visiblePlans.map((item) => (
                    <div key={item.key} className="codex-local-access-quota-pool-card">
                      <span className="codex-local-access-quota-pool-plan">
                        {item.key} ({item.count})
                      </span>
                      <span className="codex-local-access-quota-pool-value">
                        {quotaPoolLabels.hourly} {formatCodexQuotaPoolPercent(item.hourly)}
                      </span>
                      <span className="codex-local-access-quota-pool-value">
                        {quotaPoolLabels.weekly} {formatCodexQuotaPoolPercent(item.weekly)}
                      </span>
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
                  <span>{t('codex.localAccess.configTitle', '服务配置')}</span>
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
                            ariaLabel={t('codex.localAccess.addressKind', '地址类型')}
                          />
                        </div>
                        <div className="codex-local-access-config-actions">
                          <button
                            type="button"
                            className="folder-icon-btn"
                            onClick={() => void handleCopy('baseUrl', displayBaseUrl)}
                            title={t('common.copy', '复制')}
                          >
                            {copiedField === 'baseUrl' ? <Check size={14} /> : <Copy size={14} />}
                          </button>
                        </div>
                      </div>
                      <code className="codex-local-access-code" title={displayBaseUrl}>
                        {displayBaseUrl}
                      </code>
                    </div>

                    <div className="codex-local-access-config-card codex-local-access-config-card-key">
                      <div className="codex-local-access-config-head">
                        <span className="codex-local-access-config-label">
                          {t('codex.localAccess.apiKey', '密钥')}
                        </span>
                        <div className="codex-local-access-config-actions">
                          <button
                            type="button"
                            className="folder-icon-btn"
                            onClick={() => setKeyVisible((prev) => !prev)}
                            title={
                              keyVisible
                                ? t('codex.localAccess.hideKey', '隐藏密钥')
                                : t('codex.localAccess.showKey', '显示密钥')
                            }
                          >
                            {keyVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                          </button>
                          <button
                            type="button"
                            className="folder-icon-btn"
                            onClick={() => void handleCopy('apiKey', collection.apiKey)}
                            title={t('common.copy', '复制')}
                          >
                            {copiedField === 'apiKey' ? <Check size={14} /> : <Copy size={14} />}
                          </button>
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={() => void handleResetKey()}
                            disabled={saving || testing || starting}
                          >
                            {saving ? (
                              <RefreshCw size={14} className="loading-spinner" />
                            ) : (
                              <RefreshCw size={14} />
                            )}
                            {t('codex.localAccess.rotateKey', '重置密钥')}
                          </button>
                        </div>
                      </div>
                      <code className="codex-local-access-code" title={collection.apiKey}>
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
                          {t('codex.localAccess.portLabel', '服务端口')}
                        </label>
                        <div className="codex-local-access-config-actions">
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={() => void handleSavePort()}
                            disabled={saving || testing || starting}
                          >
                            {saving ? (
                              <RefreshCw size={14} className="loading-spinner" />
                            ) : (
                              <Gauge size={14} />
                            )}
                            {t('codex.localAccess.portSave', '保存端口')}
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
                          onChange={(event) => setPortInput(event.target.value)}
                          disabled={saving || testing || starting}
                        />
                      </div>
                    </div>

                    <div className="codex-local-access-config-card codex-local-access-config-card-scope">
                      <div className="codex-local-access-config-head">
                        <span className="codex-local-access-config-label">
                          {t('codex.localAccess.accessScopeLabel', '访问范围')}
                        </span>
                        <div className="codex-local-access-config-actions">
                          <SingleSelectDropdown
                            value={accessScope}
                            options={accessScopeOptions}
                            onChange={(value) => void handleChangeAccessScope(value)}
                            menuClassName="codex-local-access-scope-menu"
                            menuWidth={132}
                            menuMaxHeight={120}
                            disabled={saving || testing || starting}
                            ariaLabel={t('codex.localAccess.accessScopeLabel', '访问范围')}
                          />
                        </div>
                      </div>
                      <code className="codex-local-access-code" title={accessScopeAddress}>
                        {accessScopeAddress}
                      </code>
                    </div>
                  </div>
                ) : (
                  <div className="group-account-empty">
                    {t(
                      'codex.localAccess.configEmpty',
                      '先把账号保存到 API 服务集合，随后会自动生成地址、密钥和端口。',
                    )}
                  </div>
                )}
                {collection || modelIdOptions.length > 0 ? (
                  <div className="codex-local-access-config-extra-grid">
                    {collection ? (
                      <div className="codex-local-access-config-card codex-local-access-config-card-root">
                        <div className="codex-local-access-config-head">
                          <span className="codex-local-access-config-label">
                            {t('codex.localAccess.apiPortUrl', 'API端口URL')}
                          </span>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() => void handleCopy('apiPortUrl', apiPortUrl)}
                              title={t('common.copy', '复制')}
                            >
                              {copiedField === 'apiPortUrl' ? <Check size={14} /> : <Copy size={14} />}
                            </button>
                          </div>
                        </div>
                        <code className="codex-local-access-code" title={apiPortUrl}>
                          {apiPortUrl}
                        </code>
                      </div>
                    ) : null}

                    {modelIdOptions.length > 0 ? (
                      <div className="codex-local-access-config-card codex-local-access-config-card-model">
                        <div className="codex-local-access-config-head">
                          <span className="codex-local-access-config-label">
                            {t('codex.localAccess.modelId', '模型 ID')}
                          </span>
                          <span className="codex-local-access-view-only-badge">
                            {t('codex.localAccess.modelIdViewOnly', '仅查看使用，无切换功能')}
                          </span>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() => void handleCopy('modelId', selectedModelId)}
                              title={t('common.copy', '复制')}
                              disabled={!selectedModelId}
                            >
                              {copiedField === 'modelId' ? <Check size={14} /> : <Copy size={14} />}
                            </button>
                          </div>
                        </div>
                        <div className="codex-local-access-model-row">
                          <SingleSelectDropdown
                            value={selectedModelId}
                            options={modelIdOptions}
                            onChange={setSelectedModelId}
                            disabled={modelIdOptions.length === 0}
                            ariaLabel={t('codex.localAccess.modelId', '模型 ID')}
                            placeholder={t('codex.localAccess.modelIdPlaceholder', '选择模型 ID')}
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
                  <span>{t('codex.localAccess.accountStatsTitle', '按账号统计')}</span>
                </div>
                <div className="codex-local-access-account-stats">
                  {currentMemberStats.length === 0 ? (
                    <div className="group-account-empty">
                      {t('codex.localAccess.statsEmpty', '当前还没有统计数据')}
                    </div>
                  ) : (
                    currentMemberStats.map(({ account, presentation, stats: accountStats }) => (
                      <div key={account.id} className="codex-local-access-account-stat-row">
                        <div className="codex-local-access-account-stat-top">
                          <div className="codex-local-access-account-stat-main">
                            <span
                              className="group-account-email"
                              title={maskAccountText(presentation.displayName)}
                            >
                              {maskAccountText(presentation.displayName)}
                            </span>
                            <span className={`tier-badge ${presentation.planClass}`}>
                              {presentation.planLabel}
                            </span>
                          </div>
                          <div className="codex-local-access-account-stat-block codex-local-access-account-stat-block-quota">
                            {renderQuotaPreview(presentation, 3)}
                          </div>
                          <div className="codex-local-access-account-stat-block codex-local-access-account-stat-block-metrics">
                            <div className="codex-local-access-account-stat-metrics">
                              <span className="codex-local-access-account-stat-pill">
                                {t('codex.localAccess.stats.accountResult', {
                                  success: accountStats?.successCount ?? 0,
                                  failed: accountStats?.failureCount ?? 0,
                                  defaultValue: '成功 {{success}} / 失败 {{failed}}',
                                })}
                              </span>
                              <span className="codex-local-access-account-stat-pill">
                                {(accountStats?.totalTokens ?? 0) === 0
                                  ? t('codex.localAccess.stats.accountTokens', {
                                      count: 0,
                                      defaultValue: '0 Tokens',
                                    })
                                  : t('codex.localAccess.stats.accountTokensCompact', {
                                      value: formatCompactNumber(accountStats?.totalTokens ?? 0),
                                      defaultValue: '{{value}}',
                                    })}
                              </span>
                            </div>
                          </div>
                        </div>
                      </div>
                    ))
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
                  <span>{t('codex.localAccess.memberTitle', '集合成员')}</span>
                </div>
                <label className="codex-local-access-free-toggle">
                  <input
                    type="checkbox"
                    checked={restrictFreeAccounts}
                    onChange={() => void handleToggleRestrictFreeAccounts()}
                    disabled={actionBusy}
                  />
                  <span>
                    {t(
                      'codex.localAccess.modal.restrictFreeToggle',
                      '限制 Free 账号使用',
                    )}
                  </span>
                </label>
              </div>

              <div className="group-account-toolbar">
                <div className="group-account-search">
                  <Search size={16} className="group-account-search-icon" />
                  <input
                    ref={searchInputRef}
                    type="text"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder={t('accounts.search')}
                  />
                </div>
                <div className="group-account-picker-filters">
                  <MultiSelectFilterDropdown
                    options={tierFilterOptions}
                    selectedValues={filterTypes}
                    allLabel={allTierFilterLabel}
                    filterLabel={t('common.shared.filterLabel', '筛选')}
                    clearLabel={t('accounts.clearFilter', '清空筛选')}
                    emptyLabel={t('common.none', '暂无')}
                    ariaLabel={t('common.shared.filterLabel', '筛选')}
                    onToggleValue={(value) =>
                      setFilterTypes((prev) =>
                        prev.includes(value)
                          ? prev.filter((item) => item !== value)
                          : [...prev, value],
                      )
                    }
                    onClear={() => setFilterTypes([])}
                  />
                  <AccountTagFilterDropdown
                    availableTags={availableTags}
                    selectedTags={tagFilter}
                    onToggleTag={(value) =>
                      setTagFilter((prev) =>
                        prev.includes(value)
                          ? prev.filter((item) => item !== value)
                          : [...prev, value],
                      )
                    }
                    onClear={() => setTagFilter([])}
                  />
                  <MultiSelectFilterDropdown
                    options={groupFilterOptions}
                    selectedValues={groupFilter}
                    allLabel={t('accounts.groups.allGroups', '全部分组')}
                    filterLabel={t('accounts.groups.manageTitle', '分组管理')}
                    clearLabel={t('accounts.clearFilter', '清空筛选')}
                    emptyLabel={t('common.none', '暂无')}
                    ariaLabel={t('accounts.groups.manageTitle', '分组管理')}
                    onToggleValue={(value) =>
                      setGroupFilter((prev) =>
                        prev.includes(value)
                          ? prev.filter((item) => item !== value)
                          : [...prev, value],
                      )
                    }
                    onClear={() => setGroupFilter([])}
                  />
                </div>
              </div>

              <div className="group-account-item group-account-item-header">
                <input
                  ref={selectAllCheckboxRef}
                  type="checkbox"
                  checked={allVisibleSelected}
                  onChange={toggleSelectAllVisible}
                  disabled={actionBusy || visibleSelectableAccounts.length === 0}
                />
                <div className="group-account-main" />
              </div>

              <div className="group-account-list codex-local-access-member-list">
                {oauthAccounts.length === 0 ? (
                  <div className="group-account-empty">
                    {t('codex.localAccess.modal.empty', '暂无可加入的 OAuth 账号')}
                  </div>
                ) : visibleAccounts.length === 0 ? (
                  <div className="group-account-empty">
                    {t('common.shared.noMatch.title', '没有匹配的账号')}
                  </div>
                ) : (
                  visibleAccounts.map((account) => {
                    const presentation = buildCodexAccountPresentation(account, t);
                    const isChecked = selected.has(account.id);
                    const isFreeAccount = isCodexExplicitFreePlanType(account.plan_type);
                    const isFreeSelectionBlocked =
                      isFreeAccount && restrictFreeAccounts && !isChecked;
                    const accountStats = allStatsByAccountId.get(account.id)?.usage;

                    return (
                      <label
                        key={account.id}
                        className={`group-account-item${isChecked ? ' is-current' : ''}${
                          isFreeSelectionBlocked ? ' is-disabled' : ''
                        }`}
                      >
                        <input
                          type="checkbox"
                          checked={isChecked}
                          disabled={actionBusy || isFreeSelectionBlocked}
                          onChange={() => toggleSelect(account.id)}
                        />
                        <div className="group-account-main">
                        <div className="codex-local-access-member-mainline">
                          <span
                            className="group-account-email"
                            title={maskAccountText(presentation.displayName)}
                          >
                              {maskAccountText(presentation.displayName)}
                            </span>
                          <span className={`tier-badge ${presentation.planClass}`}>
                              {presentation.planLabel}
                            </span>
                          <span className="codex-local-access-member-metric">
                            {t('codex.localAccess.stats.accountRequests', {
                              count: accountStats?.requestCount ?? 0,
                              defaultValue: '{{count}} 次请求',
                            })}
                          </span>
                          {renderQuotaPreview(presentation, 2)}
                        </div>
                        </div>
                      </label>
                    );
                  })
                )}
              </div>
            </section>
          )}
        </div>

        <div className="modal-footer group-account-picker-footer codex-local-access-modal-footer">
          {isMembersMode ? (
            <>
              <button className="btn btn-secondary" onClick={onClose} disabled={actionBusy}>
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleSaveMembers()}
                disabled={actionBusy || !selectionDirty}
              >
                {saving ? t('common.saving') : t('codex.localAccess.modal.save', '保存集合')}
              </button>
            </>
          ) : (
            <button className="btn btn-secondary" onClick={onClose} disabled={actionBusy}>
              {t('common.close')}
            </button>
          )}
        </div>
      </div>
      </div>

      {testDialogOpen && (
        <div
          className="modal-overlay codex-local-access-test-dialog-overlay"
          onClick={closeTestDialog}
        >
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
                  <span>{t('codex.localAccess.testDialogTitle', '测试 API 服务')}</span>
                </h3>
                <p>{t('codex.localAccess.testDialogDesc', '通过 Codex CLI 发起一次真实请求，验证本地服务、密钥和上游响应。')}</p>
              </div>
              <button
                className="modal-close codex-local-access-test-dialog-close"
                onClick={closeTestDialog}
                disabled={testDialogBusy}
                aria-label={t('common.close')}
              >
                <X size={18} />
              </button>
            </div>

            <div className="modal-body codex-local-access-test-dialog-body">
              {testDialogBusy && (
                <div className="codex-local-access-test-progress" aria-live="polite">
                  <RefreshCw size={18} className="loading-spinner" />
                  <div>
                    <strong>{t('codex.localAccess.testProgressTitle', '正在测试服务')}</strong>
                    <span>{t('codex.localAccess.testProgressDesc', '正在向本地 API 服务发起真实请求，请稍候。')}</span>
                  </div>
                </div>
              )}

              {testSuccessResult && (
                <>
                  {testSuccessResult.output && (
                    <div className="codex-local-access-test-output">
                      <div className="codex-local-access-test-section-title">
                        {t('codex.localAccess.testOutputTitle', '返回结果')}
                      </div>
                      <pre>{testSuccessResult.output}</pre>
                    </div>
                  )}

                  <div className="codex-local-access-test-result">
                    <div className="codex-local-access-test-section-title">
                      {t('codex.localAccess.testResultTitle', '服务检测结果')}
                    </div>
                    <div className="codex-local-access-inline-success">
                      <Check size={14} />
                      <span>
                        {t('codex.localAccess.testSuccess', {
                          model: testSuccessResult.modelId ?? '--',
                          latency: formatLatencyMs(testSuccessResult.latencyMs ?? 0),
                          defaultValue:
                            '服务检测成功：本地服务、密钥和上游响应正常（{{model}}，{{latency}}）。如果 Codex 仍报 502，请检查 Codex 的 Base URL、API Key 或运行环境。',
                        })}
                      </span>
                    </div>
                  </div>
                </>
              )}

              {testFailure && (
                <div className="codex-local-access-test-failure" aria-live="assertive">
                  <div className="codex-local-access-test-failure-title">
                    <CircleAlert size={16} />
                    <strong>{testFailure.title}</strong>
                  </div>
                  <dl>
                    <div>
                      <dt>{t('codex.localAccess.testFailureStage', '定位阶段')}</dt>
                      <dd>{testFailure.stage}</dd>
                    </div>
                    <div>
                      <dt>{t('codex.localAccess.testFailureCause', '失败原因')}</dt>
                      <dd>{testFailure.cause}</dd>
                    </div>
                    <div>
                      <dt>{t('codex.localAccess.testFailureSuggestion', '处理建议')}</dt>
                      <dd>{testFailure.suggestion}</dd>
                    </div>
                    {typeof testFailure.status === 'number' && (
                      <div>
                        <dt>{t('codex.localAccess.testFailureStatus', 'HTTP 状态')}</dt>
                        <dd>{testFailure.status}</dd>
                      </div>
                    )}
                    {testFailure.modelId && (
                      <div>
                        <dt>{t('codex.localAccess.testFailureModel', '检测模型')}</dt>
                        <dd>{testFailure.modelId}</dd>
                      </div>
                    )}
                  </dl>
                  {testFailure.detail && (
                    <div className="codex-local-access-test-output">
                      <div className="codex-local-access-test-section-title">
                        {t('codex.localAccess.testFailureDetail', '诊断详情')}
                      </div>
                      <pre>{testFailure.detail}</pre>
                    </div>
                  )}
                  {testFailure.gatewayOutput && (
                    <div className="codex-local-access-test-output">
                      <div className="codex-local-access-test-section-title">
                        {t('codex.localAccess.testFailureGatewayOutput', '网关返回')}
                      </div>
                      <pre>{testFailure.gatewayOutput}</pre>
                    </div>
                  )}
                  {testFailure.cliOutput && (
                    <div className="codex-local-access-test-output">
                      <div className="codex-local-access-test-section-title">
                        {t('codex.localAccess.testFailureCliOutput', 'CLI 输出')}
                      </div>
                      <pre>{testFailure.cliOutput}</pre>
                    </div>
                  )}
                </div>
              )}

              {testDialogError && (
                <div className="codex-local-access-inline-error" aria-live="assertive">
                  <CircleAlert size={14} />
                  <span>{testDialogError}</span>
                </div>
              )}
            </div>

            <div className="modal-footer codex-local-access-test-dialog-footer">
              <button
                className="btn btn-secondary"
                onClick={closeTestDialog}
                disabled={testDialogBusy}
              >
                {t('common.close')}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

export default CodexLocalAccessModal;
