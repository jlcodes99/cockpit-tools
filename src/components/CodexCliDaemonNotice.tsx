import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../hooks/useModalScrollLock';
import './CodexCliDaemonNotice.css';

type RestartNotice = { command: string; codexHome?: string };

function readNotice(payload: unknown): RestartNotice | undefined {
  if (typeof payload === 'string' && payload.trim()) return { command: payload };
  if (!payload || typeof payload !== 'object' || !('command' in payload)
    || typeof payload.command !== 'string' || !payload.command.trim()) return;
  const codexHome = 'codexHome' in payload && typeof payload.codexHome === 'string'
    ? payload.codexHome : undefined;
  return { command: payload.command, codexHome };
}

/** Restarting a shared daemon requires an explicit action. */
export function CodexCliDaemonNotice() {
  const { t } = useTranslation();
  const [notices, setNotices] = useState<RestartNotice[]>([]);
  const [copyState, setCopyState] = useState<'idle' | 'pending' | 'copied' | 'error'>('idle');
  const [restartState, setRestartState] = useState<'idle' | 'pending' | 'done' | 'error'>('idle');
  const dialog = useRef<HTMLDivElement>(null);
  const error = useRef<HTMLParagraphElement>(null);
  const generation = useRef(0);
  const restarting = useRef(new Set<string>());
  const restarted = useRef<string | null>(null);
  const notice = notices[0];
  const close = () => {
    generation.current += 1;
    restarted.current = null;
    setCopyState('idle');
    setRestartState('idle');
    setNotices((pending) => pending.slice(1));
  };
  useEscCloseTopmost(Boolean(notice), close, dialog);
  useModalFocusTrap(dialog, Boolean(notice));
  useModalScrollLock(Boolean(notice));

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('codex:cli-daemon-restart-required', ({ payload }) => {
      const next = readNotice(payload);
      if (disposed || !next) return;
      const active = restarting.current.has(next.command) || restarted.current === next.command;
      if (active && !next.codexHome) return;
      setNotices((pending) => {
        const index = pending.findIndex((queued, i) => queued.command === next.command && !(active && i === 0));
        if (index === -1) return [...pending, next];
        if (pending[index].codexHome || !next.codexHome) return pending;
        return pending.map((queued, i) => i === index ? next : queued);
      });
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

  const errorKey = restartState === 'error' ? 'codex.cliDaemon.restartFailed'
    : copyState === 'error' ? 'codex.cliDaemon.copyFailed' : null;
  useEffect(() => {
    if (!errorKey) return;
    error.current?.scrollIntoView({ block: 'nearest' });
    error.current?.focus();
  }, [errorKey]);

  const copy = async () => {
    if (!notice || copyState === 'pending') return;
    const current = generation.current;
    if (restartState === 'error') setRestartState('idle');
    setCopyState('pending');
    try {
      await navigator.clipboard.writeText(notice.command);
      if (current === generation.current) setCopyState('copied');
    } catch {
      if (current === generation.current) setCopyState('error');
    }
  };

  const restart = async () => {
    if (!notice?.codexHome || restartState === 'done' || restarting.current.has(notice.command)) return;
    const current = generation.current;
    restarting.current.add(notice.command);
    setCopyState('idle');
    setRestartState('pending');
    try {
      await invoke('codex_restart_cli_daemon', { codexHome: notice.codexHome });
      if (current === generation.current) {
        restarted.current = notice.command;
        setRestartState('done');
      }
    } catch (failure) {
      console.warn('[Codex CLI daemon] Restart failed', failure);
      if (current === generation.current) setRestartState('error');
    } finally {
      restarting.current.delete(notice.command);
    }
  };

  if (!notice) return null;
  return createPortal(
    <div className="modal-overlay codex-cli-daemon-overlay">
      <div ref={dialog} className="modal codex-cli-daemon-dialog" role="dialog" aria-modal="true"
        aria-labelledby="codex-cli-daemon-title" tabIndex={-1}>
        <div className="modal-header">
          <h2 id="codex-cli-daemon-title">{t('codex.cliDaemon.title')}</h2>
          <button type="button" className="btn btn-secondary" aria-label={t('common.close')} onClick={close}><X size={18} /></button>
        </div>
        <div className="modal-body codex-cli-daemon-body">
          <p>{t(notice.codexHome ? 'codex.cliDaemon.profileChanged' : 'codex.cliDaemon.description')}</p>
          <p>{t(notice.codexHome ? 'codex.cliDaemon.restartWarning' : 'codex.cliDaemon.restartHint')}</p>
          <pre>{notice.command}</pre>
          {restartState === 'done' && <p role="status">{t('codex.cliDaemon.restarted')}</p>}
          {errorKey && <p ref={error} tabIndex={-1} role="alert" className="codex-cli-daemon-error">{t(errorKey)}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" disabled={copyState === 'pending' || restartState === 'pending'} onClick={() => { void copy(); }}>
            {t(copyState === 'copied' ? 'common.copied' : 'common.copy')}
          </button>
          <button type="button" className={notice.codexHome ? 'btn btn-secondary' : 'btn btn-primary'} onClick={close}>{t('common.close')}</button>
          {notice.codexHome && <button type="button" className="btn btn-primary"
            disabled={copyState === 'pending' || restartState === 'pending' || restartState === 'done'}
            onClick={() => { void restart(); }}>
            {t(restartState === 'pending' ? 'codex.cliDaemon.restarting' : 'codex.cliDaemon.restart')}
          </button>}
        </div>
      </div>
    </div>, document.body,
  );
}
