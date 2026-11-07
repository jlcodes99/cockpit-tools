import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CheckCircle2, Cpu, Download, FolderInput, Loader2, RefreshCw } from 'lucide-react';
import { useCodexProxyEngine } from './useCodexProxyEngine';
import '../../styles/pages/codex-proxy-engine.css';

const sizeLabel = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MiB`;

export function CodexProxyEngineCard() {
  const engine = useCodexProxyEngine();
  return <CodexProxyEnginePanel engine={engine} />;
}

/** Both entries render the same controls, progress, cancellation and retry states. */
export function CodexProxyEnginePanel({ engine, compactWhenReady = false, title, description }: {
  engine: ReturnType<typeof useCodexProxyEngine>;
  compactWhenReady?: boolean;
  title?: string;
  description?: string;
}) {
  const { t } = useTranslation();
  const headingId = useId();
  const [expanded, setExpanded] = useState(false);
  const { status, errorKey, error, reading, action, active, busy, phase, phaseKey, percent, readiness, run, refresh } = engine;
  if (compactWhenReady && readiness === 'ready' && !action && !error && !expanded) return <section className="codex-proxy-engine-ready" aria-label={t('codex.proxy.engine.title')}>
    <CheckCircle2 size={17} aria-hidden="true" /><span><strong>{t('codex.proxy.engine.ready')}</strong> · Mihomo {status?.installedVersion}</span>
    <button type="button" className="btn btn-secondary compact" onClick={() => setExpanded(true)}>{t('codex.proxy.setup.manage')}</button>
    <button type="button" className="btn btn-secondary compact" disabled={reading} onClick={refresh} aria-label={t('common.refresh')}>
      <RefreshCw size={14} className={reading ? 'spin' : undefined} /></button>
  </section>;

  return <section className="codex-proxy-engine-card" aria-labelledby={headingId}>
    <div className="codex-proxy-engine-heading">
      <span className="codex-proxy-engine-icon"><Cpu size={21} /></span>
      <div><h2 id={headingId}>{title ?? t('codex.proxy.engine.title')}</h2>
        <p>{description ?? t('codex.proxy.engine.description')}</p></div>
      {readiness !== 'unsupported' && <span className={`codex-proxy-engine-badge${readiness === 'ready' ? ' is-ready' : ''}`}>
        {active ? <Loader2 size={13} className="spin" /> : readiness === 'ready' ? <CheckCircle2 size={13} /> : null}
        {t(errorKey === 'codex.proxy.engine.statusUnavailable' ? 'codex.proxy.engine.unknown'
          : !status ? 'common.loading' : phaseKey === 'failed' ? 'common.failed' : `codex.proxy.engine.${phaseKey}`)}
      </span>}
    </div>
    <div className="codex-proxy-engine-body">
      <div className="codex-proxy-engine-info">
        {status && <div className="codex-proxy-engine-version"><strong>Mihomo {status.version}</strong>
          {status.archiveBytes !== null && <span>{sizeLabel(status.archiveBytes)}</span>}
          {status.installedVersion && <span>{t('codex.proxy.engine.installed', { version: status.installedVersion })}</span>}</div>}
        <p>{t('codex.proxy.engine.privacy')}</p>
        <p>{t('codex.proxy.engine.importHint')}{status?.assetName && <code>{status.assetName}</code>}</p>
      </div>
      <div className="codex-proxy-engine-actions">
        {active ? <button type="button" className="btn btn-secondary" disabled={!!action || !status?.jobId || phase === 'installing'} onClick={() => void run('cancel')}>
          {t(action === 'cancel' ? 'common.cancelling' : 'common.cancel')}</button> : <>
          <button type="button" className="btn btn-secondary" disabled={busy || reading || !status?.supported || errorKey === 'codex.proxy.engine.statusUnavailable'} onClick={() => void run('import')}>
            <FolderInput size={15} />{t('codex.proxy.engine.import')}</button>
          <button type="button" className="btn btn-primary" disabled={busy || reading || !status?.supported || errorKey === 'codex.proxy.engine.statusUnavailable'} onClick={() => void run('install')}>
            {action ? <Loader2 size={15} className="spin" /> : <Download size={15} />}
            {t(status?.installedVersion ? 'codex.proxy.engine.repair' : 'codex.proxy.engine.install')}</button>
        </>}
        <button type="button" className="btn btn-secondary icon-only" title={t('common.refresh')} aria-label={t('common.refresh')}
          disabled={!!action || reading} onClick={refresh}><RefreshCw size={15} className={reading ? 'spin' : undefined} /></button>
        {compactWhenReady && readiness === 'ready' && !error && <button type="button" className="btn btn-secondary" onClick={() => setExpanded(false)}>{t('codex.proxy.setup.collapse')}</button>}
      </div>
    </div>
    {active && <div className="codex-proxy-engine-progress" role="status">
      <div><span>{t(`codex.proxy.engine.${phaseKey}`)}</span><span>{sizeLabel(status!.receivedBytes)}{status?.totalBytes ? ` / ${sizeLabel(status.totalBytes)}` : ''}</span></div>
      <progress max={100} value={phase === 'downloading' || phase === 'importing' ? percent : undefined} aria-label={t(`codex.proxy.engine.${phaseKey}`)} />
      <p>{t('codex.proxy.engine.background')}</p>
    </div>}
    {status && !status.supported && <p className="codex-proxy-engine-message">{t('codex.proxy.engine.unsupported')}</p>}
    {error && <div className="codex-proxy-engine-error" role="alert"><span>{t(error)}</span>
      <button type="button" className="btn btn-secondary" disabled={!!action || reading} onClick={refresh}>{t('common.refresh')}</button></div>}
  </section>;
}
