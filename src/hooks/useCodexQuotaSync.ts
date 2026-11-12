import { hydrateDurableUiSettings } from '../utils/durableUiSettings';
import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { CURRENT_ACCOUNT_REFRESH_PREFERENCES_CHANGED_EVENT, loadAccountRefreshOverrides } from '../utils/currentAccountRefresh';
import type { CodexQuotaUpdate } from '../utils/codexQuotaSync';
import type { CodexSubscriptionUpdate } from '../utils/codexSubscriptionSync';
import { ACCOUNTS_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

export function useCodexQuotaSync() {
  useEffect(() => {
    const unsubscribe = subscribeTauriEvent<CodexQuotaUpdate>('codex:quota-updated', (event) => {
      useCodexAccountStore.getState().applyQuotaUpdate(event.payload);
    });
    const stopSubscriptionUpdates = subscribeTauriEvent<CodexSubscriptionUpdate>(
      'codex:subscription-updated',
      ({ payload }) => useCodexAccountStore.getState().applySubscriptionUpdate(payload),
    );
    const stopImportUpdates = subscribeTauriEvent<AccountSyncEventPayload>(ACCOUNTS_CHANGED_EVENT, ({ payload }) => {
      if (payload?.platformId !== 'codex' || payload.reason !== 'import-quota-refresh') return;
      void useCodexAccountStore.getState().fetchAccounts();
    });
    let disposed = false;
    let inFlight = false;
    let queued = false;
    const syncOverrides = async () => {
      if (disposed) return;
      if (inFlight) { queued = true; return; }
      inFlight = true;
      try {
        await hydrateDurableUiSettings();
        if (disposed) return;
        const emails = Object.entries(loadAccountRefreshOverrides()?.codex ?? {})
          .filter(([, minutes]) => minutes === -1).map(([email]) => email);
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
    window.addEventListener(CURRENT_ACCOUNT_REFRESH_PREFERENCES_CHANGED_EVENT, syncOverrides);
    return () => {
      disposed = true;
      unsubscribe();
      stopSubscriptionUpdates();
      stopImportUpdates();
      window.removeEventListener('config-updated', syncOverrides);
      window.removeEventListener(CURRENT_ACCOUNT_REFRESH_PREFERENCES_CHANGED_EVENT, syncOverrides);
    };
  }, []);
}
