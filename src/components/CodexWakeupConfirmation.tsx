import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../hooks/useModalScrollLock';
import './CodexWakeupConfirmation.css';

interface Request {
  requestId: string;
  taskName: string;
  accountCount: number;
  model?: string;
  prompt?: string;
  expiresAt: number;
}

export function CodexWakeupConfirmation() {
  const { t } = useTranslation();
  const [requests, setRequests] = useState<Request[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const dialog = useRef<HTMLDivElement>(null);
  const errorArea = useRef<HTMLParagraphElement>(null);
  const request = requests[0];
  const activeId = useRef<string | undefined>(undefined);
  activeId.current = request?.requestId;
  const answer = async (approved: boolean) => {
    if (!request || busy) return;
    setBusy(true);
    setError('');
    try {
      await invoke('codex_wakeup_respond_confirmation', { requestId: request.requestId, approved });
      setRequests((items) => items.filter((item) => item.requestId !== request.requestId));
    } catch {
      if (activeId.current === request.requestId) setError(t('wakeup.confirmation.expired'));
    } finally { if (activeId.current === request.requestId) setBusy(false); }
  };
  useEscCloseTopmost(Boolean(request), () => { void answer(false); }, dialog);
  useModalFocusTrap(dialog, Boolean(request));
  useModalScrollLock(Boolean(request));

  useEffect(() => {
    let disposed = false;
    const stops: (() => void)[] = [];
    const add = (item: Request) => {
      if (disposed || item.expiresAt <= Date.now() / 1000) return;
      setRequests((items) => items.some((old) => old.requestId === item.requestId) ? items : [...items, item]);
    };
    void (async () => {
      const stop = await listen<Request>('codex:wakeup-confirmation', ({ payload }) => add(payload));
      if (disposed) { stop(); return; } stops.push(stop);
      const end = await listen<string>('codex:wakeup-confirmation-ended', ({ payload }) => {
        if (!disposed) setRequests((items) => items.filter((item) => item.requestId !== payload));
      });
      if (disposed) { end(); return; } stops.push(end);
      // Recover events emitted before the frontend listener became available.
      const pending = await invoke<Request[]>('codex_wakeup_pending_confirmations');
      pending.forEach(add);
    })().catch(() => { /* Backend fails closed if no confirmation is delivered. */ });
    const timer = window.setInterval(() => {
      setRequests((items) => items.filter((item) => item.expiresAt > Date.now() / 1000));
    }, 1000);
    return () => { disposed = true; stops.forEach((stop) => stop()); window.clearInterval(timer); };
  }, []);
  useEffect(() => { setError(''); setBusy(false); }, [request?.requestId]);
  useEffect(() => { if (error) errorArea.current?.scrollIntoView({ block: 'nearest' }); }, [error]);
  if (!request) return null;
  return createPortal(
    <div className="modal-overlay codex-wakeup-confirmation-overlay">
      <div ref={dialog} tabIndex={-1} className="modal codex-wakeup-confirmation" role="dialog" aria-modal="true" aria-labelledby="codex-wakeup-confirmation-title">
        <div className="modal-header">
          <h3 id="codex-wakeup-confirmation-title">{t('wakeup.form.executionModeConfirm')}</h3>
          <button className="btn btn-secondary" aria-label={t('common.close')} disabled={busy} onClick={() => { void answer(false); }}><X size={16} /></button>
        </div>
        <div className="modal-body">
          <p>{t('wakeup.confirmation.description', { name: request.taskName, count: request.accountCount })}</p>
          {request.model && <p>{request.model}</p>}
          {request.prompt && <pre>{request.prompt}</pre>}
          <p>{new Date(request.expiresAt * 1000).toLocaleString()}</p>
          {error && <p ref={errorArea} className="codex-wakeup-confirmation-error" role="alert">{error}</p>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" disabled={busy} onClick={() => { void answer(false); }}>{t('common.cancel')}</button>
          <button className="btn btn-primary" disabled={busy} onClick={() => { void answer(true); }}>{t('common.confirm')}</button>
        </div>
      </div>
    </div>, document.body,
  );
}
