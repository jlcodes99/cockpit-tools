import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Server, X } from 'lucide-react';
import { useEscClose } from '../../hooks/useEscClose';
import { CodexSshServersPanel } from './CodexSshServersPanel';

interface CodexSshSyncModalProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Codex 切号 SSH 同步配置弹框。
 * 通过 portal 挂到 body，避免被快捷设置/设置页祖先 transform 限制宽度。
 * 仅可通过关闭按钮 / Esc 关闭（不点遮罩关闭）。
 */
export function CodexSshSyncModal({ open, onClose }: CodexSshSyncModalProps) {
  const { t } = useTranslation();
  useEscClose(open, onClose);

  if (!open || typeof document === 'undefined') return null;

  return createPortal(
    <div className="modal-overlay codex-ssh-sync-modal-overlay">
      <div
        className="modal-content codex-ssh-sync-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-ssh-sync-modal-title"
      >
        <div className="codex-ssh-sync-modal__header">
          <div className="codex-ssh-sync-modal__header-main">
            <div className="codex-ssh-sync-modal__icon" aria-hidden="true">
              <Server size={20} />
            </div>
            <div className="codex-ssh-sync-modal__titles">
              <h2 id="codex-ssh-sync-modal-title">
                {t('codex.ssh.modalTitle', 'SSH 同步配置')}
              </h2>
              <p className="codex-ssh-sync-modal__subtitle">
                {t(
                  'codex.ssh.modalHint',
                  '添加并选中一台 SSH 服务器，开启「切号后自动同步」后，Codex 切号会把当前账号同步到远端。',
                )}
              </p>
            </div>
          </div>
          <button
            type="button"
            className="codex-ssh-sync-modal__close"
            onClick={onClose}
            title={t('common.close', '关闭')}
            aria-label={t('common.close', '关闭')}
          >
            <X size={16} />
          </button>
        </div>

        <div className="codex-ssh-sync-modal__body">
          <CodexSshServersPanel embedded />
        </div>

        <div className="codex-ssh-sync-modal__footer">
          <span className="codex-ssh-sync-modal__footer-hint">
            {t(
              'codex.ssh.footerHint',
              '保存后开关会点亮；关闭开关可随时停用自动同步。',
            )}
          </span>
          <button type="button" className="btn btn-primary" onClick={onClose}>
            {t('common.done')}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
