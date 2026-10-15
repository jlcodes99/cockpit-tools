import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listenSafely } from '../../utils/tauriEventListener';
import {
  clearCodexLocalAccessRequestPayloads,
  getCodexLocalAccessRequestPayloadLogging,
  getCodexLocalAccessRequestPayloadLoggingStatus,
  updateCodexLocalAccessRequestPayloadLogging,
} from '../../services/codexLocalAccessService';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { formatRequestDiagnosticError } from '../../utils/codexRequestDiagnostics';
import './CodexRequestDiagnostics.css';

const STATE_UPDATED = 'codex-local-access-state-updated';

/** All settings entrances edit the same API-service collection, never a duplicate preference. */
export function CodexRequestPayloadSetting() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [pending, setPending] = useState(false);
  const [syncPending, setSyncPending] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const active = useRef(true);
  const version = useRef(0);
  const inFlight = useRef(false);
  const load = useCallback(async () => {
    const requestVersion = ++version.current;
    try {
      const [value, status] = await Promise.all([
        getCodexLocalAccessRequestPayloadLogging(),
        getCodexLocalAccessRequestPayloadLoggingStatus(),
      ]);
      if (active.current && version.current === requestVersion) {
        setEnabled(value);
        setSyncPending(status.pending);
        setError(status.error ? t('codex.requestDiagnostics.applyFailed', { error: status.error }) : '');
      }
    } catch (cause) {
      if (active.current && version.current === requestVersion) setError(formatRequestDiagnosticError(cause, t));
    }
  }, [t]);
  useEffect(() => {
    active.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const onUpdated = () => { if (!inFlight.current) void load(); };
    window.addEventListener(STATE_UPDATED, onUpdated);
    void listenSafely(STATE_UPDATED, onUpdated).then((cleanup) => {
      if (disposed) cleanup(); else unlisten = cleanup;
    }).catch((cause) => { if (!disposed) setError(formatRequestDiagnosticError(cause, t)); });
    void load();
    return () => {
      active.current = false;
      disposed = true;
      version.current++;
      window.removeEventListener(STATE_UPDATED, onUpdated);
      unlisten?.();
    };
  }, [load]);
  const run = async (operation: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    version.current++;
    setPending(true);
    setError('');
    setNotice('');
    try { await operation(); }
    catch (cause) { if (active.current) setError(formatRequestDiagnosticError(cause, t)); }
    finally { inFlight.current = false; if (active.current) setPending(false); }
  };
  return <div className="codex-request-payload-setting">
    <div className="codex-request-payload-row">
      <strong>{t('codex.requestDiagnostics.recordPayloads')}</strong>
      <SingleSelectDropdown
        value={String(enabled === true)}
        options={[
          { value: 'false', label: t('common.disable') },
          { value: 'true', label: t('common.enable') },
        ]}
        disabled={enabled === null || pending}
        ariaLabel={t('codex.requestDiagnostics.recordPayloads')}
        onChange={(value) => void run(async () => {
          const state = await updateCodexLocalAccessRequestPayloadLogging(value === 'true');
          if (active.current) setEnabled(state.collection?.requestPayloadLogging === true);
          window.dispatchEvent(new Event(STATE_UPDATED));
          await load();
        })}
      />
    </div>
    <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.recordPayloadsHint')}</p>
    <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.privacyHint')}</p>
    {enabled === null && !error && <p role="status">{t('common.loading')}</p>}
    {syncPending && <p role="status" className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.applying')}</p>}
    <div className="codex-request-payload-actions">
      {error && <button type="button" className="btn btn-secondary btn-sm" disabled={pending} onClick={() => void run(async () => {
        if (enabled !== null) await updateCodexLocalAccessRequestPayloadLogging(enabled);
        await load();
      })}>{t('common.retry')}</button>}
      <button type="button" className="btn btn-secondary btn-sm" disabled={pending || enabled === null} onClick={() => { setConfirmClear(true); setError(''); setNotice(''); }}>
        {t('codex.requestDiagnostics.clearPayloads')}
      </button>
      {confirmClear && <>
        <span>{t('codex.requestDiagnostics.clearConfirm')}</span>
        <button type="button" className="btn btn-danger btn-sm" disabled={pending} onClick={() => void run(async () => {
          const count = await clearCodexLocalAccessRequestPayloads();
          if (active.current) { setConfirmClear(false); setNotice(t('codex.requestDiagnostics.cleared', { count })); }
          window.dispatchEvent(new Event(STATE_UPDATED));
        })}>{t('common.confirm')}</button>
        <button type="button" className="btn btn-secondary btn-sm" disabled={pending} onClick={() => { setConfirmClear(false); setError(''); }}>{t('common.cancel')}</button>
      </>}
    </div>
    {notice && <p role="status" className="codex-request-diagnostics-hint">{notice}</p>}
    <ModalErrorMessage message={error} position="bottom" />
  </div>;
}
