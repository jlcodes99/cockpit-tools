import { useCallback, useEffect, useRef, type RefObject } from 'react';

/** WebKit mouse clicks and an inert parent do not reliably preserve activeElement. */
export function useLaunchPreviewChildReturnFocus(dialog: RefObject<HTMLElement | null>, childOpen: boolean) {
  const opener = useRef<HTMLElement | null>(null);
  const previousOpen = useRef(childOpen);
  const remember = useCallback((event?: { currentTarget: HTMLElement }) => {
    if (event?.currentTarget) opener.current = event.currentTarget;
  }, []);
  // Called after the dialog traps so their cleanup/resume cannot overwrite this return target.
  useEffect(() => {
    const closed = previousOpen.current && !childOpen;
    previousOpen.current = childOpen;
    if (!closed) return;
    const target = opener.current;
    opener.current = null;
    if (!target?.isConnected || !dialog.current?.contains(target) ||
        (target as HTMLButtonElement).disabled) return;
    const active = document.activeElement;
    const foreground = active instanceof HTMLElement ? active.closest('[aria-modal="true"]') : null;
    // A separate foreground dialog (for example a queued wakeup confirmation) keeps ownership.
    if (foreground?.isConnected && foreground !== dialog.current) return;
    target.focus({ preventScroll: true });
  }, [childOpen, dialog]);
  return remember;
}
