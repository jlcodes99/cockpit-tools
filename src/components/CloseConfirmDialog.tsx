import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { X, Minimize2, LogOut } from 'lucide-react';
import { useEscClose } from '../hooks/useEscClose';
import './CloseConfirmDialog.css';

interface CloseConfirmDialogProps {
  trayVisible: boolean;
  onClose: () => void;
}

export function CloseConfirmDialog({ trayVisible, onClose }: CloseConfirmDialogProps) {
  const { t } = useTranslation();
  const [rememberChoice, setRememberChoice] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const errorRef = useRef<HTMLParagraphElement>(null);

  useEscClose(true, onClose);
  useEffect(() => {
    if (error) errorRef.current?.scrollIntoView({ block: 'nearest' });
  }, [error]);

  const handleAction = async (action: 'minimize' | 'quit') => {
    setError('');
    setLoading(true);
    try {
      await invoke('handle_window_close', {
        action,
        remember: rememberChoice,
      });
      onClose();
    } catch (err) {
      setError(String(err));
      setLoading(false);
    }
  };

  return (
    <div className="close-dialog-overlay">
      <div className="close-dialog" onClick={(e) => e.stopPropagation()}>
        <button type="button" className="close-dialog-x" aria-label={t('common.close')} onClick={onClose}>
          <X size={18} />
        </button>
        
        <h2 className="close-dialog-title">{t('closeDialog.title')}</h2>
        <div className="modal-body close-dialog-body">
          <p className="close-dialog-desc">{t('closeDialog.description')}</p>
          <div className="close-dialog-options">
            <button
              type="button"
              className="close-option-btn minimize"
              onClick={() => handleAction('minimize')}
              disabled={loading}
            >
              <div className="option-icon">
                <Minimize2 size={24} />
              </div>
              <div className="option-content">
                <div className="option-title">
                  {t(trayVisible ? 'closeDialog.minimize' : 'closeDialog.minimizeWindow')}
                </div>
                <div className="option-desc">{t('closeDialog.minimizeDesc')}</div>
              </div>
            </button>

            <button
              type="button"
              className="close-option-btn quit"
              onClick={() => handleAction('quit')}
              disabled={loading}
            >
              <div className="option-icon">
                <LogOut size={24} />
              </div>
              <div className="option-content">
                <div className="option-title">{t('closeDialog.quit')}</div>
                <div className="option-desc">{t('closeDialog.quitDesc')}</div>
              </div>
            </button>
          </div>

          {error && (
            <p className="close-dialog-error" role="alert" ref={errorRef}>
              {t('common.failed')} · {error}
            </p>
          )}
        </div>
        
        <label className="close-dialog-remember">
          <input
            type="checkbox"
            checked={rememberChoice}
            onChange={(e) => setRememberChoice(e.target.checked)}
          />
          <span>{t('closeDialog.remember')}</span>
        </label>
      </div>
    </div>
  );
}
