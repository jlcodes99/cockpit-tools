import { useEffect, useRef } from 'react';

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

const topmostCloseHandlers: (() => void)[] = [];

function closeTopmost(event: KeyboardEvent) {
  const close = topmostCloseHandlers[topmostCloseHandlers.length - 1];
  if (event.key !== 'Escape' || !close) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  close();
}

/** One capture listener closes only the most recently opened, still-mounted dialog. */
export function useEscCloseTopmost(isOpen: boolean, onClose: () => void) {
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!isOpen) return;
    const close = () => onCloseRef.current();
    topmostCloseHandlers.push(close);
    if (topmostCloseHandlers.length === 1) window.addEventListener('keydown', closeTopmost, true);
    return () => {
      const index = topmostCloseHandlers.indexOf(close);
      if (index !== -1) topmostCloseHandlers.splice(index, 1);
      if (!topmostCloseHandlers.length) window.removeEventListener('keydown', closeTopmost, true);
    };
  }, [isOpen]);
}
