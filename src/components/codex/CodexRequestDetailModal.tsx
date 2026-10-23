import { resolveCodexAccountReferenceDisplayName } from "../../utils/codexAccountDisplayName";
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Activity, CheckCircle2, ChevronDown, Clock3, Copy, FileJson, Gauge, Info, RefreshCw, Route, X, XCircle } from 'lucide-react';
import type { CodexLocalAccessRequestDetail, CodexLocalAccessUsageEvent } from '../../types/codexLocalAccess';
import { getCodexLocalAccessRequestDetail } from '../../services/codexLocalAccessService';
import { formatRequestDiagnosticError, formatRequestPayloadBody, hasRequestFirstResponse, recordedServiceTierAdaptation, requestAuthorizationRouteKey, requestDiagnosticErrorSource, requestDiagnosticPhaseKey, requestServiceTierPresentation, requestedServiceTierLabelKey } from '../../utils/codexRequestDiagnostics';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { CodexRequestTimeoutHint } from './CodexRequestTimeoutHint';
import { CodexCapabilityFallbackTrace } from './CodexCapabilityFallbackTrace';
import { CodexAuthorizationFallbackTrace } from './CodexAuthorizationFallbackTrace';
import type { RequestTimeoutField } from '../../utils/codexRequestDiagnostics';
import { CodexOutputSpeed } from './CodexOutputSpeed';
import { CodexRequestIdentityHeaders } from './CodexRequestIdentityHeaders';
import './CodexRequestIdentityHeaders.css';
import './CodexRequestDiagnostics.css';

export function CodexRequestDetailModal({ event, accountDisplayNames = new Map(), maskAccountText, onOpenTimeoutSettings, onClose }: {
  event: CodexLocalAccessUsageEvent;
  accountDisplayNames?: ReadonlyMap<string, string>;
  maskAccountText: (value?: string | null) => string;
  onOpenTimeoutSettings?: (field: RequestTimeoutField) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const active = useRef(true);
  const version = useRef(0);
  const [detail, setDetail] = useState<CodexLocalAccessRequestDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [errorScrollKey, setErrorScrollKey] = useState(0);
  const [selectedPayload, setSelectedPayload] = useState('0');
  const [showHeaders, setShowHeaders] = useState(false);
  const [notice, setNotice] = useState('');
  useEscCloseTopmost(true, onClose, dialog);
  useModalFocusTrap(dialog, true);
  useModalScrollLock(true);
  const load = useCallback(async () => {
    const requestVersion = ++version.current;
    setLoading(true); setError(''); setNotice('');
    try {
      const value = await getCodexLocalAccessRequestDetail(event.requestId);
      if (active.current && version.current === requestVersion) { setDetail(value); setSelectedPayload('0'); setShowHeaders(false); }
    } catch (cause) {
      if (active.current && version.current === requestVersion) { setError(formatRequestDiagnosticError(cause, t)); setErrorScrollKey((value) => value + 1); }
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
      if (active.current && version.current === currentVersion) { setError(formatRequestDiagnosticError(cause, t)); setErrorScrollKey((value) => value + 1); }
    }
  };
  const payload = detail?.payloads[Number(selectedPayload)];
  const firstResponseMs = detail?.firstResponseMs ?? event.firstResponseMs;
  const tierLabel = (tier: string) => {
    const key = requestedServiceTierLabelKey(tier);
    return key ? t(key) : tier;
  };
  const tiers = requestServiceTierPresentation(event);
  const adaptation = recordedServiceTierAdaptation(event.serviceTierAdaptation);
  const originalTier = adaptation?.requestedTier ?? tiers.requested;
  const requestedTierLabel = tierLabel(originalTier);
  return createPortal(<div className="modal-overlay codex-request-detail-overlay">
    <div className="modal codex-request-detail-modal" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-request-detail-title" tabIndex={-1}>
      <div className="modal-header">
        <div className="codex-request-detail-heading"><span className="codex-request-detail-icon"><Activity size={19} aria-hidden="true" /></span><h3 id="codex-request-detail-title">{t('codex.requestDiagnostics.title')}</h3></div>
        <button type="button" className="modal-close" aria-label={t('common.close')} onClick={onClose}><X size={20} /></button>
      </div>
      <div className="modal-body codex-request-detail-body">
        <div className="codex-request-detail-id">
          <span>{t('common.requestId')}</span><code>{event.requestId}</code>
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void copy(event.requestId)}><Copy size={12} />{t('common.copy')}</button>
        </div>
        <div className="codex-request-detail-summary">
          <div className="codex-request-metric"><span><Clock3 size={14} aria-hidden="true" />{t('codex.requestDiagnostics.totalLatency')}</span><strong>{event.latencyMs} ms</strong></div>
          <div className="codex-request-metric"><span><Activity size={14} aria-hidden="true" />{t('codex.requestDiagnostics.firstResponse')}</span><strong>{hasRequestFirstResponse(firstResponseMs) ? `${firstResponseMs} ms` : '—'}</strong></div>
          <div className="codex-request-metric"><span><Gauge size={14} aria-hidden="true" />{t('common.outputSpeed.label')}</span><strong><CodexOutputSpeed event={event} t={t} hideLabel /></strong></div>
        </div>
        <p className="codex-request-diagnostics-hint codex-request-detail-explanation"><Info size={14} aria-hidden="true" /><span>{t('codex.requestDiagnostics.firstResponseHint')}</span></p>
        <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.authorizationRoute')}: {t(requestAuthorizationRouteKey(event.accountId, event.authorizationRoute))}</p>
        <section className="codex-request-detail-section" aria-labelledby="codex-request-tier-title">
          <h4 id="codex-request-tier-title"><Gauge size={16} aria-hidden="true" />{t('codex.apiService.logs.serviceTier')}</h4>
          <p>{t(adaptation ? 'codex.apiService.logs.originalTier' : 'codex.apiService.logs.requestedTier', { tier: requestedTierLabel || '—' })}</p>
          {adaptation && <><p>{t('codex.apiService.logs.sentTier', { tier: t('codex.speed.standard') })}</p>
            <p className="codex-request-diagnostics-hint">{t('codex.apiService.logs.officialDefaultSpeed')}</p></>}
          <p>{t('codex.apiService.logs.responseTier', { tier: tiers.response || '—' })}</p>
          <p className="codex-request-diagnostics-hint">{t('codex.apiService.logs.serviceTierHint')}</p>
          {tiers.legacy && <><p>{t('codex.apiService.logs.legacyTier', { tier: tiers.legacy })}</p><p className="codex-request-diagnostics-hint">{t('codex.apiService.logs.legacyTierHint')}</p></>}
        </section>
        {detail?.failurePhase && <div className="codex-request-detail-failure"><span>{t('codex.requestDiagnostics.failurePhase')}</span><strong>{t(requestDiagnosticPhaseKey(detail.failurePhase))}</strong></div>}
        {event.success === false && <section className="codex-request-detail-section codex-request-error-evidence" aria-labelledby="codex-request-error-title">
          <h4 id="codex-request-error-title"><XCircle size={16} aria-hidden="true" />{t('codex.localAccess.requestLogFailed')}</h4>
          <p>{t('codex.requestDiagnostics.errorSource')}: <strong>{t(`codex.requestDiagnostics.sources.${requestDiagnosticErrorSource(detail)}`)}</strong></p>
          {event.errorCategory && <code>{event.errorCategory}</code>}
          {event.errorMessage && <p>{event.errorMessage}</p>}
          <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.toolExecutionHint')}</p>
        </section>}
        <CodexRequestTimeoutHint event={event} onOpenSettings={onOpenTimeoutSettings} />
        {loading && <p className="codex-request-diagnostics-hint" role="status">{t('common.loading')}</p>}
        {!loading && !detail && !error && <p>{t('codex.requestDiagnostics.unavailable')}</p>}
        {detail && <>
          <CodexRequestIdentityHeaders detail={detail} t={t} accountDisplayNames={accountDisplayNames} maskAccountText={maskAccountText} onCopy={copy} />
          <section className="codex-request-detail-section" aria-labelledby="codex-request-attempts-title">
            <h4 id="codex-request-attempts-title"><Route size={16} aria-hidden="true" />{t('codex.requestDiagnostics.attempts')}<span className="codex-request-section-count">{detail.attempts.length}</span></h4>
            {detail.attempts.length === 0 ? <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.noAttempts')}</p> : <ol className="codex-request-attempts">
              {detail.attempts.map((attempt) => <li key={attempt.sequence}>
                <span className="codex-request-attempt-number">{attempt.sequence}</span>
                <div className="codex-request-attempt-content">
                  <div className="codex-request-attempt-heading">
                    <strong>{maskAccountText(resolveCodexAccountReferenceDisplayName(attempt.accountId, attempt.accountEmail, accountDisplayNames, t("common.apiKeyAccount")))}</strong>
                    <span className={`codex-request-attempt-status ${attempt.success ? 'success' : 'error'}`}>
                      {attempt.success ? <CheckCircle2 size={13} aria-hidden="true" /> : <XCircle size={13} aria-hidden="true" />}
                      {t(attempt.success ? 'codex.localAccess.requestLogSuccess' : 'codex.localAccess.requestLogFailed')}
                    </span>
                  </div>
                  <div className="codex-request-attempt-meta"><code>{attempt.modelId}</code><span className="codex-request-transport">{attempt.transport}</span>
                    <span>{t(requestAuthorizationRouteKey(attempt.accountId, attempt.authorizationRoute))}</span>
                    {attempt.transport === 'preflight'
                      ? <span>{t('codex.requestDiagnostics.authorizationFallback.localPreflight')}</span>
                      : attempt.status != null && attempt.status >= 100 && attempt.status <= 599 && <span>HTTP {attempt.status}</span>}
                    <span className="codex-request-attempt-latency"><Clock3 size={12} aria-hidden="true" />{attempt.latencyMs} ms</span>
                  </div>
                  {attempt.startedAtMs > 0 && <time dateTime={new Date(attempt.startedAtMs).toISOString()}>{new Date(attempt.startedAtMs).toLocaleString()}</time>}
                  {attempt.failurePhase && <p className="codex-request-attempt-error">{t(requestDiagnosticPhaseKey(attempt.failurePhase))}</p>}
                  {attempt.errorMessage && <p className="codex-request-attempt-error">{attempt.errorMessage}</p>}
                  {attempt.errorCategory && <p className="codex-request-diagnostics-hint"><code>{attempt.errorCategory}</code></p>}
                  {attempt.upstreamRequestId && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.upstreamRequestId')}: <code>{attempt.upstreamRequestId}</code></p>}
                  <CodexAuthorizationFallbackTrace trace={attempt.authorizationFallback} />
                  {recordedServiceTierAdaptation(attempt.serviceTierAdaptation) && <div className="codex-request-diagnostics-hint">
                    <p>{t('codex.apiService.logs.speedAdjusted', { tier: tierLabel(attempt.serviceTierAdaptation!.requestedTier) })}</p>
                    <p>{t('codex.apiService.logs.officialDefaultSpeed')}</p>
                  </div>}
                  <CodexCapabilityFallbackTrace trace={attempt.capabilityFallback} accountDisplayNames={accountDisplayNames} maskAccountText={maskAccountText} />
                </div>
              </li>)}
            </ol>}
            {detail.truncated && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.detailTruncated')}</p>}
          </section>
          <section className="codex-request-detail-section" aria-labelledby="codex-request-payloads-title">
            <h4 id="codex-request-payloads-title"><FileJson size={16} aria-hidden="true" />{t('codex.requestDiagnostics.payloads')}</h4>
            {detail.payloads.length === 0 ? <div className="codex-request-payload-empty"><FileJson size={24} aria-hidden="true" /><p>{t('codex.requestDiagnostics.noPayloads')}</p></div> : <div className="codex-request-payload-panel">
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
            </div>}
          </section>
        </>}
        <ModalErrorMessage message={error} position="bottom" scrollKey={errorScrollKey} />
        {notice && <p className="codex-request-detail-notice" role="status">{notice}</p>}
      </div>
      <div className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={loading} onClick={() => void load()}><RefreshCw size={14} />{t('common.refresh')}</button>
        <button type="button" className="btn btn-primary" onClick={onClose}>{t('common.close')}</button>
      </div>
    </div>
  </div>, document.body);
}
