import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getCodexProxyEnv, setCodexProxyEnv, type CodexProxyEnv } from '../../services/codexProxyCountryService';
import { ModalErrorMessage } from '../ModalErrorMessage';

export function CodexProxyEnvEditor({ accountId, disabled, onSaved, onDirtyChange }: {
  accountId: string; disabled?: boolean; onSaved: () => void; onDirtyChange?: (dirty: boolean) => void;
}) {
  const { t } = useTranslation();
  const [view, setView] = useState<CodexProxyEnv | null>(null);
  const [document, setDocument] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const [reload, setReload] = useState(0);
  const current = useRef(accountId); current.current = accountId;
  useEffect(() => {
    let alive = true;
    setView(null); setDocument(''); setError(''); setSaved(false); setBusy(false);
    void getCodexProxyEnv(accountId).then(value => {
      if (alive) { setView(value); setDocument(value.document); }
    }).catch(() => { if (alive) setError(t('codex.proxy.env.readFailed')); });
    return () => { alive = false; };
  }, [accountId, reload, t]);
  useEffect(() => {
    onDirtyChange?.(Boolean(view && document !== view.document));
    return () => { onDirtyChange?.(false); };
  }, [document, view, onDirtyChange]);
  const save = async () => {
    if (!view || busy || disabled) return;
    const id = accountId; setBusy(true); setError(''); setSaved(false);
    try {
      const value = await setCodexProxyEnv(id, document, view.revision);
      if (current.current === id) { setView(value); setDocument(value.document); setSaved(true); onSaved(); }
    } catch (caught) {
      if (current.current === id) {
        const code = String(caught).replace(/^Error:\s*/, '');
        setError(t(code === 'PROXY_ENV_CHANGED' ? 'codex.proxy.env.changed' : code === 'PROXY_ENV_INVALID'
          ? 'codex.proxy.env.invalid' : 'codex.proxy.env.saveFailed'));
      }
    } finally { if (current.current === id) setBusy(false); }
  };
  return <section className="codex-proxy-env-editor" aria-label={t('codex.proxy.env.title')}>
    <h3>{t('codex.proxy.env.title')}</h3>
    <p className="codex-proxy-page-note">{t('codex.proxy.env.source', {
      source: t('codex.proxy.env.source_' + (view?.source ?? 'unknown')),
    })}</p>
    {view?.recognitionFailed && <p className="codex-proxy-page-note" role="status">{t('codex.proxy.env.unrecognized')}</p>}
    {view?.source === 'none' && <p className="codex-proxy-page-note">{t('codex.proxy.env.noneHint')}</p>}
    <p className="codex-proxy-page-note">{t('codex.proxy.env.hint')}</p>
    <label className="codex-proxy-field"><span>{t('codex.proxy.env.variables')}</span>
      <textarea className="codex-proxy-env-input" spellCheck={false} value={document} rows={5}
        maxLength={16384} disabled={disabled || busy || !view}
        placeholder={'HTTP_PROXY=http://127.0.0.1:7890\nHTTPS_PROXY=http://127.0.0.1:7890'}
        onChange={event => { setDocument(event.target.value); setSaved(false); }} /></label>
    <ModalErrorMessage message={error} />
    <div className="codex-proxy-account-check-actions">
      <button type="button" className="btn btn-secondary compact" disabled={busy || disabled}
        onClick={() => setReload(value => value + 1)}>{t('codex.proxy.env.recognize')}</button>
      <button type="button" className="btn btn-secondary compact" disabled={busy || disabled || !view || document === view.document}
        onClick={() => { void save(); }}>{t(busy ? 'common.saving' : 'codex.proxy.env.save')}</button>
      {saved && <span className="codex-proxy-page-note" role="status">{t('codex.proxy.env.saved')}</span>}
    </div>
  </section>;
}
