import { useEffect, useState } from 'react';
import type { CodexAccount } from '../types/codex';
import type { WorkbuddyAccount } from '../types/workbuddy';
import { listWorkbuddyAccounts } from '../services/workbuddyService';
import { ACCOUNTS_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { subscribeTauriEvent } from '../utils/tauriEventListener';
import { codexWorkbuddySourceId, workbuddyCreditDisplaySource } from '../utils/codexWorkbuddyCredits';

export function useCodexWorkbuddyCredits(accounts: readonly CodexAccount[]) {
  const [sources, setSources] = useState<WorkbuddyAccount[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [readFailed, setReadFailed] = useState(false);
  useEffect(() => {
    const sourceIds = new Set(accounts.map(codexWorkbuddySourceId).filter(Boolean));
    if (!sourceIds.size) return;
    let active = true;
    let request = 0;
    const read = async () => {
      if (!active) return;
      const current = ++request;
      try {
        // Local storage read only. Never rotate credentials just to render a card.
        const latest = await listWorkbuddyAccounts();
        if (!active || current !== request) return;
        // An authoritative empty list must remove deleted sources, not keep a cache.
        setSources(latest.filter(source => sourceIds.has(source.id)).map(workbuddyCreditDisplaySource));
        setReadFailed(false);
      } catch {
        if (active && current === request) setReadFailed(true);
      } finally {
        if (active && current === request) setLoaded(true);
      }
    };
    void read();
    const stop = subscribeTauriEvent<AccountSyncEventPayload>(ACCOUNTS_CHANGED_EVENT, ({ payload }) => {
      if (payload?.platformId === 'workbuddy') void read();
    });
    return () => { active = false; stop(); };
  }, [accounts]);
  return { sources, loaded, readFailed };
}
