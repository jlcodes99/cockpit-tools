import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../hooks/useModalScrollLock';
import './CodexCliDaemonNotice.css';

/** Guidance only: restarting a shared daemon is always the user's decision. */
export function CodexCliDaemonNotice() {
  const { t } = useTranslation();
  const [commands, setCommands] = useState<string[]>([]);
  const [copyState, setCopyState] = useState<'idle' | 'pending' | 'copied' | 'error'>('idle');
  const dialog = useRef<HTMLDivElement>(null);
  const error = useRef<HTMLParagraphElement>(null);
  const generation = useRef(0);
  const command = commands[0];
  const close = () => {
    generation.current += 1;
    setCopyState('idle');
    setCommands((pending) => pending.slice(1));
  };
  useEscCloseTopmost(Boolean(command), close);
  useModalFocusTrap(dialog, Boolean(command));
  useModalScrollLock(Boolean(command));

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('codex:cli-daemon-restart-required', ({ payload }) => {
      if (disposed || typeof payload !== 'string' || !payload.trim()) return;
      setCommands((pending) => pending.includes(payload) ? pending : [...pending, payload]);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch((failure) => console.warn('[Codex CLI daemon] Notice listener failed', failure));
    return () => {
      disposed = true;
      generation.current += 1;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (copyState !== 'error') return;
    error.current?.scrollIntoView({ block: 'nearest' });
    error.current?.focus();
  }, [copyState]);

  const copy = async () => {
    if (!command || copyState === 'pending') return;
    const current = ++generation.current;
    setCopyState('pending');
    try {
      await navigator.clipboard.writeText(command);
      if (current === generation.current) setCopyState('copied');
    } catch {
      if (current === generation.current) setCopyState('error');
    }
  };

  if (!command) return null;
  return createPortal(
    <div className="modal-overlay codex-cli-daemon-overlay">
      <div ref={dialog} className="modal codex-cli-daemon-dialog" role="dialog" aria-modal="true"
        aria-labelledby="codex-cli-daemon-title" tabIndex={-1}>
        <div className="modal-header">
          <h2 id="codex-cli-daemon-title">{t('codex.cliDaemon.title')}</h2>
          <button type="button" className="btn btn-secondary" aria-label={t('common.close')} onClick={close}><X size={18} /></button>
        </div>
        <div className="modal-body codex-cli-daemon-body">
          <p>{t('codex.cliDaemon.description')}</p>
          <p>{t('codex.cliDaemon.restartHint')}</p>
          <pre>{command}</pre>
          {copyState === 'error' && <p ref={error} tabIndex={-1} role="alert" className="codex-cli-daemon-error">{t('codex.cliDaemon.copyFailed')}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" disabled={copyState === 'pending'} onClick={() => { void copy(); }}>
            {t(copyState === 'copied' ? 'common.copied' : 'common.copy')}
          </button>
          <button type="button" className="btn btn-primary" onClick={close}>{t('common.close')}</button>
        </div>
      </div>
    </div>, document.body,
  );
}
