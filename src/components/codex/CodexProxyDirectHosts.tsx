import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { subscribeTauriEvent } from '../../utils/tauriEventListener';
import './CodexProxyDirectHosts.css';

export function CodexProxyDirectHosts() {
  const { t } = useTranslation();
  const id = useId();
  const field = useRef<HTMLTextAreaElement>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [loadAttempt, setLoadAttempt] = useState(0);

  useEffect(() => {
    let current = true;
    let updated = false;
    const stop = subscribeTauriEvent<string[]>('codex:proxy-direct-hosts-changed', ({ payload }) => {
      updated = true;
      setSaved(payload.join('\n'));
    });
    void invoke<string[]>('codex_proxy_direct_hosts_get').then((rules) => {
      if (current && !updated) setSaved(rules.join('\n'));
    }, () => {
      if (current && !updated) setError('codex.proxy.directHosts.loadFailed');
    });
    return () => { current = false; stop(); };
  }, [loadAttempt]);

  const invalid = error === 'codex.proxy.directHosts.invalid';
  useEffect(() => {
    if (invalid && !saving) field.current?.focus();
  }, [invalid, saving]);

  const save = async () => {
    if (saving || draft === null || saved === null) return;
    setError('');
    setSaving(true);
    try {
      const rules = await invoke<string[]>('codex_proxy_direct_hosts_save', { input: draft });
      setSaved(rules.join('\n'));
      setDraft(null);
    } catch (cause) {
      const code = String(cause);
      if (code === 'codex.proxy.directHosts.invalid') {
        setError(code);
      } else {
        setError(code === 'common.configSaveTimeout' ? code : 'codex.proxy.directHosts.saveFailed');
      }
    } finally {
      setSaving(false);
    }
  };

  return <section className="codex-proxy-direct-hosts" aria-labelledby={`${id}-title`}>
    <label id={`${id}-title`} htmlFor={id}>{t('codex.proxy.directHosts.title')}</label>
    <p id={`${id}-hint`}>{t('codex.proxy.directHosts.hint')}</p>
    <textarea ref={field} id={id} rows={3} maxLength={4096} spellCheck={false} autoComplete="off"
      value={draft ?? saved ?? ''} disabled={saving || saved === null} aria-invalid={invalid}
      aria-describedby={invalid ? `${id}-hint ${id}-error` : `${id}-hint`}
      placeholder={'mcp.lan\n10.20.0.0/16'}
      onChange={(event) => { setDraft(event.target.value); setError(''); }} />
    {error && <p id={`${id}-error`} className="codex-proxy-direct-hosts-error" role="alert">{t(error)}</p>}
    <div>
      {saved === null && error ? <button className="btn btn-secondary compact" type="button"
        onClick={() => { setError(''); setLoadAttempt((attempt) => attempt + 1); }}>{t('common.retry')}</button>
        : <button className="btn btn-secondary compact" type="button" onClick={() => void save()}
          disabled={saving || saved === null || draft === null || draft === saved}>
          {t(saving ? 'common.saving' : 'common.save')}
        </button>}
    </div>
  </section>;
}
