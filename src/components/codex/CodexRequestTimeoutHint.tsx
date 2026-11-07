import { useTranslation } from 'react-i18next';
import type { CodexLocalAccessUsageEvent } from '../../types/codexLocalAccess';
import { requestTimeoutDiagnostic, type RequestTimeoutField } from '../../utils/codexRequestDiagnostics';

export function CodexRequestTimeoutHint({ event, onOpenSettings }: {
  event: CodexLocalAccessUsageEvent;
  onOpenSettings?: (field: RequestTimeoutField) => void;
}) {
  const { t } = useTranslation();
  const diagnostic = requestTimeoutDiagnostic(event);
  if (!diagnostic) return null;
  const { kind, seconds, field } = diagnostic;
  return <div className="codex-request-timeout-hint">
    <p className="codex-request-diagnostics-hint">{t(`codex.requestDiagnostics.timeout.${kind}`, { setting: t(`codex.apiService.timeouts.${field === 'sidecarImageStreamOpenTimeoutMs' ? 'imageOpen' : 'streamOpen'}`) })}</p>
    {seconds != null && <p className="codex-request-diagnostics-hint">{t('codex.requestDiagnostics.timeout.threshold', { seconds })}</p>}
    {field && onOpenSettings && <button type="button" className="btn btn-secondary btn-sm" onClick={() => onOpenSettings(field)}>
      {t('codex.requestDiagnostics.timeout.settings')}
    </button>}
  </div>;
}
