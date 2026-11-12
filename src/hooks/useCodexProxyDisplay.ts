import { useSyncExternalStore } from 'react';
import { CODEX_PROXY_DISPLAY_EVENT, getCodexProxyDisplay } from '../utils/codexProxyDisplay';

function subscribe(listener: () => void) {
  window.addEventListener(CODEX_PROXY_DISPLAY_EVENT, listener);
  return () => window.removeEventListener(CODEX_PROXY_DISPLAY_EVENT, listener);
}

export function useCodexProxyDisplay() {
  return useSyncExternalStore(subscribe, getCodexProxyDisplay, () => 'compact' as const);
}
