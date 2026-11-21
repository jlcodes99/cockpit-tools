import { useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { isMainWindowNavigablePage, type Page } from '../types/navigation';
import { requestCodexRequestLogsNavigation } from '../utils/codexRequestLogsNavigation';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

export function useMainWindowNavigation(
  setPage: (page: Page) => void,
  applyStartupPreference: (preference?: string) => void,
) {
  const hasNavigated = useRef(false);
  const pendingNavigation = useRef<Promise<string | null> | null>(null);
  const navigate = useCallback((page: Page) => {
    hasNavigated.current = true;
    setPage(page);
  }, [setPage]);

  useEffect(() => {
    let disposed = false;
    const navigateTarget = (target: string) => {
      if (disposed) return;
      if (requestCodexRequestLogsNavigation(target)) {
        navigate('codex-api-service');
      } else if (isMainWindowNavigablePage(target)) {
        navigate(target);
      }
    };
    const unsubscribe = subscribeTauriEvent<string>('tray:navigate', ({ payload }) => {
      navigateTarget(String(payload || ''));
    });

    // The backend consumes the pending target on read. StrictMode can rerun
    // the effect, so both passes must observe the same request.
    pendingNavigation.current ??= invoke<string | null>('main_window_take_pending_navigation');
    void pendingNavigation.current.then((target) => {
      if (target && !hasNavigated.current) navigateTarget(target);
    }).catch(() => { /* Older backends do not expose pending navigation. */ });

    void invoke<{ startup_page?: string }>('get_general_config').then((config) => {
      if (!disposed && !hasNavigated.current) applyStartupPreference(config.startup_page);
    }).catch((error) => {
      if (!disposed) console.warn('Failed to apply startup page preference:', error);
    });

    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [applyStartupPreference, navigate]);

  return navigate;
}
