import { ChevronRight, Shield, X } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { startInstance } from '../../services/t3codeInstanceService';
import {
  getT3codePrivacySettings,
  updateT3codePrivacySettings,
  type T3codePrivacyPatch,
  type T3codePrivacySettings,
} from '../../services/t3codeService';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import './T3codePrivacySettings.css';

type PrivacyField = keyof T3codePrivacyPatch;
const FIELDS: PrivacyField[] = ['analytics', 'tracing', 'agentActivity'];
const ENABLE_PRIVACY = { analytics: false, tracing: false, agentActivity: false };

function T3codePrivacyDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const [settings, setSettings] = useState<T3codePrivacySettings>();
  const [busy, setBusy] = useState(true);
  const { clear, message, report, scrollKey } = useModalErrorState();

  useEscCloseTopmost(true, onClose, dialogRef);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);

  const load = useCallback(async () => {
    setBusy(true);
    clear();
    try {
      setSettings(await getT3codePrivacySettings());
    } catch (cause) {
      report(String(cause));
    } finally {
      setBusy(false);
    }
  }, [clear, report]);

  useEffect(() => { void load(); }, [load]);

  async function change(patch: T3codePrivacyPatch) {
    setBusy(true);
    clear();
    try {
      setSettings(await updateT3codePrivacySettings(patch));
    } catch (cause) {
      setSettings(undefined);
      report(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function restart() {
    setBusy(true);
    clear();
    try {
      await startInstance('__default__');
      setSettings(await getT3codePrivacySettings());
    } catch (cause) {
      report(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return createPortal(
    <div className="modal-overlay t3code-privacy-overlay">
      <div ref={dialogRef} className="modal-content t3code-privacy-dialog"
        role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        <div className="modal-header">
          <h2 id={titleId}>{t('t3code.desktopPrivacy.title')}</h2>
          <button type="button" className="modal-close" onClick={onClose} aria-label={t('common.close')}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <div className="modal-body">
          <ModalErrorMessage message={message} scrollKey={scrollKey} />
          <div className="t3code-privacy-heading">
            <p>{t('t3code.desktopPrivacy.connection')}</p>
            <button type="button" className="btn btn-primary"
              disabled={busy || !settings || settings.privacyModeSaved} onClick={() => void change(ENABLE_PRIVACY)}>
              {t('t3code.desktopPrivacy.enable')}
            </button>
          </div>
          <div className="t3code-privacy-options" aria-busy={busy}>
            {FIELDS.map((field) => {
              const option = field === 'agentActivity'
                ? settings && { enabled: settings.agentActivity, blockedBy: null }
                : settings?.[field];
              const blockedBy = option?.blockedBy;
              const linked = field === 'tracing' && settings && !settings.analytics.enabled && !blockedBy;
              const title = t(`t3code.desktopPrivacy.${field}.label`);
              const status = option && t(option.enabled ? 'common.enabled' : 'common.disabled');
              const descriptionId = `${titleId}-${field}`;
              return (
                <div key={field} className="t3code-privacy-option">
                  <div className="t3code-privacy-copy" id={descriptionId}>
                    <strong>{title}</strong>
                    <p>{t(`t3code.desktopPrivacy.${field}.description`)}</p>
                    {blockedBy && (
                      <p>{blockedBy === 'managedPolicy'
                        ? t('t3code.desktopPrivacy.policyBlocked')
                        : t('t3code.desktopPrivacy.environmentBlocked', { variable: blockedBy })}</p>
                    )}
                    {field === 'analytics' && <p>{t('t3code.desktopPrivacy.linked')}</p>}
                  </div>
                  <div className="t3code-privacy-control">
                    <span aria-hidden="true">{status ?? '—'}</span>
                    <label className="switch">
                      <input type="checkbox" role="switch" aria-label={title}
                        aria-describedby={descriptionId} checked={option?.enabled ?? false}
                        disabled={busy || !option || Boolean(blockedBy) || Boolean(linked)}
                        onChange={(event) => void change({ [field]: event.target.checked })} />
                      <span className="slider" />
                    </label>
                  </div>
                </div>
              );
            })}
          </div>
          <p className="t3code-privacy-scope">{t('t3code.desktopPrivacy.launchScope')}</p>
          {busy && <p role="status">{t('common.loading')}</p>}
          {settings?.restartRequired && (
            <p role="status">{t('t3code.desktopPrivacy.restartPending')}</p>
          )}
        </div>
        <div className="modal-footer">
          {message && !settings && (
            <button type="button" className="btn btn-secondary"
              disabled={busy} onClick={() => void load()}>{t('common.retry')}</button>
          )}
          {settings?.restartRequired && (
            <button type="button" className="btn btn-primary"
              disabled={busy} onClick={() => void restart()}>{t('instances.actions.restart')}</button>
          )}
          <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.done')}</button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export function T3codePrivacySettingsButton({ compact = false }: { compact?: boolean }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        className={compact ? 'qs-btn t3code-privacy-trigger' : 'btn btn-secondary btn-sm t3code-privacy-trigger'}
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
      >
        <Shield size={14} aria-hidden="true" />
        <span>{t('t3code.desktopPrivacy.title')}</span>
        <ChevronRight size={14} aria-hidden="true" />
      </button>
      {open && <T3codePrivacyDialog onClose={() => setOpen(false)} />}
    </>
  );
}
