import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { RefreshCw, RotateCcw } from 'lucide-react';
import { SingleSelectDropdown, type SingleSelectOption } from '../SingleSelectDropdown';
import * as codexInstanceService from '../../services/codexInstanceService';
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

type UsageRange = '7d' | '30d' | 'month' | 'all';

function startOfLocalDay(value: Date): Date {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate());
}

function buildUsageQuery(range: UsageRange, instanceId: string): CodexSessionUsageQuery {
  const now = new Date();
  const toTimestamp = Math.floor(now.getTime() / 1000);
  let fromTimestamp: number | null = null;
  if (range === '7d') {
    fromTimestamp = Math.floor(startOfLocalDay(now).getTime() / 1000) - 6 * 24 * 60 * 60;
  } else if (range === '30d') {
    fromTimestamp = Math.floor(startOfLocalDay(now).getTime() / 1000) - 29 * 24 * 60 * 60;
  } else if (range === 'month') {
    fromTimestamp = Math.floor(new Date(now.getFullYear(), now.getMonth(), 1).getTime() / 1000);
  }
  return {
    fromTimestamp,
    toTimestamp,
    instanceId: instanceId || null,
  };
}

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
}: {
  rows: CodexSessionUsageBreakdownRow[];
  emptyLabel: string;
  keyLabel: string;
  lang: string;
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
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={6}>{emptyLabel}</td>
            </tr>
          ) : (
            rows.map((row) => (
              <tr key={row.key || row.label}>
                <td title={row.label || row.key}>{row.label || row.key || '—'}</td>
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
  const query = useMemo(() => buildUsageQuery('30d', ''), []);

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
        {t('codex.sessionUsage.summary.title', '近 30 天')}
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
}: {
  onBack?: () => void;
} = {}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language || 'zh-CN';
  const isZh = lang.toLowerCase().startsWith('zh');
  const [range, setRange] = useState<UsageRange>('30d');
  const [instanceId, setInstanceId] = useState('');
  const [report, setReport] = useState<CodexSessionUsageReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [rebuilding, setRebuilding] = useState(false);
  const [error, setError] = useState('');
  const requestVersionRef = useRef(0);

  const query = useMemo(() => buildUsageQuery(range, instanceId), [instanceId, range]);

  const loadCached = useCallback(async (nextQuery: CodexSessionUsageQuery) => {
    const version = ++requestVersionRef.current;
    setLoading(true);
    setError('');
    try {
      const nextReport = await codexInstanceService.querySessionUsage(nextQuery);
      if (version === requestVersionRef.current) {
        setReport(nextReport);
      }
    } catch (loadError) {
      if (version === requestVersionRef.current) {
        setError(
          t('codex.sessionUsage.errors.loadFailed', {
            error: loadError instanceof Error ? loadError.message : String(loadError),
            defaultValue: '读取会话用量失败：{{error}}',
          }),
        );
      }
    } finally {
      if (version === requestVersionRef.current) {
        setLoading(false);
      }
    }
  }, [t]);

  const runSync = useCallback(async (rebuild: boolean) => {
    const version = ++requestVersionRef.current;
    setSyncing(true);
    if (rebuild) {
      setRebuilding(true);
    }
    setError('');
    try {
      const result = await codexInstanceService.syncSessionUsage({
        rebuild,
        query,
      });
      if (version === requestVersionRef.current) {
        if (result.report) {
          setReport(result.report);
        } else {
          const nextReport = await codexInstanceService.querySessionUsage(query);
          setReport(nextReport);
        }
        if (result.errors.length > 0) {
          setError(result.errors[0]);
        }
      }
    } catch (syncError) {
      if (version === requestVersionRef.current) {
        setError(
          t('codex.sessionUsage.errors.syncFailed', {
            error: syncError instanceof Error ? syncError.message : String(syncError),
            defaultValue: '扫描会话用量失败：{{error}}',
          }),
        );
      }
    } finally {
      if (version === requestVersionRef.current) {
        setSyncing(false);
        setRebuilding(false);
        setLoading(false);
      }
    }
  }, [query, t]);

  useEffect(() => {
    void loadCached(query);
  }, [loadCached, query]);

  useEffect(() => {
    void runSync(false);
    // 仅在首次进入面板时增量扫描，切换筛选只读缓存。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  const rangeOptions = useMemo<SingleSelectOption[]>(
    () => [
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
      ...(report?.instances ?? []).map((instance) => ({
        value: instance.id,
        label: instance.name || instance.id,
      })),
    ],
    [report?.instances, t],
  );

  const totals = report?.totals;
  const hasData = (totals?.requestCount ?? 0) > 0 || (report?.eventCount ?? 0) > 0;
  const lastSyncedLabel = formatRelativeSyncTime(report?.lastSyncedAt, isZh);

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
            onChange={(value) => setRange(value as UsageRange)}
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
          : hasData
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

      <p className="codex-session-usage__hint">
        {t(
          'codex.sessionUsage.hint',
          '官方配额是套餐额度；API 服务日志只覆盖走本地网关的请求。这里按会话 JSONL 的每次请求用量增量汇总。',
        )}
      </p>

      {loading && !report ? (
        <div className="codex-session-usage__empty">
          {t('common.loading', '加载中...')}
        </div>
      ) : !hasData ? (
        <div className="codex-session-usage__empty">
          <h4>{t('codex.sessionUsage.empty.title', '还没有会话用量')}</h4>
          <p>
            {t(
              'codex.sessionUsage.empty.desc',
              '打开此页后会在后台扫描本机 Codex 会话日志及多开会话目录。若刚用过 Codex，点刷新即可。',
            )}
          </p>
        </div>
      ) : (
        <>
          <div className="codex-session-usage__cards">
            <article>
              <span>{t('codex.sessionUsage.cards.input', '输入 Tokens')}</span>
              <TokenAmount
                value={totals?.inputTokens ?? 0}
                lang={lang}
              />
            </article>
            <article>
              <span>{t('codex.sessionUsage.cards.cached', '缓存输入 Tokens')}</span>
              <TokenAmount
                value={totals?.cachedInputTokens ?? 0}
                lang={lang}
              />
            </article>
            <article>
              <span>{t('codex.sessionUsage.cards.output', '输出 Tokens')}</span>
              <TokenAmount
                value={totals?.outputTokens ?? 0}
                lang={lang}
              />
            </article>
            <article>
              <span>{t('codex.sessionUsage.cards.total', '合计 Tokens')}</span>
              <TokenAmount
                value={totals?.totalTokens ?? 0}
                lang={lang}
                compactDecimals={2}
              />
            </article>
            <article>
              <span>{t('codex.sessionUsage.cards.requests', '请求数')}</span>
              <strong>{formatSessionUsageCount(totals?.requestCount ?? 0)}</strong>
            </article>
            <article>
              <span>{t('codex.sessionUsage.cards.cost', '估算费用')}</span>
              <strong>{formatSessionUsageCostUsd(totals?.estimatedCostUsd)}</strong>
            </article>
          </div>

          <div className="codex-session-usage__grids">
            <section>
              <h4>{t('codex.sessionUsage.tables.model', '按模型')}</h4>
              <UsageBreakdownTable
                rows={report?.byModel ?? []}
                keyLabel={t('codex.sessionUsage.tables.modelName', '模型')}
                emptyLabel={t('codex.sessionUsage.empty.title', '还没有会话用量')}
                lang={lang}
              />
            </section>
            <section>
              <h4>{t('codex.sessionUsage.tables.instance', '按实例')}</h4>
              <UsageBreakdownTable
                rows={report?.byInstance ?? []}
                keyLabel={t('codex.sessionUsage.tables.instanceName', '实例')}
                emptyLabel={t('codex.sessionUsage.empty.title', '还没有会话用量')}
                lang={lang}
              />
            </section>
          </div>

          <section>
            <h4>{t('codex.sessionUsage.tables.day', '按日期')}</h4>
            <UsageBreakdownTable
              rows={report?.byDay ?? []}
              keyLabel={t('codex.sessionUsage.tables.dayName', '日期')}
              emptyLabel={t('codex.sessionUsage.empty.title', '还没有会话用量')}
              lang={lang}
            />
          </section>
        </>
      )}
    </div>
  );
}
