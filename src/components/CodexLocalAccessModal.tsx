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
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessState,
  CodexLocalAccessStatsWindow,
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
import './GroupAccountPickerModal.css';
import './CodexLocalAccessModal.css';

interface CodexLocalAccessModalProps {
  isOpen: boolean;
  mode: 'panel' | 'members';
  state: CodexLocalAccessState | null;
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
  onRotateApiKey: () => Promise<unknown> | unknown;
  onKillPort: () => Promise<unknown> | unknown;
  onToggleEnabled: () => Promise<unknown> | unknown;
  onTest: () => Promise<number> | number;
  saving: boolean;
  testing: boolean;
  starting: boolean;
  portCleanupBusy: boolean;
}

type StatsRangeKey = 'daily' | 'weekly' | 'monthly';
type CopyableField = 'apiPortUrl' | 'baseUrl' | 'apiKey' | 'modelId';
const CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY =
  'agtools.codex.local_access.stats_range.v1';

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
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filterTypes, setFilterTypes] = useState<string[]>([]);
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const [groupFilter, setGroupFilter] = useState<string[]>([]);
  const [restrictFreeAccounts, setRestrictFreeAccounts] = useState(true);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
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
  const modelIds = state?.modelIds ?? [];
  const stats = state?.stats;
  const statsRangeOptions = useMemo(
    () =>
      [
        { key: 'daily', label: t('codex.localAccess.statsRange.daily', "Day") },
        { key: 'weekly', label: t('codex.localAccess.statsRange.weekly', "Week") },
        { key: 'monthly', label: t('codex.localAccess.statsRange.monthly', "Month") },
      ] satisfies Array<{ key: StatsRangeKey; label: string }>,
    [t],
  );
  const quotaPoolLabels = useMemo(
    () => ({
      hourly: t('codex.localAccess.quotaPool.hourlyShort', '5h'),
      weekly: t('codex.localAccess.quotaPool.weeklyShort', "Week"),
      title: t('codex.localAccess.quotaPool.title', "Quota Pool"),
    }),
    [t],
  );
  const selectedStatsWindow = useMemo<CodexLocalAccessStatsWindow | null>(() => {
    if (!stats) return null;
    return stats[statsRange];
  }, [stats, statsRange]);
  const selectedTotals = selectedStatsWindow?.totals;
  const routingStrategy = collection?.routingStrategy ?? 'auto';
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
  const actionBusy = saving || testing || starting || portCleanupBusy;
  const summaryStats = useMemo(
    () => [
      {
        key: 'requests',
        label: t('codex.localAccess.stats.requests', "Requests"),
        value: formatCompactNumber(selectedTotals?.requestCount ?? 0),
        detail: t('codex.localAccess.stats.requestsDetail', {
          success: formatCompactNumber(selectedTotals?.successCount ?? 0),
          failed: formatCompactNumber(selectedTotals?.failureCount ?? 0),
          defaultValue: '成功 {{success}} / 失败 {{failed}}',
        }),
      },
      {
        key: 'tokens',
        label: t('codex.localAccess.stats.tokens', "Tokens"),
        value: formatCompactNumber(selectedTotals?.totalTokens ?? 0),
        detail: t('codex.localAccess.stats.tokensDetail', {
          input: formatCompactNumber(selectedTotals?.inputTokens ?? 0),
          output: formatCompactNumber(selectedTotals?.outputTokens ?? 0),
          defaultValue: '输入 {{input}} / 输出 {{output}}',
        }),
      },
      {
        key: 'specialTokens',
        label: t('codex.localAccess.stats.specialTokens', "Cache / Reasoning"),
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
        label: t('codex.localAccess.stats.avgLatency', "Avg Latency"),
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
        label: t('codex.localAccess.routingStrategy.auto', "Auto (Recommended)"),
      },
      {
        value: 'quota_high_first',
        label: t('codex.localAccess.routingStrategy.quotaHighFirst', "High Quota First"),
      },
      {
        value: 'quota_low_first',
        label: t('codex.localAccess.routingStrategy.quotaLowFirst', "Low Quota First"),
      },
      {
        value: 'plan_high_first',
        label: t('codex.localAccess.routingStrategy.planHighFirst', "High Plan First"),
      },
      {
        value: 'plan_low_first',
        label: t('codex.localAccess.routingStrategy.planLowFirst', "Low Plan First"),
      },
      {
        value: 'expiry_soon_first',
        label: t('codex.localAccess.routingStrategy.expirySoonFirst', "Expiry Soon First"),
      },
    ] satisfies Array<{ value: CodexLocalAccessRoutingStrategy; label: string }>,
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
      setError(t('common.shared.export.copyFailed', "Copy failed, please copy manually"));
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
      setError(t('codex.localAccess.portInvalid', "Enter a port between 1 and 65535"));
      return;
    }

    await runAction(
      async () => {
        await onUpdatePort(nextPort);
      },
      t('codex.localAccess.portSaveSuccess', "API service port updated"),
    );
  };

  const handleChangeRoutingStrategy = async (nextStrategy: string) => {
    if (!collection) return;
    if (nextStrategy === routingStrategy) return;

    await runAction(
      async () => {
        await onUpdateRoutingStrategy(nextStrategy as CodexLocalAccessRoutingStrategy);
      },
      t('codex.localAccess.routingSaveSuccess', "API service routing updated"),
    );
  };

  const handleResetKey = async () => {
    const confirmed = await confirmDialog(
      t(
        'codex.localAccess.rotateConfirmMessage',
        "The current API service key will stop working immediately after reset, and in-flight requests may become unavailable. Continue?",
      ),
      {
        title: t('codex.localAccess.rotateKey', "Reset Key"),
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
      t('codex.localAccess.rotateSuccess', "API service key reset"),
    );
  };

  const handleClearStats = async () => {
    const confirmed = await confirmDialog(
      t('codex.localAccess.clearStatsConfirm', "Clear all API service stats?"),
      {
        title: t('codex.localAccess.clearStats', "Clear Stats"),
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
    }, t('codex.localAccess.clearStatsSuccess', "API service stats have been cleared"));
  };

  const handleKillPort = async () => {
    await runAction(
      async () => {
        await onKillPort();
      },
      t('codex.localAccess.killPortSuccessUnknown', "API service port cleared"),
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
        ? t('codex.localAccess.disabledSuccess', "API service disabled")
        : t('codex.localAccess.enabledSuccess', "API service enabled"),
    );
  };

  const handleTest = async () => {
    setError('');
    setNotice('');
    try {
      const modelCount = await onTest();
      setNotice(
        t('codex.localAccess.testSuccess', {
          count: modelCount,
          defaultValue:
            modelCount > 0 ? 'API 服务测试成功（{{count}} 个模型）' : 'API 服务测试成功',
        }),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  if (!isOpen) return null;
  const isMembersMode = mode === 'members';

  return (
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
                  ? t('codex.localAccess.entryAction', "Add to API Service")
                  : t('codex.localAccess.title', "API Service")}
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
                        ? t('codex.localAccess.statusRunning', "Running")
                        : t('codex.localAccess.statusStopped', "Stopped")
                      : t('codex.localAccess.statusDisabled', "Disabled")}
                  </span>
                  <span className="codex-local-access-subtle-badge">
                    {t('codex.localAccess.memberOnlyLocal', "Local/LAN")}
                  </span>
                </div>
                <div className="codex-local-access-header-tools">
                  <button
                    type="button"
                    className="folder-icon-btn codex-local-access-toolbar-btn"
                    onClick={() => void handleRefreshStats()}
                    disabled={!collection || actionBusy}
                    title={t('codex.localAccess.refreshStats', "Refresh Stats")}
                    aria-label={t('codex.localAccess.refreshStats', "Refresh Stats")}
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
                        ariaLabel={t('codex.localAccess.routingLabel', "Routing")}
                      />
                    </div>
                  )}
                  <button
                    type="button"
                    className="folder-icon-btn codex-local-access-toolbar-btn"
                    onClick={() => void handleTest()}
                    disabled={!collection || testing || saving}
                    title={t('codex.localAccess.testAction', "Test API Service")}
                    aria-label={t('codex.localAccess.testAction', "Test API Service")}
                  >
                    <ShieldCheck size={14} className={testing ? 'loading-spinner' : ''} />
                  </button>
                  <button
                    type="button"
                    className={`folder-icon-btn codex-local-access-toolbar-btn ${
                      collection?.enabled ? 'is-danger' : 'is-primary'
                    }`}
                    onClick={() => void handleToggleEnabled()}
                    disabled={!collection || saving || testing || starting}
                    title={
                      collection?.enabled
                        ? t('codex.localAccess.disableService', "Disable Service")
                        : t('codex.localAccess.enableService', "Enable Service")
                    }
                    aria-label={
                      collection?.enabled
                        ? t('codex.localAccess.disableService', "Disable Service")
                        : t('codex.localAccess.enableService', "Enable Service")
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
                  {t('codex.localAccess.killPortAction', "Clear Port")}
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
                  <span>{t('codex.localAccess.statsTitle', "Totals")}</span>
                </div>
                <div className="codex-local-access-summary-actions">
                  <div
                    className="codex-local-access-stats-range-tabs"
                    role="tablist"
                    aria-label={t('codex.localAccess.statsRange.label', "Stats Range")}
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
                    title={t('codex.localAccess.clearStats', "Clear Stats")}
                    aria-label={t('codex.localAccess.clearStats', "Clear Stats")}
                  >
                    <Trash2 size={14} />
                    {t('codex.localAccess.clearStats', "Clear Stats")}
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
                  <span>{t('codex.localAccess.configTitle', "Service Config")}</span>
                </div>
                {collection ? (
                  <div className="codex-local-access-config-grid">
                    <div className="codex-local-access-config-card codex-local-access-config-card-base">
                      <div className="codex-local-access-config-head">
                        <span className="codex-local-access-config-label">
                          {t('codex.localAccess.baseUrl', "URL")}
                        </span>
                        <div className="codex-local-access-config-actions">
                          <button
                            type="button"
                            className="folder-icon-btn"
                            onClick={() => void handleCopy('baseUrl', baseUrl)}
                            title={t('common.copy', "Copy")}
                          >
                            {copiedField === 'baseUrl' ? <Check size={14} /> : <Copy size={14} />}
                          </button>
                        </div>
                      </div>
                      <code className="codex-local-access-code" title={baseUrl}>
                        {baseUrl}
                      </code>
                    </div>

                    <div className="codex-local-access-config-card codex-local-access-config-card-key">
                      <div className="codex-local-access-config-head">
                        <span className="codex-local-access-config-label">
                          {t('codex.localAccess.apiKey', "API Key")}
                        </span>
                        <div className="codex-local-access-config-actions">
                          <button
                            type="button"
                            className="folder-icon-btn"
                            onClick={() => setKeyVisible((prev) => !prev)}
                            title={
                              keyVisible
                                ? t('codex.localAccess.hideKey', "Hide API key")
                                : t('codex.localAccess.showKey', "Show API key")
                            }
                          >
                            {keyVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                          </button>
                          <button
                            type="button"
                            className="folder-icon-btn"
                            onClick={() => void handleCopy('apiKey', collection.apiKey)}
                            title={t('common.copy', "Copy")}
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
                            {t('codex.localAccess.rotateKey', "Reset Key")}
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
                          {t('codex.localAccess.portLabel', "Service Port")}
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
                            {t('codex.localAccess.portSave', "Save Port")}
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
                  </div>
                ) : (
                  <div className="group-account-empty">
                    {t(
                      'codex.localAccess.configEmpty',
                      "Save at least one account to the API service collection before the URL, key, and port are generated.",
                    )}
                  </div>
                )}
                {collection || modelIdOptions.length > 0 ? (
                  <div className="codex-local-access-config-extra-grid">
                    {collection ? (
                      <div className="codex-local-access-config-card codex-local-access-config-card-root">
                        <div className="codex-local-access-config-head">
                          <span className="codex-local-access-config-label">
                            {t('codex.localAccess.apiPortUrl', "API Port URL")}
                          </span>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() => void handleCopy('apiPortUrl', apiPortUrl)}
                              title={t('common.copy', "Copy")}
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
                            {t('codex.localAccess.modelId', "Model ID")}
                          </span>
                          <span className="codex-local-access-view-only-badge">
                            {t('codex.localAccess.modelIdViewOnly', "View only, no switching")}
                          </span>
                          <div className="codex-local-access-config-actions">
                            <button
                              type="button"
                              className="folder-icon-btn"
                              onClick={() => void handleCopy('modelId', selectedModelId)}
                              title={t('common.copy', "Copy")}
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
                            ariaLabel={t('codex.localAccess.modelId', "Model ID")}
                            placeholder={t('codex.localAccess.modelIdPlaceholder', "Select model ID")}
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
                  <span>{t('codex.localAccess.accountStatsTitle', "By Account")}</span>
                </div>
                <div className="codex-local-access-account-stats">
                  {currentMemberStats.length === 0 ? (
                    <div className="group-account-empty">
                      {t('codex.localAccess.statsEmpty', "No stats yet")}
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
                  <span>{t('codex.localAccess.memberTitle', "Collection Members")}</span>
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
                      "Limit Free Accounts",
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
                    filterLabel={t('common.shared.filterLabel', "Filter")}
                    clearLabel={t('accounts.clearFilter', "Clear Filter")}
                    emptyLabel={t('common.none', "None")}
                    ariaLabel={t('common.shared.filterLabel', "Filter")}
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
                    allLabel={t('accounts.groups.allGroups', "All Groups")}
                    filterLabel={t('accounts.groups.manageTitle', "Group Management")}
                    clearLabel={t('accounts.clearFilter', "Clear Filter")}
                    emptyLabel={t('common.none', "None")}
                    ariaLabel={t('accounts.groups.manageTitle', "Group Management")}
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
                    {t('codex.localAccess.modal.empty', "No OAuth accounts available")}
                  </div>
                ) : visibleAccounts.length === 0 ? (
                  <div className="group-account-empty">
                    {t('common.shared.noMatch.title', "No matching accounts")}
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
                {saving ? t('common.saving') : t('codex.localAccess.modal.save', "Save Collection")}
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
  );
}

export default CodexLocalAccessModal;
