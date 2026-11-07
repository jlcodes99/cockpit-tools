import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowDown, ArrowUp, Network, Pause, Play, RefreshCw, Search, Trash2, TriangleAlert } from 'lucide-react';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import type { ProxyActiveConnection, ProxyActivityChannel } from '../../services/codexProxyActivityService';
import { formatProxyBytes, formatProxyClock, formatProxyDuration, PROXY_UNKNOWN, proxyTimeMs } from '../../utils/codexProxyFormat';
import { useCodexProxyActivity } from './useCodexProxyActivity';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import '../../styles/pages/codex-proxy-overview.css';

type ProxyConnectionSortKey = 'upload' | 'download' | 'duration';
type SortDirection = 'asc' | 'desc';

interface ProxyConnectionRow {
  connection: ProxyActiveConnection;
  durationMs: number | null;
}

function ariaSort(active: boolean, direction: SortDirection): 'ascending' | 'descending' | 'none' {
  if (!active) return 'none';
  return direction === 'asc' ? 'ascending' : 'descending';
}

function CodexProxyConnectionsState({ icon, title, hint, children }: {
  icon: ReactNode; title: string; hint: string; children?: ReactNode;
}) {
  return <div className="codex-proxy-connections-state">
    <div className="codex-proxy-connections-state-icon" aria-hidden="true">{icon}</div>
    <h4>{title}</h4>
    <p>{hint}</p>
    {children}
  </div>;
}

/**
 * 连接页：只读展示当前账号的活跃连接。
 * 数据全部来自既有 useCodexProxyActivity；采集关闭时不显示任何旧行，
 * 空记录、未开启采集和读取失败各有独立文案与下一步动作。
 */
export function CodexProxyConnectionsSection() {
  const { t } = useTranslation();
  const { accounts, selectedId } = useCodexProxyWorkspace();
  const resolveName = useCodexProxyAccountName();
  const activity = useCodexProxyActivity(selectedId);
  const snapshot = activity.snapshot;
  const [channel, setChannel] = useState<'all' | ProxyActivityChannel>('all');
  const [search, setSearch] = useState('');
  const [sortKey, setSortKey] = useState<ProxyConnectionSortKey | ''>('');
  const [direction, setDirection] = useState<SortDirection>('desc');
  const [now, setNow] = useState(() => Date.now());
  const account = accounts.find((entry) => entry.id === selectedId);

  // 每次快照到达时刷新“当前时间”，暂停刷新时时长随之冻结，与暂停语义一致。
  useEffect(() => { setNow(Date.now()); }, [snapshot]);

  const visible = useMemo(() => (snapshot?.connections ?? []).filter((entry) =>
    (channel === 'all' || entry.channel === channel)
    && (!search.trim() || [entry.target, entry.rule, ...entry.chains]
      .some((value) => value?.toLowerCase().includes(search.trim().toLowerCase()))),
  ), [snapshot?.connections, channel, search]);

  const rows = useMemo<ProxyConnectionRow[]>(() => {
    const prepared = visible.map((connection) => {
      const started = proxyTimeMs(connection.startedAt);
      return { connection, durationMs: started === null ? null : Math.max(0, now - started) };
    });
    if (!sortKey) return prepared;
    const value = (row: ProxyConnectionRow) => sortKey === 'duration'
      ? row.durationMs
      : Number.isFinite(row.connection[sortKey]) ? row.connection[sortKey] : 0;
    const factor = direction === 'asc' ? 1 : -1;
    // 未知时长永远排在最后；同值时按开始时间倒序，保证顺序稳定可预期。
    return prepared.sort((left, right) => {
      const first = value(left);
      const second = value(right);
      if (first === null && second === null) return 0;
      if (first === null) return 1;
      if (second === null) return -1;
      return (first - second) * factor || (proxyTimeMs(right.connection.startedAt) ?? 0) - (proxyTimeMs(left.connection.startedAt) ?? 0);
    });
  }, [direction, now, sortKey, visible]);

  const toggleSort = (key: ProxyConnectionSortKey) => {
    if (sortKey === key) {
      setDirection(direction === 'asc' ? 'desc' : 'asc');
      return;
    }
    setSortKey(key);
    setDirection('desc');
  };
  const resetFilters = () => { setChannel('all'); setSearch(''); };
  const filtered = channel !== 'all' || Boolean(search.trim());
  // 提示描述“点击后会变成什么”，未排序的列点击后同样是降序。
  const sortTitle = (key: ProxyConnectionSortKey) => t('codex.proxy.connections.sortBy', {
    direction: t(sortKey === key && direction === 'desc' ? 'codex.proxy.connections.sortAsc' : 'codex.proxy.connections.sortDesc'),
  });
  const sortIcon = (key: ProxyConnectionSortKey) => sortKey !== key ? null
    : direction === 'asc' ? <ArrowUp size={12} aria-hidden="true" /> : <ArrowDown size={12} aria-hidden="true" />;
  const column = (key: ProxyConnectionSortKey, label: string) => <th scope="col" className="is-number" aria-sort={ariaSort(sortKey === key, direction)}>
    <button type="button" className="btn btn-secondary compact"
      title={sortTitle(key)} onClick={() => toggleSort(key)}>{label}{sortIcon(key)}</button>
  </th>;

  const ready = Boolean(snapshot?.supported && snapshot.enabled);
  const table = ready ? <>
    <div className="codex-proxy-connections-toolbar">
      <SingleSelectDropdown value={channel} ariaLabel={t('codex.proxy.activity.channel')}
        options={(['all', 'account', 'desktop', 'sidecar'] as const).map((value) => ({
          value, label: t(`codex.proxy.activity.channel_${value}`),
        }))}
        onChange={(value) => setChannel(value as typeof channel)} />
      <label className="codex-proxy-search"><Search size={16} aria-hidden="true" />
        <input type="search" value={search} onChange={(event) => setSearch(event.target.value)}
          placeholder={t('codex.proxy.connections.search')} aria-label={t('codex.proxy.connections.search')} /></label>
      <span className="codex-proxy-connections-count" role="status">
        {t(filtered ? 'codex.proxy.connections.filteredCount' : 'codex.proxy.connections.count',
          filtered ? { shown: rows.length, total: snapshot?.connections.length ?? 0 } : { count: snapshot?.connections.length ?? 0 })}</span>
    </div>
    {rows.length === 0
      ? <CodexProxyConnectionsState icon={<Network size={22} />}
        title={t(filtered ? 'codex.proxy.connections.noMatch' : 'codex.proxy.connections.empty')}
        hint={t(filtered ? 'codex.proxy.connections.noMatchHint' : 'codex.proxy.connections.emptyHint')}>
        <button type="button" className="btn btn-secondary compact" onClick={() => filtered ? resetFilters() : activity.refresh()}>
          {t(filtered ? 'codex.proxy.connections.resetFilters' : 'common.refresh')}</button>
      </CodexProxyConnectionsState>
      : <div className="codex-proxy-connections-table-wrap">
        <table className="codex-proxy-connections-table">
          <thead><tr>
            <th scope="col">{t('codex.proxy.activity.channel')}</th>
            <th scope="col">{t('codex.proxy.connections.columnTarget')}</th>
            <th scope="col">{t('codex.proxy.connections.columnNetwork')}</th>
            <th scope="col">{t('codex.proxy.connections.columnChains')}</th>
            <th scope="col">{t('codex.proxy.connections.columnRule')}</th>
            {column('upload', t('codex.proxy.connections.columnUpload'))}
            {column('download', t('codex.proxy.connections.columnDownload'))}
            <th scope="col">{t('codex.proxy.connections.columnStarted')}</th>
            {column('duration', t('codex.proxy.connections.columnDuration'))}
          </tr></thead>
          <tbody>{rows.map(({ connection, durationMs }) => <tr key={connection.id}>
            <td><span className="codex-proxy-connections-pill">{t(`codex.proxy.activity.channel_${connection.channel}`)}</span></td>
            <td className="is-target" title={connection.target}>{connection.target}</td>
            <td>{connection.network ? connection.network.toUpperCase() : PROXY_UNKNOWN}</td>
            <td className="is-chains" title={connection.chains.join(' → ')}>{connection.chains.join(' → ') || PROXY_UNKNOWN}</td>
            <td className="is-rule" title={connection.rule ?? undefined}>{connection.rule || PROXY_UNKNOWN}</td>
            <td className="is-number">↑ {formatProxyBytes(connection.upload)}</td>
            <td className="is-number">↓ {formatProxyBytes(connection.download)}</td>
            <td>{formatProxyClock(connection.startedAt)}</td>
            <td className="is-number">{durationMs === null ? PROXY_UNKNOWN : formatProxyDuration(durationMs)}</td>
          </tr>)}</tbody>
        </table>
      </div>}
  </> : null;

  return <section className="codex-proxy-connections" aria-label={t('codex.proxy.activity.connections')}>
    <header className="codex-proxy-connections-head">
      <div>
        <p className="codex-proxy-connections-scope">
          {t('codex.proxy.connections.scope', { name: account ? resolveName(account) : PROXY_UNKNOWN })}</p>
        <p className="codex-proxy-connections-hint">{t('codex.proxy.connections.hint')}</p>
      </div>
      <div className="codex-proxy-connections-actions">
        <button type="button" className="btn btn-secondary compact" disabled={activity.busy || !snapshot?.supported}
          onClick={() => void activity.changeEnabled(!snapshot?.enabled)}>
          {activity.busy ? <RefreshCw size={14} className="loading-spinner" aria-hidden="true" />
            : snapshot?.enabled ? <Pause size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
          {t(snapshot?.enabled ? 'codex.proxy.activity.stop' : 'codex.proxy.activity.start')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={!snapshot?.enabled}
          onClick={() => activity.setPaused(!activity.paused)}>
          {activity.paused ? <Play size={14} aria-hidden="true" /> : <Pause size={14} aria-hidden="true" />}
          {t(activity.paused ? 'codex.proxy.activity.resume' : 'codex.proxy.activity.pause')}</button>
        <button type="button" className="btn btn-secondary compact" title={t('codex.proxy.connections.clearHint')}
          disabled={activity.busy || !snapshot?.logs.length} onClick={() => void activity.clear()}>
          <Trash2 size={14} aria-hidden="true" />{t('codex.proxy.activity.clear')}</button>
        <button type="button" className="btn btn-secondary compact" onClick={() => activity.refresh()}>
          <RefreshCw size={14} aria-hidden="true" />{t('common.refresh')}</button>
      </div>
    </header>
    {snapshot?.enabled && snapshot.captureError && <p className="codex-proxy-connections-warning" role="status">
      <TriangleAlert size={14} aria-hidden="true" />{t('codex.proxy.activity.captureFailed')}</p>}
    {snapshot?.connectionsError && <p className="codex-proxy-connections-warning" role="status">
      <TriangleAlert size={14} aria-hidden="true" />{t('codex.proxy.activity.connectionsFailed')}
      <button type="button" className="btn btn-secondary compact" onClick={() => activity.refresh()}>{t('common.retry')}</button></p>}
    {activity.error && snapshot && <p className="codex-proxy-connections-warning" role="alert">
      <TriangleAlert size={14} aria-hidden="true" />{t('codex.proxy.activity.actionFailed')}
      <button type="button" className="btn btn-secondary compact" onClick={() => activity.refresh()}>{t('common.retry')}</button></p>}
    <section className="codex-proxy-connections-table-card" aria-labelledby="codex-proxy-connections-table-title">
      <div className="codex-proxy-page-section-heading">
        <h3 id="codex-proxy-connections-table-title">{t('codex.proxy.activity.connections')}</h3>
        {activity.paused && <span className="codex-proxy-connections-paused" role="status">{t('codex.proxy.activity.pause')}</span>}
      </div>
      {table ?? (activity.loading && !snapshot
        ? <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>
        : !snapshot
          ? <CodexProxyConnectionsState icon={<TriangleAlert size={22} />}
            title={t('codex.proxy.connections.failed')} hint={t('codex.proxy.connections.failedHint')}>
            <button type="button" className="btn btn-secondary compact" onClick={() => activity.refresh()}>{t('common.retry')}</button>
          </CodexProxyConnectionsState>
          : !snapshot.supported
            ? <CodexProxyConnectionsState icon={<Network size={22} />} title={t('codex.proxy.activity.unsupported')}
              hint={t('codex.proxy.connections.unsupportedHint')} />
            : <CodexProxyConnectionsState icon={<Pause size={22} />} title={t('codex.proxy.connections.off')}
              hint={t('codex.proxy.connections.offHint')}>
              <button type="button" className="btn btn-primary compact" disabled={activity.busy}
                onClick={() => void activity.changeEnabled(true)}>{t('codex.proxy.activity.start')}</button>
            </CodexProxyConnectionsState>)}
    </section>
  </section>;
}
