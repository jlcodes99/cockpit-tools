import { useEffect, useMemo, useRef, useState } from 'react';
import type { TFunction } from 'i18next';
import type { CodexApiKeyUsageState } from '../../services/codexApiKeyUsageRefreshService';
import type { CodexRemoteKeyRequests as RequestPage, CodexRemoteKeyStats } from '../../types/codexRemoteService';
import { useCodexRemoteKeyData } from '../../services/codexRemoteKeyData';
import { customRemoteKeyRange, remoteKeyRange, type RemoteKeyRange } from '../../utils/codexRemoteKeyRange';
import { formatBudgetUsd } from '../../utils/codexKeyBudget';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { CodexUsageTrend } from './CodexUsageTrend';
import { CodexRemoteKeyRequests } from './CodexRemoteKeyRequests';
import './CodexRemoteService.css';

type Cursor = { beforeTime: number; beforeId: number } | null;
export function CodexRemoteServiceDetails({ state, baseUrl, apiKey, t }: {
  state: CodexApiKeyUsageState; baseUrl: string; apiKey: string; t: TFunction;
}) {
  const remote = state.summary!.cockpit!;
  const budget = remote.budget;
  const unlimited = t('codex.apiService.keys.tokenLimitUnlimited');
  const [preset, setPreset] = useState<RemoteKeyRange>('7d');
  const [range, setRange] = useState(() => remoteKeyRange('7d'));
  const [dates, setDates] = useState(() => ({ start: new Date(range.startAt).toISOString().slice(0, 10), end: new Date(range.endAt - 1).toISOString().slice(0, 10) }));
  const [dateError, setDateError] = useState(false);
  const [revision, setRevision] = useState(0);
  const [model, setModel] = useState('');
  const [status, setStatus] = useState('all');
  const [kind, setKind] = useState('all');
  const [cursors, setCursors] = useState<Cursor[]>([null]);
  const [costBasis, setCostBasis] = useState('settled');
  const dateRef = useRef<HTMLInputElement>(null);
  const errorRef = useRef<HTMLDivElement>(null);
  const stats = useCodexRemoteKeyData<CodexRemoteKeyStats>(baseUrl, apiKey, 'stats', range, revision);
  const query = useMemo(() => ({ ...range, ...cursors[cursors.length - 1], pageSize: 30,
    ...(model ? { model } : {}), ...(status === 'all' ? {} : { success: status === 'success' }),
    ...(kind === 'all' ? {} : { kind }),
  }), [range, cursors, model, status, kind]);
  const logs = useCodexRemoteKeyData<RequestPage>(baseUrl, apiKey, 'requests', query, revision);
  useEffect(() => { setCursors([null]); }, [range.startAt, range.endAt]);
  const failed = stats.error || logs.error || Boolean(state.error);
  useEffect(() => { if (failed) errorRef.current?.scrollIntoView({ block: 'nearest' }); }, [failed]);
  useEffect(() => { setRevision(value => value + 1); }, [state.updatedAt]);
  useEffect(() => {
    if (preset === 'custom') return;
    const update = () => {
      const next = remoteKeyRange(preset);
      setRange(previous => previous.startAt === next.startAt && previous.endAt === next.endAt ? previous : next);
    };
    const timer = setInterval(update, 60_000);
    return () => clearInterval(timer);
  }, [preset]);
  const selectRange = (next: RemoteKeyRange) => {
    setDateError(false); setPreset(next); setCursors([null]);
    if (next !== 'custom') setRange(remoteKeyRange(next));
  };
  const applyDates = () => {
    const next = customRemoteKeyRange(dates.start, dates.end);
    setDateError(!next);
    if (!next) { dateRef.current?.focus(); dateRef.current?.scrollIntoView({ block: 'nearest' }); return; }
    setRange(next); setCursors([null]);
  };
  const count = (value?: number | null) => value == null ? '—' : new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value);
  const money = (value: number | null) => value == null ? unlimited : formatBudgetUsd(value);
  const metric = (label: string, value: string, note?: string) => <div className="codex-remote-detail-value" key={label}><span>{label}</span><strong>{value}</strong>{note && <small className="codex-remote-note">{note}</small>}</div>;
  const usage = stats.data?.summary;
  const statusKey = ['active', 'disabled', 'service_disabled', 'token_limit_reached', 'budget_limit_reached'].includes(remote.status) ? remote.status : 'disabled';
  const outcomes = usage ? `${t('common.codexUsage.success')} ${count(usage.successCount)} · ${t('common.codexUsage.failed')} ${count(Math.max(0, usage.requestCount - usage.successCount - usage.canceledCount))} · ${t('common.codexUsage.cancelled')} ${count(usage.canceledCount)}` : undefined;
  const points = stats.data?.trend?.map(point => ({ ...point, usage: { ...point.usage,
    estimatedCostUsd: costBasis === 'settled' ? point.usage.settledMicrousd / 1_000_000 : point.usage.estimatedCostUsd,
  } }));
  return <div className="codex-remote-details">
    <section className="cockpit-api-panel-section codex-remote-section codex-remote-budget">
      <h3>{t('common.apiBudget.budget')} · {t(`common.apiBudget.${budget.policy.period}`)}</h3>
      <div className="codex-remote-details-grid">
        {metric(t('codex.modelProviders.usage.limit'), money(budget.policy.limitMicrousd))}
        {metric(t('common.apiBudget.consumed'), money(budget.usedMicrousd))}
        {metric(t('codex.modelProviders.usage.remaining'), money(budget.remainingMicrousd))}
        {budget.resetsAt != null && metric(t('common.apiBudget.resetsAt'), new Date(budget.resetsAt).toLocaleString())}
        {budget.policy.dailyLimitMicrousd != null && metric(t('common.apiBudget.dailyLimit'), `${money(budget.dailyRemainingMicrousd)} / ${money(budget.policy.dailyLimitMicrousd)}`)}
        {metric(t('codex.apiService.keys.tokenLimitUsage'), remote.tokenLimit == null ? unlimited : `${count(Math.max(0, remote.tokenLimit - remote.tokenUsed))} / ${count(remote.tokenLimit)}`)}
        {budget.policy.maxConcurrent != null && metric(t('common.apiBudget.concurrency'), count(budget.policy.maxConcurrent))}
        {budget.policy.dailyImageLimit != null && metric(t('common.apiBudget.dailyImages'), `${count(budget.dailyImageRequests)} / ${count(budget.policy.dailyImageLimit)}`)}
      </div>
      <div className="codex-remote-budget-notes">
        <p className="codex-remote-note">{t('common.apiBudget.accountingNote')}</p>
        {budget.unmeteredRequests > 0 && <p className="codex-remote-note">{t('common.apiBudget.unmetered')}</p>}
      </div>
    </section>
    <details className="cockpit-api-panel-section codex-remote-section codex-remote-raw">
      <summary>{t('codex.modelProviders.usage.rawFields')}</summary>
      <div className="codex-remote-details-grid">
        {metric(t('codex.localAccess.apiKey'), remote.keyLabel)}
        {metric(t('common.status'), t(`common.apiBudget.${statusKey}`))}
        {metric(t('common.apiBudget.images'), t(remote.capabilities.images ? 'common.enabled' : 'common.disabled'))}
        {metric(t('codex.modelProviders.usage.fields.totalRequests'), count(state.summary?.totalRequests))}
        {metric(t('codex.modelProviders.usage.fields.totalTokens'), count(state.summary?.totalTotalTokens))}
        {metric(t('common.codexUsage.estimatedValue'), state.summary?.totalCost == null ? '—' : formatBudgetUsd(Math.round(state.summary.totalCost * 1_000_000)))}
      </div>
      <p className="codex-remote-note">{t('codex.apiService.keys.allowedModels')}</p><code>{remote.models.join(', ') || '—'}</code>
    </details>
    <div className="codex-remote-toolbar codex-remote-range-toolbar">
      <div className="codex-remote-range-options">
        {(['today', '7d', 'month', 'custom'] as const).map(value => <button type="button" key={value}
          className={`btn ${preset === value ? 'btn-primary' : 'btn-secondary'} btn-sm`} aria-pressed={preset === value}
          onClick={() => selectRange(value)}>{t(value === 'today' ? 'common.codexUsage.periodToday' : value === '7d' ? 'common.codexUsage.periodWeek' : value === 'month' ? 'codex.localAccess.statsRange.monthly' : 'codex.localAccess.statsRange.custom')}</button>)}
        <span className="codex-remote-note">UTC</span>
      </div>
      <button type="button" className="btn btn-secondary btn-sm" onClick={() => setRevision(value => value + 1)} disabled={stats.loading || logs.loading}>{t('common.refresh')}</button>
    </div>
    {preset === 'custom' && <div>
      <div className="codex-remote-toolbar">
        <label>{t('codex.localAccess.statsRange.start')}<input ref={dateRef} type="date" value={dates.start} aria-invalid={dateError} onChange={event => { setDateError(false); setDates({ ...dates, start: event.target.value }); }} /></label>
        <label>{t('codex.localAccess.statsRange.end')}<input type="date" value={dates.end} aria-invalid={dateError} onChange={event => { setDateError(false); setDates({ ...dates, end: event.target.value }); }} /></label>
        <button type="button" className="btn btn-secondary btn-sm" onClick={applyDates}>{t('codex.localAccess.statsRange.apply')}</button>
      </div>
      {dateError && <p className="codex-key-budget-error" role="alert">{t('common.apiBudget.rangeLimit')}</p>}
    </div>}
    {failed && <div className="codex-remote-feedback error" ref={errorRef} role="alert">{t('common.codexUsage.dataFailed')}</div>}
    {!stats.data?.ready && !stats.error && <p className="codex-remote-feedback" role="status">{t(stats.loading && !stats.data ? 'common.loading' : 'common.apiBudget.preparing')}</p>}
    <div className="codex-remote-details-grid codex-remote-stats" aria-busy={stats.loading}>
      {metric(t('common.codexUsage.requestsCount'), count(usage?.requestCount), outcomes)}
      {metric(t('common.codexUsage.totalTokens'), count(usage?.totalTokens), usage ? `${t('common.codexUsage.input')} ${count(usage.inputTokens)} · ${t('common.codexUsage.output')} ${count(usage.outputTokens)} · ${t('common.codexUsage.cached')} ${count(usage.cachedTokens)}` : undefined)}
      {metric(t('common.apiBudget.settled'), usage ? money(usage.settledMicrousd) : '—')}
      {metric(t('common.codexUsage.avgLatency'), usage?.requestCount ? `${(usage.totalLatencyMs / usage.requestCount / 1000).toFixed(2)}s` : '—', usage?.requestCount ? `${t('common.codexUsage.successRate')} ${(usage.successCount / usage.requestCount * 100).toFixed(1)}%` : undefined)}
      {metric(t('codex.localAccess.stats.images'), count(usage?.imageRequestCount))}
    </div>
    {(stats.data?.ready || stats.data?.previousSnapshot) && <>
      <div className="codex-remote-toolbar">
        {['settled', 'estimated'].map(value => <button type="button" key={value} className={`btn ${costBasis === value ? 'btn-primary' : 'btn-secondary'} btn-sm`}
          aria-pressed={costBasis === value} onClick={() => setCostBasis(value)}>{t(value === 'settled' ? 'common.apiBudget.settled' : 'common.codexUsage.estimatedValue')}</button>)}
      </div>
      <CodexUsageTrend points={points} hourly={stats.data.hourly} timeZone="UTC" title={t('common.codexUsage.statistics')}
        labels={{ estimatedCostUsd: t(costBasis === 'settled' ? 'common.apiBudget.settled' : 'common.codexUsage.estimatedValue') }} />
      {Boolean(usage && usage.unrecordedCount + usage.unmeteredCount > 0) && <p className="codex-remote-note">{t('common.apiBudget.partialCharges', { count: (usage?.unrecordedCount ?? 0) + (usage?.unmeteredCount ?? 0) })}</p>}
    </>}
    <p className="codex-remote-note">{t('common.apiBudget.retention', { time: stats.data?.firstRetainedAt == null ? '—' : `${new Date(stats.data.firstRetainedAt).toLocaleString(undefined, { timeZone: 'UTC' })} UTC` })}</p>
    <section className="cockpit-api-panel-section codex-remote-section codex-remote-logs">
      <h3>{t('codex.localAccess.requestLogTitle')}</h3>
      <div className="codex-remote-toolbar">
        <SingleSelectDropdown value={model} ariaLabel={t('common.codexUsage.model')} onChange={value => { setModel(value); setCursors([null]); }}
          options={[{ value: '', label: t('accounts.filters.all') }, ...remote.models.map(value => ({ value, label: value }))]} />
        <SingleSelectDropdown value={status} ariaLabel={t('common.status')} onChange={value => { setStatus(value); setCursors([null]); }}
          options={[{ value: 'all', label: t('codex.apiService.logs.allStatuses') }, { value: 'success', label: t('common.success') }, { value: 'failed', label: t('common.failed') }]} />
        <SingleSelectDropdown value={kind} ariaLabel={t('codex.apiService.logs.kindFilter')} onChange={value => { setKind(value); setCursors([null]); }}
          options={[{ value: 'all', label: t('codex.apiService.logs.allKinds') }, { value: 'text', label: t('common.apiBudget.textRequests') }, { value: 'image_generation', label: t('common.apiBudget.images') }, { value: 'image_edit', label: t('common.apiBudget.imageEdits') }]} />
      </div>
      <p className="codex-remote-note">{t('common.apiBudget.logFiltersNote')}</p>
      <CodexRemoteKeyRequests data={logs.data} loading={logs.loading} t={t} />
      <div className="codex-remote-pagination">
        <button type="button" className="btn btn-secondary btn-sm" disabled={cursors.length < 2 || logs.loading} onClick={() => setCursors(previous => previous.slice(0, -1))}>{t('pagination.prev')}</button>
        <span className="codex-remote-note">{cursors.length}</span>
        <button type="button" className="btn btn-secondary btn-sm" disabled={!logs.data?.nextCursor || logs.loading} onClick={() => setCursors(previous => [...previous, logs.data!.nextCursor!])}>{t('pagination.next')}</button>
      </div>
    </section>
  </div>;
}
