import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { X, Minimize2, LogOut } from 'lucide-react';
import { useEscClose } from '../hooks/useEscClose';
import './CloseConfirmDialog.css';

interface CloseConfirmDialogProps {
  onClose: () => void;
}

export function CloseConfirmDialog({ onClose }: CloseConfirmDialogProps) {
  const { t } = useTranslation();
  const [rememberChoice, setRememberChoice] = useState(false);
  const [loading, setLoading] = useState(false);

  useEscClose(true, onClose);

  const handleAction = async (action: 'minimize' | 'quit') => {
    setLoading(true);
    try {
      await invoke('handle_window_close', {
        action,
        remember: rememberChoice,
      });
      onClose();
    } catch (err) {
      console.error('Failed to handle window close:', err);
      setLoading(false);
    }
  };

  return (
    <div className="close-dialog-overlay" onClick={onClose}>
      <div className="close-dialog" onClick={(e) => e.stopPropagation()}>
        <button className="close-dialog-x" onClick={onClose}>
          <X size={18} />
        </button>
        
        <h2 className="close-dialog-title">{t('closeDialog.title')}</h2>
        <p className="close-dialog-desc">{t('closeDialog.description')}</p>
        
        <div className="close-dialog-options">
          <button
            className="close-option-btn minimize"
            onClick={() => handleAction('minimize')}
            disabled={loading}
          >
            <div className="option-icon">
              <Minimize2 size={24} />
            </div>
            <div className="option-content">
              <div className="option-title">{t('closeDialog.minimize')}</div>
              <div className="option-desc">{t('closeDialog.minimizeDesc')}</div>
            </div>
          </button>
          
          <button
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
