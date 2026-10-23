import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import './CodexAppEnhancementSettings.css';

interface EnhancementConfig {
  codex_app_native_fast_enabled: boolean;
  codex_app_api_quota_compat_enabled: boolean;
}

const DEFAULTS: EnhancementConfig = {
  codex_app_native_fast_enabled: true,
  codex_app_api_quota_compat_enabled: true,
};
const KEY = 'settings.general.codexEnhancements.';
const SWITCHES = [
  ['codex_app_native_fast_enabled', 'nativeFast'],
  ['codex_app_api_quota_compat_enabled', 'quotaCompat'],
] as const;

async function boundedInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      invoke<T>(command, args),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('common.configSaveTimeout')), 10_000);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

/** Shared by the general page and Codex quick settings; saves only its own changed fields. */
export function CodexAppEnhancementSettings({ variant = 'settings', onError }: {
  variant?: 'settings' | 'quick';
  /** Quick settings keeps non-field errors in its fixed dialog error area. */
  onError?: (message: string | null) => void;
}) {
  const { t } = useTranslation();
  const [config, setConfig] = useState(DEFAULTS);
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const busy = useRef(false);
  const revision = useRef(0);
  const alive = useRef(true);
  const reportedError = useRef(false);
  const errorRef = useRef<HTMLDivElement>(null);
  const quick = variant === 'quick';

  const load = useCallback(async () => {
    if (busy.current) return;
    const current = ++revision.current;
    setLoading(true);
    setError('');
    try {
      const value = await boundedInvoke<Partial<EnhancementConfig>>('get_general_config');
      if (!alive.current || current !== revision.current) return;
      const next = { ...DEFAULTS, ...value };
      setConfig(next);
      setLoaded(true);
    } catch (cause) {
      if (alive.current && current === revision.current) setError(String(cause));
    } finally {
      if (alive.current && current === revision.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void load();
    const updated = () => { void load(); };
    window.addEventListener('config-updated', updated);
    return () => {
      alive.current = false;
      revision.current += 1;
      window.removeEventListener('config-updated', updated);
    };
  }, [load]);

  const errorMessage = error ? t(KEY + 'configFailed', {
    error: error.includes('common.configSaveTimeout') ? t('common.configSaveTimeout') : error,
  }) : '';

  useEffect(() => {
    if (onError && (errorMessage || reportedError.current)) {
      onError(errorMessage || null);
      reportedError.current = Boolean(errorMessage);
    }
  }, [errorMessage, onError]);

  useEffect(() => {
    if (error && !onError) errorRef.current?.scrollIntoView({ block: 'nearest' });
  }, [error, onError]);

  const save = async (updates: Partial<EnhancementConfig>) => {
    if (!loaded || busy.current || loading) return;
    busy.current = true;
    revision.current += 1;
    setSaving(true);
    setError('');
    try {
      await boundedInvoke('patch_general_config', { updates });
      if (!alive.current) return;
      setConfig(previous => ({ ...previous, ...updates }));
      window.dispatchEvent(new Event('config-updated'));
    } catch (cause) {
      if (alive.current) setError(String(cause));
    } finally {
      busy.current = false;
      if (alive.current) setSaving(false);
    }
  };

  const disabled = !loaded || loading || saving;

  return (
    <div className={`codex-app-enhancements codex-app-enhancements--${variant}`}>
      {SWITCHES.map(([field, label]) => (
        <div key={field} className="codex-app-enhancement-option">
          <div className={quick ? 'qs-row' : 'settings-row'}>
            <div className={quick ? 'qs-row-label' : 'row-label'}>
              <div className={quick ? undefined : 'row-title'}>{t(KEY + label)}</div>
              {!quick && <div className="row-desc">{t(KEY + label + 'Desc')}</div>}
            </div>
            <div className={quick ? 'qs-row-control' : 'row-control'}>
              <label className={quick ? 'qs-switch' : 'switch'}>
                <input type="checkbox" checked={config[field]} disabled={disabled}
                  aria-label={t(KEY + label)}
                  onChange={event => void save({ [field]: event.target.checked })} />
                <span className={quick ? 'qs-switch-slider' : 'slider'} />
              </label>
            </div>
          </div>
          {quick && <div className="qs-hint">{t(KEY + label + 'Desc')}</div>}
        </div>
      ))}
      <div className={quick ? 'qs-hint' : 'row-desc'}>{t(KEY + 'restartHint')}</div>
      {(loading || saving) && <div className="codex-app-enhancements-status" role="status">{t(saving ? 'common.saving' : 'common.loading')}</div>}
      {error && <div ref={errorRef} className="codex-app-enhancements-error" role="alert">
        {!onError && <span>{errorMessage}</span>}
        <button type="button" className="btn btn-secondary" disabled={saving || loading} onClick={() => { void load(); }}>{t('common.retry')}</button>
      </div>}
    </div>
  );
}
