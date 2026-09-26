import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { Bird, History, Minus, Square, X } from 'lucide-react';
import { useCodexPelicanStore } from '../../../stores/useCodexPelicanStore';
import { activePelican, cancelPelican, cleanupExpiredPelican } from '../../../services/codexPelicanService';
import { isPelicanRunning, pelicanCompletedCount, type CodexPelicanBatch } from '../../../types/codexPelican';
import { isPrivacyModeEnabledByDefault, maskSensitiveValue, PRIVACY_MODE_CHANGED_EVENT } from '../../../utils/privacy';
import { ModalErrorMessage } from '../../ModalErrorMessage';
import { PelicanSetup } from './PelicanSetup';
import { PelicanResults } from './PelicanResults';
import { PelicanHistory } from './PelicanHistory';
import { PelicanConfirm } from './PelicanConfirm';
import { pelicanError } from './pelicanUtils';
import './pelican.css';

export function CodexPelicanHost() {
  const { t } = useTranslation();
  const state = useCodexPelicanStore();
  const [privacy, setPrivacy] = useState(isPrivacyModeEnabledByDefault);
  const [confirm, setConfirm] = useState<'stop' | 'close' | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [listenerAttempt, setListenerAttempt] = useState(0);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const mask = (value: string) => maskSensitiveValue(value, privacy);

  useEffect(() => {
    void cleanupExpiredPelican().catch((cause) => console.warn('[Pelican] automatic cleanup failed', cause));
  }, []);

  useEffect(() => {
    const sync = () => setPrivacy(isPrivacyModeEnabledByDefault());
    window.addEventListener(PRIVACY_MODE_CHANGED_EVENT, sync);
    return () => window.removeEventListener(PRIVACY_MODE_CHANGED_EVENT, sync);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    setConnectionError(null);
    void listen<CodexPelicanBatch>('codex://pelican-progress', ({ payload }) => {
      if (!disposed) useCodexPelicanStore.getState().receive(payload);
    }).then(async (unsubscribe) => {
      if (disposed) { unsubscribe(); return; }
      unlisten = unsubscribe;
      const batch = await activePelican();
      if (!disposed && batch) useCodexPelicanStore.getState().receive(batch);
    }).catch((cause) => { if (!disposed) setConnectionError(String(cause)); });
    return () => { disposed = true; unlisten?.(); };
  }, [listenerAttempt]);

  useEffect(() => {
    if (!state.visible) return;
    headingRef.current?.focus();
    let disposed = false;
    void activePelican().then((batch) => {
      if (!disposed && batch) {
        const store = useCodexPelicanStore.getState();
        store.receive(batch);
        if (store.view === 'setup') store.show('results');
      }
    }).catch((cause) => { if (!disposed) setConnectionError(String(cause)); });
    return () => { disposed = true; };
  }, [state.visible]);

  useEffect(() => {
    if (!state.active || !isPelicanRunning(state.active)) return;
    let disposed = false;
    let fetching = false;
    // A bounded, single-flight snapshot recovers from dropped native events, including while minimized.
    const timer = window.setInterval(() => {
      if (fetching) return;
      fetching = true;
      void activePelican().then((batch) => {
        if (!disposed && batch) useCodexPelicanStore.getState().receive(batch);
      }).catch((cause) => { if (!disposed) setConnectionError(String(cause)); })
        .finally(() => { fetching = false; });
    }, 3000);
    return () => { disposed = true; clearInterval(timer); };
  }, [state.active?.id, state.active?.status]);

  if (!state.visible) {
    return state.active ? createPortal(<button className="pelican-floating btn btn-primary" onClick={() => state.open()}>
      <Bird size={18} /><span>{t('pelican.title')} · {pelicanCompletedCount(state.active)}/{state.active.items.length} · {t(`pelican.${state.active.status}`)}</span>
    </button>, document.body) : null;
  }
  const batch = state.batch;
  const resultsView = state.view === 'results' && !!batch;
  const activeView = state.view === 'results' && batch?.id === state.active?.id;
  const ask = (action: 'stop' | 'close') => { setError(null); setConfirm(action); };
  const close = () => {
    if (activeView) ask('close');
    else state.minimize();
  };
  return createPortal(<div className="pelican-overlay">
    <section key={resultsView ? 'results' : 'configuration'} className={`pelican-dialog${resultsView ? ' pelican-results-dialog' : ''}`} role="dialog" aria-modal="true" aria-labelledby="pelican-title" onKeyDown={(event) => {
      if (event.key === 'Escape' && !confirm) { event.stopPropagation(); close(); }
      if (event.key === 'Tab' && !confirm) {
        const controls = event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), summary');
        const first = controls[0]; const last = controls[controls.length - 1];
        if (first && event.shiftKey && (document.activeElement === first || document.activeElement === headingRef.current)) { event.preventDefault(); last.focus(); }
        if (last && !event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }
    }}>
      <header className="pelican-header"><h2 id="pelican-title" ref={headingRef} tabIndex={-1}><Bird size={23} />{t(resultsView ? 'pelican.results' : 'pelican.title')}</h2>
        <div className="pelican-actions"><button className="btn btn-secondary" aria-label={t('pelican.minimize')} title={t('pelican.minimize')} onClick={() => state.minimize()}><Minus size={18} /></button>
          <button className="btn btn-secondary" aria-label={t('common.close')} title={t('common.close')} onClick={close}><X size={18} /></button></div></header>
      {!resultsView && <nav className="pelican-tabs">
        <button className={`btn pelican-tab${state.view === 'setup' ? ' is-active' : ''}`} aria-current={state.view === 'setup' ? 'page' : undefined} disabled={state.starting || isPelicanRunning(state.active)} onClick={() => { setError(null); state.show('setup'); }}>{t('pelican.newTest')}</button>
        {state.active && <button className={`btn pelican-tab${state.view === 'results' ? ' is-active' : ''}`} aria-current={state.view === 'results' ? 'page' : undefined} onClick={() => { setError(null); state.show('results'); }}>{t('pelican.current')}</button>}
        <button className={`btn pelican-tab${state.view === 'history' ? ' is-active' : ''}`} aria-current={state.view === 'history' ? 'page' : undefined} onClick={() => { setError(null); state.show('history'); }}><History size={15} />{t('pelican.history')}</button>
      </nav>}
      <div className="pelican-body">
        <ModalErrorMessage message={connectionError ? pelicanError(connectionError, t) : null} />
        {connectionError && <button className="btn btn-secondary" onClick={() => setListenerAttempt((attempt) => attempt + 1)}>{t('common.refresh')}</button>}
        {state.view === 'setup' && <PelicanSetup mask={mask} />}
        {state.view === 'results' && batch && <PelicanResults batch={batch} mask={mask} />}
        {state.view === 'history' && <PelicanHistory />}
      </div>
      {activeView && batch && <footer className="pelican-footer"><button className="btn btn-danger" disabled={!isPelicanRunning(batch) || batch.status === 'cancelling'} onClick={() => ask('stop')}><Square size={14} />{t('pelican.stop')}</button>
        <button className="btn btn-secondary" onClick={() => ask('close')}>{t('pelican.close')}</button></footer>}
    </section>
    {confirm && batch && <PelicanConfirm title={t(confirm === 'stop' ? 'pelican.confirmStop' : 'pelican.confirmClose')}
      description={t(confirm === 'stop' ? 'pelican.stopHelp' : 'pelican.closeHelp')} busy={busy} error={error}
      confirmLabel={t(confirm === 'stop' ? 'pelican.stop' : 'pelican.close')}
      onCancel={() => { setError(null); setConfirm(null); }} onConfirm={() => void (async () => {
        setBusy(true); setError(null);
        try {
          if (confirm === 'stop') state.receive(await cancelPelican(batch.id));
          else await state.dismiss(batch.id);
          setConfirm(null);
        } catch (cause) { setError(pelicanError(cause, t)); }
        finally { setBusy(false); }
      })()} />}
  </div>, document.body);
}
