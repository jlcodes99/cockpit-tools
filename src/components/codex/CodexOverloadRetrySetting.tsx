import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { getCodexOverloadStatus, updateCodexOverloadRetry } from '../../services/codexLocalAccessService';
import { formatRequestDiagnosticError } from '../../utils/codexRequestDiagnostics';
import './CodexApiSetting.css';
import './CodexRequestDiagnostics.css';

const SETTING_UPDATED = 'codex-overload-setting-updated';

export function CodexOverloadRetrySetting({ showProgress = false }: { showProgress?: boolean }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Awaited<ReturnType<typeof getCodexOverloadStatus>> | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const inFlight = useRef(false);
  const revision = useRef(0);
  const [reload, setReload] = useState(0);

  useEffect(() => {
    let disposed = false;
    let loading = false;
    const load = async () => {
      if (inFlight.current || loading) return;
      loading = true;
      const version = revision.current;
      try {
        const value = await getCodexOverloadStatus();
        if (!disposed && version === revision.current) setStatus(value);
      } catch (cause) {
        if (!disposed && version === revision.current) setError(formatRequestDiagnosticError(cause, t));
      } finally {
        loading = false;
      }
    };
    const onSettingUpdated = () => { setError(''); void load(); };
    void load();
    window.addEventListener(SETTING_UPDATED, onSettingUpdated);
    const timer = showProgress ? window.setInterval(() => void load(), 2_000) : undefined;
    return () => {
      disposed = true;
      window.removeEventListener(SETTING_UPDATED, onSettingUpdated);
      window.clearInterval(timer);
    };
  }, [showProgress, reload, t]);

  async function save(enabled: boolean) {
    if (inFlight.current) return;
    inFlight.current = true;
    revision.current++;
    setSaving(true);
    setError('');
    try {
      const saved = await updateCodexOverloadRetry(enabled);
      setStatus((previous) => previous && { ...previous, enabled: saved });
      window.dispatchEvent(new Event(SETTING_UPDATED));
    } catch (cause) {
      setError(formatRequestDiagnosticError(cause, t));
    } finally {
      inFlight.current = false;
      setSaving(false);
    }
  }

  return <div className="codex-api-setting codex-request-payload-setting">
    <div className="codex-request-payload-row">
      <strong>{t('codex.overload.setting')}</strong>
      <label className="switch">
        <input type="checkbox" role="switch" aria-label={t('codex.overload.setting')}
          checked={status?.enabled === true} disabled={saving || !status}
          onChange={(event) => void save(event.target.checked)} />
        <span className="slider" />
      </label>
    </div>
    <p className="codex-request-diagnostics-hint">{t('codex.overload.settingHint')}</p>
    {!status && !error && <p role="status">{t('common.loading')}</p>}
    {showProgress && !!status?.activeRequests &&
      <p role="status">{t('codex.overload.retrying', { count: status.activeRequests })}</p>}
    <ModalErrorMessage message={error} position="bottom" />
    {!status && error && <button type="button" className="btn btn-secondary btn-sm" onClick={() => {
      setError('');
      setReload((value) => value + 1);
    }}>{t('common.retry')}</button>}
  </div>;
}
