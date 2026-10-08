import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertTriangle } from 'lucide-react';
import { ModalErrorMessage } from '../../ModalErrorMessage';

export function PelicanConfirm({ title, description, busy, error, confirmLabel, onCancel, onConfirm }: {
  title: string; description: string; busy: boolean; error?: string | null; confirmLabel?: string; onCancel: () => void; onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const button = useRef<HTMLButtonElement>(null);
  useEffect(() => { button.current?.focus(); }, []);
  return <div className="pelican-overlay pelican-confirm-overlay">
    <section className="pelican-dialog pelican-confirm" role="alertdialog" aria-modal="true" aria-labelledby="pelican-confirm-title" aria-describedby="pelican-confirm-description" onKeyDown={(event) => {
      if (event.key === 'Escape' && !busy) { event.stopPropagation(); onCancel(); }
      if (event.key === 'Tab') {
        const controls = event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
        if (!controls.length) { event.preventDefault(); return; }
        const first = controls[0]; const last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
        if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }
    }}>
      <div className="pelican-confirm-heading"><span className="pelican-confirm-icon" aria-hidden="true"><AlertTriangle size={19} /></span><h3 id="pelican-confirm-title">{title}</h3></div>
      <p id="pelican-confirm-description">{description}</p><ModalErrorMessage message={error} />
      <div className="pelican-confirm-actions"><button type="button" ref={button} className="btn btn-secondary" disabled={busy} onClick={onCancel}>{t('common.cancel')}</button>
        <button type="button" className="btn btn-danger" disabled={busy} onClick={onConfirm}>{busy ? t('common.loading') : confirmLabel ?? t('common.confirm')}</button></div>
    </section>
  </div>;
}
