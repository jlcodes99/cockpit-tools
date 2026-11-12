import { useRef } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import { ModalErrorMessage } from './ModalErrorMessage';

export function AccountFormReadDialog({ title, status, error, onRetry, onClose }: {
  title: string;
  status: 'loading' | 'failed';
  error: string | null;
  onRetry: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, onClose, dialog);
  return createPortal(
    <div className="modal-overlay">
      <div className="modal" ref={dialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label={title}>
        <div className="modal-header">
          <h2>{title}</h2>
          <button type="button" className="modal-close" onClick={onClose} aria-label={t('common.close')}><X /></button>
        </div>
        <div className="modal-body">
          <ModalErrorMessage message={error} />
          {status === 'loading' && <p role="status">{t('common.loading')}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.cancel')}</button>
          {status === 'failed' && <button type="button" className="btn btn-primary" onClick={onRetry}>{t('common.retry')}</button>}
        </div>
      </div>
    </div>, document.body,
  );
}
