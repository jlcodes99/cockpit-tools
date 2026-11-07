import { useEffect, useRef, type RefObject } from 'react';
import { modalCloseHandler, registerModal, topModal, unregisterModal } from './modalStack';

/** ESC-to-close for modals without an explicit stacking policy. */
export function useEscClose(isOpen: boolean, onClose: () => void) {
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onCloseRef.current();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen]);
}

let topmostCloseHandlerCount = 0;

function closeTopmost(event: KeyboardEvent) {
  if (event.key !== 'Escape') return;
  const top = topModal(window);
  if (!top) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  modalCloseHandler(top)?.();
}

/** Close only the visible top layer. Callers without a ref retain legacy LIFO order. */
export function useEscCloseTopmost(
  isOpen: boolean,
  onClose: () => void,
  dialog?: RefObject<HTMLElement | null>,
) {
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!isOpen) return;
    const entry = registerModal({ node: () => dialog?.current ?? null,
      withoutNode: !dialog, close: () => onCloseRef.current() });
    if (++topmostCloseHandlerCount === 1) window.addEventListener('keydown', closeTopmost, true);
    return () => {
      unregisterModal(entry);
      if (--topmostCloseHandlerCount === 0) window.removeEventListener('keydown', closeTopmost, true);
    };
  }, [isOpen, dialog]);
}
