import { useTranslation } from 'react-i18next';
import type { CodexAuthorizationFallbackTrace as Trace } from '../../types/codexLocalAccess';
import { authorizationFallbackReasonKey, authorizationFallbackSummaryKey } from '../../utils/codexRequestDiagnostics';
import './CodexCapabilityFallback.css';

export function CodexAuthorizationFallbackTrace({ trace }: { trace?: Trace | null }) {
  const { t } = useTranslation();
  if (!trace || !authorizationFallbackSummaryKey(trace)) return null;
  const upstream = !trace.triggerStage || trace.triggerStage === 'upstream';
  return <div className="codex-capability-fallback-trace">
    <strong>{t('codex.requestDiagnostics.authorizationFallback.title')}</strong>
    <p>{t('codex.requestDiagnostics.routeOfficial')} → {t('codex.requestDiagnostics.routeLocal')}</p>
    {trace.triggerStage === 'preflight' && <p>{t('codex.requestDiagnostics.authorizationFallback.officialPreflight')}</p>}
    {trace.triggerStage === 'health_pause' && <p>{t('codex.requestDiagnostics.authorizationFallback.officialSkipped')}</p>}
    <p>{t(authorizationFallbackReasonKey(trace.reason))}</p>
    {((upstream && trace.status != null) || trace.errorCode) && <p>
      {upstream && trace.status != null && trace.status >= 100 && trace.status <= 599 && <span>HTTP {trace.status} </span>}
      {trace.errorCode && <code>{trace.errorCode}</code>}
    </p>}
    {trace.errorMessage && <p>{trace.errorMessage}</p>}
  </div>;
}
