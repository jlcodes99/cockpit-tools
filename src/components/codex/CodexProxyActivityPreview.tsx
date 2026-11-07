import { useTranslation } from 'react-i18next';
import { ArrowRight, Play, Pause } from 'lucide-react';
import { proxyErrorKey } from '../../services/codexAccountProxyService';
import { useCodexProxyActivity } from './useCodexProxyActivity';

export function CodexProxyActivityPreview({ accountId, onManage }: { accountId: string; onManage: () => void }) {
  const { t } = useTranslation();
  const activity = useCodexProxyActivity(accountId);
  const snapshot = activity.snapshot;
  return <>
    <div className="codex-proxy-preview-log-actions">
      <span>{t('codex.proxy.activity.previewHint')}</span>
      <button type="button" className="btn btn-secondary" disabled={activity.busy || !snapshot?.supported}
        onClick={() => void activity.changeEnabled(!snapshot?.enabled)}>
        {snapshot?.enabled ? <Pause size={13} /> : <Play size={13} />}
        {t(snapshot?.enabled ? 'codex.proxy.activity.stop' : 'codex.proxy.activity.start')}
      </button>
    </div>
    {activity.error && <p className="codex-proxy-preview-log-error" role="alert">{t('codex.proxy.activity.actionFailed')}</p>}
    {snapshot?.captureError && <p className="codex-proxy-preview-log-error" role="status">{t('codex.proxy.activity.captureFailed')}</p>}
    <ol className="codex-proxy-preview-requests codex-proxy-preview-logs" tabIndex={0} aria-label={t('codex.proxy.activity.logs')}>
      {snapshot?.logs.slice(0, 8).map((entry) => <li key={entry.id}>
        <div><strong>{entry.target || (entry.errorCode ? t(proxyErrorKey(entry.errorCode, 'probeFailed')) : '—')}</strong>
          <span>{entry.network || entry.level}</span></div>
        <small><span>{t(`codex.proxy.activity.channel_${entry.channel}`)} · {entry.rule || '—'} → {entry.outbound || '—'}</span>
          <time dateTime={new Date(entry.timestamp).toISOString()}>{new Date(entry.timestamp).toLocaleTimeString()}</time></small>
      </li>)}
      {!snapshot?.logs.length && <li className="codex-proxy-preview-empty">{t(activity.loading && !snapshot ? 'common.loading' : !snapshot?.supported ? 'codex.proxy.activity.unsupported' : snapshot.enabled ? 'codex.proxy.activity.empty' : 'codex.proxy.activity.off')}</li>}
    </ol>
    <button type="button" className="btn btn-secondary codex-proxy-preview-log-manage" onClick={onManage}>
      {t('codex.proxy.activity.viewAll')}<ArrowRight size={13} />
    </button>
  </>;
}
