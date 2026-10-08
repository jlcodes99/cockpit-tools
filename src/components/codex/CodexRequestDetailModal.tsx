import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { ChevronDown, Copy, RefreshCw, X } from 'lucide-react';
import type { CodexLocalAccessRequestDetail, CodexLocalAccessUsageEvent } from '../../types/codexLocalAccess';
import { getCodexLocalAccessRequestDetail } from '../../services/codexLocalAccessService';
import { formatRequestDiagnosticError, formatRequestPayloadBody, hasRequestFirstResponse, requestDiagnosticPhaseKey } from '../../utils/codexRequestDiagnostics';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import './CodexRequestDiagnostics.css';

export function CodexRequestDetailModal({ event, maskAccountText, onClose }: {
  event: CodexLocalAccessUsageEvent;
  maskAccountText: (value?: string | null) => string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const active = useRef(true);
  const version = useRef(0);
  const [detail, setDetail] = useState<CodexLocalAccessRequestDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [selectedPayload, setSelectedPayload] = useState('0');
  const [showHeaders, setShowHeaders] = useState(false);
  const [notice, setNotice] = useState('');
  useEscCloseTopmost(true, onClose);
  useModalFocusTrap(dialog, true);
  useModalScrollLock(true);
  const load = useCallback(async () => {
    const requestVersion = ++version.current;
    setLoading(true); setError(''); setNotice('');
    try {
      const value = await getCodexLocalAccessRequestDetail(event.requestId);
      if (active.current && version.current === requestVersion) { setDetail(value); setSelectedPayload('0'); setShowHeaders(false); }
    } catch (cause) {
      if (active.current && version.current === requestVersion) setError(formatRequestDiagnosticError(cause, t));
    } finally {
      if (active.current && version.current === requestVersion) setLoading(false);
    }
  }, [event.requestId, t]);
  useEffect(() => {
    active.current = true;
    setDetail(null);
    void load();
    return () => { active.current = false; version.current++; };
  }, [load]);
  const copy = async (value: string) => {
    const currentVersion = version.current;
    setError(''); setNotice('');
    try {
      await navigator.clipboard.writeText(value);
      if (active.current && version.current === currentVersion) setNotice(t('common.success'));
    } catch (cause) {
      if (active.current && version.current === currentVersion) setError(formatRequestDiagnosticError(cause, t));
    }
  };
  const payload = detail?.payloads[Number(selectedPayload)];
  const firstResponseMs = detail?.firstResponseMs ?? event.firstResponseMs;
  return createPortal(<div className="modal-overlay codex-request-detail-overlay">
    <div className="modal codex-request-detail-modal" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-request-detail-title" tabIndex={-1}>
      <div className="modal-header">
        <h3 id="codex-request-detail-title">{t('codex.requestDiagnostics.title')}</h3>
        <button type="button" className="modal-close" aria-label={t('common.close')} onClick={onClose}><X size={20} /></button>
      </div>
      <div className="modal-body codex-request-detail-body">
        <div className="codex-request-detail-summary">
          <div><span>{t('common.requestId')}</span><code>{event.requestId}</code><button type="button" className="btn btn-secondary btn-sm" onClick={() => void copy(event.requestId)}><Copy size={12} />{t('common.copy')}</button></div>
          <div><span>{t('codex.requestDiagnostics.totalLatency')}</span><strong>{event.latencyMs} ms</strong></div>
          <div><span>{t('codex.requestDiagnostics.firstResponse')}</span><strong>{hasRequestFirstResponse(firstResponseMs) ? `${firstResponseMs} ms` : '—'}</strong></div>
          {detail?.failurePhase && <div><span>{t('codex.requestDiagnostics.failurePhase')}</span><strong>{t(requestDiagnosticPhaseKey(detail.failurePhase))}</strong></div>}
        </div>
        <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.firstResponseHint')}</p>
        {loading && <p role="status">{t('common.loading')}</p>}
        {!loading && !detail && !error && <p>{t('codex.requestDiagnostics.unavailable')}</p>}
        {detail && <>
          <h4>{t('codex.requestDiagnostics.attempts')}</h4>
          {detail.attempts.length === 0 ? <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.noAttempts')}</p> : <ol className="codex-request-attempts">
            {detail.attempts.map((attempt) => <li key={attempt.sequence}>
              <div><strong>{maskAccountText(attempt.accountEmail || attempt.accountId)}</strong><code>{attempt.modelId}</code><span>{attempt.transport}</span><span>{attempt.latencyMs} ms</span>
                <span className={`codex-api-service-pill ${attempt.success ? 'success' : 'error'}`}>{t(attempt.success ? 'codex.localAccess.requestLogSuccess' : 'codex.localAccess.requestLogFailed')}</span>
                {attempt.status != null && <span>HTTP {attempt.status}</span>}
              </div>
              {attempt.startedAtMs > 0 && <time dateTime={new Date(attempt.startedAtMs).toISOString()}>{new Date(attempt.startedAtMs).toLocaleString()}</time>}
              {attempt.failurePhase && <p>{t(requestDiagnosticPhaseKey(attempt.failurePhase))}</p>}
              {attempt.errorMessage && <p className="codex-request-attempt-error">{attempt.errorMessage}</p>}
            </li>)}
          </ol>}
          {detail.truncated && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.detailTruncated')}</p>}
          <h4>{t('codex.requestDiagnostics.payloads')}</h4>
          {detail.payloads.length === 0 ? <p>{t('codex.requestDiagnostics.noPayloads')}</p> : <>
            <SingleSelectDropdown value={selectedPayload} options={detail.payloads.map((item, index) => ({ value: String(index), label: item.stage === 'client'
              ? t('codex.requestDiagnostics.clientPayload')
              : t('codex.requestDiagnostics.upstreamPayload', { sequence: item.attemptSequence ?? index }) }))}
              ariaLabel={t('codex.requestDiagnostics.payloads')} onChange={(value) => { setSelectedPayload(value); setShowHeaders(false); setError(''); setNotice(''); }} />
            {payload && <>
              <div className="codex-request-payload-meta"><span>{payload.transport}</span><span>{payload.contentType}</span><span>{payload.originalBytes} B</span>
                <button type="button" className="btn btn-secondary btn-sm" onClick={() => void copy(payload.body)}><Copy size={12} />{t('common.copy')}</button></div>
              {payload.headers && Object.keys(payload.headers).length > 0 && <>
                <button type="button" className="btn btn-ghost btn-sm codex-request-headers-toggle" aria-expanded={showHeaders} onClick={() => setShowHeaders((value) => !value)}>
                  <ChevronDown size={14} />{t('codex.requestDiagnostics.headers')}
                </button>
                {showHeaders && <pre>{JSON.stringify(payload.headers, null, 2)}</pre>}
              </>}
              <pre className="codex-request-payload-body">{formatRequestPayloadBody(payload)}</pre>
              {payload.truncated && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.payloadTruncated')}</p>}
              {payload.sha256 && <p className="codex-request-payload-hash">SHA-256: <code>{payload.sha256}</code></p>}
            </>}
            <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.privacyHint')}</p>
          </>}
        </>}
        <ModalErrorMessage message={error} position="bottom" />
        {notice && <p role="status">{notice}</p>}
      </div>
      <div className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={loading} onClick={() => void load()}><RefreshCw size={14} />{t('common.refresh')}</button>
        <button type="button" className="btn btn-primary" onClick={onClose}>{t('common.close')}</button>
      </div>
    </div>
  </div>, document.body);
}
