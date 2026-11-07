import { useEffect, useId, useRef, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { ExternalLink, Eye, EyeOff, LoaderCircle, X } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { TFunction } from 'i18next';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { OPENAI_ADMIN_KEYS_URL, OPENAI_OFFICIAL_USAGE_BASE_URL, openAIUsageErrorKey } from '../../utils/openaiOfficialUsage';
import { queryModelProviderUsage } from '../../services/modelProviderUsageService';
import {
  updateCodexOpenAIUsageKey, upsertCodexModelProviderFromCredential,
  type CodexModelProvider,
} from '../../services/codexModelProviderService';
import './OpenAIOfficialUsagePanel.css';

export function OpenAIUsageKeyField({ value, onChange, disabled, error, inputRef, onError, t }: {
  value: string; onChange: (value: string) => void; disabled?: boolean; error?: string;
  inputRef?: RefObject<HTMLInputElement | null>; onError?: (message: string) => void; t: TFunction;
}) {
  const [visible, setVisible] = useState(false);
  const linkError = useModalErrorState();
  useEffect(() => { linkError.clear(); }, [disabled, value, linkError.clear]);
  const id = useId();
  return <div className="openai-usage-key-field">
    <label htmlFor={id}>{t('codex.openaiUsage.queryKey')}</label>
    <div className="openai-usage-key-input">
      <input id={id} ref={inputRef} className="form-input" type={visible ? 'text' : 'password'}
        value={value} onChange={event => onChange(event.target.value)} disabled={disabled}
        autoComplete="off" spellCheck={false} aria-invalid={Boolean(error)} aria-describedby={error ? `${id}-error` : undefined} />
      <button type="button" className="btn btn-secondary" onClick={() => setVisible(!visible)}
        aria-label={t(visible ? 'codex.api.hideApiKey' : 'codex.api.showApiKey')}>
        {visible ? <EyeOff size={16} /> : <Eye size={16} />}
      </button>
    </div>
    {error && <p id={`${id}-error`} className="openai-usage-field-error" role="alert">{error}</p>}
    <p className="openai-official-usage-note">{t('codex.openaiUsage.permissionGuide')}</p>
    <p className="openai-official-usage-note">{t('codex.openaiUsage.purpose')}</p>
    <a className="openai-usage-create-link" href={OPENAI_ADMIN_KEYS_URL} target="_blank" rel="noreferrer"
      onClick={event => {
        event.preventDefault(); linkError.clear();
        void openUrl(OPENAI_ADMIN_KEYS_URL).catch(() => (onError ?? linkError.report)(t('codex.openaiUsage.openFailed')));
      }}>
      <ExternalLink size={13} />{t('codex.openaiUsage.openAdminKeys')}
    </a>
    <ModalErrorMessage message={linkError.message} scrollKey={linkError.scrollKey} />
  </div>;
}

export function OpenAIUsageSettingsDialog({ provider, apiKey, onClose, t }: {
  provider?: CodexModelProvider | null; apiKey: string; onClose: () => void; t: TFunction;
}) {
  // Capture the original credential so a slow verification cannot overwrite newer edits.
  const [original] = useState(() => ({ provider, key: provider?.apiKeys.find(item => item.apiKey.trim() === apiKey.trim()) }));
  const [value, setValue] = useState(original.key?.usageApiKey ?? '');
  const [busy, setBusy] = useState(false);
  const [fieldError, setFieldError] = useState<string>();
  const error = useModalErrorState();
  const dialogRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const active = useRef(true);
  const submitting = useRef(false);
  const titleId = useId();
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  useEffect(() => { if (fieldError && !busy) inputRef.current?.focus(); }, [fieldError, busy]);
  useEscCloseTopmost(true, onClose, dialogRef);
  useModalScrollLock(true);

  const submit = async (remove = false) => {
    if (submitting.current) return;
    error.clear(); setFieldError(undefined);
    const next = remove ? '' : value.trim();
    if (!remove && !next) {
      setFieldError(t('codex.openaiUsage.keyRequired'));
      inputRef.current?.focus();
      return;
    }
    submitting.current = true; setBusy(true);
    let stage: 'query' | 'save' = 'query';
    try {
      if (!remove) await queryModelProviderUsage({ baseUrl: OPENAI_OFFICIAL_USAGE_BASE_URL, apiKey, usageApiKey: next });
      if (!active.current) return;
      stage = 'save';
      let target = original.provider;
      let key = original.key;
      if (!target || !key) {
        target = await upsertCodexModelProviderFromCredential({ apiBaseUrl: OPENAI_OFFICIAL_USAGE_BASE_URL, apiKey });
        key = target.apiKeys.find(item => item.apiKey.trim() === apiKey.trim());
      }
      if (!key) throw new Error('OPENAI_USAGE_CONFIG_CHANGED');
      await updateCodexOpenAIUsageKey(target.id, key.id, next, { apiKey: original.key?.apiKey ?? apiKey.trim(), usageApiKey: original.key?.usageApiKey });
      if (active.current) onClose();
    } catch (cause) {
      if (!active.current) return;
      const message = String(cause);
      const key = message.includes('OPENAI_USAGE_') ? openAIUsageErrorKey(message)
        : stage === 'save' ? 'codex.openaiUsage.saveFailed' : 'codex.openaiUsage.queryFailed';
      if (message.includes('OPENAI_USAGE_INVALID_KEY') || message.includes('OPENAI_USAGE_PERMISSION_REQUIRED')) {
        setFieldError(t(key)); inputRef.current?.focus();
      } else error.report(t(key));
    } finally {
      submitting.current = false;
      if (active.current) setBusy(false);
    }
  };

  return createPortal(<div className="modal-overlay openai-usage-overlay">
    <div ref={dialogRef} className="modal openai-usage-settings-modal" role="dialog" aria-modal="true" aria-labelledby={titleId}>
      <div className="modal-header">
        <h2 id={titleId}>{t('codex.openaiUsage.title')}</h2>
        <button type="button" className="btn btn-secondary" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
      </div>
      <div className="modal-body">
        <OpenAIUsageKeyField value={value} onChange={next => { setValue(next); error.clear(); setFieldError(undefined); }}
          disabled={busy} error={fieldError} inputRef={inputRef} onError={error.report} t={t} />
        <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
      </div>
      <div className="modal-footer">
        {original.key?.usageApiKey && <button type="button" className="btn btn-danger" disabled={busy} onClick={() => void submit(true)}>{t('codex.openaiUsage.remove')}</button>}
        <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void submit()}>
          {busy && <LoaderCircle size={15} className="loading-spinner" />}{t('codex.openaiUsage.saveVerify')}
        </button>
      </div>
    </div>
  </div>, document.body);
}

export function OpenAIUsageSettingsButton({ provider, apiKey, t }: {
  provider?: CodexModelProvider | null; apiKey: string; t: TFunction;
}) {
  const [open, setOpen] = useState(false);
  const configured = Boolean(provider?.apiKeys.find(item => item.apiKey.trim() === apiKey.trim())?.usageApiKey);
  return <>
    <button type="button" className="btn btn-secondary" disabled={!apiKey.trim()} onClick={() => setOpen(true)}>
      {t(configured ? 'codex.openaiUsage.settings' : 'codex.openaiUsage.configure')}
    </button>
    {open && <OpenAIUsageSettingsDialog key={apiKey} provider={provider} apiKey={apiKey} t={t} onClose={() => setOpen(false)} />}
  </>;
}
