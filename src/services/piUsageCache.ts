import * as piService from './piService';
import type { PiProviderUsage } from './piService';

export interface PiUsageState {
  loading: boolean;
  data?: PiProviderUsage[];
  error?: string;
}

// Shared by cards/table rows, remounts and the auto-refresh scheduler.
const usageCache = new Map<string, PiUsageState>();
const listeners = new Map<string, Set<() => void>>();

function setState(accountId: string, state: PiUsageState) {
  usageCache.set(accountId, state);
  listeners.get(accountId)?.forEach((fn) => fn());
}

export function getPiUsageState(accountId: string): PiUsageState | undefined {
  return usageCache.get(accountId);
}

export function subscribePiUsage(accountId: string, fn: () => void): () => void {
  const set = listeners.get(accountId) ?? new Set();
  set.add(fn);
  listeners.set(accountId, set);
  return () => {
    set.delete(fn);
  };
}

/** Query usage for one account; cached unless `force`. Never throws. */
export async function loadPiAccountUsage(accountId: string, force = false): Promise<void> {
  const current = usageCache.get(accountId);
  if (current?.loading || (current?.data && !force)) return;
  setState(accountId, { ...current, loading: true });
  try {
    const data = await piService.queryPiAccountUsage(accountId);
    setState(accountId, { loading: false, data });
  } catch (error) {
    setState(accountId, { loading: false, error: String(error) });
  }
}

export async function refreshAllPiUsage(): Promise<void> {
  const accounts = await piService.listPiAccounts();
  await Promise.all(accounts.map((account) => loadPiAccountUsage(account.id, true)));
}
