import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, RefreshCw, X } from 'lucide-react';
import { isCodexPathMissingError, useCodexPathRecoveryStore } from '../../stores/useCodexPathRecoveryStore';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { scrollElementIntoView } from '../../utils/reducedMotion';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import './CodexPathRecoveryDialog.css';

const IO_TIMEOUT_MS = 15_000;
const IO_TIMEOUT = 'CODEX_PATH_RECOVERY_TIMEOUT';
type Config = { codex_app_path: string; codex_launch_on_switch: boolean };
type Candidate = { target: string; label: string; target_type: string };

/** Expiry stops the continuation even if a native command finishes later. */
function bounded<T>(operation: Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(IO_TIMEOUT)), IO_TIMEOUT_MS);
    operation.then(resolve, reject).finally(() => clearTimeout(timer));
  });
}
// These native commands persist configuration, including automatic detection on
// macOS/Linux. Notify readers even if this dialog was canceled while they ran.
function notifyConfigChange<T>(operation: Promise<T>): Promise<T> {
  return operation.then((value) => {
    window.dispatchEvent(new Event('config-updated'));
    return value;
  });
}
function windowsPlatform(): boolean {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  return (nav.userAgentData?.platform || nav.platform || '').toLowerCase().includes('win');
}

/** A single app-root recovery surface, above the launch flow that requested it. */
export function CodexPathRecoveryDialog() {
  const { t } = useTranslation();
  const request = useCodexPathRecoveryStore((state) => state.request);
  const dialog = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const alive = useRef(true);
  const working = useRef(false);
  const originalPath = useRef('');
  const generation = useRef(0);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState('');
  const [launchOnSwitch, setLaunchOnSwitch] = useState(true);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [fieldErrorRevision, setFieldErrorRevision] = useState(0);
  const error = useModalErrorState();
  const requestId = request?.id;
  const markBusy = (value: boolean) => { working.current = value; setBusy(value); };
  const current = (id: number | undefined, epoch: number) => alive.current
    && generation.current === epoch && useCodexPathRecoveryStore.getState().request?.id === id;
  const reportFieldError = (value: string) => { setFieldError(value); setFieldErrorRevision((revision) => revision + 1); };
  const clearErrors = () => { setFieldError(null); error.clear(); };
  const message = (cause: unknown) => {
    const text = String(cause).replace(/^Error:\s*/, '');
    if (text === 'CODEX_APP_PATH_VALIDATION_BUSY') return t('codex.pathRecovery.validationBusy');
    return text === IO_TIMEOUT || text === 'CODEX_APP_PATH_VALIDATION_TIMEOUT' ? t('codex.pathRecovery.timeout') : text;
  };
  const cancel = useCallback(() => {
    generation.current++;
    request?.cancel();
  }, [request]);
  useModalFocusTrap(dialog, !!request);
  useEscCloseTopmost(!!request, cancel, dialog);
  useModalScrollLock(!!request);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; generation.current++; };
  }, []);

  const load = useCallback(async () => {
    if (requestId == null) return;
    const epoch = ++generation.current;
    setLoading(true); setLoaded(false); markBusy(false); setDraft(''); setCandidates([]);
    setFieldError(null); error.clear();
    try {
      const config = await bounded(invoke<Config>('get_general_config'));
      if (!current(requestId, epoch)) return;
      originalPath.current = config.codex_app_path || '';
      setDraft(originalPath.current);
      setLaunchOnSwitch(config.codex_launch_on_switch !== false);
      setLoaded(true);
    } catch (cause) {
      if (current(requestId, epoch)) error.report(message(cause));
    } finally {
      if (current(requestId, epoch)) setLoading(false);
    }
  // Error helpers are stable; translation changes should not discard an edited path.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestId]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (fieldError && input.current) {
      scrollElementIntoView(input.current, { block: 'nearest' });
      input.current.focus();
    }
  }, [fieldError, fieldErrorRevision]);

  const pick = async () => {
    if (working.current || !loaded) return;
    const epoch = generation.current;
    clearErrors(); markBusy(true);
    try {
      // A system file picker may remain open as long as the user needs.
      const selected = await open({ multiple: false, directory: false });
      if (!current(requestId, epoch)) return;
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (path) { setDraft(path); setCandidates([]); }
    } catch (cause) {
      if (current(requestId, epoch)) error.report(message(cause));
    } finally {
      if (current(requestId, epoch)) markBusy(false);
    }
  };
  const detect = async () => {
    if (working.current || !loaded) return;
    const epoch = generation.current;
    clearErrors(); markBusy(true); setCandidates([]);
    try {
      if (windowsPlatform()) {
        const found = await bounded(invoke<Candidate[]>('scan_app_launch_targets', { app: 'codex' }));
        if (!current(requestId, epoch)) return;
        setCandidates(found);
        if (!found.length) error.report(t('codex.pathRecovery.noCandidates'));
        else if (found.length === 1) setDraft(found[0].target);
      } else {
        const path = await bounded(notifyConfigChange(invoke<string | null>('detect_app_path', { app: 'codex', force: true })));
        if (!current(requestId, epoch)) return;
        if (path) setDraft(path);
        else error.report(t('codex.pathRecovery.noCandidates'));
      }
    } catch (cause) {
      if (current(requestId, epoch)) error.report(message(cause));
    } finally {
      if (current(requestId, epoch)) markBusy(false);
    }
  };
  const save = async (retry: boolean) => {
    if (!request || working.current || !loaded) return;
    const path = draft.trim();
    clearErrors();
    if (retry && !path) { reportFieldError(t('codex.pathRecovery.pathRequired')); return; }
    const epoch = generation.current;
    markBusy(true);
    try {
      if (retry) {
        await bounded(invoke('validate_codex_app_path', { path }));
        if (!current(requestId, epoch)) return;
      }
      if (retry || path !== originalPath.current.trim()) {
        await bounded(notifyConfigChange(invoke('set_app_path', { app: 'codex', path })));
        if (!current(requestId, epoch)) return;
      }
      await bounded(notifyConfigChange(invoke('set_codex_launch_on_switch', { enabled: launchOnSwitch })));
      if (!current(requestId, epoch)) return;
      if (retry) await request.retry();
      else if (request.continueWithoutDesktop) await request.continueWithoutDesktop();
      else request.cancel();
    } catch (cause) {
      if (!current(requestId, epoch)) return;
      const text = String(cause).replace(/^Error:\s*/, '');
      if (text === 'CODEX_APP_PATH_INVALID' || isCodexPathMissingError(cause)) {
        reportFieldError(t('codex.pathRecovery.invalidPath'));
      } else error.report(message(cause));
    } finally {
      if (current(requestId, epoch)) markBusy(false);
    }
  };

  if (!request) return null;
  return createPortal(
    <div className="modal-overlay codex-path-recovery-overlay">
      <div ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-path-recovery-title"
        tabIndex={-1} className="modal codex-path-recovery-modal">
        <div className="modal-header">
          <h2 id="codex-path-recovery-title">{t('codex.pathRecovery.title')}</h2>
          <button type="button" className="btn btn-secondary codex-path-recovery-close"
            aria-label={t('common.close')} onClick={cancel}><X size={18} /></button>
        </div>
        <div className="modal-body codex-path-recovery-body">
          <p>{t('codex.pathRecovery.description')}</p>
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
          {loading && <p role="status">{t('common.loading')}</p>}
          {!loaded && !loading && <button type="button" className="btn btn-secondary" onClick={() => void load()}>
            {t('codex.pathRecovery.reload')}
          </button>}
          <label htmlFor="codex-path-recovery-input" className="codex-path-recovery-label">
            <FolderOpen size={16} />{t('settings.general.codexAppPath')}
          </label>
          <input id="codex-path-recovery-input" ref={input} className="codex-path-recovery-input"
            value={draft} placeholder={t('settings.general.codexAppPathPlaceholder')}
            disabled={!loaded || busy} aria-invalid={!!fieldError}
            aria-describedby={fieldError ? 'codex-path-recovery-field-error' : undefined}
            onChange={(event) => { clearErrors(); setDraft(event.target.value); setCandidates([]); }} />
          {fieldError && <p id="codex-path-recovery-field-error" role="alert" className="codex-path-recovery-field-error">{fieldError}</p>}
          <div className="codex-path-recovery-actions">
            <button type="button" className="btn btn-secondary" disabled={!loaded || busy} onClick={() => void pick()}>
              {t('settings.general.codexPathSelect')}
            </button>
            <button type="button" className="btn btn-secondary" disabled={!loaded || busy} onClick={() => void detect()}>
              <RefreshCw size={14} />{t(windowsPlatform() ? 'appPath.missing.scanApps' : 'settings.general.codexPathReset')}
            </button>
          </div>
          {candidates.length > 0 && <div className="codex-path-recovery-candidates" aria-label={t('codex.pathRecovery.candidates')}>
            {candidates.map((candidate) => <button key={candidate.target} type="button"
              className={`btn btn-secondary codex-path-recovery-candidate${draft === candidate.target ? ' selected' : ''}`}
              aria-pressed={draft === candidate.target} disabled={busy}
              onClick={() => { clearErrors(); setDraft(candidate.target); }}>
              <strong>{candidate.label || 'Codex'}</strong><span>{candidate.target}</span>
            </button>)}
          </div>}
          <div className="codex-path-recovery-setting">
            <span id="codex-path-recovery-auto-label">{t('settings.general.codexLaunchOnSwitch')}</span>
            <button type="button" role="switch" aria-checked={launchOnSwitch}
              aria-labelledby="codex-path-recovery-auto-label" disabled={!loaded || busy}
              className="btn codex-path-recovery-switch" onClick={() => { clearErrors(); setLaunchOnSwitch(!launchOnSwitch); }}>
              <span aria-hidden="true" />
            </button>
          </div>
          <p className="codex-path-recovery-hint">{t('codex.pathRecovery.autoLaunchHint')}</p>
          {!request.continueWithoutDesktop && !launchOnSwitch &&
            <p className="codex-path-recovery-hint">{t('codex.pathRecovery.manualLaunchHint')}</p>}
          {busy && <p role="status">{t('common.loading')}</p>}
        </div>
        <div className="modal-footer codex-path-recovery-footer">
          <button type="button" className="btn btn-secondary" onClick={cancel}>{t('common.cancel')}</button>
          {!launchOnSwitch && <button type="button" className="btn btn-secondary" disabled={!loaded || busy || !draft.trim()}
            onClick={() => void save(true)}>{t('codex.pathRecovery.saveAndRetry')}</button>}
          <button type="button" className="btn btn-primary" disabled={!loaded || busy}
            onClick={() => void save(launchOnSwitch)}>
            {t(launchOnSwitch ? 'codex.pathRecovery.saveAndRetry'
              : request.continueWithoutDesktop ? 'codex.pathRecovery.saveWithoutLaunch' : 'codex.pathRecovery.saveOnly')}
          </button>
        </div>
      </div>
    </div>, document.body,
  );
}
