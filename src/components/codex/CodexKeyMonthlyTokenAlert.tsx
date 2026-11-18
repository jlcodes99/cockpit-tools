import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../ModalErrorMessage';
import {
  getCodexMonthlyTokenAlert,
  type CodexMonthlyTokenAlertStatus,
} from '../../services/codexLocalAccessService';

export function CodexKeyMonthlyTokenAlert({ keyId, disabled }: { keyId: string; disabled?: boolean }) {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<CodexMonthlyTokenAlertStatus | null>(null);
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [updating, setUpdating] = useState(false);
  const pending = useRef(false);
  const version = useRef(0);
  const labels = 'codex.apiService.keys.monthlyTokens';
  const format = new Intl.NumberFormat(i18n.resolvedLanguage || i18n.language);

  useEffect(() => {
    const request = ++version.current;
    setStatus(null);
    setError(null);
    setDraft('');
    setUpdating(false);
    pending.current = false;
    getCodexMonthlyTokenAlert(keyId).then((next) => {
      if (request !== version.current) return;
      setStatus(next);
      setDraft(next.threshold == null ? '' : String(next.threshold));
    }).catch((reason) => {
      if (request === version.current) setError(String(reason));
    });
    return () => { version.current += 1; };
  }, [keyId, retry]);

  function save() {
    const value = draft.trim();
    const threshold = Number(value);
    if (!/^\d*$/.test(value) || !Number.isSafeInteger(threshold)) {
      setError(t(`${labels}.invalid`));
      return;
    }
    void update(threshold);
  }

  async function update(threshold?: number) {
    if (!status || disabled || pending.current) return;
    const request = version.current;
    pending.current = true;
    setUpdating(true);
    setError(null);
    try {
      const next = await getCodexMonthlyTokenAlert(keyId, threshold);
      if (request !== version.current) return;
      setStatus(next);
      const previous = status.threshold == null ? '' : String(status.threshold);
      const saved = next.threshold == null ? '' : String(next.threshold);
      setDraft((current) => threshold !== undefined || current.trim() === previous ? saved : current);
    } catch (reason) {
      if (request === version.current) setError(String(reason));
    } finally {
      if (request === version.current) {
        pending.current = false;
        setUpdating(false);
      }
    }
  }

  return (
    <div>
      <div className="api-key-token-limit-editor">
        <label>
          <span>{t(`${labels}.title`)}</span>
          <input
            value={draft}
            inputMode="numeric"
            placeholder="100000000"
            onChange={(event) => setDraft(event.target.value)}
            disabled={disabled || updating || !status}
          />
        </label>
        <div>
          {status && <strong>{t(`${labels}.used`, { used: format.format(status.used) })}</strong>}
          <small>{t(`${labels}.hint`)}</small>
          {status ? (
            <div className="api-key-token-limit-actions">
              <button type="button" className="btn btn-secondary" onClick={() => void update()}
                disabled={disabled || updating}>
                {t('common.refresh')}
              </button>
              <button type="button" className="btn btn-secondary" onClick={save}
                disabled={disabled || updating || draft.trim() === (status.threshold == null ? '' : String(status.threshold))}>
                {t('common.save')}
              </button>
            </div>
          ) : error && (
            <button type="button" className="btn btn-secondary" onClick={() => setRetry((n) => n + 1)}>
              {t('common.refresh')}
            </button>
          )}
        </div>
      </div>
      <ModalErrorMessage message={error} />
    </div>
  );
}
