import { useEffect, type RefObject } from 'react';

/**
 * Keep Tab inside an open dialog and restore the previous focus on close.
 * ESC handling and overlay behaviour stay with each dialog's own markup.
 */
export function useModalFocusTrap(
  dialog: RefObject<HTMLElement | null>,
  open: boolean,
  restoreFocus = true,
): void {
  useEffect(() => {
    if (!open) return;
    const node = dialog.current;
    const previous = document.activeElement;
    node?.focus();
    if (!node) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return;
      const controls = node.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]');
      if (!controls.length) {
        event.preventDefault();
        return;
      }
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === node)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (document.activeElement === last || document.activeElement === node)) {
        event.preventDefault();
        first.focus();
      }
    };
    node.addEventListener('keydown', onKeyDown);
    return () => {
      node.removeEventListener('keydown', onKeyDown);
      if (restoreFocus && previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, [dialog, open, restoreFocus]);
}
