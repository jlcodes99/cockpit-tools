import { useEffect, useRef } from 'react';

/** Initialize on open or target change; refreshed objects belong to the same editing session. */
export function useDialogSession(sessionKey: string | null, initialize: () => void) {
  const initialized = useRef<string | null>(null);
  useEffect(() => {
    if (sessionKey === null) {
      initialized.current = null;
      return;
    }
    if (initialized.current === sessionKey) return;
    initialized.current = sessionKey;
    initialize();
  }, [sessionKey, initialize]);
}
