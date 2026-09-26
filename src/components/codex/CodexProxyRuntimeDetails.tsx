import { useTranslation } from 'react-i18next';
import type { ProxyRuntimeRow } from '../../utils/codexProxyPreview';

/** The launch entry and lazy engine are separate local listeners. */
export function CodexProxyRuntimePort({ row }: { row: ProxyRuntimeRow }) {
  const { t } = useTranslation();
  return <span className="codex-proxy-runtime-port">
    {row.kind === 'desktop' && <small>{t(row.entry ? 'codex.proxy.desktopEntryPort' : row.state === 'direct' ? 'codex.proxy.port' : 'codex.proxy.desktopKernel')}</small>}
    <code>{row.port ? `127.0.0.1:${row.port}` : '—'}</code>
  </span>;
}

export function CodexProxyRuntimeDetails({ row }: { row: ProxyRuntimeRow }) {
  const { t } = useTranslation();
  if (!row.entry && !row.selection) return null;
  const { entry } = row;
  return <div className="codex-proxy-runtime-details">
    {row.selection && <small>
      {row.selection.delayMs != null ? `${row.selection.delayMs} ms` : t(row.selection.checkedAt != null ? 'common.failed' : 'codex.proxy.latencyPending')}
      {row.selection.checkedAt != null && <> · {t('codex.proxy.latencyCheckedAt')} <time dateTime={new Date(row.selection.checkedAt).toISOString()}>{new Date(row.selection.checkedAt).toLocaleTimeString()}</time></>}
    </small>}
    {entry && <>
    <small className={entry.lastRequestState === 'failed' ? 'is-failure' : undefined}>
      <span title={t('codex.proxy.desktopEntryRequestsHint')}>{t('codex.proxy.desktopEntryRequests', { count: entry.requestCount })}</span>
      {(entry.lastRequestState !== 'none' || entry.state === 'listening') && <>{' · '}{t(`codex.proxy.desktopRequest_${entry.lastRequestState}`)}</>}
    </small>
    {row.state === 'ready' && row.kernelState === 'idle' && <small>{t('codex.proxy.desktopEntryReadyHint')}</small>}
    {row.kernelState && !['idle', 'direct', 'unbound'].includes(row.kernelState) && <small>
      {t('codex.proxy.desktopKernel')}{' · '}{t(`codex.proxy.runtime_${row.kernelState}`)}
      {row.kernelPort ? <> · <code>127.0.0.1:{row.kernelPort}</code></> : null}
    </small>}
    </>}
  </div>;
}
