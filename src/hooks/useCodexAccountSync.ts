import { useEffect, useRef } from 'react';
import { ACCOUNTS_CHANGED_EVENT, CURRENT_ACCOUNT_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

/** Stable native subscriptions always read current actions, without leaking listeners during rapid renders. */
export function useCodexAccountSync(actions: {
  fetchAccounts: () => Promise<void>;
  fetchCurrentAccount: () => Promise<void>;
  refreshApiKeyUsageByAccountId: (accountId: string, options: { force: boolean }) => Promise<void>;
}) {
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    let disposed = false;
    let reloadRunning = false;
    let reloadPending = false;
    let reloadTimer: ReturnType<typeof setTimeout> | undefined;
    const hidden = () => typeof document !== 'undefined' && document.visibilityState === 'hidden';
    const scheduleReload = () => {
      if (disposed || hidden() || reloadRunning || reloadTimer !== undefined || !reloadPending) return;
      reloadTimer = setTimeout(() => { reloadTimer = undefined; void reload(); }, 250);
    };
    async function reload() {
      if (disposed || hidden() || reloadRunning || !reloadPending) return;
      reloadRunning = true;
      reloadPending = false;
      try {
        const current = latest.current;
        // Both reads must settle before another authoritative reload can start.
        const results = await Promise.allSettled([current.fetchAccounts(), current.fetchCurrentAccount()]);
        for (const result of results) if (result.status === 'rejected' && !disposed) {
          console.warn('[Codex Account Sync] authoritative reload failed:', result.reason);
        }
      } finally {
        reloadRunning = false;
        scheduleReload();
      }
    }
    const onEvent = async (event: { payload: AccountSyncEventPayload }, accountsChanged: boolean) => {
      const payload = event.payload;
      if (disposed || payload?.platformId !== 'codex' || payload.reason === 'delete') return;
      const current = latest.current;
      if (accountsChanged && (payload.reason === 'client-auth-observation' || payload.reason === 'client-auth-launch')) {
        reloadPending = true;
        scheduleReload();
      } else if (payload.accountId) {
        await current.refreshApiKeyUsageByAccountId(payload.accountId, { force: false });
      }
    };
    const handle = (event: { payload: AccountSyncEventPayload }, accountsChanged: boolean) => {
      void onEvent(event, accountsChanged).catch((error) => {
        if (!disposed) console.warn('[Codex Account Sync] background refresh failed:', error);
      });
    };
    const stopAccounts = subscribeTauriEvent<AccountSyncEventPayload>(ACCOUNTS_CHANGED_EVENT, (event) => handle(event, true));
    const stopCurrent = subscribeTauriEvent<AccountSyncEventPayload>(CURRENT_ACCOUNT_CHANGED_EVENT, (event) => handle(event, false));
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', scheduleReload);
    return () => {
      disposed = true;
      reloadPending = false;
      if (reloadTimer !== undefined) clearTimeout(reloadTimer);
      if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', scheduleReload);
      stopAccounts(); stopCurrent();
    };
  }, []);
}
