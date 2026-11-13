import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Bell } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import './CodexQuotaRecoverySettings.css';

const SETTINGS_CHANGED = 'codex:quota-recovery-settings-changed';

export type CodexQuotaRecoverySettingsVariant = 'default' | 'quick-settings' | 'settings-row';

export function CodexQuotaRecoverySettings({
  variant = 'default',
}: {
  variant?: CodexQuotaRecoverySettingsVariant;
} = {}) {
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

  const statusBadge = enabled !== null && (
    <span className={`codex-quota-recovery-status${enabled ? ' is-enabled' : ''}`}>
      {t(enabled ? 'common.enabled' : 'common.disabled')}
    </span>
  );

  const errorBlock = error && (
    <p className="codex-quota-recovery-error" ref={errorRef} role="alert">
      {t('codex.quotaRecovery.settingsError')}
    </p>
  );

  if (variant === 'quick-settings') {
    return (
      <div className="codex-quota-recovery-qs">
        <div className="qs-row">
          <div className="qs-row-label">
            <Bell size={15} aria-hidden="true" />
            <span>{t('codex.quotaRecovery.title')}</span>
          </div>
          <div className="qs-row-control">{statusBadge}</div>
        </div>
        <div className="qs-hint qs-hint--compact">{t('codex.quotaRecovery.description')}</div>
        <form
          className="codex-quota-recovery-qs-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (!saving && token.trim()) void update(token);
          }}
        >
          <input
            id={inputId}
            type="password"
            className="qs-path-input"
            autoComplete="off"
            placeholder={t('codex.quotaRecovery.token')}
            aria-label={t('codex.quotaRecovery.token')}
            value={token}
            disabled={saving}
            onChange={(event) => {
              setError(false);
              setToken(event.target.value);
            }}
          />
          <button
            type="submit"
            className="qs-btn qs-btn--primary"
            disabled={saving || !token.trim()}
          >
            {t(enabled ? 'common.save' : 'common.enable')}
          </button>
          {enabled && (
            <button
              type="button"
              className="qs-btn"
              disabled={saving}
              onClick={() => void update(null)}
            >
              {t('common.disable')}
            </button>
          )}
        </form>
        {errorBlock}
      </div>
    );
  }

  if (variant === 'settings-row') {
    return (
      <div className="settings-row codex-quota-recovery-settings-row">
        <div className="row-label">
          <div className="row-title codex-quota-recovery-settings-title">
            <span>{t('codex.quotaRecovery.title')}</span>
            {statusBadge}
          </div>
          <div className="row-desc">{t('codex.quotaRecovery.description')}</div>
          {errorBlock}
        </div>
        <div className="row-control">
          <form
            className="codex-quota-recovery-controls"
            onSubmit={(event) => {
              event.preventDefault();
              if (!saving && token.trim()) void update(token);
            }}
          >
            <input
              id={inputId}
              type="password"
              autoComplete="off"
              placeholder={t('codex.quotaRecovery.token')}
              aria-label={t('codex.quotaRecovery.token')}
              value={token}
              disabled={saving}
              onChange={(event) => {
                setError(false);
                setToken(event.target.value);
              }}
            />
            <button type="submit" className="btn btn-secondary btn-sm" disabled={saving || !token.trim()}>
              {t(enabled ? 'common.save' : 'common.enable')}
            </button>
            {enabled && (
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                disabled={saving}
                onClick={() => void update(null)}
              >
                {t('common.disable')}
              </button>
            )}
          </form>
        </div>
      </div>
    );
  }

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
      {errorBlock}
    </section>
  );
}
