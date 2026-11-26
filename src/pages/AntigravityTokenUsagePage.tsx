import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import {
  confirm as confirmDialog,
  open as openDialog,
} from '@tauri-apps/plugin-dialog';
import {
  ArrowDown,
  ArrowDownToLine,
  ArrowUp,
  ArrowUpFromLine,
  Brain,
  Calendar,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Database,
  FolderOpen,
  GitBranch,
  MessageSquare,
  RefreshCw,
  RotateCcw,
  Search,
  Send,
  X,
  Zap,
} from 'lucide-react';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { OverviewTabsHeader } from '../components/OverviewTabsHeader';
import {
  SingleSelectDropdown,
  type SingleSelectOption,
} from '../components/SingleSelectDropdown';
import { AntigravityUsageTrend } from '../components/antigravity/AntigravityUsageTrend';
import {
  AntigravityActivityHeatmap,
  formatUsdShort,
} from '../components/antigravity/AntigravityActivityHeatmap';
import {
  getAntigravityConversationTokenDetail,
  hasRecentAntigravityTokenSync,
  normalizeAntigravityUsageRange,
  peekCachedAntigravityTokenUsage,
  queryAntigravityTokenUsage,
  setAntigravityRehydrateEnabled,
  setAntigravityTokenUsageDataDir,
  syncAntigravityTokenUsage,
  useAntigravityUsageRange,
} from '../services/antigravityTokenUsageService';
import type {
  ConversationDetail,
  ConversationKind,
  ConversationSortField,
  DashboardBundleResponse,
  DashboardQueryRequest,
  SortDirection,
  TimeGranularity,
} from '../types/antigravityTokenUsage';
import type { Page } from '../types/navigation';
import {
  formatSessionUsageCount,
  formatSessionUsageTokensShort,
} from '../utils/codexSessionUsageFormat';

function TokenAmount({
  value,
  lang,
  compactDecimals = 1,
}: {
  value: number;
  lang: string;
  compactDecimals?: 1 | 2;
}) {
  const exact = formatSessionUsageCount(value);
  const compact = formatSessionUsageTokensShort(value, lang, compactDecimals);
  return (
    <strong className="ag-token-amount" title={exact}>
      {compact}
    </strong>
  );
}

function formatDateTimeShort(iso: string | null | undefined, lang: string): string {
  if (!iso) return '—';
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return iso;
  return parsed.toLocaleString(lang, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function sharePct(part: number, total: number): string {
  if (!total || total <= 0 || !part || part <= 0) return '0%';
  const pct = (part / total) * 100;
  return pct >= 10 ? `${pct.toFixed(1)}%` : `${pct.toFixed(2)}%`;
}

function KindBadge({ kind }: { kind: ConversationKind }) {
  const { t } = useTranslation();
  const label =
    kind === 'swarm'
      ? t('antigravityTokenUsage.kinds.swarm', 'Swarm')
      : kind === 'subagent'
        ? t('antigravityTokenUsage.kinds.subagent', 'Subagent')
        : t('antigravityTokenUsage.kinds.main', 'Main');
  return <span className={`ag-kind-badge is-${kind}`}>{label}</span>;
}

export function AntigravityTokenUsagePage({
  onNavigate,
}: {
  onNavigate?: (page: Page) => void;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || 'en';

  const [range, setRange] = useAntigravityUsageRange();
  const [granularity, setGranularity] = useState<TimeGranularity>('day');
  const [kindFilter, setKindFilter] = useState<string>('');
  const [modelFilter, setModelFilter] = useState<string>('');
  const [workspaceFilter, setWorkspaceFilter] = useState<string>('');
  const [searchInput, setSearchInput] = useState<string>('');
  const [debouncedSearch, setDebouncedSearch] = useState<string>('');
  const [sortField, setSortField] =
    useState<ConversationSortField>('lastModifiedTime');
  const [sortOrder, setSortOrder] = useState<SortDirection>('desc');
  const [page, setPage] = useState<number>(1);
  const pageSize = 20;

  const initialCachedBundle = useMemo(
    () =>
      peekCachedAntigravityTokenUsage({
        preset: range,
        granularity: 'day',
        kind: '',
        model: '',
        workspace: '',
        search: '',
        sort: 'lastModifiedTime',
        order: 'desc',
        page: 1,
        pageSize,
      }),
    // Only evaluate once for initial state hydration.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const [bundle, setBundle] = useState<DashboardBundleResponse | null>(
    () => initialCachedBundle,
  );
  const [loading, setLoading] = useState<boolean>(
    () => initialCachedBundle === null,
  );
  const [syncing, setSyncing] = useState<boolean>(false);
  const [rebuilding, setRebuilding] = useState<boolean>(false);
  const [error, setError] = useState<string>('');

  const [selectedConvId, setSelectedConvId] = useState<string | null>(null);
  const [convDetail, setConvDetail] = useState<ConversationDetail | null>(null);
  const [convDetailLoading, setConvDetailLoading] = useState<boolean>(false);
  const [convDetailError, setConvDetailError] = useState<string>('');
  const [selectedHeatmapDate, setSelectedHeatmapDate] = useState<string | null>(
    null,
  );
  const [rehydrateBusy, setRehydrateBusy] = useState<boolean>(false);

  const queryVersionRef = useRef(0);
  const syncVersionRef = useRef(0);
  const latestQueryRef = useRef<DashboardQueryRequest | null>(null);
  const detailReqRef = useRef(0);
  const detailDialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setDebouncedSearch(searchInput.trim());
      setPage(1);
    }, 240);
    return () => window.clearTimeout(timer);
  }, [searchInput]);

  const queryPayload = useMemo<DashboardQueryRequest>(
    () => ({
      preset: selectedHeatmapDate ? 'custom' : range,
      from: selectedHeatmapDate ?? undefined,
      to: selectedHeatmapDate ?? undefined,
      granularity,
      kind: kindFilter,
      model: modelFilter,
      workspace: workspaceFilter,
      search: debouncedSearch,
      sort: sortField,
      order: sortOrder,
      page,
      pageSize,
    }),
    [
      debouncedSearch,
      granularity,
      kindFilter,
      modelFilter,
      page,
      range,
      selectedHeatmapDate,
      sortField,
      sortOrder,
      workspaceFilter,
    ],
  );
  latestQueryRef.current = queryPayload;

  const loadQuery = useCallback(
    async (nextQuery: DashboardQueryRequest) => {
      const qVer = ++queryVersionRef.current;
      const memHit = peekCachedAntigravityTokenUsage(nextQuery);
      if (memHit) {
        setBundle(memHit);
        setLoading(false);
      } else {
        setLoading(true);
      }
      setError('');
      try {
        const res = await queryAntigravityTokenUsage(nextQuery);
        if (qVer === queryVersionRef.current) {
          setBundle(res);
        }
      } catch (err) {
        if (qVer === queryVersionRef.current) {
          setError(
            t('antigravityTokenUsage.errors.loadFailed', {
              error: err instanceof Error ? err.message : String(err),
              defaultValue: 'Failed to load token usage: {{error}}',
            }),
          );
        }
      } finally {
        if (qVer === queryVersionRef.current) {
          setLoading(false);
        }
      }
    },
    [t],
  );

  const runSync = useCallback(
    async (rebuild: boolean) => {
      const sVer = ++syncVersionRef.current;
      const qAtStart = latestQueryRef.current ?? queryPayload;
      const qVerAtStart = queryVersionRef.current;
      setSyncing(true);
      if (rebuild) setRebuilding(true);
      setError('');
      try {
        const res = await syncAntigravityTokenUsage(rebuild, qAtStart);
        if (sVer !== syncVersionRef.current) {
          return;
        }
        if (queryVersionRef.current === qVerAtStart) {
          setBundle(res);
          setLoading(false);
        } else if (latestQueryRef.current) {
          void loadQuery(latestQueryRef.current);
        }
      } catch (err) {
        if (sVer === syncVersionRef.current) {
          setError(
            t('antigravityTokenUsage.errors.syncFailed', {
              error: err instanceof Error ? err.message : String(err),
              defaultValue: 'Failed to scan token usage: {{error}}',
            }),
          );
        }
      } finally {
        if (sVer === syncVersionRef.current) {
          setSyncing(false);
          setRebuilding(false);
        }
      }
    },
    [loadQuery, queryPayload, t],
  );

  useEffect(() => {
    void loadQuery(queryPayload);
  }, [loadQuery, queryPayload]);

  useEffect(() => {
    if (!hasRecentAntigravityTokenSync(4_000)) {
      void runSync(false);
    }
    // Only trigger background incremental sync on initial page mount if not already freshly synced.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleRebuild = useCallback(async () => {
    const confirmed = await confirmDialog(
      t(
        'antigravityTokenUsage.rebuildConfirm.message',
        'Clear Cockpit Tools local scan cache and re-read all Antigravity session files in read-only mode. Continue?',
      ),
      {
        title: t(
          'antigravityTokenUsage.rebuildConfirm.title',
          'Full Rescan',
        ),
        kind: 'warning',
      },
    );
    if (!confirmed) return;
    await runSync(true);
  }, [runSync, t]);

  const handleChooseDataDir = useCallback(async () => {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: bundle?.settings.effectiveDataDir || undefined,
    });
    if (!picked || typeof picked !== 'string') return;
    const qVer = ++queryVersionRef.current;
    const sVer = ++syncVersionRef.current;
    setSyncing(true);
    setError('');
    try {
      const res = await setAntigravityTokenUsageDataDir(picked, {
        ...queryPayload,
        page: 1,
      });
      if (qVer === queryVersionRef.current && sVer === syncVersionRef.current) {
        setPage(1);
        setBundle(res);
        setLoading(false);
      }
    } catch (err) {
      if (sVer === syncVersionRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (sVer === syncVersionRef.current) {
        setSyncing(false);
      }
    }
  }, [bundle?.settings.effectiveDataDir, queryPayload]);

  const handleResetDataDir = useCallback(async () => {
    const qVer = ++queryVersionRef.current;
    const sVer = ++syncVersionRef.current;
    setSyncing(true);
    setError('');
    try {
      const res = await setAntigravityTokenUsageDataDir(null, {
        ...queryPayload,
        page: 1,
      });
      if (qVer === queryVersionRef.current && sVer === syncVersionRef.current) {
        setPage(1);
        setBundle(res);
        setLoading(false);
      }
    } catch (err) {
      if (sVer === syncVersionRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (sVer === syncVersionRef.current) {
        setSyncing(false);
      }
    }
  }, [queryPayload]);

  const handleToggleRehydrate = useCallback(async () => {
    if (!bundle?.settings) return;
    const nextEnabled = !(bundle.settings.rehydrate?.enabled ?? false);
    setRehydrateBusy(true);
    setError('');
    try {
      const nextStatus = await setAntigravityRehydrateEnabled(nextEnabled);
      setBundle((prev) =>
        prev
          ? {
              ...prev,
              settings: {
                ...prev.settings,
                rehydrate: nextStatus,
              },
            }
          : prev,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setRehydrateBusy(false);
    }
  }, [bundle?.settings]);

  const openConversationDetail = useCallback(async (conversationId: string) => {
    const seq = ++detailReqRef.current;
    setSelectedConvId(conversationId);
    setConvDetail(null);
    setConvDetailError('');
    setConvDetailLoading(true);
    try {
      const res = await getAntigravityConversationTokenDetail(conversationId);
      if (seq === detailReqRef.current) {
        setConvDetail(res.conversation);
      }
    } catch (err) {
      if (seq === detailReqRef.current) {
        setConvDetailError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (seq === detailReqRef.current) {
        setConvDetailLoading(false);
      }
    }
  }, []);

  const closeConversationDetail = useCallback(() => {
    detailReqRef.current += 1;
    setSelectedConvId(null);
    setConvDetail(null);
    setConvDetailError('');
    setConvDetailLoading(false);
  }, []);

  useEscCloseTopmost(Boolean(selectedConvId), closeConversationDetail, detailDialogRef);

  const handleSortColumn = useCallback(
    (field: ConversationSortField) => {
      setPage(1);
      if (sortField === field) {
        setSortOrder((prev) => (prev === 'desc' ? 'asc' : 'desc'));
      } else {
        setSortField(field);
        setSortOrder('desc');
      }
    },
    [sortField],
  );

  const rangeOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: 'today', label: t('antigravityTokenUsage.range.today', 'Today') },
      { value: '7d', label: t('antigravityTokenUsage.range.7d', 'Last 7 Days') },
      { value: '30d', label: t('antigravityTokenUsage.range.30d', 'Last 30 Days') },
      { value: 'month', label: t('antigravityTokenUsage.range.month', 'This Month') },
      { value: 'all', label: t('antigravityTokenUsage.range.all', 'All Time') },
    ],
    [t],
  );

  const granularityOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: 'day', label: t('antigravityTokenUsage.granularity.day', 'Daily') },
      { value: 'week', label: t('antigravityTokenUsage.granularity.week', 'Weekly') },
      { value: 'month', label: t('antigravityTokenUsage.granularity.month', 'Monthly') },
    ],
    [t],
  );

  const kindOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: '', label: t('antigravityTokenUsage.kinds.all', 'All Types') },
      { value: 'main', label: t('antigravityTokenUsage.kinds.main', 'Main') },
      { value: 'swarm', label: t('antigravityTokenUsage.kinds.swarm', 'Swarm') },
      { value: 'subagent', label: t('antigravityTokenUsage.kinds.subagent', 'Subagent') },
    ],
    [t],
  );

  const modelOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: '', label: t('antigravityTokenUsage.filters.allModels', 'All Models') },
      ...(bundle?.summary.catalog.models ?? []).map((item) => ({
        value: item.modelId,
        label: `${item.modelDisplayName} (${item.count})`,
      })),
    ],
    [bundle?.summary.catalog.models, t],
  );

  const workspaceOptions = useMemo<SingleSelectOption[]>(
    () => [
      {
        value: '',
        label: t('antigravityTokenUsage.filters.allWorkspaces', 'All Workspaces'),
      },
      ...(bundle?.summary.catalog.workspaces ?? []).map((item) => ({
        value: item.workspaceKey,
        label: `${item.workspaceName} (${item.count})`,
      })),
    ],
    [bundle?.summary.catalog.workspaces, t],
  );

  const summary = bundle?.summary;
  const kpi = summary?.kpi;
  const exact = kpi?.exact;
  const estimated = kpi?.estimated;
  const conversations = bundle?.conversations;
  const settings = bundle?.settings;
  const cacheTotal =
    (exact?.cacheReadTokens ?? 0) + (exact?.cacheWriteTokens ?? 0);
  const hasAnyData =
    (kpi?.totalConversations ?? 0) > 0 || (kpi?.totalTurns ?? 0) > 0;

  const renderSortIcon = (field: ConversationSortField) => {
    if (sortField !== field) return null;
    return sortOrder === 'desc' ? (
      <ArrowDown size={12} />
    ) : (
      <ArrowUp size={12} />
    );
  };

  return (
    <main className="main-content accounts-page">
      <OverviewTabsHeader
        active="antigravity-usage"
        onNavigate={onNavigate}
        onOpenManual={() => onNavigate?.('manual')}
        subtitle={t(
          'antigravityTokenUsage.subtitle',
          'Local token usage from Antigravity sessions',
        )}
      />

      <div className="ag-token-usage-panel">
        <div className="ag-token-usage__intro">
          <div>
            <h3>
              {t(
                'antigravityTokenUsage.title',
                'Antigravity Session Token Usage',
              )}
            </h3>
            <p>
              {t(
                'antigravityTokenUsage.desc',
                'Aggregates real token usage from local Antigravity session databases (~/.gemini/antigravity) in read-only mode, strictly separating Thinking and Output tokens.',
              )}
            </p>
          </div>
          {onNavigate ? (
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => onNavigate('overview')}
            >
              {t('antigravityTokenUsage.actions.backToAccounts', 'Back to Accounts')}
            </button>
          ) : null}
        </div>

        {settings ? (
          <div className="ag-token-usage__dir-bar">
            <span className="ag-token-usage__dir-path">
              <FolderOpen size={14} />
              <span>
                {t('antigravityTokenUsage.dataDir.label', 'Data Dir:')}
              </span>
              <code title={settings.effectiveDataDir}>
                {settings.effectiveDataDir}
              </code>
              {settings.isCustomDataDir ? (
                <span className="ag-token-usage__custom-pill">
                  {t('antigravityTokenUsage.dataDir.customBadge', 'Custom')}
                </span>
              ) : null}
            </span>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => void handleChooseDataDir()}
              disabled={syncing}
            >
              {t('antigravityTokenUsage.dataDir.choose', 'Change Folder')}
            </button>
            {settings.isCustomDataDir ? (
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => void handleResetDataDir()}
                disabled={syncing}
              >
                {t('antigravityTokenUsage.dataDir.reset', 'Reset Default')}
              </button>
            ) : null}
          </div>
        ) : null}

        {settings?.rehydrate ? (
          <div className="ag-rehydrate-bar">
            <div className="ag-rehydrate-bar__info">
              <span className="ag-rehydrate-bar__title">
                <Brain size={14} />
                {t(
                  'antigravityTokenUsage.rehydrate.title',
                  'Post-Compact Context Rehydration Hook',
                )}
              </span>
              <span
                className={`ag-rehydrate-bar__badge ${
                  settings.rehydrate.enabled ? 'is-on' : 'is-off'
                }`}
              >
                {settings.rehydrate.enabled
                  ? t('antigravityTokenUsage.rehydrate.enabledBadge', 'Active')
                  : t('antigravityTokenUsage.rehydrate.disabledBadge', 'Off')}
              </span>
              <span
                className="ag-rehydrate-bar__desc"
                title={settings.rehydrate.hooksConfigPath}
              >
                {t(
                  'antigravityTokenUsage.rehydrate.desc',
                  'Auto-restores key request timeline & recent files via PreInvocation hook after context compaction (zero binary patch).',
                )}
              </span>
            </div>
            <div className="ag-rehydrate-bar__actions">
              <button
                type="button"
                className={`btn btn-sm ${
                  settings.rehydrate.enabled ? 'btn-secondary' : 'btn-primary'
                }`}
                onClick={() => void handleToggleRehydrate()}
                disabled={rehydrateBusy || !settings.rehydrate.pythonAvailable}
                title={
                  !settings.rehydrate.pythonAvailable
                    ? t(
                        'antigravityTokenUsage.rehydrate.noPython',
                        'Requires Python 3.10+ on PATH',
                      )
                    : settings.rehydrate.scriptPath
                }
              >
                {settings.rehydrate.enabled
                  ? t('antigravityTokenUsage.rehydrate.disableBtn', 'Disable Hook')
                  : t('antigravityTokenUsage.rehydrate.enableBtn', 'Enable Hook')}
              </button>
            </div>
          </div>
        ) : null}

        <div className="ag-token-usage__toolbar">
          <label className="ag-token-usage__field">
            <span>
              {t('antigravityTokenUsage.range.label', 'Time Range')}
            </span>
            <SingleSelectDropdown
              value={range}
              options={rangeOptions}
              onChange={(val) => {
                setPage(1);
                setRange(normalizeAntigravityUsageRange(val));
              }}
              ariaLabel={t(
                'antigravityTokenUsage.range.label',
                'Time Range',
              )}
              menuWidth={150}
              menuMaxHeight={240}
            />
          </label>

          <label className="ag-token-usage__field">
            <span>
              {t('antigravityTokenUsage.granularity.label', 'Granularity')}
            </span>
            <SingleSelectDropdown
              value={granularity}
              options={granularityOptions}
              onChange={(val) => {
                setGranularity(
                  val === 'week' || val === 'month' ? val : 'day',
                );
              }}
              ariaLabel={t(
                'antigravityTokenUsage.granularity.label',
                'Granularity',
              )}
              menuWidth={150}
              menuMaxHeight={220}
            />
          </label>

          <label className="ag-token-usage__field">
            <span>
              {t('antigravityTokenUsage.filters.kindLabel', 'Session Type')}
            </span>
            <SingleSelectDropdown
              value={kindFilter}
              options={kindOptions}
              onChange={(val) => {
                setPage(1);
                setKindFilter(val);
              }}
              ariaLabel={t(
                'antigravityTokenUsage.filters.kindLabel',
                'Session Type',
              )}
              menuWidth={170}
              menuMaxHeight={240}
            />
          </label>

          <label className="ag-token-usage__field">
            <span>
              {t('antigravityTokenUsage.filters.modelLabel', 'Model')}
            </span>
            <SingleSelectDropdown
              value={modelFilter}
              options={modelOptions}
              onChange={(val) => {
                setPage(1);
                setModelFilter(val);
              }}
              ariaLabel={t(
                'antigravityTokenUsage.filters.modelLabel',
                'Model',
              )}
              menuWidth={250}
              menuMaxHeight={280}
            />
          </label>

          <label className="ag-token-usage__field">
            <span>
              {t(
                'antigravityTokenUsage.filters.workspaceLabel',
                'Workspace',
              )}
            </span>
            <SingleSelectDropdown
              value={workspaceFilter}
              options={workspaceOptions}
              onChange={(val) => {
                setPage(1);
                setWorkspaceFilter(val);
              }}
              ariaLabel={t(
                'antigravityTokenUsage.filters.workspaceLabel',
                'Workspace',
              )}
              menuWidth={250}
              menuMaxHeight={280}
            />
          </label>

          <div className="ag-token-usage__actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void runSync(false)}
              disabled={syncing}
            >
              <RefreshCw
                size={14}
                className={syncing && !rebuilding ? 'spin' : undefined}
              />
              {t('antigravityTokenUsage.actions.refresh', 'Refresh')}
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void handleRebuild()}
              disabled={syncing}
            >
              <RotateCcw
                size={14}
                className={rebuilding ? 'spin' : undefined}
              />
              {t('antigravityTokenUsage.actions.rebuild', 'Rescan')}
            </button>
          </div>
        </div>

        <div className="ag-token-usage__status" role="status">
          <span className="ag-token-usage__status-item">
            {syncing
              ? t(
                  'antigravityTokenUsage.status.scanning',
                  'Scanning Antigravity session databases…',
                )
              : summary
                ? t('antigravityTokenUsage.status.ready', {
                    dbFiles: summary.scan.totalDbFiles,
                    sessions: kpi?.totalConversations ?? 0,
                    turns: kpi?.totalTurns ?? 0,
                    defaultValue: 'Scanned {{dbFiles}} DB files · {{sessions}} sessions · {{turns}} turns',
                  })
                : t('antigravityTokenUsage.status.idle', 'Not scanned yet')}
          </span>
          {summary ? (
            <>
              <span className="ag-token-usage__status-item">
                {t('antigravityTokenUsage.status.cacheStats', {
                  hits: summary.scan.cacheHits,
                  misses: summary.scan.cacheMisses,
                  durationMs: summary.scan.scanDurationMs,
                  defaultValue: 'Cache {{hits}} hit / {{misses}} new ({{durationMs}} ms)',
                })}
              </span>
              <span className="ag-token-usage__status-item">
                {t('antigravityTokenUsage.status.scannedAt', {
                  time: formatDateTimeShort(summary.scan.scannedAt, lang),
                  defaultValue: 'Synced {{time}}',
                })}
              </span>
            </>
          ) : null}
        </div>

        {error ? <div className="ag-token-usage__error">{error}</div> : null}

        <p className="ag-token-usage__hint">
          {t(
            'antigravityTokenUsage.hint',
            'Read-only scan of SQLite steps/gen_metadata (WAL-aware), RPC cache, and transcript JSONL. Output tokens exclude Thinking tokens (zero double-counting); estimated tokens (if any) are kept separate from exact API totals.',
          )}
        </p>

        {loading && !bundle ? (
          <div className="ag-token-usage__empty">
            {t('common.loading', 'Loading...')}
          </div>
        ) : !hasAnyData ? (
          <div className="ag-token-usage__empty">
            <h4>
              {t(
                'antigravityTokenUsage.empty.title',
                'No token usage found for current filters',
              )}
            </h4>
            <p>
              {summary?.scan.dataDirExists === false
                ? t(
                    'antigravityTokenUsage.empty.dirMissing',
                    'Antigravity data directory was not found. Please check the folder path above.',
                  )
                : t(
                    'antigravityTokenUsage.empty.desc',
                    'Try switching Time Range to "All Time" or click Refresh after using Antigravity.',
                  )}
            </p>
          </div>
        ) : (
          <>
            <section className="ag-token-usage__hero">
              <div className="ag-token-usage__hero-head">
                <div className="ag-token-usage__hero-total">
                  <div
                    className="ag-token-usage__hero-icon"
                    aria-hidden="true"
                  >
                    <Zap size={24} />
                  </div>
                  <div className="ag-token-usage__hero-total-meta">
                    <span>
                      {t(
                        'antigravityTokenUsage.cards.totalExact',
                        'Total Exact Tokens',
                      )}
                    </span>
                    <TokenAmount
                      value={exact?.totalTokens ?? 0}
                      lang={lang}
                      compactDecimals={2}
                    />
                    <span className="ag-token-usage__hero-sub">
                      {formatSessionUsageCount(exact?.totalTokens ?? 0)} tokens
                    </span>
                  </div>
                </div>

                <div className="ag-token-usage__hero-side">
                  <span>
                    <MessageSquare size={13} />
                    {t('antigravityTokenUsage.cards.conversations', 'Conversations')}
                  </span>
                  <strong>
                    {formatSessionUsageCount(kpi?.totalConversations ?? 0)}
                  </strong>

                  <span>
                    <Send size={13} />
                    {t('antigravityTokenUsage.cards.turns', 'Turns')}
                  </span>
                  <strong>
                    {formatSessionUsageCount(kpi?.totalTurns ?? 0)}
                  </strong>

                  <span>
                    <CheckCircle2 size={13} />
                    {t('antigravityTokenUsage.cards.coverage', 'Exact Coverage')}
                  </span>
                  <strong>{`${(kpi?.exactCoveragePercent ?? 0).toFixed(1)}%`}</strong>

                  <span>
                    <Calendar size={13} />
                    {t('antigravityTokenUsage.cards.avgPerDay', 'Avg / Active Day')}
                  </span>
                  <strong>
                    {formatSessionUsageTokensShort(
                      kpi?.avgExactTokensPerActiveDay ?? 0,
                      lang,
                      1,
                    )}
                  </strong>

                  <span>
                    <Zap size={13} />
                    {t('antigravityTokenUsage.cards.refCost', 'API Equiv. Cost')}
                  </span>
                  <strong
                    title={t('antigravityTokenUsage.cards.cacheSavingsHint', {
                      savings: formatUsdShort(kpi?.cacheSavingsUsd),
                      defaultValue: 'Prompt cache saved ≈ {{savings}}',
                    })}
                  >
                    ≈ {formatUsdShort(kpi?.estimatedCostUsd)}
                  </strong>
                </div>
              </div>

              <div className="ag-token-usage__cards">
                <article className="is-input">
                  <div className="ag-token-usage__card-head">
                    <ArrowDownToLine size={15} aria-hidden="true" />
                    <span>
                      {t('antigravityTokenUsage.cards.input', 'Input Tokens')}
                    </span>
                  </div>
                  <TokenAmount value={exact?.inputTokens ?? 0} lang={lang} />
                  <span className="ag-token-usage__card-foot">
                    {sharePct(exact?.inputTokens ?? 0, exact?.totalTokens ?? 0)}{' '}
                    · {formatSessionUsageCount(exact?.inputTokens ?? 0)}
                  </span>
                </article>

                <article className="is-thinking">
                  <div className="ag-token-usage__card-head">
                    <Brain size={15} aria-hidden="true" />
                    <span>
                      {t(
                        'antigravityTokenUsage.cards.thinking',
                        'Thinking Tokens',
                      )}
                    </span>
                  </div>
                  <TokenAmount value={exact?.thinkingTokens ?? 0} lang={lang} />
                  <span className="ag-token-usage__card-foot">
                    {sharePct(
                      exact?.thinkingTokens ?? 0,
                      exact?.totalTokens ?? 0,
                    )}{' '}
                    · {formatSessionUsageCount(exact?.thinkingTokens ?? 0)}
                  </span>
                </article>

                <article className="is-output">
                  <div className="ag-token-usage__card-head">
                    <ArrowUpFromLine size={15} aria-hidden="true" />
                    <span>
                      {t('antigravityTokenUsage.cards.output', 'Output Tokens')}
                    </span>
                  </div>
                  <TokenAmount value={exact?.outputTokens ?? 0} lang={lang} />
                  <span className="ag-token-usage__card-foot">
                    {sharePct(
                      exact?.outputTokens ?? 0,
                      exact?.totalTokens ?? 0,
                    )}{' '}
                    · {formatSessionUsageCount(exact?.outputTokens ?? 0)}
                  </span>
                </article>

                <article className="is-cache">
                  <div className="ag-token-usage__card-head">
                    <Database size={15} aria-hidden="true" />
                    <span>
                      {t('antigravityTokenUsage.cards.cache', 'Cache Tokens')}
                    </span>
                  </div>
                  <TokenAmount value={cacheTotal} lang={lang} />
                  <span className="ag-token-usage__card-foot">
                    R:{' '}
                    {formatSessionUsageTokensShort(
                      exact?.cacheReadTokens ?? 0,
                      lang,
                      1,
                    )}{' '}
                    · W:{' '}
                    {formatSessionUsageTokensShort(
                      exact?.cacheWriteTokens ?? 0,
                      lang,
                      1,
                    )}
                  </span>
                </article>
              </div>

              {(estimated?.totalTokens ?? 0) > 0 ? (
                <div className="ag-token-usage__estimated-banner">
                  <span>
                    {t('antigravityTokenUsage.estimated.notice', {
                      turns: estimated?.estimatedTurnCount ?? 0,
                      defaultValue: '{{turns}} turns lacked raw API usage and were estimated separately (never added to Exact Total):',
                    })}
                  </span>
                  <strong title={formatSessionUsageCount(estimated?.totalTokens ?? 0)}>
                    ~
                    {formatSessionUsageTokensShort(
                      estimated?.totalTokens ?? 0,
                      lang,
                      2,
                    )}{' '}
                    tokens
                  </strong>
                </div>
              ) : null}
            </section>

            <AntigravityUsageTrend series={summary?.timeSeries ?? []} />

            <AntigravityActivityHeatmap
              points={summary?.heatmap ?? []}
              selectedDate={selectedHeatmapDate}
              onSelectDate={(nextDate) => {
                setPage(1);
                setSelectedHeatmapDate(nextDate);
              }}
              lang={lang}
            />

            <div className="ag-token-usage__grids">
              <section className="ag-token-section">
                <div className="ag-token-section__head">
                  <h4>
                    {t(
                      'antigravityTokenUsage.tables.byModel',
                      'By Model',
                    )}
                  </h4>
                </div>
                <div className="ag-token-table-wrap">
                  <table className="ag-token-table">
                    <thead>
                      <tr>
                        <th>
                          {t(
                            'antigravityTokenUsage.tables.modelName',
                            'Model',
                          )}
                        </th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.turns', 'Turns')}
                        </th>
                        <th className="is-num">Input</th>
                        <th className="is-num">Thinking</th>
                        <th className="is-num">Output</th>
                        <th className="is-num">Cache</th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.total', 'Total')}
                        </th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.cards.refCost', 'API Equiv. Cost')}
                        </th>
                        <th className="is-num">%</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(summary?.byModel ?? []).length === 0 ? (
                        <tr>
                          <td colSpan={9}>
                            {t(
                              'antigravityTokenUsage.empty.noRows',
                              'No matching data',
                            )}
                          </td>
                        </tr>
                      ) : (
                        (summary?.byModel ?? []).map((row) => (
                          <tr
                            key={row.modelId}
                            className="is-clickable"
                            onClick={() => {
                              setPage(1);
                              setModelFilter(
                                modelFilter === row.modelId ? '' : row.modelId,
                              );
                            }}
                          >
                            <td>
                              <div className="ag-token-cell-primary">
                                <span
                                  className="ag-token-cell-title"
                                  title={row.modelDisplayName}
                                >
                                  {row.modelDisplayName}
                                </span>
                                <span className="ag-token-cell-sub">
                                  {row.apiProvider} · {row.conversationCount}{' '}
                                  {t(
                                    'antigravityTokenUsage.summary.sessionsUnit',
                                    'sessions',
                                  )}
                                </span>
                              </div>
                            </td>
                            <td className="is-num">
                              {formatSessionUsageCount(row.turnCount)}
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.inputTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.thinkingTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.outputTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={
                                  row.exact.cacheReadTokens +
                                  row.exact.cacheWriteTokens
                                }
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.totalTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              ≈ {formatUsdShort(row.estimatedCostUsd)}
                            </td>
                            <td className="is-num">
                              <div className="ag-share-bar">
                                <div className="ag-share-bar__track">
                                  <div
                                    className="ag-share-bar__fill"
                                    style={{
                                      width: `${Math.min(100, Math.max(0, row.sharePercent))}%`,
                                    }}
                                  />
                                </div>
                                <span>{`${row.sharePercent.toFixed(1)}%`}</span>
                              </div>
                            </td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>
              </section>

              <section className="ag-token-section">
                <div className="ag-token-section__head">
                  <h4>
                    {t(
                      'antigravityTokenUsage.tables.byWorkspace',
                      'By Workspace',
                    )}
                  </h4>
                </div>
                <div className="ag-token-table-wrap">
                  <table className="ag-token-table">
                    <thead>
                      <tr>
                        <th>
                          {t(
                            'antigravityTokenUsage.tables.workspaceName',
                            'Workspace',
                          )}
                        </th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.sessions', 'Sessions')}
                        </th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.turns', 'Turns')}
                        </th>
                        <th className="is-num">Input</th>
                        <th className="is-num">Thinking</th>
                        <th className="is-num">Output</th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.total', 'Total')}
                        </th>
                        <th className="is-num">
                          {t(
                            'antigravityTokenUsage.cards.refCost',
                            'API Equiv. Cost',
                          )}
                        </th>
                        <th className="is-num">%</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(summary?.byWorkspace ?? []).length === 0 ? (
                        <tr>
                          <td colSpan={9}>
                            {t(
                              'antigravityTokenUsage.empty.noRows',
                              'No matching data',
                            )}
                          </td>
                        </tr>
                      ) : (
                        (summary?.byWorkspace ?? []).map((row) => (
                          <tr
                            key={row.workspaceKey}
                            className="is-clickable"
                            onClick={() => {
                              setPage(1);
                              setWorkspaceFilter(
                                workspaceFilter === row.workspaceKey
                                  ? ''
                                  : row.workspaceKey,
                              );
                            }}
                          >
                            <td>
                              <div className="ag-token-cell-primary">
                                <span
                                  className="ag-token-cell-title"
                                  title={row.workspacePath || row.workspaceName}
                                >
                                  {row.workspaceName}
                                </span>
                                {row.workspacePath ? (
                                  <span
                                    className="ag-token-cell-sub"
                                    title={row.workspacePath}
                                  >
                                    {row.workspacePath}
                                  </span>
                                ) : null}
                              </div>
                            </td>
                            <td className="is-num">
                              {formatSessionUsageCount(row.conversationCount)}
                            </td>
                            <td className="is-num">
                              {formatSessionUsageCount(row.turnCount)}
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.inputTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.thinkingTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.outputTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              <TokenAmount
                                value={row.exact.totalTokens}
                                lang={lang}
                              />
                            </td>
                            <td className="is-num">
                              ≈ {formatUsdShort(row.estimatedCostUsd)}
                            </td>
                            <td className="is-num">
                              <div className="ag-share-bar">
                                <div className="ag-share-bar__track">
                                  <div
                                    className="ag-share-bar__fill"
                                    style={{
                                      width: `${Math.min(100, Math.max(0, row.sharePercent))}%`,
                                    }}
                                  />
                                </div>
                                <span>{`${row.sharePercent.toFixed(1)}%`}</span>
                              </div>
                            </td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>
              </section>
            </div>

            {(summary?.byKind ?? []).length > 0 ? (
              <section className="ag-token-section">
                <div className="ag-token-section__head">
                  <h4>
                    {t(
                      'antigravityTokenUsage.tables.byKind',
                      'By Session Type (Main / Swarm / Subagent)',
                    )}
                  </h4>
                </div>
                <div className="ag-token-table-wrap">
                  <table className="ag-token-table">
                    <thead>
                      <tr>
                        <th>
                          {t(
                            'antigravityTokenUsage.filters.kindLabel',
                            'Session Type',
                          )}
                        </th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.sessions', 'Sessions')}
                        </th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.turns', 'Turns')}
                        </th>
                        <th className="is-num">Input</th>
                        <th className="is-num">Thinking</th>
                        <th className="is-num">Output</th>
                        <th className="is-num">Cache</th>
                        <th className="is-num">
                          {t('antigravityTokenUsage.tables.total', 'Total')}
                        </th>
                        <th className="is-num">%</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(summary?.byKind ?? []).map((row) => (
                        <tr
                          key={row.kind}
                          className="is-clickable"
                          onClick={() => {
                            setPage(1);
                            setKindFilter(
                              kindFilter === row.kind ? '' : row.kind,
                            );
                          }}
                        >
                          <td>
                            <KindBadge kind={row.kind} />
                          </td>
                          <td className="is-num">
                            {formatSessionUsageCount(row.conversationCount)}
                          </td>
                          <td className="is-num">
                            {formatSessionUsageCount(row.turnCount)}
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={row.exact.inputTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={row.exact.thinkingTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={row.exact.outputTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={
                                row.exact.cacheReadTokens +
                                row.exact.cacheWriteTokens
                              }
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={row.exact.totalTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">{`${row.sharePercent.toFixed(1)}%`}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </section>
            ) : null}

            <section className="ag-token-section">
              <div className="ag-token-section__head">
                <h4>
                  {t(
                    'antigravityTokenUsage.tables.conversations',
                    'Conversation Sessions',
                  )}{' '}
                  ({formatSessionUsageCount(conversations?.totalItems ?? 0)})
                </h4>
                <div className="ag-conv-controls">
                  <div className="ag-conv-search">
                    <Search size={14} />
                    <input
                      type="text"
                      value={searchInput}
                      onChange={(e) => setSearchInput(e.target.value)}
                      placeholder={t(
                        'antigravityTokenUsage.filters.searchPlaceholder',
                        'Search title, session ID, workspace, model…',
                      )}
                    />
                    {searchInput ? (
                      <button
                        type="button"
                        className="ag-sort-th-btn"
                        onClick={() => setSearchInput('')}
                        aria-label="Clear"
                      >
                        <X size={13} />
                      </button>
                    ) : null}
                  </div>
                </div>
              </div>

              <div className="ag-token-table-wrap">
                <table className="ag-token-table">
                  <thead>
                    <tr>
                      <th>
                        {t(
                          'antigravityTokenUsage.tables.conversationTitle',
                          'Conversation',
                        )}
                      </th>
                      <th>
                        {t('antigravityTokenUsage.filters.kindLabel', 'Session Type')}
                      </th>
                      <th>
                        {t(
                          'antigravityTokenUsage.tables.modelName',
                          'Model',
                        )}
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('turnCount')}
                        >
                          {t('antigravityTokenUsage.tables.turns', 'Turns')}
                          {renderSortIcon('turnCount')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('inputTokens')}
                        >
                          Input
                          {renderSortIcon('inputTokens')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('thinkingTokens')}
                        >
                          Thinking
                          {renderSortIcon('thinkingTokens')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('outputTokens')}
                        >
                          Output
                          {renderSortIcon('outputTokens')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('cacheReadTokens')}
                        >
                          Cache
                          {renderSortIcon('cacheReadTokens')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('totalTokens')}
                        >
                          {t('antigravityTokenUsage.tables.total', 'Total')}
                          {renderSortIcon('totalTokens')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('estimatedCostUsd')}
                        >
                          {t('antigravityTokenUsage.cards.refCost', 'API Equiv. Cost')}
                          {renderSortIcon('estimatedCostUsd')}
                        </button>
                      </th>
                      <th className="is-num">
                        <button
                          type="button"
                          className="ag-sort-th-btn"
                          onClick={() => handleSortColumn('lastModifiedTime')}
                        >
                          {t(
                            'antigravityTokenUsage.tables.updatedAt',
                            'Updated',
                          )}
                          {renderSortIcon('lastModifiedTime')}
                        </button>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {(conversations?.items ?? []).length === 0 ? (
                      <tr>
                        <td colSpan={11}>
                          {t(
                            'antigravityTokenUsage.empty.noRows',
                            'No matching data',
                          )}
                        </td>
                      </tr>
                    ) : (
                      (conversations?.items ?? []).map((conv) => (
                        <tr
                          key={conv.conversationId}
                          className="is-clickable"
                          onClick={() =>
                            void openConversationDetail(conv.conversationId)
                          }
                        >
                          <td>
                            <div className="ag-token-cell-primary">
                              <span
                                className="ag-token-cell-title"
                                title={conv.displayTitle}
                              >
                                {conv.displayTitle}
                              </span>
                              <span
                                className="ag-token-cell-sub"
                                title={conv.workspacePath || conv.conversationId}
                              >
                                {conv.workspaceName} ·{' '}
                                {conv.conversationId.slice(0, 8)}
                              </span>
                            </div>
                          </td>
                          <td>
                            <KindBadge kind={conv.kind} />
                            {conv.clientSource === 'cli' ? (
                              <>
                                {' '}
                                <span className="ag-source-badge is-cli">
                                  CLI
                                </span>
                              </>
                            ) : null}
                          </td>
                          <td title={conv.modelDisplayNames.join(', ')}>
                            {conv.primaryModelDisplayName || '—'}
                          </td>
                          <td className="is-num">
                            {formatSessionUsageCount(conv.turnCount)}
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={conv.exact.inputTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={conv.exact.thinkingTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={conv.exact.outputTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={
                                conv.exact.cacheReadTokens +
                                conv.exact.cacheWriteTokens
                              }
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            <TokenAmount
                              value={conv.exact.totalTokens}
                              lang={lang}
                            />
                          </td>
                          <td className="is-num">
                            ≈ {formatUsdShort(conv.estimatedCostUsd)}
                          </td>
                          <td className="is-num">
                            {formatDateTimeShort(conv.lastModifiedTime, lang)}
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>

              {(conversations?.totalPages ?? 1) > 1 ? (
                <div className="ag-conv-pagination">
                  <span>
                    {t('antigravityTokenUsage.pagination.info', {
                      page: conversations?.page ?? 1,
                      totalPages: conversations?.totalPages ?? 1,
                      totalItems: conversations?.totalItems ?? 0,
                      defaultValue: 'Page {{page}} / {{totalPages}} ({{totalItems}} sessions)',
                    })}
                  </span>
                  <div className="ag-conv-pagination__btns">
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      disabled={(conversations?.page ?? 1) <= 1}
                      onClick={() => setPage((p) => Math.max(1, p - 1))}
                    >
                      <ChevronLeft size={14} />
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      disabled={
                        (conversations?.page ?? 1) >=
                        (conversations?.totalPages ?? 1)
                      }
                      onClick={() => setPage((p) => p + 1)}
                    >
                      <ChevronRight size={14} />
                    </button>
                  </div>
                </div>
              ) : null}
            </section>
          </>
        )}
      </div>

      {selectedConvId
        ? createPortal(
            <div className="modal-overlay ag-conv-modal-backdrop">
              <div
                ref={detailDialogRef}
                className="modal ag-conv-modal"
                role="dialog"
                aria-modal="true"
                aria-labelledby="ag-conv-modal-title"
              >
                <div className="ag-conv-modal__header">
                  <div className="ag-conv-modal__title-wrap">
                    <h3
                      id="ag-conv-modal-title"
                      className="ag-conv-modal__title"
                      title={convDetail?.displayTitle || selectedConvId}
                    >
                      {convDetail?.displayTitle || selectedConvId}
                    </h3>
                    {convDetail ? (
                      <div className="ag-conv-modal__meta">
                        <KindBadge kind={convDetail.kind} />
                        <span title={convDetail.conversationId}>{convDetail.conversationId}</span>
                        {convDetail.workspacePath ? (
                          <span title={convDetail.workspacePath}>{convDetail.workspacePath}</span>
                        ) : null}
                        <span>
                          {formatDateTimeShort(
                            convDetail.lastModifiedTime,
                            lang,
                          )}
                        </span>
                        {convDetail.parentConversationId ? (
                          <button
                            type="button"
                            className="ag-conv-child-btn"
                            onClick={() =>
                              void openConversationDetail(
                                convDetail.parentConversationId,
                              )
                            }
                          >
                            <GitBranch size={12} />
                            {t(
                              'antigravityTokenUsage.detail.openParent',
                              'Open Parent Session',
                            )}
                          </button>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                  <button
                    type="button"
                    className="ag-token-summary__icon-btn"
                    onClick={closeConversationDetail}
                    aria-label={t('common.close', 'Close')}
                  >
                    <X size={16} />
                  </button>
                </div>

                <div className="modal-body ag-conv-modal__body">
                  {convDetailLoading ? (
                    <div className="ag-token-usage__empty">
                      {t('common.loading', 'Loading...')}
                    </div>
                  ) : convDetailError ? (
                    <div className="ag-token-usage__error" role="alert">
                      {convDetailError}
                    </div>
                  ) : convDetail ? (
                    <>
                      <div className="ag-conv-modal__kpis">
                        <div className="ag-conv-modal__kpi">
                          <span>
                            {t(
                              'antigravityTokenUsage.cards.totalExact',
                              'Total Exact Tokens',
                            )}
                          </span>
                          <strong
                            title={formatSessionUsageCount(
                              convDetail.exact.totalTokens,
                            )}
                          >
                            {formatSessionUsageTokensShort(
                              convDetail.exact.totalTokens,
                              lang,
                              2,
                            )}
                          </strong>
                        </div>
                        <div className="ag-conv-modal__kpi">
                          <span>Input</span>
                          <strong
                            title={formatSessionUsageCount(
                              convDetail.exact.inputTokens,
                            )}
                          >
                            {formatSessionUsageTokensShort(
                              convDetail.exact.inputTokens,
                              lang,
                              1,
                            )}
                          </strong>
                        </div>
                        <div className="ag-conv-modal__kpi">
                          <span>Thinking</span>
                          <strong
                            title={formatSessionUsageCount(
                              convDetail.exact.thinkingTokens,
                            )}
                          >
                            {formatSessionUsageTokensShort(
                              convDetail.exact.thinkingTokens,
                              lang,
                              1,
                            )}
                          </strong>
                        </div>
                        <div className="ag-conv-modal__kpi">
                          <span>Output</span>
                          <strong
                            title={formatSessionUsageCount(
                              convDetail.exact.outputTokens,
                            )}
                          >
                            {formatSessionUsageTokensShort(
                              convDetail.exact.outputTokens,
                              lang,
                              1,
                            )}
                          </strong>
                        </div>
                        <div className="ag-conv-modal__kpi">
                          <span>Cache Read / Write</span>
                          <strong
                            title={`${formatSessionUsageCount(convDetail.exact.cacheReadTokens)} / ${formatSessionUsageCount(convDetail.exact.cacheWriteTokens)}`}
                          >
                            {formatSessionUsageTokensShort(
                              convDetail.exact.cacheReadTokens +
                                convDetail.exact.cacheWriteTokens,
                              lang,
                              1,
                            )}
                          </strong>
                        </div>
                        <div className="ag-conv-modal__kpi">
                          <span>
                            {t(
                              'antigravityTokenUsage.cards.turns',
                              'Turns',
                            )}
                          </span>
                          <strong>
                            {convDetail.turnCount} / {convDetail.stepCount}
                          </strong>
                        </div>
                        <div className="ag-conv-modal__kpi">
                          <span>
                            {t(
                              'antigravityTokenUsage.cards.refCost',
                              'API Equiv. Cost',
                            )}
                          </span>
                          <strong
                            title={t(
                              'antigravityTokenUsage.cards.cacheSavingsHint',
                              {
                                savings: formatUsdShort(
                                  convDetail.cacheSavingsUsd,
                                ),
                                defaultValue:
                                  'Prompt cache saved ≈ {{savings}}',
                              },
                            )}
                          >
                            ≈ {formatUsdShort(convDetail.estimatedCostUsd)}
                          </strong>
                        </div>
                      </div>

                      {convDetail.audit ? (
                        <section className="ag-token-section">
                          <h4>
                            {t(
                              'antigravityTokenUsage.audit.title',
                              'Session Context & Execution Audit',
                            )}
                          </h4>
                          <div className="ag-conv-audit">
                            {convDetail.audit.peakContextTokens > 0 ? (
                              <>
                                <div className="ag-conv-audit__bar-row">
                                  <span>
                                    {t(
                                      'antigravityTokenUsage.audit.peakContext',
                                      'Peak Context Footprint',
                                    )}
                                    :{' '}
                                    <strong>
                                      {formatSessionUsageCount(
                                        convDetail.audit.peakContextTokens,
                                      )}{' '}
                                      /{' '}
                                      {formatSessionUsageCount(
                                        convDetail.audit.contextLimitTokens,
                                      )}{' '}
                                      (
                                      {convDetail.audit.peakContextPercent.toFixed(
                                        1,
                                      )}
                                      %)
                                    </strong>
                                  </span>
                                  {convDetail.audit.compactionCount > 0 ? (
                                    <span className="ag-conv-audit__pill is-compaction">
                                      {t(
                                        'antigravityTokenUsage.audit.compactions',
                                        {
                                          count:
                                            convDetail.audit.compactionCount,
                                          defaultValue:
                                            '{{count}} Compaction(s)',
                                        },
                                      )}
                                    </span>
                                  ) : null}
                                </div>
                                <div className="ag-conv-audit__bar-track">
                                  <div
                                    className="ag-conv-audit__bar-fill"
                                    style={{
                                      width: `${Math.min(100, Math.max(2, convDetail.audit.peakContextPercent))}%`,
                                    }}
                                  />
                                </div>
                              </>
                            ) : null}

                            <div className="ag-conv-audit__pills">
                              <span className="ag-conv-audit__pill">
                                {t(
                                  'antigravityTokenUsage.audit.userPrompts',
                                  'User Prompts',
                                )}
                                :{' '}
                                <strong>
                                  {convDetail.audit.userPromptCount}
                                </strong>
                              </span>
                              <span className="ag-conv-audit__pill">
                                {t(
                                  'antigravityTokenUsage.audit.toolCalls',
                                  'Tool Calls',
                                )}
                                :{' '}
                                <strong>
                                  {convDetail.audit.toolCallCount}
                                </strong>
                              </span>
                              <span className="ag-conv-audit__pill">
                                {t(
                                  'antigravityTokenUsage.audit.fileOps',
                                  'File R/W',
                                )}
                                :{' '}
                                <strong>
                                  {convDetail.audit.fileReadCount} /{' '}
                                  {convDetail.audit.fileEditCount}
                                </strong>
                              </span>
                              <span className="ag-conv-audit__pill">
                                {t(
                                  'antigravityTokenUsage.audit.commands',
                                  'Commands',
                                )}
                                :{' '}
                                <strong>
                                  {convDetail.audit.commandExecutionCount}
                                </strong>
                              </span>
                              {convDetail.audit.subagentSpawnCount > 0 ? (
                                <span className="ag-conv-audit__pill">
                                  {t(
                                    'antigravityTokenUsage.audit.subagents',
                                    'Subagents',
                                  )}
                                  :{' '}
                                  <strong>
                                    {convDetail.audit.subagentSpawnCount}
                                  </strong>
                                </span>
                              ) : null}
                              {convDetail.audit.topTools.map((tool) => (
                                <span
                                  key={tool.toolName}
                                  className="ag-conv-audit__pill"
                                >
                                  <code>{tool.toolName}</code> ×{tool.count}
                                </span>
                              ))}
                            </div>
                          </div>
                        </section>
                      ) : null}

                      {convDetail.children.length > 0 ? (
                        <section className="ag-token-section">
                          <h4>
                            {t(
                              'antigravityTokenUsage.detail.children',
                              'Child Sessions (Swarm / Subagent)',
                            )}{' '}
                            ({convDetail.children.length})
                          </h4>
                          <div className="ag-conv-children">
                            {convDetail.children.map((child) => (
                              <button
                                key={child.conversationId}
                                type="button"
                                className="ag-conv-child-btn"
                                onClick={() =>
                                  void openConversationDetail(
                                    child.conversationId,
                                  )
                                }
                              >
                                <KindBadge kind={child.kind} />
                                <span>
                                  {child.agentName ||
                                    child.title ||
                                    child.conversationId.slice(0, 8)}
                                </span>
                                <strong>
                                  {formatSessionUsageTokensShort(
                                    child.exactTotalTokens,
                                    lang,
                                    1,
                                  )}
                                </strong>
                              </button>
                            ))}
                          </div>
                        </section>
                      ) : null}

                      {convDetail.modelBreakdown.length > 0 ? (
                        <section className="ag-token-section">
                          <h4>
                            {t(
                              'antigravityTokenUsage.tables.byModel',
                              'By Model',
                            )}
                          </h4>
                          <div className="ag-token-table-wrap">
                            <table className="ag-token-table">
                              <thead>
                                <tr>
                                  <th>
                                    {t(
                                      'antigravityTokenUsage.tables.modelName',
                                      'Model',
                                    )}
                                  </th>
                                  <th className="is-num">
                                    {t(
                                      'antigravityTokenUsage.tables.turns',
                                      'Turns',
                                    )}
                                  </th>
                                  <th className="is-num">Input</th>
                                  <th className="is-num">Thinking</th>
                                  <th className="is-num">Output</th>
                                  <th className="is-num">Cache</th>
                                  <th className="is-num">
                                    {t(
                                      'antigravityTokenUsage.tables.total',
                                      'Total',
                                    )}
                                  </th>
                                </tr>
                              </thead>
                              <tbody>
                                {convDetail.modelBreakdown.map((slice) => (
                                  <tr key={slice.modelId}>
                                    <td>{slice.modelDisplayName}</td>
                                    <td className="is-num">
                                      {formatSessionUsageCount(slice.turnCount)}
                                    </td>
                                    <td className="is-num">
                                      <TokenAmount
                                        value={slice.exact.inputTokens}
                                        lang={lang}
                                      />
                                    </td>
                                    <td className="is-num">
                                      <TokenAmount
                                        value={slice.exact.thinkingTokens}
                                        lang={lang}
                                      />
                                    </td>
                                    <td className="is-num">
                                      <TokenAmount
                                        value={slice.exact.outputTokens}
                                        lang={lang}
                                      />
                                    </td>
                                    <td className="is-num">
                                      <TokenAmount
                                        value={
                                          slice.exact.cacheReadTokens +
                                          slice.exact.cacheWriteTokens
                                        }
                                        lang={lang}
                                      />
                                    </td>
                                    <td className="is-num">
                                      <TokenAmount
                                        value={slice.exact.totalTokens}
                                        lang={lang}
                                      />
                                    </td>
                                  </tr>
                                ))}
                              </tbody>
                            </table>
                          </div>
                        </section>
                      ) : null}

                      <section className="ag-token-section">
                        <h4>
                          {t(
                            'antigravityTokenUsage.detail.turnsTimeline',
                            'Turn-by-Turn Token Timeline',
                          )}{' '}
                          ({convDetail.turns.length})
                        </h4>
                        <div className="ag-token-table-wrap">
                          <table className="ag-token-table">
                            <thead>
                              <tr>
                                <th>#</th>
                                <th>
                                  {t(
                                    'antigravityTokenUsage.tables.updatedAt',
                                    'Updated',
                                  )}
                                </th>
                                <th>
                                  {t(
                                    'antigravityTokenUsage.tables.modelName',
                                    'Model',
                                  )}
                                </th>
                                <th className="is-num">Input</th>
                                <th className="is-num">Thinking</th>
                                <th className="is-num">Output</th>
                                <th className="is-num">Cache</th>
                                <th className="is-num">
                                  {t(
                                    'antigravityTokenUsage.tables.total',
                                    'Total',
                                  )}
                                </th>
                              </tr>
                            </thead>
                            <tbody>
                              {convDetail.turns.map((turn, idx) => (
                                <tr key={`${turn.stepIndex}-${idx}`}>
                                  <td>#{turn.stepIndex}</td>
                                  <td>
                                    {formatDateTimeShort(turn.timestamp, lang)}
                                  </td>
                                  <td>
                                    {turn.modelDisplayName}
                                    {!turn.isExact ? ' (~)' : ''}
                                  </td>
                                  <td className="is-num">
                                    <TokenAmount
                                      value={
                                        turn.isExact
                                          ? turn.exact.inputTokens
                                          : turn.estimated.inputTokens
                                      }
                                      lang={lang}
                                    />
                                  </td>
                                  <td className="is-num">
                                    {turn.isExact ? (
                                      <TokenAmount
                                        value={turn.exact.thinkingTokens}
                                        lang={lang}
                                      />
                                    ) : (
                                      '—'
                                    )}
                                  </td>
                                  <td className="is-num">
                                    <TokenAmount
                                      value={
                                        turn.isExact
                                          ? turn.exact.outputTokens
                                          : turn.estimated.outputTokens
                                      }
                                      lang={lang}
                                    />
                                  </td>
                                  <td className="is-num">
                                    {turn.isExact ? (
                                      <TokenAmount
                                        value={
                                          turn.exact.cacheReadTokens +
                                          turn.exact.cacheWriteTokens
                                        }
                                        lang={lang}
                                      />
                                    ) : (
                                      '—'
                                    )}
                                  </td>
                                  <td className="is-num">
                                    <TokenAmount
                                      value={
                                        turn.isExact
                                          ? turn.exact.totalTokens
                                          : turn.estimated.totalTokens
                                      }
                                      lang={lang}
                                    />
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      </section>
                    </>
                  ) : null}
                </div>
              </div>
            </div>,
            document.body,
          )
        : null}
    </main>
  );
}
