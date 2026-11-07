import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Activity, Download, Pause, Play, Trash2, TriangleAlert } from 'lucide-react';
import { save as saveFileDialog } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { proxyErrorKey, type CodexProxyProbeResult } from '../../services/codexAccountProxyService';
import type { ProxyActivityChannel, ProxyActivityLog } from '../../services/codexProxyActivityService';
import { useCodexProxyActivity } from './useCodexProxyActivity';
import '../../styles/pages/codex-proxy-activity.css';

interface Props {
  accountId: string;
  probeResult?: CodexProxyProbeResult;
  inModal?: boolean;
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

export function CodexProxyActivityPanel({ accountId, probeResult, inModal = false }: Props) {
  const { t } = useTranslation();
  const activity = useCodexProxyActivity(accountId);
  const [view, setView] = useState<'logs' | 'connections'>('logs');
  const [channel, setChannel] = useState<'all' | ProxyActivityChannel>('all');
  const [level, setLevel] = useState<'all' | ProxyActivityLog['level']>('all');
  const [search, setSearch] = useState('');
  const [exportError, setExportError] = useState(false);
  const snapshot = activity.snapshot;
  const logs = useMemo(() => (snapshot?.logs ?? []).filter((entry) =>
    (channel === 'all' || entry.channel === channel)
    && (level === 'all' || entry.level === level)
    && (!search.trim() || [entry.target, entry.rule, entry.outbound, entry.errorCode]
      .some((value) => value?.toLowerCase().includes(search.trim().toLowerCase()))),
  ), [snapshot?.logs, channel, level, search]);
  const connections = useMemo(() => (snapshot?.connections ?? []).filter((entry) =>
    (channel === 'all' || entry.channel === channel)
    && (!search.trim() || [entry.target, entry.rule, ...entry.chains]
      .some((value) => value?.toLowerCase().includes(search.trim().toLowerCase()))),
  ), [snapshot?.connections, channel, search]);
  const alerts = snapshot?.logs.filter((entry) => entry.errorCode).length ?? 0;

  const exportLogs = async () => {
    setExportError(false);
    try {
      const file = await saveFileDialog({
        defaultPath: `codex-proxy-activity-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!file) return;
      // The backend exposes only parsed route fields and fixed error codes.
      await writeTextFile(file, JSON.stringify({ exportedAt: new Date().toISOString(), logs }, null, 2));
    } catch {
      setExportError(true);
    }
  };

  return <section className="codex-proxy-activity" aria-label={t('codex.proxy.activity.title')}>
    <div className="codex-proxy-activity-head">
      <div><h3><Activity size={17} />{t('codex.proxy.activity.title')}</h3>
        <p>{t('codex.proxy.activity.hint')}</p></div>
      <div className="codex-proxy-activity-actions">
        <button type="button" className="btn btn-secondary compact" disabled={activity.busy || !snapshot?.supported}
          onClick={() => void activity.changeEnabled(!snapshot?.enabled)}>
          {snapshot?.enabled ? <Pause size={14} /> : <Play size={14} />}
          {t(snapshot?.enabled ? 'codex.proxy.activity.stop' : 'codex.proxy.activity.start')}
        </button>
        <button type="button" className="btn btn-secondary compact" disabled={!snapshot?.enabled}
          onClick={() => activity.setPaused(!activity.paused)}>{activity.paused ? <Play size={14} /> : <Pause size={14} />}
          {t(activity.paused ? 'codex.proxy.activity.resume' : 'codex.proxy.activity.pause')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={activity.busy || !snapshot?.logs.length}
          onClick={() => void activity.clear()}><Trash2 size={14} />{t('codex.proxy.activity.clear')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={!logs.length}
          onClick={() => void exportLogs()}><Download size={14} />{t('codex.proxy.activity.export')}</button>
      </div>
    </div>
    {!activity.loading && !snapshot?.supported && <p className="codex-proxy-activity-notice">{t('codex.proxy.activity.unsupported')}</p>}
    {snapshot?.supported && !snapshot.enabled && <p className="codex-proxy-activity-notice">{t('codex.proxy.activity.off')}</p>}
    {snapshot?.enabled && snapshot.captureError && <p className="codex-proxy-activity-warning" role="status">{t('codex.proxy.activity.captureFailed')}</p>}
    {snapshot?.connectionsError && <p className="codex-proxy-activity-warning" role="status">{t('codex.proxy.activity.connectionsFailed')}</p>}
    {(activity.error || exportError) && (inModal
      ? <ModalErrorMessage message={t('codex.proxy.activity.actionFailed')} />
      : <p className="codex-proxy-activity-warning" role="alert">{t('codex.proxy.activity.actionFailed')}</p>)}
    {alerts > 0 && <p className="codex-proxy-activity-warning"><TriangleAlert size={14} />{t('codex.proxy.activity.alerts', { count: alerts })}</p>}
    {probeResult && <p className="codex-proxy-activity-diagnostic">{t('codex.proxy.activity.probe')}：{probeResult.ip} · {probeResult.latencyMs} ms <small>{t('codex.proxy.activity.probeHint')}</small></p>}
    <div className="codex-proxy-activity-tabs" role="tablist" aria-label={t('codex.proxy.activity.title')}>
      <button type="button" className={`btn ${view === 'logs' ? 'is-active' : ''}`} role="tab" aria-selected={view === 'logs'} onClick={() => setView('logs')}>{t('codex.proxy.activity.logs')} <span>{snapshot?.logs.length ?? 0}</span></button>
      <button type="button" className={`btn ${view === 'connections' ? 'is-active' : ''}`} role="tab" aria-selected={view === 'connections'} onClick={() => setView('connections')}>{t('codex.proxy.activity.connections')} <span>{snapshot?.connections.length ?? 0}</span></button>
    </div>
    <div className="codex-proxy-activity-filters">
      <SingleSelectDropdown value={channel} ariaLabel={t('codex.proxy.activity.channel')}
        options={(['all', 'account', 'desktop', 'sidecar'] as const).map((value) => ({ value, label: t(`codex.proxy.activity.channel_${value}`) }))}
        onChange={(value) => setChannel(value as typeof channel)} />
      {view === 'logs' && <SingleSelectDropdown value={level} ariaLabel={t('codex.proxy.activity.level')}
        options={(['all', 'info', 'warning', 'error'] as const).map((value) => ({ value, label: t(`codex.proxy.activity.level_${value}`) }))}
        onChange={(value) => setLevel(value as typeof level)} />}
      <input type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t('codex.proxy.activity.search')} aria-label={t('codex.proxy.activity.search')} />
    </div>
    {view === 'logs' ? <ol className="codex-proxy-activity-list" aria-label={t('codex.proxy.activity.logs')}>
      {logs.map((entry) => <li key={entry.id} className={entry.errorCode ? 'is-error' : ''}>
        <time dateTime={new Date(entry.timestamp).toISOString()}>{new Date(entry.timestamp).toLocaleTimeString()}</time>
        <span className="codex-proxy-activity-pill">{t(`codex.proxy.activity.channel_${entry.channel}`)}</span>
        {entry.network && <span className="codex-proxy-activity-pill">{entry.network}</span>}
        <strong>{entry.target || (entry.errorCode ? t(proxyErrorKey(entry.errorCode, 'probeFailed')) : '—')}</strong>
        {entry.rule && <small>{entry.rule} → {entry.outbound}</small>}
      </li>)}
      {!logs.length && <li className="codex-proxy-activity-empty">{t(snapshot?.enabled ? 'codex.proxy.activity.empty' : 'codex.proxy.activity.off')}</li>}
    </ol> : <ol className="codex-proxy-activity-list" aria-label={t('codex.proxy.activity.connections')}>
      {connections.map((entry) => <li key={entry.id}>
        <time>{entry.startedAt ? new Date(entry.startedAt).toLocaleTimeString() : '—'}</time>
        <span className="codex-proxy-activity-pill">{t(`codex.proxy.activity.channel_${entry.channel}`)}</span>
        {entry.network && <span className="codex-proxy-activity-pill">{entry.network.toUpperCase()}</span>}
        <strong>{entry.target}</strong><small>{entry.rule || '—'} → {entry.chains.join(' → ') || '—'}</small>
        <small>↑ {formatBytes(entry.upload)} · ↓ {formatBytes(entry.download)}</small>
      </li>)}
      {!connections.length && <li className="codex-proxy-activity-empty">{t('codex.proxy.activity.noConnections')}</li>}
    </ol>}
  </section>;
}
