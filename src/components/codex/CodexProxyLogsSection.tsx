import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Download, Pause, Play, RefreshCw, Trash2, TriangleAlert } from 'lucide-react';
import { save as saveFileDialog } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { useCodexProxyActivity } from './useCodexProxyActivity';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { proxyErrorKey } from '../../services/codexAccountProxyService';
import type { ProxyActivityChannel, ProxyActivityLog } from '../../services/codexProxyActivityService';
import { formatProxyClock } from '../../utils/codexProxyFormat';
import '../../styles/pages/codex-proxy-logs.css';
import '../../styles/pages/codex-proxy-activity.css';

/** Logs page: the selected account's in-memory routing log with filters, pause and a masked export. */
export function CodexProxyLogsSection() {
  const { t } = useTranslation();
  const { accounts, selectedId } = useCodexProxyWorkspace();
  const activity = useCodexProxyActivity(selectedId);
  const [level, setLevel] = useState<'all' | ProxyActivityLog['level']>('all');
  const [channel, setChannel] = useState<'all' | ProxyActivityChannel>('all');
  const [search, setSearch] = useState('');
  const [exportError, setExportError] = useState(false);
  const account = accounts.find((entry) => entry.id === selectedId);
  const resolveName = useCodexProxyAccountName();
  const name = resolveName(account);
  const snapshot = activity.snapshot;
  const logs = useMemo(() => (snapshot?.logs ?? []).filter((entry) =>
    (channel === 'all' || entry.channel === channel)
    && (level === 'all' || entry.level === level)
    && (!search.trim() || [entry.target, entry.rule, entry.outbound, entry.errorCode]
      .some((value) => value?.toLowerCase().includes(search.trim().toLowerCase()))),
  ), [snapshot?.logs, channel, level, search]);
  const alerts = logs.filter((entry) => entry.errorCode).length;

  const exportLogs = async () => {
    setExportError(false);
    try {
      const file = await saveFileDialog({
        defaultPath: `codex-proxy-logs-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!file) return;
      // Only parsed route fields and fixed error codes leave the app.
      await writeTextFile(file, JSON.stringify({ exportedAt: new Date().toISOString(), logs }, null, 2));
    } catch {
      setExportError(true);
    }
  };

  return <section className="codex-proxy-logs" aria-label={t('codex.proxy.workspace.logs')}>
    <header className="codex-proxy-logs-head">
      <div>
        <p className="codex-proxy-logs-scope">{name ? t('codex.proxy.overview.intro', { name }) : t('codex.proxy.logs.noAccount')}</p>
        <p className="codex-proxy-page-note">{t('codex.proxy.logs.retention')}</p>
      </div>
      <div className="codex-proxy-logs-actions">
        <button type="button" className="btn btn-secondary compact" disabled={activity.busy || !snapshot?.supported}
          onClick={() => void activity.changeEnabled(!snapshot?.enabled)}>
          {snapshot?.enabled ? <Pause size={14} /> : <Play size={14} />}
          {t(snapshot?.enabled ? 'codex.proxy.activity.stop' : 'codex.proxy.activity.start')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={!snapshot?.enabled}
          onClick={() => activity.setPaused(!activity.paused)}>
          {activity.paused ? <Play size={14} /> : <Pause size={14} />}
          {t(activity.paused ? 'codex.proxy.activity.resume' : 'codex.proxy.activity.pause')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={activity.busy || !snapshot?.logs.length}
          onClick={() => void activity.clear()}><Trash2 size={14} />{t('codex.proxy.activity.clear')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={!logs.length}
          onClick={() => void exportLogs()}><Download size={14} />{t('codex.proxy.activity.export')}</button>
        <button type="button" className="btn btn-secondary compact" disabled={activity.loading}
          onClick={() => activity.refresh()}><RefreshCw size={14} />{t('common.refresh')}</button>
      </div>
    </header>
    {!activity.loading && !snapshot?.supported && <p className="codex-proxy-activity-notice">{t('codex.proxy.activity.unsupported')}</p>}
    {snapshot?.supported && !snapshot.enabled && <div className="codex-proxy-logs-state">
      <p>{t('codex.proxy.connections.off')}</p>
      <p className="codex-proxy-page-note">{t('codex.proxy.connections.offHint')}</p>
    </div>}
    {snapshot?.enabled && snapshot.captureError && <p className="codex-proxy-activity-warning" role="status">{t('codex.proxy.activity.captureFailed')}</p>}
    {(activity.error || exportError) && <p className="codex-proxy-activity-warning" role="alert">{t('codex.proxy.logs.failed')}</p>}
    {snapshot?.enabled && <>
      <div className="codex-proxy-activity-filters">
        <SingleSelectDropdown value={channel} ariaLabel={t('codex.proxy.activity.channel')}
          options={(['all', 'account', 'desktop', 'sidecar'] as const).map((value) => ({ value, label: t(`codex.proxy.activity.channel_${value}`) }))}
          onChange={(value) => setChannel(value as typeof channel)} />
        <SingleSelectDropdown value={level} ariaLabel={t('codex.proxy.activity.level')}
          options={(['all', 'info', 'warning', 'error'] as const).map((value) => ({ value, label: t(`codex.proxy.activity.level_${value}`) }))}
          onChange={(value) => setLevel(value as typeof level)} />
        <input type="search" value={search} onChange={(event) => setSearch(event.target.value)}
          placeholder={t('codex.proxy.activity.search')} aria-label={t('codex.proxy.activity.search')} />
        <span className="codex-proxy-logs-count">{t('codex.proxy.connections.filteredCount', { shown: logs.length, total: snapshot.logs.length })}</span>
      </div>
      {alerts > 0 && <p className="codex-proxy-activity-warning"><TriangleAlert size={14} aria-hidden="true" />{t('codex.proxy.activity.alerts', { count: alerts })}</p>}
      {activity.paused && <p className="codex-proxy-logs-paused" role="status">{t('codex.proxy.logs.paused')}</p>}
      <ol className="codex-proxy-activity-list codex-proxy-logs-list" aria-label={t('codex.proxy.activity.logs')}>
        {logs.map((entry) => <li key={entry.id} className={entry.errorCode ? 'is-error' : ''} role={entry.errorCode ? 'alert' : undefined}>
          <time dateTime={new Date(entry.timestamp).toISOString()}>{formatProxyClock(entry.timestamp)}</time>
          <span className={`codex-proxy-logs-level is-${entry.level}`}>{t(`codex.proxy.activity.level_${entry.level}`)}</span>
          <span className="codex-proxy-activity-pill">{t(`codex.proxy.activity.channel_${entry.channel}`)}</span>
          {entry.network && <span className="codex-proxy-activity-pill">{entry.network}</span>}
          <strong>{entry.target || (entry.errorCode ? t(proxyErrorKey(entry.errorCode, 'probeFailed')) : '—')}</strong>
          {entry.rule && <small>{entry.rule} → {entry.outbound}</small>}
        </li>)}
        {!logs.length && <li className="codex-proxy-activity-empty">
          {snapshot.logs.length ? t('codex.proxy.connections.noMatch') : t('codex.proxy.activity.empty')}</li>}
      </ol>
      <p className="codex-proxy-page-note">{t('codex.proxy.activity.hint')}</p>
    </>}
  </section>;
}
