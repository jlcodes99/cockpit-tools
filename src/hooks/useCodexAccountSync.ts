import { useEffect, useRef } from 'react';
import { ACCOUNTS_CHANGED_EVENT, CURRENT_ACCOUNT_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

/** Stable native subscriptions always read current actions. */
export function useCodexAccountSync(actions: {
  refreshApiKeyUsageByAccountId: (accountId: string, options: { force: boolean }) => Promise<void>;
}) {
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    let disposed = false;
    const handle = (event: { payload: AccountSyncEventPayload }) => {
      const payload = event.payload;
      if (disposed || payload?.platformId !== 'codex' || payload.reason === 'delete'
          || payload.reason?.startsWith('client-auth-') || !payload.accountId) return;
      void latest.current.refreshApiKeyUsageByAccountId(payload.accountId, { force: false }).catch((error) => {
        if (!disposed) console.warn('[Codex Account Sync] background refresh failed:', error);
      });
    };
    const stopAccounts = subscribeTauriEvent<AccountSyncEventPayload>(ACCOUNTS_CHANGED_EVENT, handle);
    const stopCurrent = subscribeTauriEvent<AccountSyncEventPayload>(CURRENT_ACCOUNT_CHANGED_EVENT, handle);
    return () => { disposed = true; stopAccounts(); stopCurrent(); };
  }, []);
}
