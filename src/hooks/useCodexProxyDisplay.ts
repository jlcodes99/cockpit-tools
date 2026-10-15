import { useSyncExternalStore } from 'react';
import { CODEX_PROXY_DISPLAY_EVENT, CODEX_PROXY_DISPLAY_KEY, getCodexProxyDisplay } from '../utils/codexProxyDisplay';

function subscribe(listener: () => void) {
  const storage = (event: StorageEvent) => {
    if (event.key === CODEX_PROXY_DISPLAY_KEY || event.key === null) listener();
  };
  window.addEventListener(CODEX_PROXY_DISPLAY_EVENT, listener);
  window.addEventListener('storage', storage);
  return () => {
    window.removeEventListener(CODEX_PROXY_DISPLAY_EVENT, listener);
    window.removeEventListener('storage', storage);
  };
}

export function useCodexProxyDisplay() {
  return useSyncExternalStore(subscribe, getCodexProxyDisplay, () => 'compact' as const);
}
