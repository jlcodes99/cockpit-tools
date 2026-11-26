import { arrayMove } from '@dnd-kit/sortable';
import { GripVertical, ListOrdered, X } from 'lucide-react';
import { useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useDurableUiSetting } from '../../hooks/useDurableUiSetting';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { saveDurableUiSetting, T3CODE_PLATFORM_ORDER_STORAGE_KEY } from '../../utils/durableUiSettings';
import { renderPlatformIcon } from '../../utils/platformMeta';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { SortableLayoutItem, SortableLayoutList } from '../SortableLayoutList';
import { getOrderedT3codePlatforms, type T3codePlatformSpec } from './t3codePlatformRegistry';
import './T3codePlatformOrder.css';

export function T3codePlatformOrderDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const titleId = useId();
  const hintId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const savedOrder = useDurableUiSetting(T3CODE_PLATFORM_ORDER_STORAGE_KEY);
  const savedPlatforms = useMemo(() => getOrderedT3codePlatforms(savedOrder), [savedOrder]);
  const [draft, setDraft] = useState<readonly T3codePlatformSpec[]>();
  const [sorting, setSorting] = useState(false);
  const [saving, setSaving] = useState(false);
  const { clear, message, report, scrollKey } = useModalErrorState();
  const platforms = draft ?? savedPlatforms;

  useEscCloseTopmost(!sorting, () => { if (!saving) onClose(); }, dialogRef);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);

  async function save() {
    if (!draft || draft.every((spec, index) => spec === savedPlatforms[index])) {
      onClose();
      return;
    }
    clear();
    setSaving(true);
    try {
      await saveDurableUiSetting(T3CODE_PLATFORM_ORDER_STORAGE_KEY, JSON.stringify(draft.map((spec) => spec.driver)));
      onClose();
    } catch (cause) {
      report(t('t3code.platformOrder.saveFailed', { error: String(cause) }));
    } finally {
      setSaving(false);
    }
  }

  return createPortal(
    <div className="modal-overlay t3code-platform-order-overlay">
      <div
        ref={dialogRef}
        className="modal t3code-platform-order-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={hintId}
        tabIndex={-1}
      >
        <div className="modal-header">
          <h2 id={titleId}>{t('t3code.platformOrder.title')}</h2>
          <button type="button" className="modal-close" onClick={onClose} disabled={saving} aria-label={t('common.close')}>
            <X aria-hidden="true" />
          </button>
        </div>
        <div className="modal-body">
          <p className="t3code-platform-order-hint" id={hintId}>{t('t3code.platformOrder.hint')}</p>
          <SortableLayoutList
            items={platforms.map((spec) => spec.driver)}
            className="t3code-platform-order-list"
            onSortingChange={setSorting}
            onReorder={(activeId, overId) => {
              const from = platforms.findIndex((spec) => spec.driver === activeId);
              const to = platforms.findIndex((spec) => spec.driver === overId);
              if (from >= 0 && to >= 0) {
                clear();
                setDraft(arrayMove([...platforms], from, to));
              }
            }}
          >
            {platforms.map((spec) => (
              <SortableLayoutItem key={spec.driver} id={spec.driver} className="t3code-platform-order-item">
                {(handle) => (
                  <>
                    <span className="t3code-platform-order-icon" aria-hidden="true">
                      {renderPlatformIcon(spec.platformId, 16)}
                    </span>
                    <span className="t3code-platform-order-name" title={spec.label}>{spec.label}</span>
                    <button
                      {...handle}
                      type="button"
                      className="btn btn-secondary icon-only t3code-platform-order-handle"
                      disabled={saving}
                      aria-label={`${t('platformLayout.dragHandleLabel')} · ${spec.label}`}
                    >
                      <GripVertical size={16} aria-hidden="true" />
                    </button>
                  </>
                )}
              </SortableLayoutItem>
            ))}
          </SortableLayoutList>
          <ModalErrorMessage message={message} position="bottom" scrollKey={scrollKey} />
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={save} disabled={saving || sorting}>
            {t(saving ? 'common.saving' : 'common.save')}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

export function T3codePlatformOrderButton({ compact = false, label }: { compact?: boolean; label?: string }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const title = t('t3code.platformOrder.title');

  return (
    <>
      <button
        type="button"
        className={`${compact ? 'qs-btn' : 'btn btn-secondary btn-sm'} t3code-platform-order-trigger`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={label ? `${label} · ${title}` : title}
        onClick={() => setOpen(true)}
      >
        <ListOrdered size={14} aria-hidden="true" />
        <span title={label ?? title}>{label ?? title}</span>
      </button>
      {open && <T3codePlatformOrderDialog onClose={() => setOpen(false)} />}
    </>
  );
}
