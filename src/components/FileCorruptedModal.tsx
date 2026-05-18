import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { FolderOpen, X } from 'lucide-react';
import { useEscClose } from '../hooks/useEscClose';

export interface FileCorruptedError {
  error_type: 'file_corrupted';
  file_name: string;
  file_path: string;
  original_error: string;
}

interface FileCorruptedModalProps {
  error: FileCorruptedError;
  onClose: () => void;
}

export function isFileCorruptedError(error: unknown): error is FileCorruptedError {
  if (typeof error === 'string') {
    try {
      const parsed = JSON.parse(error);
      return parsed?.error_type === 'file_corrupted';
    } catch {
      return false;
    }
  }
  if (typeof error === 'object' && error !== null) {
    return (error as FileCorruptedError).error_type === 'file_corrupted';
  }
  return false;
}

export function parseFileCorruptedError(error: unknown): FileCorruptedError | null {
  if (typeof error === 'string') {
    try {
      const parsed = JSON.parse(error);
      if (parsed?.error_type === 'file_corrupted') {
        return parsed as FileCorruptedError;
      }
    } catch {
      return null;
    }
  }
  if (isFileCorruptedError(error)) {
    return error;
  }
  return null;
}

export function FileCorruptedModal({ error, onClose }: FileCorruptedModalProps) {
  const { t } = useTranslation();
  const [actionError, setActionError] = useState<string | null>(null);

  useEscClose(true, onClose);

  const handleOpenFolder = async () => {
    try {
      setActionError(null);
      // 获取文件所在目录
      const folderPath = error.file_path.substring(0, error.file_path.lastIndexOf('/'));
      await invoke('open_folder', { path: folderPath });
    } catch (e) {
      console.error('Failed to open folder:', e);
      setActionError(
        t('error.fileCorrupted.openFolderFailed', {
          error: String(e),
          defaultValue: '打开文件夹失败：{{error}}',
        })
      );
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ maxWidth: 520 }}>
        <div className="modal-header">
          <h2 style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span>⚠️</span>
            {t('error.fileCorrupted.title', '文件读取失败')}
          </h2>
          <button className="modal-close" onClick={onClose}><X size={20} /></button>
        </div>

        <div className="modal-body">
          <p style={{ marginBottom: 16, color: 'var(--text-primary)' }}>
            {t('error.fileCorrupted.description', '文件 {{fileName}} 已损坏，无法解析。', {
              fileName: error.file_name,
            })}
          </p>

          <div className="profile-card" style={{ marginBottom: 16 }}>
            <div className="fp-fields">
              <div className="fp-field">
                <span className="fp-label">{t('error.fileCorrupted.errorInfo', '错误信息')}:</span>
                <span className="fp-value" style={{ color: 'var(--danger)' }}>{error.original_error}</span>
              </div>
              <div className="fp-field">
                <span className="fp-label">{t('error.fileCorrupted.filePath', '文件位置')}:</span>
                <span className="fp-value">{error.file_path}</span>
              </div>
            </div>
          </div>

          <p style={{ fontSize: 13, color: 'var(--text-secondary)', marginBottom: 0 }}>
            {t(
              'error.fileCorrupted.helpText',
              '请打开文件夹手动修复或删除该文件，然后重新启动应用。'
            )}
          </p>

          {actionError && (
            <div style={{
              marginTop: 12,
              padding: '8px 12px',
              borderRadius: 8,
              background: 'rgba(239, 68, 68, 0.08)',
              border: '1px solid rgba(239, 68, 68, 0.2)',
              color: 'var(--danger, #ef4444)',
              fontSize: 13,
            }}>
              {actionError}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={handleOpenFolder}>
            <FolderOpen size={16} />
            {t('error.fileCorrupted.openFolder', '打开文件夹')}
          </button>
          <button className="btn btn-primary" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default FileCorruptedModal;
