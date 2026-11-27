import { useTranslation } from 'react-i18next';
import type { CodexLocalAccessUsageEvent } from '../../types/codexLocalAccess';
import { requestTimeoutDiagnostic, type RequestTimeoutField } from '../../utils/codexRequestDiagnostics';

const timeoutSettingLabels: Record<RequestTimeoutField, string> = {
  sidecarStreamOpenTimeoutMs: 'streamOpen',
  sidecarStreamIdleTimeoutMs: 'streamIdle',
  sidecarImageStreamOpenTimeoutMs: 'imageOpen',
  sidecarImageStreamIdleTimeoutMs: 'imageIdle',
};

export function CodexRequestTimeoutHint({ event, requestBytes, onOpenSettings }: {
  event: CodexLocalAccessUsageEvent;
  requestBytes?: number | null;
  onOpenSettings?: (field: RequestTimeoutField) => void;
}) {
  const { t } = useTranslation();
  const diagnostic = requestTimeoutDiagnostic(event, requestBytes);
  if (!diagnostic) return null;
  const { kind, seconds, field, largeRequestSize } = diagnostic;
  return <div className="codex-request-timeout-hint">
    <p className="codex-request-diagnostics-hint">{t(`codex.requestDiagnostics.timeout.${kind}`, { setting: t(`codex.apiService.timeouts.${field ? timeoutSettingLabels[field] : 'streamOpen'}`) })}</p>
    {largeRequestSize && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.timeout.largeRequest', { size: largeRequestSize })}</p>}
    {seconds != null && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.timeout.threshold', { seconds })}</p>}
    {field && onOpenSettings && <button type="button" className="btn btn-secondary btn-sm" onClick={() => onOpenSettings(field)}>
      {t('codex.requestDiagnostics.timeout.settings')}
    </button>}
  </div>;
}
