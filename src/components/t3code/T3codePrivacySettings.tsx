import { CheckCircle2, ChevronRight, Info, RotateCw, Shield, X } from 'lucide-react';
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
const FIELDS: PrivacyField[] = ['doNotTrack', 'analytics', 'tracing', 'agentActivity'];
const ENABLE_TELEMETRY: T3codePrivacyPatch = {
  doNotTrack: false,
  analytics: true,
  tracing: true,
  agentActivity: true,
};
const DISABLE_TELEMETRY: T3codePrivacyPatch = {
  doNotTrack: true,
  analytics: false,
  tracing: false,
  agentActivity: false,
};

export function T3codePrivacyDialog({ onClose }: { onClose: () => void }) {
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

  useEffect(() => {
    void load();
  }, [load]);

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

  const allTelemetryDisabled = Boolean(settings?.privacyModeSaved);

  return createPortal(
    <div className="modal-overlay t3code-privacy-overlay">
      <div
        ref={dialogRef}
        className="modal-content t3code-privacy-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
      >
        <div className="modal-header t3code-privacy-header">
          <div className="t3code-privacy-title-group">
            <span className="t3code-privacy-title-icon" aria-hidden="true">
              <Shield size={18} />
            </span>
            <h2 id={titleId}>{t('t3code.desktopPrivacy.title')}</h2>
          </div>
          <button
            type="button"
            className="modal-close"
            onClick={onClose}
            aria-label={t('common.close')}
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>

        <div className="modal-body t3code-privacy-body">
          <ModalErrorMessage message={message} scrollKey={scrollKey} />

          <div className={`t3code-privacy-heading${allTelemetryDisabled ? ' is-protected' : ''}`}>
            <div className="t3code-privacy-heading-copy">
              <span className="t3code-privacy-heading-icon" aria-hidden="true">
                {allTelemetryDisabled ? <CheckCircle2 size={18} /> : <Shield size={18} />}
              </span>
              <p>{t('t3code.desktopPrivacy.connection')}</p>
            </div>
            <div className="t3code-privacy-quick-actions">
              <button
                type="button"
                className="btn btn-secondary btn-sm t3code-privacy-quick-btn"
                disabled={busy || !settings || settings.allEnabledSaved}
                onClick={() => void change(ENABLE_TELEMETRY)}
              >
                {t('t3code.desktopPrivacy.enableAll')}
              </button>
              <button
                type="button"
                className="btn btn-primary btn-sm t3code-privacy-quick-btn"
                disabled={busy || !settings || allTelemetryDisabled}
                onClick={() => void change(DISABLE_TELEMETRY)}
              >
                {t('t3code.desktopPrivacy.disableAll')}
              </button>
            </div>
          </div>

          <div className="t3code-privacy-options" aria-busy={busy}>
            {FIELDS.map((field) => {
              const option =
                field === 'agentActivity' || field === 'doNotTrack'
                  ? settings && { enabled: settings[field], selected: settings[field], blockedBy: null }
                  : settings?.[field];
              const blockedBy = option?.blockedBy;
              const linked =
                field === 'tracing' && settings && !settings.analytics.enabled && !blockedBy;
              const title = t(`t3code.desktopPrivacy.${field}.label`);
              const isEnabled = Boolean(option?.enabled);
              const isSelected = Boolean(option?.selected);
              const status = option
                ? t(isEnabled
                  ? 't3code.desktopPrivacy.statusOn'
                  : isSelected
                    ? 't3code.desktopPrivacy.statusPending'
                    : 't3code.desktopPrivacy.statusOff')
                : '—';
              const descriptionId = `${titleId}-${field}`;
              const isSwitchDisabled = busy || !option;

              return (
                <div
                  key={field}
                  className={`t3code-privacy-option${isEnabled ? ' is-enabled' : ' is-disabled'}`}
                >
                  <div className="t3code-privacy-copy" id={descriptionId}>
                    <div className="t3code-privacy-option-head">
                      <strong>{title}</strong>
                      <span
                        className={`t3code-privacy-status-pill ${isEnabled ? 'is-on' : 'is-off'}`}
                      >
                        {status}
                      </span>
                    </div>
                    <p className="t3code-privacy-desc">
                      {t(`t3code.desktopPrivacy.${field}.description`)}
                    </p>
                    {blockedBy && (
                      <p className={`t3code-privacy-note${blockedBy === 'managedPolicy' ? ' is-blocked' : ''}`}>
                        {blockedBy === 'managedPolicy'
                          ? t('t3code.desktopPrivacy.policyBlocked')
                          : t('t3code.desktopPrivacy.environmentBlocked', {
                              variable: blockedBy,
                            })}
                      </p>
                    )}
                    {linked && (
                      <p className="t3code-privacy-note">{t('t3code.desktopPrivacy.linked')}</p>
                    )}
                  </div>
                  <div className="t3code-privacy-control">
                    <label className="switch t3code-privacy-switch">
                      <input
                        type="checkbox"
                        role="switch"
                        aria-label={title}
                        aria-describedby={descriptionId}
                        checked={isSelected}
                        disabled={isSwitchDisabled}
                        onChange={(event) => void change({ [field]: event.target.checked })}
                      />
                      <span className="slider t3code-privacy-slider" aria-hidden="true" />
                    </label>
                  </div>
                </div>
              );
            })}
          </div>

          <div className="t3code-privacy-meta">
            <p className="t3code-privacy-scope">
              <Info size={14} aria-hidden="true" />
              <span>{t('t3code.desktopPrivacy.launchScope')}</span>
            </p>
            {settings?.restartRequired && (
              <div className="t3code-privacy-restart-notice" role="status">
                <RotateCw size={14} aria-hidden="true" />
                <span>{t('t3code.desktopPrivacy.restartPending')}</span>
              </div>
            )}
          </div>
        </div>

        <div className="modal-footer t3code-privacy-footer">
          {busy && (
            <span className="t3code-privacy-loading-status" role="status">
              {t('common.loading')}
            </span>
          )}
          {message && !settings && (
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy}
              onClick={() => void load()}
            >
              {t('common.retry')}
            </button>
          )}
          {settings?.restartRequired && (
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() => void restart()}
            >
              <RotateCw size={14} aria-hidden="true" />
              {t('instances.actions.restart')}
            </button>
          )}
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            {t('common.done')}
          </button>
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
        className={
          compact
            ? 'qs-btn t3code-privacy-trigger is-compact'
            : 'btn btn-secondary btn-sm t3code-privacy-trigger'
        }
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(true)}
      >
        <Shield size={14} aria-hidden="true" />
        <span>{t('t3code.desktopPrivacy.title')}</span>
        <ChevronRight size={14} className="t3code-privacy-trigger-arrow" aria-hidden="true" />
      </button>
      {open && <T3codePrivacyDialog onClose={() => setOpen(false)} />}
    </>
  );
}
