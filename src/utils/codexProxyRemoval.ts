import type { ProxyCatalog } from '../services/codexProxyCatalogService';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';

export interface ProxyRemovalProgress {
  /** Retain a committed deletion when only the subsequent read failed. */
  catalog?: ProxyCatalog;
}

export class ProxyAccountRefreshError extends Error {}

export async function refreshCodexProxyAccounts(): Promise<void> {
  const store = useCodexAccountStore.getState();
  const results = await Promise.allSettled([
    store.fetchAccounts({ throwOnError: true }),
    store.fetchCurrentAccount({ throwOnError: true }),
  ]);
  const failed = results.find((result) => result.status === 'rejected');
  if (failed?.status === 'rejected') throw failed.reason;
}

/** Deletion can partially unbind accounts even when it rejects. Always reread. */
export async function removeProxySourceWithRefresh(
  progress: ProxyRemovalProgress,
  remove: () => Promise<ProxyCatalog>,
  refresh: () => Promise<void>,
): Promise<ProxyCatalog> {
  let removalFailed = false;
  let removalError: unknown;
  if (!progress.catalog) {
    try { progress.catalog = await remove(); }
    catch (error) { removalFailed = true; removalError = error; }
  }
  try { await refresh(); }
  catch { throw new ProxyAccountRefreshError('CATALOG_ACCOUNT_REFRESH'); }
  if (removalFailed) throw removalError;
  return progress.catalog!;
}

export function proxyRemovalErrorKey(error: unknown, fallback: (error: unknown) => string): string {
  return error instanceof ProxyAccountRefreshError
    ? 'codex.proxy.catalog.accountRefreshFailed'
    : fallback(error);
}
