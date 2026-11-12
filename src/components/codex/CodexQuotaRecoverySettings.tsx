import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import './CodexQuotaRecoverySettings.css';

const SETTINGS_CHANGED = 'codex:quota-recovery-settings-changed';

export function CodexQuotaRecoverySettings() {
  const { t } = useTranslation();
  const inputId = useId();
  const errorRef = useRef<HTMLParagraphElement>(null);
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [token, setToken] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);

  useEffect(() => {
    let disposed = false;
    let generation = 0;
    const refresh = async () => {
      const request = ++generation;
      try {
        const result = await invoke<boolean>('codex_quota_recovery_enabled');
        if (!disposed && request === generation) {
          setEnabled(result);
          setError(false);
        }
      } catch {
        if (!disposed && request === generation) setError(true);
      }
    };
    const subscription = listen(SETTINGS_CHANGED, () => void refresh()).catch(() => undefined);
    void subscription.then(() => {
      if (!disposed) void refresh();
    });
    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten?.());
    };
  }, []);

  useEffect(() => {
    if (error) errorRef.current?.scrollIntoView({ block: 'nearest' });
  }, [error]);

  const update = async (newToken: string | null) => {
    setError(false);
    setSaving(true);
    try {
      const result = await invoke<boolean>('codex_quota_recovery_set_token', { newToken });
      setEnabled(result);
      setToken('');
    } catch {
      setError(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="codex-quota-recovery">
      <div className="codex-quota-recovery-title">
        {t('codex.quotaRecovery.title')}
        {enabled !== null && <span>{t(enabled ? 'common.enabled' : 'common.disabled')}</span>}
      </div>
      <p className="codex-quota-recovery-description">{t('codex.quotaRecovery.description')}</p>
      <form className="codex-quota-recovery-controls" onSubmit={(event) => {
        event.preventDefault();
        if (!saving && token.trim()) void update(token);
      }}>
        <label htmlFor={inputId}>{t('codex.quotaRecovery.token')}</label>
        <input
          id={inputId}
          type="password"
          autoComplete="off"
          value={token}
          disabled={saving}
          onChange={(event) => {
            setError(false);
            setToken(event.target.value);
          }}
        />
        <button type="submit" className="btn btn-secondary" disabled={saving || !token.trim()}>
          {t(enabled ? 'common.save' : 'common.enable')}
        </button>
        {enabled && (
          <button type="button" className="btn btn-secondary" disabled={saving} onClick={() => void update(null)}>
            {t('common.disable')}
          </button>
        )}
      </form>
      {error && (
        <p className="codex-quota-recovery-error" ref={errorRef} role="alert">
          {t('codex.quotaRecovery.settingsError')}
        </p>
      )}
    </section>
  );
}
