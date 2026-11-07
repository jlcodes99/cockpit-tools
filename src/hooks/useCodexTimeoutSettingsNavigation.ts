import { useCallback, useRef, useState } from 'react';
import type { RequestTimeoutField } from '../utils/codexRequestDiagnostics';

/** Reuse the existing settings dialog and focus exactly once per opening. */
export function useCodexTimeoutSettingsNavigation(initialize: () => void, show: () => void) {
  const [field, setField] = useState<RequestTimeoutField | null>(null);
  const needsFocus = useRef(false);
  const open = (target: RequestTimeoutField | null = null) => {
    initialize(); setField(target); needsFocus.current = target != null; show();
  };
  const fieldRef = useCallback((node: HTMLInputElement | null) => {
    if (!node || !needsFocus.current) return;
    needsFocus.current = false;
    node.focus({ preventScroll: true });
    node.scrollIntoView({ block: 'center' });
  }, [field]);
  return { openTimeoutSettings: open, timeoutFocusField: field, timeoutFieldRef: fieldRef };
}
