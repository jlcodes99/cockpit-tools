import { useEffect, type RefObject } from 'react';
import { registerModal, topModal, unregisterModal } from './modalStack';

function topFocusTrap(exclude?: HTMLElement | null, closingNode?: HTMLElement | null) {
  return topModal(window, { focusOnly: true, exclude, closingNode });
}

/**
 * Keep Tab inside an open dialog and restore the previous focus on close.
 * ESC handling and overlay behaviour stay with each dialog's own markup.
 */
export function useModalFocusTrap(
  dialog: RefObject<HTMLElement | null>,
  open: boolean,
  restoreFocus = true,
  active = true,
): void {
  useEffect(() => {
    if (!open) return;
    const node = dialog.current;
    const previous = document.activeElement;
    return () => {
      const remaining = topFocusTrap(node);
      // React clears refs and detaches nodes before effect cleanup. Retain this node
      // for its own closing priority, while ignoring other detached registry entries.
      const closingWasTop = topFocusTrap(undefined, node)?.node() === node;
      if (restoreFocus && previous instanceof HTMLElement && previous.isConnected
        && (!remaining || remaining.node()?.contains(previous))) previous.focus();
      else if (restoreFocus && closingWasTop && remaining) remaining.node()?.focus();
    };
  }, [dialog, open, restoreFocus]);

  // Suspension must keep the original return target when a child dialog opens.
  useEffect(() => {
    if (!open || !active) return;
    const node = dialog.current;
    if (!node) return;
    const entry = registerModal({ node: () => node, focus: true });
    if (topFocusTrap() === entry && !node.contains(document.activeElement)) node.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab' || topFocusTrap() !== entry) return;
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
    // Clicking a backdrop can move focus to body in WebKit. Recover the next Tab
    // before browser navigation reaches background controls, without intercepting
    // focus/keys in portaled menus or another registered dialog.
    const onDocumentKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab' || topFocusTrap() !== entry) return;
      const focused = document.activeElement;
      if (focused && focused !== document.body && focused !== document.documentElement) return;
      const controls = node.querySelectorAll<HTMLElement>('button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]');
      event.preventDefault();
      (event.shiftKey ? controls[controls.length - 1] : controls[0])?.focus();
      if (!controls.length) node.focus();
    };
    node.addEventListener('keydown', onKeyDown);
    document.addEventListener('keydown', onDocumentKeyDown, true);
    return () => {
      document.removeEventListener('keydown', onDocumentKeyDown, true);
      node.removeEventListener('keydown', onKeyDown);
      unregisterModal(entry);
    };
  }, [dialog, open, active]);
}
