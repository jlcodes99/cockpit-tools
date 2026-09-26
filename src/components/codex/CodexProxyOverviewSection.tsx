import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Activity, ArrowDown, ArrowRight, ArrowUp, Boxes, CalendarClock, Check, RefreshCw, Route, Server, ShieldCheck, Star, Trash2, TriangleAlert,
} from 'lucide-react';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import {
  cancelProxyCatalog, catalogErrorKey, refreshProxyCatalog, type ProxyCatalog,
  type ProxyCatalogSource,
} from '../../services/codexProxyCatalogService';
import type { CodexUnifiedProxyView } from '../../services/codexUnifiedProxyService';
import { sourceDefaultDraft } from '../../utils/codexProxySelection';
import { formatTrafficBytes, type CodexProxyTraffic } from '../../utils/codexProxyTraffic';
import { formatProxyDateTime, formatProxyBytes, proxyUsagePercent, PROXY_UNKNOWN } from '../../utils/codexProxyFormat';
import { codexProxyExitModeKey, resolveExitMode, unifiedFollowingIds, unifiedProxyActive } from '../../utils/codexProxyDraft';
import { CodexProxyPicker } from './CodexProxyPicker';
import { useProxyLatency } from './useProxyLatency';
import { useCodexProxyAccountName, useCodexProxyExitEditor, type CodexProxyExitEditor } from './useCodexProxyExitEditor';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import '../../styles/pages/codex-proxy-overview.css';

/** 后端后续补充的来源用量字段；缺失时整块用量都不渲染。 */
interface ProxySourceUsage {
  upload: number;
  download: number;
  total: number;
  expireAt: number | null;
}

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/**
 * 只有四个字段都能解析、且总量大于 0 时才返回用量；
 * 数据缺失时宁可整块不显示，也不画出假的进度条或到期时间。
 */
function readSourceUsage(source: unknown): ProxySourceUsage | null {
  if (!source || typeof source !== 'object') return null;
  const record = source as Record<string, unknown>;
  const upload = finiteNumber(record.upload);
  const download = finiteNumber(record.download);
  const total = finiteNumber(record.total);
  if (upload === null || download === null || total === null || total <= 0) return null;
  const expireAt = finiteNumber(record.expireAt);
  return { upload, download, total, expireAt: expireAt !== null && expireAt > 0 ? expireAt : null };
}

function sourceKindKey(kind: ProxyCatalogSource['kind']): string {
  if (kind === 'subscription') return 'codex.proxy.overview.kindSubscription';
  if (kind === 'strategy') return 'codex.proxy.overview.kindStrategy';
  return 'codex.proxy.overview.kindManual';
}

interface SourceCardProps {
  sourceId: string;
  source: ProxyCatalogSource | undefined;
  loading: boolean;
  error: string;
  reload: () => void;
  goResources: () => void;
}

/**
 * 选中账号正在使用的来源。刷新只复用既有 catalog 服务，
 * 订阅刷新成功后重新读取列表，失败保留上一份可用数据并把错误留在卡片内。
 */
function CodexProxySourceCard({ sourceId, source, loading, error, reload, goResources }: SourceCardProps) {
  const { t } = useTranslation();
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState('');
  const request = useRef<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (request.current) void cancelProxyCatalog(request.current).catch(() => {});
    };
  }, []);

  const refresh = async () => {
    if (!source || refreshing || source.kind !== 'subscription') return;
    const requestId = crypto.randomUUID();
    request.current = requestId;
    setRefreshing(true);
    setRefreshError('');
    try {
      await refreshProxyCatalog(requestId, source.id);
    } catch (caught) {
      const key = catalogErrorKey(caught);
      if (mounted.current && key !== 'common.cancelled') setRefreshError(t(key));
    } finally {
      request.current = null;
      if (mounted.current) {
        setRefreshing(false);
        // 无论成功还是失败都重新读一次：失败时后端会记录来源错误状态，列表仍保留可用数据。
        reload();
      }
    }
  };

  const usage = readSourceUsage(source);
  const defaultItem = source?.default
    ? [...source.nodes, ...source.groups].find((entry) => entry.id === source.default?.itemId)
    : undefined;
  const defaultLabel = source?.default
    ? defaultItem?.name ?? t('codex.proxy.catalog.defaultUnavailable')
    : t('codex.proxy.catalog.defaultEmpty');
  const refreshable = source?.kind === 'subscription';

  const body = error
    ? <div className="codex-proxy-page-error" role="alert">{error}
      <button type="button" className="btn btn-secondary compact" onClick={() => reload()}>{t('common.retry')}</button></div>
    : loading && !source
      ? <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>
      : !sourceId
        ? <p className="codex-proxy-page-note">{t('codex.proxy.overview.sourceNone')}</p>
        : !source
          ? <div className="codex-proxy-overview-callout is-warning" role="status">
            <div><strong>{t('codex.proxy.overview.sourceInvalid')}</strong>
              <p>{t('codex.proxy.overview.sourceInvalidHint')}</p></div>
            <button type="button" className="btn btn-secondary compact" onClick={() => goResources()}>
              {t('codex.proxy.catalog.resources')}<ArrowRight size={14} aria-hidden="true" /></button>
          </div>
          : <>
            <div className="codex-proxy-overview-source-head">
              <strong title={source.name}>{source.name}</strong>
              <span className="codex-proxy-overview-chip">{t(sourceKindKey(source.kind))}</span>
              {source.error && <span className="codex-proxy-overview-chip is-warning">
                <TriangleAlert size={12} aria-hidden="true" />{t('codex.proxy.overview.sourceError')}</span>}
            </div>
            {/* 参考首页的订阅卡片：每行一个图标 + 标签: 值，便于快速扫读。 */}
            <ul className="codex-proxy-overview-rows">
              <li><CalendarClock size={16} aria-hidden="true" />
                <span>{t('codex.proxy.overview.updated')}:</span><b>{formatProxyDateTime(source.updatedAt)}</b></li>
              <li><Boxes size={16} aria-hidden="true" />
                <span>{t('codex.proxy.overview.resources')}:</span>
                <b>{t('codex.proxy.overview.nodeGroupCount', { nodes: source.nodes.length, groups: source.groups.length })}</b></li>
              <li><Star size={16} aria-hidden="true" />
                <span>{t('codex.proxy.catalog.defaultLabel')}:</span><b title={defaultLabel}>{defaultLabel}</b></li>
              <li><RefreshCw size={16} aria-hidden="true" />
                <span>{t('codex.proxy.overview.autoUpdate')}:</span>
                <b>{t(source.autoUpdate ? 'common.enabled' : 'common.disabled')}</b></li>
            </ul>
            {source.defaultInvalidated && <p className="codex-proxy-page-note" role="status">{t('codex.proxy.catalog.defaultInvalidated')}</p>}
            {usage && <div className="codex-proxy-overview-usage">
              <div className="codex-proxy-overview-usage-line">
                <span>{t('codex.proxy.overview.usageUsed')} {formatProxyBytes(usage.upload + usage.download)}</span>
                <span>{t('codex.proxy.overview.usageTotal')} {formatProxyBytes(usage.total)}</span>
                {usage.expireAt !== null && <span>{t('codex.proxy.overview.usageExpire')} {formatProxyDateTime(usage.expireAt)}</span>}
              </div>
              <div className="codex-proxy-overview-progress" role="progressbar" aria-label={t('codex.proxy.overview.usageProgress')}
                aria-valuemin={0} aria-valuemax={100} aria-valuenow={proxyUsagePercent(usage.upload + usage.download, usage.total)}>
                <span style={{ width: `${proxyUsagePercent(usage.upload + usage.download, usage.total)}%` }} />
              </div>
            </div>}
          </>;

  return <section className="codex-proxy-page-card codex-proxy-overview-card" aria-labelledby="codex-proxy-overview-source-title">
    <div className="codex-proxy-page-section-heading">
      <div className="codex-proxy-section-label"><Server size={18} aria-hidden="true" />
        <h3 id="codex-proxy-overview-source-title">{t('codex.proxy.overview.source')}</h3></div>
      <div className="codex-proxy-overview-actions">
        <button type="button" className="btn btn-secondary compact" disabled={!refreshable || refreshing || !source}
          title={refreshable ? undefined : t('codex.proxy.overview.refreshUnsupported')} onClick={() => void refresh()}>
          <RefreshCw size={14} className={refreshing ? 'loading-spinner' : undefined} aria-hidden="true" />
          {t('codex.proxy.overview.sourceRefresh')}</button>
        {refreshing && <button type="button" className="btn btn-secondary compact"
          onClick={() => { if (request.current) void cancelProxyCatalog(request.current).catch(() => {}); }}>{t('common.cancel')}</button>}
        <button type="button" className="btn btn-secondary compact" onClick={() => goResources()}>
          {t('codex.proxy.catalog.resources')}<ArrowRight size={14} aria-hidden="true" /></button>
      </div>
    </div>
    {refreshError && <div className="codex-proxy-page-error" role="alert">{refreshError}
      <button type="button" className="btn btn-secondary compact" disabled={refreshing} onClick={() => void refresh()}>{t('common.retry')}</button></div>}
    {body}
  </section>;
}

interface ExitCardProps {
  editor: CodexProxyExitEditor;
  mode: ReturnType<typeof resolveExitMode>;
  scope: 'account' | 'unified';
  onScopeChange: (scope: 'account' | 'unified') => void;
  catalog: ProxyCatalog;
  catalogLoading: boolean;
  catalogError: string;
  reloadCatalog: () => void;
  unified: CodexUnifiedProxyView | null;
  unifiedErrorKey: string;
  reloadUnified: () => void;
  goResources: () => void;
  goRules: () => void;
}

/** 当前出口：账号档位直接复用出口编辑器，统一代理档位只读并跳转到出口规则页。 */
function CodexProxyExitCard({
  editor, mode, scope, onScopeChange, catalog, catalogLoading, catalogError, reloadCatalog,
  unified, unifiedErrorKey, reloadUnified, goResources, goRules,
}: ExitCardProps) {
  const { t } = useTranslation();
  const latency = useProxyLatency(editor.source);
  const busy = Boolean(editor.busy);
  const availableSources = useMemo(() => catalog.sources.filter((entry) => entry.id === editor.savedBinding?.sourceId
    || entry.nodes.some((node) => node.supported) || entry.groups.some((group) => group.supported)), [catalog, editor.savedBinding]);
  const unifiedActive = unifiedProxyActive(unified);
  const unifiedLabel = unified?.binding
    ? [unified.binding.sourceName, unified.binding.name, unified.binding.selectedName].filter(Boolean).join(' · ')
    : '';

  return <section className="codex-proxy-page-card codex-proxy-overview-card" aria-labelledby="codex-proxy-overview-exit-title">
    <div className="codex-proxy-page-section-heading">
      <div className="codex-proxy-section-label"><ShieldCheck size={18} aria-hidden="true" />
        <h3 id="codex-proxy-overview-exit-title">{t('codex.proxy.overview.exit')}</h3></div>
      <span className={`codex-proxy-page-state is-${scope === 'unified' ? 'unified' : mode}`}>
        <span className="codex-proxy-page-state-dot" />
        {t(scope === 'unified' ? 'codex.proxy.modeUnified' : codexProxyExitModeKey(mode))}</span>
    </div>
    <div className="codex-proxy-mode" role="group" aria-label={t('codex.proxy.overview.exit')}>
      {(['account', 'unified'] as const).map((option) => <button key={option} type="button"
        className={'btn btn-secondary compact' + (scope === option ? ' active' : '')} aria-pressed={scope === option}
        onClick={() => onScopeChange(option)}>
        {t(option === 'account' ? 'codex.proxy.overview.scopeAccount' : 'codex.proxy.overview.scopeUnified')}</button>)}
    </div>
    {scope === 'unified' ? <div className="codex-proxy-overview-unified">
      <div className="codex-proxy-overview-unified-head">
        <Route size={20} aria-hidden="true" />
        <div>
          <strong>{unifiedActive ? unifiedLabel || t('codex.proxy.unified.active') : t('codex.proxy.unified.offState')}</strong>
          <p>{t(unifiedActive ? 'codex.proxy.unified.accountsNotice' : 'codex.proxy.unified.hint')}</p>
        </div>
      </div>
      {unifiedErrorKey && <div className="codex-proxy-page-error" role="alert">{t(unifiedErrorKey)}
        <button type="button" className="btn btn-secondary compact" onClick={() => reloadUnified()}>{t('common.retry')}</button></div>}
      {!unifiedErrorKey && unified?.staleError && <p className="codex-proxy-page-note" role="status">{t('codex.proxy.unified.staleWarning')}</p>}
      <button type="button" className="btn btn-secondary" onClick={() => goRules()}>
        {t('codex.proxy.overview.goRules')}<ArrowRight size={15} aria-hidden="true" /></button>
    </div> : !editor.account ? <p className="codex-proxy-page-note">{t('codex.proxy.unsupportedAccount')}</p> : <>
      {!editor.bound && <p className="codex-proxy-page-note" role="status">
        {t(mode === 'unified' ? 'codex.proxy.modeUnifiedHint' : 'codex.proxy.modeDefaultHint')}</p>}
      {catalogLoading && <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>}
      {catalogError && <div className="codex-proxy-page-error" role="alert">{catalogError}
        <button type="button" className="btn btn-secondary compact" onClick={() => reloadCatalog()}>{t('common.retry')}</button></div>}
      {!catalogLoading && !catalogError && availableSources.length === 0
        ? <div className="codex-proxy-overview-callout">
          <div><strong>{t('codex.proxy.catalog.accountEmptyTitle')}</strong><p>{t('codex.proxy.catalog.accountEmptyHint')}</p></div>
          <button type="button" className="btn btn-primary compact" onClick={() => goResources()}>
            {t('codex.proxy.catalog.addSource')}<ArrowRight size={14} aria-hidden="true" /></button>
        </div>
        : availableSources.length > 0 && <div className="codex-proxy-overview-exit-form">
          <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.sources')}</span>
            <SingleSelectDropdown value={editor.sourceId} disabled={busy} ariaLabel={t('codex.proxy.catalog.sources')}
              options={availableSources.map((entry) => ({ value: entry.id, label: entry.name }))}
              onChange={(id) => {
                // 切换来源只带入该来源的默认项，保存前不写入账号。
                const preset = sourceDefaultDraft(catalog.sources.find((entry) => entry.id === id));
                editor.select({ sourceId: id, itemId: preset?.itemId ?? '', groupId: preset?.groupId ?? '', selections: preset?.selections ?? {} });
              }} /></label>
          {editor.source && <CodexProxyPicker key={editor.account.id + ':' + editor.source.id} source={editor.source} itemId={editor.itemId}
            selectedGroupId={editor.groupId} selections={editor.selections} busy={busy} latency={latency}
            choose={(id, nextGroup) => editor.select({ sourceId: editor.sourceId, itemId: id, groupId: nextGroup, selections: {} })}
            chooseMember={(selectorId, member) => editor.select({
              sourceId: editor.sourceId, itemId: editor.itemId, groupId: editor.groupId,
              selections: { ...editor.selections, [selectorId]: member },
            })} />}
        </div>}
      {editor.bound && mode === 'stale' && <p className="codex-proxy-page-note" role="status">{t('codex.proxy.modeStaleHint')}</p>}
      {editor.error && <div className="codex-proxy-page-error" role="alert">{editor.error}
        <button type="button" className="btn btn-secondary compact" onClick={() => editor.clearError()}>{t('common.close')}</button></div>}
      {editor.notice && <p className="codex-proxy-overview-notice" role="status">{editor.notice}</p>}
      <div className="codex-proxy-page-actions">
        <button type="button" className="btn btn-secondary codex-proxy-remove" disabled={busy || !editor.bound} onClick={() => editor.unbind()}>
          {editor.busy === 'unbind' ? <RefreshCw size={15} className="loading-spinner" aria-hidden="true" /> : <Trash2 size={15} aria-hidden="true" />}
          {t('codex.proxy.unbind')}</button>
        <div className="codex-proxy-page-actions-right">
          {editor.testing && <button type="button" className="btn btn-secondary" onClick={() => editor.cancelTest()}>{t('codex.proxy.cancelCheck')}</button>}
          <button type="button" className="btn btn-secondary" disabled={busy || (!editor.selectionReady && !editor.bound)} onClick={() => editor.test()}>
            {editor.testing ? <RefreshCw size={15} className="loading-spinner" aria-hidden="true" /> : <Activity size={15} aria-hidden="true" />}
            {t(editor.testing ? 'codex.proxy.testing' : 'codex.proxy.test')}</button>
          <button type="button" className="btn btn-primary" disabled={busy || !editor.selectionReady || editor.saved} onClick={() => editor.save()}>
            {editor.busy === 'save' ? <RefreshCw size={15} className="loading-spinner" aria-hidden="true" /> : <Check size={15} aria-hidden="true" />}
            {t('codex.proxy.catalog.bind')}</button>
        </div>
      </div>
      {editor.result && <output className="codex-proxy-check-result" aria-live="polite"><ShieldCheck size={18} aria-hidden="true" /><div>
        <strong>{t('codex.proxy.testPassed')}</strong>
        <span>{editor.result.protocol} · {editor.result.ip} · {editor.result.latencyMs} ms</span>
        {editor.resultScope === 'selection' && <span>{t('codex.proxy.draftTested')}</span>}
        <time dateTime={new Date(editor.result.checkedAt).toISOString()}>{formatProxyDateTime(editor.result.checkedAt)}</time></div></output>}
    </>}
    {/* 生效说明固定显示，与档位无关。 */}
    <p className="codex-proxy-page-note">{t('codex.proxy.restartHint')}</p>
  </section>;
}

const CHART_WIDTH = 100;
const CHART_HEIGHT = 32;

/** 十分钟内存曲线的当前值、峰值与会话累计。 */
function CodexProxyTrafficCard({ traffic }: { traffic: CodexProxyTraffic }) {
  const { t } = useTranslation();
  const series = useMemo(() => {
    const samples = traffic.samples;
    if (samples.length < 2) return null;
    const first = samples[0].at;
    const span = samples[samples.length - 1].at - first;
    if (!Number.isFinite(span) || span <= 0) return null;
    const peak = samples.reduce((value, sample) => Math.max(value, sample.up, sample.down), 0);
    const line = (key: 'up' | 'down') => samples.map((sample, index) => {
      const x = ((sample.at - first) / span) * CHART_WIDTH;
      const value = Number.isFinite(sample[key]) ? Math.max(sample[key], 0) : 0;
      const y = peak > 0 ? CHART_HEIGHT - (Math.min(value, peak) / peak) * CHART_HEIGHT : CHART_HEIGHT;
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(2)} ${y.toFixed(2)}`;
    }).join(' ');
    return { up: line('up'), down: line('down'), peak };
  }, [traffic.samples]);
  const rate = (value: number) => traffic.unavailable ? PROXY_UNKNOWN : `${formatTrafficBytes(value)}/s`;

  return <section className="codex-proxy-page-card codex-proxy-overview-card" aria-labelledby="codex-proxy-overview-traffic-title">
    <div className="codex-proxy-page-section-heading">
      <div className="codex-proxy-section-label"><Activity size={18} aria-hidden="true" />
        <h3 id="codex-proxy-overview-traffic-title">{t('codex.proxy.overview.traffic')}</h3></div>
      <span className="codex-proxy-overview-chip">{t('codex.proxy.overview.trafficWindow')}</span>
    </div>
    {!traffic.ready ? <p className="codex-proxy-overview-placeholder" role="status">{t('codex.proxy.overview.trafficPending')}</p> : <>
      {traffic.unavailable && <p className="codex-proxy-overview-notice is-warning" role="status">
        <TriangleAlert size={14} aria-hidden="true" />{t('codex.proxy.overview.trafficUnavailable')}</p>}
      <div className="codex-proxy-overview-stats">
        <div className="codex-proxy-overview-stat is-up"><span><ArrowUp size={13} aria-hidden="true" />{t('codex.proxy.workspace.trafficUpload')}</span>
          <strong>{rate(traffic.upPerSecond)}</strong></div>
        <div className="codex-proxy-overview-stat is-down"><span><ArrowDown size={13} aria-hidden="true" />{t('codex.proxy.workspace.trafficDownload')}</span>
          <strong>{rate(traffic.downPerSecond)}</strong></div>
        <div className="codex-proxy-overview-stat"><span>{t('codex.proxy.workspace.trafficSession')}</span>
          <strong>{formatTrafficBytes(traffic.sessionUpload + traffic.sessionDownload)}</strong></div>
        <div className="codex-proxy-overview-stat"><span>{t('codex.proxy.workspace.trafficConnections')}</span>
          <strong>{traffic.activeConnections}</strong></div>
      </div>
      {series ? <div className="codex-proxy-overview-chart-wrap">
        <div className="codex-proxy-overview-chart" role="img" aria-label={t('codex.proxy.overview.trafficChartLabel')}>
          <svg viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`} preserveAspectRatio="none" aria-hidden="true" focusable="false">
            <line className="is-axis" x1={0} y1={CHART_HEIGHT} x2={CHART_WIDTH} y2={CHART_HEIGHT} vectorEffect="non-scaling-stroke" />
            <path className="is-up" d={series.up} vectorEffect="non-scaling-stroke" />
            <path className="is-down" d={series.down} vectorEffect="non-scaling-stroke" />
          </svg>
        </div>
        <div className="codex-proxy-overview-chart-legend">
          <span className="is-up"><ArrowUp size={12} aria-hidden="true" />{t('codex.proxy.workspace.trafficUpload')}</span>
          <span className="is-down"><ArrowDown size={12} aria-hidden="true" />{t('codex.proxy.workspace.trafficDownload')}</span>
          <span className="codex-proxy-overview-chart-peak">
            {t('codex.proxy.overview.trafficPeak', { value: `${formatTrafficBytes(series.peak)}/s` })}</span>
        </div>
      </div> : <p className="codex-proxy-overview-placeholder" role="status">{t('codex.proxy.overview.trafficEmpty')}</p>}
    </>}
    <p className="codex-proxy-page-note">{t('codex.proxy.overview.trafficHint')}</p>
  </section>;
}

/**
 * 概览页：左侧来源卡片、右侧当前出口卡片，下方整宽流量统计。
 * 三个卡片各自在卡片内提示失败，共用的选择与出口逻辑全部来自工作台上下文。
 */
export function CodexProxyOverviewSection() {
  const { t } = useTranslation();
  const {
    accounts, selectedId, catalog, catalogLoading, catalogError, reloadCatalog,
    unified, unifiedErrorKey, reloadUnified, traffic, goSection,
  } = useCodexProxyWorkspace();
  const resolveName = useCodexProxyAccountName();
  const editor = useCodexProxyExitEditor(selectedId);
  const [scope, setScope] = useState<'account' | 'unified'>('account');
  const following = useMemo(
    () => new Set(unifiedFollowingIds(accounts, (entry) => entry.egress_proxy ?? null, unified)),
    [accounts, unified],
  );
  const mode = resolveExitMode(editor.savedBinding, { catalog, loading: catalogLoading, failed: Boolean(catalogError) }, following.has(selectedId));
  const boundSourceId = editor.savedBinding?.sourceId ?? '';
  const boundSource = useMemo(() => catalog.sources.find((entry) => entry.id === boundSourceId), [catalog, boundSourceId]);
  const account = accounts.find((entry) => entry.id === selectedId);

  return <section className="codex-proxy-overview" aria-labelledby="codex-proxy-overview-intro">
    <p className="codex-proxy-page-note" id="codex-proxy-overview-intro">
      {account ? t('codex.proxy.overview.intro', { name: resolveName(account) }) : t('codex.proxy.accounts.detailEmpty')}</p>
    <div className="codex-proxy-overview-grid">
      <CodexProxySourceCard sourceId={boundSourceId} source={boundSource} loading={catalogLoading} error={catalogError}
        reload={() => reloadCatalog()} goResources={() => goSection('resources')} />
      <CodexProxyExitCard editor={editor} mode={mode} scope={scope} onScopeChange={setScope} catalog={catalog}
        catalogLoading={catalogLoading} catalogError={catalogError} reloadCatalog={() => reloadCatalog()}
        unified={unified} unifiedErrorKey={unifiedErrorKey} reloadUnified={() => reloadUnified()}
        goResources={() => goSection('resources')} goRules={() => goSection('rules')} />
    </div>
    <CodexProxyTrafficCard traffic={traffic} />
  </section>;
}
