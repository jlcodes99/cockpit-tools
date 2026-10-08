import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Loader2, RefreshCw, X } from 'lucide-react';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { preflightCodexProxyEngine } from '../../services/codexProxyEngineService';
import { useCodexProxyEngine } from './useCodexProxyEngine';
import { CodexProxyEnginePanel } from './CodexProxyEngineCard';
import '../../styles/pages/codex-proxy-engine-required.css';

/** Local prerequisite only; the proxy management workspace remains a separate page. */
export function CodexProxyEngineRequiredDialog({ reason, onClose }: { reason: string; onClose: () => void }) {
  const { t } = useTranslation();
  const engine = useCodexProxyEngine();
  const root = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [checking, setChecking] = useState(false);
  const checkingRef = useRef(false);
  const mounted = useRef(true);
  const installJob = useRef<string | null>(null);
  useModalFocusTrap(root, true);
  useModalScrollLock(true);
  useEscCloseTopmost(true, onClose);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (engine.active && engine.status?.jobId) installJob.current = engine.status.jobId;
    if (installJob.current && engine.status?.jobId === installJob.current && engine.phase === 'completed') onClose();
  }, [engine.active, engine.phase, engine.status?.jobId, onClose]);
  const recheck = async () => {
    if (checkingRef.current) return;
    checkingRef.current = true; setChecking(true);
    try {
      await preflightCodexProxyEngine();
      if (mounted.current) { engine.clearPreflightFailure(); engine.refresh(); onClose(); }
    } catch { /* The shared prerequisite handler preserves the failure and installer. */ }
    finally { checkingRef.current = false; if (mounted.current) setChecking(false); }
  };
  const missing = reason === 'PROXY_ENGINE_MISSING';
  return createPortal(<div className="modal-overlay codex-proxy-engine-required-overlay">
    <div ref={root} className="modal codex-proxy-engine-required-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
      <header className="modal-header"><h2 id={titleId}>{t(`codex.proxy.engine.${missing ? 'requiredTitle' : 'repairTitle'}`)}</h2>
        <button type="button" className="btn btn-secondary compact" aria-label={t('common.close')} onClick={onClose}><X size={17} /></button></header>
      <div className="modal-body">
        <p className="codex-proxy-page-note">{t(`codex.proxy.engine.${missing ? 'requiredHint' : 'repairHint'}`)}</p>
        <CodexProxyEnginePanel engine={engine} />
        {checking && <p className="codex-proxy-page-note" role="status">{t('codex.proxy.engine.checkingPrerequisite')}</p>}
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>
        <button type="button" className="btn btn-primary" disabled={checking || engine.busy} onClick={() => void recheck()}>
          {checking ? <Loader2 size={15} className="spin" /> : <RefreshCw size={15} />}{t('codex.proxy.engine.recheck')}</button>
      </footer>
    </div>
  </div>, document.body);
}
