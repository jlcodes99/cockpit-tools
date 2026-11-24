import { buildCodexSessionUsageQuery as buildUsageQuery } from '../../utils/codexSessionUsageQuery';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { ChevronRight, RefreshCw, RotateCcw, X } from 'lucide-react';
import { SingleSelectDropdown, type SingleSelectOption } from '../SingleSelectDropdown';
import * as codexInstanceService from '../../services/codexInstanceService';
import { getCodexProfileLifetimeTokens } from '../../services/codexService';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { isStandardCodexOAuthAccount } from '../../types/codex';
import type {
  CodexSessionUsageQuery,
  CodexSessionUsageReport,
  CodexSessionUsageBreakdownRow,
} from '../../types/codex';
import {
  SESSION_USAGE_SUMMARY_PENDING,
  formatSessionUsageCostUsd,
  formatSessionUsageCount,
  formatSessionUsageTokensShort,
  hasTrustedSessionUsageCache,
  resolveSessionUsageSummaryStatus,
  type SessionUsageSummaryLoadState,
} from '../../utils/codexSessionUsageFormat';
import { CodexUsageTrend, type UsageTrendPoint } from './CodexUsageTrend';
import {
  normalizeCodexSessionUsageRange,
  CODEX_SESSION_USAGE_RANGE_STORAGE_KEY,
} from '../../utils/codexStatsRangePreference';
import { useDurableUiSetting } from '../../hooks/useDurableUiSetting';
import { previewDurableUiSetting } from '../../utils/durableUiSettings';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { CodexAccountPlanBadge } from './CodexAccountPlanBadge';
import { CodexSessionUsageRequests, usageProjectLabel } from './CodexSessionUsageRequests';

const ANALYSIS = 'codex.sessionUsage.analysis';
type UsageTab = 'requests' | 'model' | 'instance' | 'project';
type UsageSort = 'totalTokens' | 'requestCount' | 'estimatedCostUsd';


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
    <strong className="codex-session-usage-amount" title={exact}>
      {compact}
    </strong>
  );
}

export function CodexProfileUsageTotal({ lang }: { lang: string }) {
  const { t } = useTranslation();
  const accounts = useCodexAccountStore((state) => state.accounts);
  const currentAccountId = useCodexAccountStore((state) => state.currentAccount?.id);
  const accountName = useCodexProxyAccountName();
  const [selectedAccountId, setSelectedAccountId] = useState('');
  const [lookup, setLookup] = useState<{
    accountId: string;
    status: 'loading' | 'ready' | 'error';
    value?: number | null;
  } | null>(null);
  const requestId = useRef(0);
  const options = accounts.filter(isStandardCodexOAuthAccount).map((account) => ({
    value: account.id,
    label: accountName(account),
    badge: <CodexAccountPlanBadge account={account} />,
  }));
  const accountId = options.find((option) => option.value === selectedAccountId)?.value
    ?? options.find((option) => option.value === currentAccountId)?.value
    ?? options[0]?.value;
  const result = lookup?.accountId === accountId ? lookup : null;

  const load = async () => {
    if (!accountId) return;
    const request = ++requestId.current;
    setLookup({ accountId, status: 'loading' });
    try {
      const value = await getCodexProfileLifetimeTokens(accountId);
      if (request === requestId.current) setLookup({ accountId, status: 'ready', value });
    } catch {
      if (request === requestId.current) setLookup({ accountId, status: 'error' });
    }
  };

  return (
    <section className="codex-session-usage__profile-source">
      <div className="codex-session-usage__profile-heading">
        <h4>{t('codex.sessionUsage.comparison.profileTotal')}</h4>
        {accountId && (
          <div className="codex-session-usage__profile-value" role="status">
            {result?.status === 'loading' ? t('common.loading')
              : result?.status === 'error' ? t('codex.sessionUsage.comparison.loadError')
                : result?.status === 'ready'
                  ? result.value == null
                    ? t('codex.sessionUsage.comparison.unavailable')
                    : <TokenAmount value={result.value} lang={lang} compactDecimals={2} />
                  : '—'}
          </div>
        )}
      </div>
      {accountId ? (
        <div className="codex-session-usage__profile-controls">
          <SingleSelectDropdown
            value={accountId}
            options={options}
            ariaLabel={t('codex.sessionUsage.comparison.account')}
            onChange={(id) => {
              ++requestId.current;
              setLookup(null);
              setSelectedAccountId(id);
            }}
          />
          <button
            type="button"
            className="btn btn-secondary"
            disabled={result?.status === 'loading'}
            onClick={() => void load()}
          >
            <RefreshCw size={14} aria-hidden="true" />
            {t(result?.status === 'error' ? 'common.retry' : 'common.refresh')}
          </button>
        </div>
      ) : <span>{t('codex.sessionUsage.comparison.noAccount')}</span>}
    </section>
  );
}

function formatRelativeSyncTime(value: number | null | undefined, isZh: boolean): string {
  if (!value) {
    return '';
  }
  const diffSeconds = Math.max(0, Math.floor(Date.now() / 1000) - value);
  if (diffSeconds < 60) {
    return isZh ? '刚刚' : 'just now';
  }
  if (diffSeconds < 3600) {
    const minutes = Math.floor(diffSeconds / 60);
    return isZh ? `${minutes} 分钟前` : `${minutes}m ago`;
  }
  if (diffSeconds < 86400) {
    const hours = Math.floor(diffSeconds / 3600);
    return isZh ? `${hours} 小时前` : `${hours}h ago`;
  }
  const days = Math.floor(diffSeconds / 86400);
  return isZh ? `${days} 天前` : `${days}d ago`;
}

function UsageBreakdownTable({
  rows,
  emptyLabel,
  keyLabel,
  lang,
  showEstimatedCost = false,
  onSelect,
}: {
  rows: CodexSessionUsageBreakdownRow[];
  emptyLabel: string;
  keyLabel: string;
  lang: string;
  showEstimatedCost?: boolean;
  onSelect?: (row: CodexSessionUsageBreakdownRow) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="codex-session-usage-table-wrap">
      <table className="codex-session-usage-table">
        <thead>
          <tr>
            <th>{keyLabel}</th>
            <th>{t('codex.sessionUsage.tables.input', '输入 Tokens')}</th>
            <th>{t('codex.sessionUsage.tables.cached', '缓存 Tokens')}</th>
            <th>{t('codex.sessionUsage.tables.output', '输出 Tokens')}</th>
            <th>{t('codex.sessionUsage.tables.total', '合计 Tokens')}</th>
            <th>{t('codex.sessionUsage.tables.requests', '请求')}</th>
            {showEstimatedCost ? <th>{t('codex.sessionUsage.cards.cost')}</th> : null}
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={showEstimatedCost ? 7 : 6}>{emptyLabel}</td>
            </tr>
          ) : (
            rows.map((row) => (
              <tr key={row.key || row.label}>
                <td title={row.key || row.label}>
                  {onSelect ? (
                    <button type="button" className="btn codex-session-usage__row-button" onClick={() => onSelect(row)}>
                      <span>{row.label || row.key || '—'}</span><ChevronRight size={14} aria-hidden="true" />
                    </button>
                  ) : row.label || row.key || '—'}
                </td>
                <td>
                  <TokenAmount value={row.inputTokens} lang={lang} />
                </td>
                <td>
                  <TokenAmount value={row.cachedInputTokens} lang={lang} />
                </td>
                <td>
                  <TokenAmount value={row.outputTokens} lang={lang} />
                </td>
                <td>
                  <TokenAmount value={row.totalTokens} lang={lang} />
                </td>
                <td>{formatSessionUsageCount(row.requestCount)}</td>
                {showEstimatedCost ? <td title={row.estimatedCostUsd == null ? t(`${ANALYSIS}.unpriced`) : undefined}>
                  {row.estimatedCostUsd == null ? '—' : formatSessionUsageCostUsd(row.estimatedCostUsd)}
                </td> : null}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}

export function CodexSessionUsageSummary({
  onOpenDetail,
}: {
  onOpenDetail: () => void;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || 'zh-CN';
  const [report, setReport] = useState<CodexSessionUsageReport | null>(null);
  const [loadState, setLoadState] = useState<SessionUsageSummaryLoadState>('scanning');
  const range = normalizeCodexSessionUsageRange(useDurableUiSetting(CODEX_SESSION_USAGE_RANGE_STORAGE_KEY));
  const query = useMemo(() => buildUsageQuery(range, ''), [range]);

  useEffect(() => {
    let cancelled = false;
    setLoadState('scanning');
    void (async () => {
      let trusted = false;
      try {
        const cached = await codexInstanceService.querySessionUsage(query);
        if (cancelled) return;
        trusted = hasTrustedSessionUsageCache(cached);
        setReport(cached);
        setLoadState(trusted ? 'updating' : 'scanning');
      } catch {
        if (!cancelled) setReport(null);
      }
      try {
        const synced = await codexInstanceService.syncSessionUsage({
          rebuild: false,
          query,
        });
        if (cancelled) return;
        if (synced.report) {
          setReport(synced.report);
        }
        setLoadState('ready');
      } catch {
        if (cancelled) return;
        // 列表页只展示已有汇总，扫描失败不挡会话列表。
        setLoadState(trusted ? 'ready' : 'failed');
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [query]);

  const trusted = hasTrustedSessionUsageCache(report);
  const status = resolveSessionUsageSummaryStatus(loadState, trusted);
  const totals = report?.totals;
  const metrics = [
    [
      t('codex.sessionUsage.summary.input', '输入'),
      trusted
        ? formatSessionUsageTokensShort(totals?.inputTokens ?? 0, lang)
        : SESSION_USAGE_SUMMARY_PENDING,
    ],
    [
      t('codex.sessionUsage.summary.cached', '缓存'),
      trusted
        ? formatSessionUsageTokensShort(totals?.cachedInputTokens ?? 0, lang)
        : SESSION_USAGE_SUMMARY_PENDING,
    ],
    [
      t('codex.sessionUsage.summary.output', '输出'),
      trusted
        ? formatSessionUsageTokensShort(totals?.outputTokens ?? 0, lang)
        : SESSION_USAGE_SUMMARY_PENDING,
    ],
    [
      t('codex.sessionUsage.summary.total', '合计'),
      trusted
        ? formatSessionUsageTokensShort(totals?.totalTokens ?? 0, lang, 2)
        : SESSION_USAGE_SUMMARY_PENDING,
    ],
    [
      t('codex.sessionUsage.summary.requests', '请求'),
      trusted
        ? formatSessionUsageCount(totals?.requestCount ?? 0)
        : SESSION_USAGE_SUMMARY_PENDING,
    ],
    [
      t('codex.sessionUsage.summary.cost', '费用'),
      trusted
        ? formatSessionUsageCostUsd(totals?.estimatedCostUsd)
        : SESSION_USAGE_SUMMARY_PENDING,
    ],
  ] as const;

  return (
    <div className="codex-session-usage-summary">
      <div className="codex-session-usage-summary__title">
        {t(`codex.sessionUsage.range.${range}`)}
        {status ? (
          <span
            className={`codex-session-usage-summary__status is-${status}`}
            role="status"
          >
            {status === 'scanning'
              ? t('codex.sessionUsage.summary.scanning', '扫描中')
              : t('codex.sessionUsage.summary.updating', '更新中')}
          </span>
        ) : null}
      </div>
      <div className="codex-session-usage-summary__metrics">
        {metrics.map(([label, value]) => (
          <span key={label} className="codex-session-usage-summary__metric">
            <em>{label}</em>
            <strong className={trusted ? undefined : 'is-pending'}>{value}</strong>
          </span>
        ))}
      </div>
      <button
        type="button"
        className="btn btn-sm btn-secondary codex-session-usage-summary__detail"
        onClick={onOpenDetail}
      >
        {t('codex.sessionUsage.summary.detail', '详情')}
      </button>
    </div>
  );
}

export function CodexSessionUsagePanel({
  onBack,
  sessionNames,
}: {
  onBack?: () => void;
  sessionNames?: ReadonlyMap<string, string>;
} = {}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || 'zh-CN';
  const isZh = lang.toLowerCase().startsWith('zh');
  const range = normalizeCodexSessionUsageRange(useDurableUiSetting(CODEX_SESSION_USAGE_RANGE_STORAGE_KEY));
  const [instanceId, setInstanceId] = useState('');
  const [model, setModel] = useState('');
  const [project, setProject] = useState<string>();
  const [session, setSession] = useState<{ id: string; name: string }>();
  const [tab, setTab] = useState<UsageTab>('requests');
  const [sort, setSort] = useState<UsageSort>('totalTokens');
  const [snapshot, setSnapshot] = useState<{
    scope: CodexSessionUsageQuery;
    report: CodexSessionUsageReport;
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [rebuilding, setRebuilding] = useState(false);
  const [readError, setReadError] = useState('');
  const [scanError, setScanError] = useState('');
  const readVersion = useRef(0);
  const scanInProgress = useRef(false);
  const mounted = useRef(false);

  const query = useMemo<CodexSessionUsageQuery>(() => ({
    ...buildUsageQuery(range, instanceId),
    model: model || undefined,
    projectKey: project,
    sessionId: session?.id,
    includeProjects: true,
  }), [instanceId, model, project, range, session?.id]);
  const latestQuery = useRef(query);
  const latestReadQuery = useRef(query);

  const loadCached = useCallback(async (
    scope: CodexSessionUsageQuery,
    nextQuery = scope,
  ) => {
    const version = ++readVersion.current;
    latestReadQuery.current = nextQuery;
    setLoading(true);
    setReadError('');
    try {
      const nextReport = await codexInstanceService.querySessionUsage(nextQuery);
      if (version === readVersion.current) {
        setSnapshot({ scope, report: nextReport });
      }
    } catch (loadError) {
      if (version === readVersion.current) {
        setReadError(
          t('codex.sessionUsage.errors.loadFailed', {
            error: loadError instanceof Error ? loadError.message : String(loadError),
            defaultValue: '读取会话用量失败：{{error}}',
          }),
        );
      }
    } finally {
      if (version === readVersion.current) {
        setLoading(false);
      }
    }
  }, [t]);

  const runSync = useCallback(async (rebuild: boolean) => {
    if (!mounted.current || scanInProgress.current) return;
    scanInProgress.current = true;
    const initialQuery = latestQuery.current;
    const syncQuery = { ...initialQuery, ...buildUsageQuery(range, instanceId) };
    setSyncing(true);
    setRebuilding(rebuild);
    setScanError('');
    try {
      const result = await codexInstanceService.syncSessionUsage({
        rebuild,
        query: syncQuery,
      });
      if (!mounted.current) return;
      setScanError(result.errors[0] ?? '');
      const sameQuery = initialQuery === latestQuery.current;
      if (result.report && sameQuery) {
        // An older cache read may still be in flight.
        ++readVersion.current;
        latestReadQuery.current = syncQuery;
        setSnapshot({ scope: initialQuery, report: result.report });
        setReadError('');
        setLoading(false);
      } else {
        // Keep the query identity used for the current filters, even when refresh
        // moves its end timestamp forward.
        if (sameQuery) {
          await loadCached(initialQuery, syncQuery);
        } else {
          await loadCached(latestQuery.current);
        }
      }
    } catch (syncError) {
      if (mounted.current) {
        setScanError(
          t('codex.sessionUsage.errors.syncFailed', {
            error: syncError instanceof Error ? syncError.message : String(syncError),
            defaultValue: '扫描会话用量失败：{{error}}',
          }),
        );
      }
    } finally {
      scanInProgress.current = false;
      if (mounted.current) {
        setSyncing(false);
        setRebuilding(false);
      }
    }
  }, [instanceId, loadCached, range, t]);

  useEffect(() => {
    latestQuery.current = query;
    void loadCached(query);
    return () => { ++readVersion.current; };
  }, [loadCached, query]);

  useEffect(() => {
    mounted.current = true;
    void runSync(false);
    return () => { mounted.current = false; };
    // 仅在首次进入面板时增量扫描，切换筛选只读缓存。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let active = true;
    const subscription = listen('codex-session-usage-projects-updated', () => {
      if (active) void loadCached(latestQuery.current, latestReadQuery.current);
    });
    void subscription.catch(console.error);
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten(), () => {});
    };
  }, [loadCached]);

  const handleRebuild = useCallback(async () => {
    const confirmed = await confirmDialog(
      t(
        'codex.sessionUsage.rebuildConfirm.message',
        '会清空已汇总的会话用量缓存，再重新读取全部会话日志。官方配额和 API 服务日志不受影响。确认继续？',
      ),
      {
        title: t('codex.sessionUsage.rebuildConfirm.title', '重新扫描'),
        kind: 'warning',
      },
    );
    if (!confirmed) {
      return;
    }
    await runSync(true);
  }, [runSync, t]);

  const report = snapshot?.scope === query ? snapshot.report : null;
  const knownReport = snapshot?.report;
  const detailQuery = useMemo<CodexSessionUsageQuery>(() => report ? {
    ...query,
    fromTimestamp: report.fromTimestamp ?? query.fromTimestamp,
    toTimestamp: report.toTimestamp ?? query.toTimestamp,
  } : query, [query, report]);

  const rangeOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: 'today', label: t('codex.sessionUsage.range.today') },
      { value: '7d', label: t('codex.sessionUsage.range.7d', '近 7 天') },
      { value: '30d', label: t('codex.sessionUsage.range.30d', '近 30 天') },
      { value: 'month', label: t('codex.sessionUsage.range.month', '本月') },
      { value: 'all', label: t('codex.sessionUsage.range.all', '全部') },
    ],
    [t],
  );
  const instanceOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: '', label: t('codex.sessionUsage.instance.all', '全部实例') },
      ...(knownReport?.instances ?? []).map((instance) => ({
        value: instance.id,
        label: instance.name || instance.id,
      })),
    ],
    [knownReport?.instances, t],
  );

  const modelOptions = [
    { value: '', label: t(`${ANALYSIS}.allModels`) },
    ...(knownReport?.modelOptions ?? []).map(({ id, name }) => ({ value: id, label: name })),
  ];
  if (model && !modelOptions.some((option) => option.value === model)) {
    modelOptions.push({ value: model, label: model });
  }

  const projectOptions = [
    { value: '', label: t(`${ANALYSIS}.allProjects`) },
    ...(knownReport?.projectOptions ?? []).map(({ id, name }) => ({
      value: `project:${id}`, label: name || t(`${ANALYSIS}.unknownProject`),
    })),
  ];
  if (project !== undefined && !projectOptions.some((option) => option.value === `project:${project}`)) {
    projectOptions.push({ value: `project:${project}`, label: project || t(`${ANALYSIS}.unknownProject`) });
  }
  const sortOptions: SingleSelectOption[] = [
    { value: 'totalTokens', label: t('codex.sessionUsage.cards.total') },
    { value: 'requestCount', label: t('codex.sessionUsage.cards.requests') },
    { value: 'estimatedCostUsd', label: t(`${ANALYSIS}.referenceCost`) },
  ];
  const breakdown = {
    model: report?.byModel ?? [],
    instance: report?.byInstance ?? [],
    project: report?.byProject ?? [],
  };
  const sortedRows = tab === 'requests' ? [] : [...breakdown[tab]].sort((left, right) =>
    (right[sort] ?? -1) - (left[sort] ?? -1) || left.label.localeCompare(right.label, lang),
  );
  const selectBreakdown = (row: CodexSessionUsageBreakdownRow) => {
    if (tab === 'model') setModel(row.key);
    if (tab === 'instance') setInstanceId(row.key);
    if (tab === 'project') setProject(row.key);
    setTab('requests');
  };

  const totals = report?.totals;
  const missingPrice = report?.byModel.some((row) => row.estimatedCostUsd == null);
  const hasData = (totals?.requestCount ?? 0) > 0;
  const lastSyncedLabel = formatRelativeSyncTime(report?.lastSyncedAt, isZh);
  const error = scanError || readError;
  const sessionTrend = useMemo<UsageTrendPoint[]>(
    () =>
      (report?.byDay ?? [])
        .map((row) => ({
          bucketStart: new Date(`${row.key}T00:00:00`).getTime(),
          usage: {
            totalTokens: row.totalTokens,
            inputTokens: row.inputTokens,
            outputTokens: row.outputTokens,
            requestCount: row.requestCount,
            estimatedCostUsd: 0,
          },
        }))
        .filter((point) => Number.isFinite(point.bucketStart))
        .sort((left, right) => left.bucketStart - right.bucketStart),
    [report?.byDay],
  );

  return (
    <div className="codex-session-usage">
      <div className="codex-session-usage__intro">
        <div>
          <h3>{t('codex.sessionUsage.title', '会话用量')}</h3>
          <p>{t('codex.sessionUsage.desc', '从本机 Codex 会话日志汇总真实 Token 用量，不依赖官方配额或 API 服务请求日志。')}</p>
        </div>
        {onBack ? (
          <button
            type="button"
            className="btn btn-secondary"
            onClick={onBack}
          >
            {t('codex.sessionUsage.summary.back', '返回')}
          </button>
        ) : null}
      </div>

      <div className="codex-session-usage__toolbar">
        <label className="codex-session-usage__field">
          <span>{t('codex.sessionUsage.range.label', '时间范围')}</span>
          <SingleSelectDropdown
            value={range}
            options={rangeOptions}
            onChange={(value) => {
              const nextRange = normalizeCodexSessionUsageRange(value);
              previewDurableUiSetting(CODEX_SESSION_USAGE_RANGE_STORAGE_KEY, nextRange);
            }}
            ariaLabel={t('codex.sessionUsage.range.label', '时间范围')}
            menuWidth={140}
            menuMaxHeight={220}
          />
        </label>
        <label className="codex-session-usage__field">
          <span>{t('codex.sessionUsage.instance.label', '实例')}</span>
          <SingleSelectDropdown
            value={instanceId}
            options={instanceOptions}
            onChange={setInstanceId}
            ariaLabel={t('codex.sessionUsage.instance.label', '实例')}
            menuWidth={200}
            menuMaxHeight={260}
          />
        </label>
        <div className="codex-session-usage__field">
          <span>{t('codex.sessionUsage.tables.modelName')}</span>
          <SingleSelectDropdown value={model} options={modelOptions} onChange={setModel}
            ariaLabel={t('codex.sessionUsage.tables.modelName')}
            search={{ label: t('common.search'), emptyLabel: t(`${ANALYSIS}.noResults`) }}
          />
        </div>
        <div className="codex-session-usage__field">
          <span>{t(`${ANALYSIS}.project`)}</span>
          <SingleSelectDropdown value={project === undefined ? '' : `project:${project}`}
            options={projectOptions} onChange={(value) => setProject(value ? value.slice('project:'.length) : undefined)}
            ariaLabel={t(`${ANALYSIS}.project`)}
            search={{ label: t('common.search'), emptyLabel: t(`${ANALYSIS}.noResults`) }}
          />
        </div>
        <div className="codex-session-usage__actions">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => void runSync(false)}
            disabled={syncing}
          >
            <RefreshCw size={14} className={syncing && !rebuilding ? 'spin' : undefined} />
            {t('codex.sessionUsage.actions.refresh', '刷新')}
          </button>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => void handleRebuild()}
            disabled={syncing}
          >
            <RotateCcw size={14} className={rebuilding ? 'spin' : undefined} />
            {t('codex.sessionUsage.actions.rebuild', '重新扫描')}
          </button>
        </div>
      </div>

      <div className="codex-session-usage__status" role="status">
        {syncing
          ? t('codex.sessionUsage.status.scanning', '正在扫描会话日志…')
          : hasData || lastSyncedLabel
            ? t('codex.sessionUsage.status.ready', {
                files: report?.filesTracked ?? 0,
                requests: report?.eventCount ?? 0,
                defaultValue: '已扫描 {{files}} 个文件，累计 {{requests}} 次请求',
              })
            : t('codex.sessionUsage.status.idle', '尚未扫描')}
        {lastSyncedLabel ? (
          <span>
            {t('codex.sessionUsage.status.lastSynced', {
              time: lastSyncedLabel,
              defaultValue: '上次同步 {{time}}',
            })}
          </span>
        ) : null}
        {(report?.deferredFiles ?? 0) > 0 ? (
          <span>
            {t('codex.sessionUsage.status.deferred', {
              count: report?.deferredFiles ?? 0,
              defaultValue: '{{count}} 个分叉会话待父会话就绪',
            })}
          </span>
        ) : null}
        {(report?.lastErrorCount ?? 0) > 0 ? (
          <span>
            {t('codex.sessionUsage.status.errors', {
              count: report?.lastErrorCount ?? 0,
              defaultValue: '{{count}} 个文件解析失败',
            })}
          </span>
        ) : null}
      </div>

      {error ? <div className="codex-session-usage__error">{error}</div> : null}

      {session && (
        <button type="button" className="btn btn-secondary codex-session-usage__session-filter"
          title={session.name} onClick={() => setSession(undefined)}
          aria-label={t(`${ANALYSIS}.clearSession`)}>
          <span>{session.name}</span><X size={14} aria-hidden="true" />
        </button>
      )}

      <section className="codex-session-usage__overview" aria-label={t(`${ANALYSIS}.localUsage`)} aria-busy={loading}>
        <div className="codex-session-usage__overview-header">
          <h4>{t(`${ANALYSIS}.localUsage`)}</h4>
          <span>{t(`${ANALYSIS}.selectedScope`)}</span>
        </div>
        <dl className="codex-session-usage__metrics">
          <div>
            <dt>{t('codex.sessionUsage.cards.total')}</dt>
            <dd>{totals ? <TokenAmount value={totals.totalTokens} lang={lang} compactDecimals={2} /> : '—'}</dd>
          </div>
          <div>
            <dt>{t('codex.sessionUsage.cards.requests')}</dt>
            <dd>{totals ? formatSessionUsageCount(totals.requestCount) : '—'}</dd>
          </div>
          <div>
            <dt>{t(`${ANALYSIS}.referenceCost`)}</dt>
            <dd title={t(missingPrice ? `${ANALYSIS}.unpriced` : `${ANALYSIS}.costHelp`)}>
              {!totals || missingPrice ? '—' : formatSessionUsageCostUsd(totals.estimatedCostUsd)}
            </dd>
          </div>
          {([
            ['codex.sessionUsage.cards.input', totals?.inputTokens],
            ['codex.sessionUsage.cards.cached', totals?.cachedInputTokens],
            ['codex.sessionUsage.cards.output', totals?.outputTokens],
          ] as const).map(([label, value]) => (
            <div key={label}>
              <dt title={t(label)}>{t(label)}</dt>
              <dd>{value != null ? <TokenAmount value={value} lang={lang} /> : '—'}</dd>
            </div>
          ))}
        </dl>
      </section>

      <CodexProfileUsageTotal lang={lang} />

      {report && hasData && (
        <CodexUsageTrend
          points={sessionTrend}
          metrics={['totalTokens', 'requestCount']}
          hourly={false}
          title={t('codex.sessionUsage.title')}
          emptyLabel={t('codex.sessionUsage.empty.title')}
          labels={{
            totalTokens: t('codex.sessionUsage.cards.total'),
            requestCount: t('codex.sessionUsage.cards.requests'),
          }}
        />
      )}

      <section className="codex-session-usage__analysis" aria-busy={loading}>
        <div className="codex-session-usage__analysis-header">
          <div className="codex-session-usage__tabs" role="group" aria-label={t('codex.sessionUsage.title')}>
            {(['requests', 'model', 'instance', 'project'] as const).map((item) => (
              <button key={item} type="button" className="btn codex-session-usage__tab"
                aria-pressed={tab === item} onClick={() => setTab(item)}>
                {t(item === 'model' ? 'codex.sessionUsage.tables.modelName'
                  : item === 'instance' ? 'codex.sessionUsage.tables.instanceName'
                    : `${ANALYSIS}.${item}`)}
              </button>
            ))}
          </div>
          {tab !== 'requests' && (
            <SingleSelectDropdown value={sort} options={sortOptions}
              ariaLabel={t(`${ANALYSIS}.sort`)} onChange={(value) => setSort(value as UsageSort)} />
          )}
        </div>
        {report?.projectsPending && (
          <p className="codex-session-usage__index-status" role="status">{t(`${ANALYSIS}.indexingProjects`)}</p>
        )}
        {tab === 'requests' ? (
          <CodexSessionUsageRequests key={JSON.stringify(query)} query={detailQuery} revision={report}
            lang={lang} sessionNames={sessionNames}
            onSelectSession={(id, name) => setSession({ id, name })} />
        ) : (
          <UsageBreakdownTable
            rows={tab === 'project' ? sortedRows.map((row) => ({
              ...row, label: usageProjectLabel(row.label) || t(`${ANALYSIS}.unknownProject`),
            })) : sortedRows}
            keyLabel={t(tab === 'model' ? 'codex.sessionUsage.tables.modelName'
              : tab === 'instance' ? 'codex.sessionUsage.tables.instanceName' : `${ANALYSIS}.project`)}
            emptyLabel={t(loading ? 'common.loading' : `${ANALYSIS}.noResults`)}
            showEstimatedCost
            lang={lang}
            onSelect={selectBreakdown}
          />
        )}
      </section>

      {report?.byDay.length ? (
        <details className="codex-session-usage__daily">
          <summary>{t('codex.sessionUsage.tables.day')}</summary>
          <UsageBreakdownTable
            rows={report.byDay}
            showEstimatedCost
            keyLabel={t('codex.sessionUsage.tables.dayName')}
            emptyLabel={t(`${ANALYSIS}.noResults`)}
            lang={lang}
          />
          {report.byDay.some((row) => row.estimatedCostUsd == null) && (
            <p className="codex-session-usage__index-status">{t('codex.sessionUsage.costUnavailable')}</p>
          )}
        </details>
      ) : null}
    </div>
  );
}
