import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { loadAccountRefreshOverrides } from '../utils/currentAccountRefresh';
import type { CodexQuotaUpdate } from '../utils/codexQuotaSync';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

export function useCodexQuotaSync() {
  useEffect(() => {
    const unsubscribe = subscribeTauriEvent<CodexQuotaUpdate>('codex:quota-updated', (event) => {
      useCodexAccountStore.getState().applyQuotaUpdate(event.payload);
    });
    let disposed = false;
    let inFlight = false;
    let queued = false;
    const syncOverrides = async () => {
      if (disposed) return;
      if (inFlight) { queued = true; return; }
      inFlight = true;
      const emails = Object.entries(loadAccountRefreshOverrides()?.codex ?? {})
        .filter(([, minutes]) => minutes === -1).map(([email]) => email);
      try {
        await invoke('set_codex_quota_sync_disabled_emails', { emails });
      } catch (error) {
        console.warn('[Codex Quota Sync] settings sync failed:', error);
      } finally {
        inFlight = false;
        if (queued && !disposed) { queued = false; void syncOverrides(); }
      }
    };
    void syncOverrides();
    window.addEventListener('config-updated', syncOverrides);
    window.addEventListener('storage', syncOverrides);
    return () => {
      disposed = true;
      unsubscribe();
      window.removeEventListener('config-updated', syncOverrides);
      window.removeEventListener('storage', syncOverrides);
    };
  }, []);
}
