import { useEffect, useRef, useState } from 'react';
import type { CodexAccount } from '../types/codex';
import type { CodexQuotaUpdate } from '../utils/codexQuotaSync';
import { CURRENT_ACCOUNT_CHANGED_EVENT, type AccountSyncEventPayload } from '../utils/accountSyncEvents';
import { subscribeTauriEvent } from '../utils/tauriEventListener';
import { queryCodexLocalAccessAccountWindowStats } from '../services/codexLocalAccessService';
import { buildCodexAccountWindowStatsScope, createCodexAccountWindowStatsReader, EMPTY_CODEX_ACCOUNT_WINDOW_STATS } from '../utils/codexAccountWindowStats';

/** Window statistics follow semantic quota windows; background request activity refreshes them in bounded batches. */
export function useCodexAccountWindowStats(active: boolean, accounts: readonly CodexAccount[], memberIds: readonly string[]) {
  const inputs = useRef({ active, accounts, memberIds });
  inputs.current = { active, accounts, memberIds };
  const [snapshot, setSnapshot] = useState(EMPTY_CODEX_ACCOUNT_WINDOW_STATS);
  const readerRef = useRef<ReturnType<typeof createCodexAccountWindowStatsReader> | null>(null);
  const scope = active ? buildCodexAccountWindowStatsScope(accounts, memberIds, Math.floor(Date.now() / 1000)) : null;

  useEffect(() => {
    const reader = createCodexAccountWindowStatsReader({
      read: queryCodexLocalAccessAccountWindowStats,
      canRead: () => inputs.current.active && document.visibilityState !== 'hidden',
      onChange: setSnapshot,
      onError: (error) => console.warn('[Codex Window Stats] background refresh failed:', error),
    });
    readerRef.current = reader;
    return () => { reader.dispose(); readerRef.current = null; };
  }, []);

  useEffect(() => {
    const reader = readerRef.current;
    if (!active || !reader) return;
    let disposed = false;
    const refresh = () => {
      if (disposed || document.visibilityState === 'hidden') return;
      const latest = inputs.current;
      reader.setScope(buildCodexAccountWindowStatsScope(latest.accounts, latest.memberIds, Math.floor(Date.now() / 1000)));
      reader.refresh();
    };
    refresh();
    // Some successful requests do not carry quota headers, so events alone are insufficient.
    const timer = window.setInterval(refresh, 30_000);
    const stopQuota = subscribeTauriEvent<CodexQuotaUpdate>('codex:quota-updated', ({ payload }) => {
      if (inputs.current.memberIds.includes(payload.account_id)) refresh();
    });
    const stopSwitch = subscribeTauriEvent<AccountSyncEventPayload>(CURRENT_ACCOUNT_CHANGED_EVENT, ({ payload }) => {
      if (payload?.platformId === 'codex') refresh();
    });
    const stopService = subscribeTauriEvent('codex-local-access-state-updated', refresh);
    window.addEventListener('codex-local-access-state-updated', refresh);
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => {
      disposed = true;
      // Keep the physical worker occupied until its native read actually ends,
      // even if the user leaves and re-enters the overview meanwhile.
      reader.setScope(null);
      window.clearInterval(timer);
      stopQuota(); stopSwitch(); stopService();
      window.removeEventListener('codex-local-access-state-updated', refresh);
      window.removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refresh);
    };
  }, [active]);

  useEffect(() => { readerRef.current?.setScope(scope); }, [active, scope?.key]);
  // Render-time gating prevents even one frame of another window's statistics after a scope change.
  return active && snapshot.scope === scope?.key ? snapshot : EMPTY_CODEX_ACCOUNT_WINDOW_STATS;
}
