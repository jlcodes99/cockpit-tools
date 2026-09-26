import type { CodexAccount } from '../types/codex';
import type { ProxyCatalogSelections } from '../services/codexProxyCatalogService';

export type ProxyBindingValue = CodexAccount['egress_proxy'];

export interface BatchBindTarget {
  account: CodexAccount;
  /** The account already has a different node and would lose it. */
  willOverwrite: boolean;
}

function isSameBinding(current: ProxyBindingValue, sourceId: string, itemId: string, groupId?: string, selections?: ProxyCatalogSelections): boolean {
  // The summary does not carry every nested manual choice or the source revision.
  // Rebinding an explicitly selected group must not be skipped by identity alone.
  if (selections && Object.keys(selections).length > 0) return false;
  return Boolean(current && current.sourceId === sourceId && current.itemId === itemId
    && (groupId === undefined || (current.groupId ?? '') === groupId));
}

/**
 * Batch binding overwrites the selected accounts by default; only accounts already on that
 * exact node with the same group context are skipped, avoiding unnecessary tunnel rebuilds.
 */
export function batchBindTargets(
  accounts: CodexAccount[],
  saved: (account: CodexAccount) => ProxyBindingValue,
  sourceId: string,
  itemId: string,
  groupId?: string,
  selections?: ProxyCatalogSelections,
): BatchBindTarget[] {
  const targets: BatchBindTarget[] = [];
  for (const account of accounts) {
    const current = saved(account);
    if (isSameBinding(current, sourceId, itemId, groupId, selections)) continue;
    targets.push({ account, willOverwrite: Boolean(current) });
  }
  return targets;
}

/** Counts for the confirmation summary: nothing is skipped silently. */
export function batchBindSkipped(
  accounts: CodexAccount[],
  saved: (account: CodexAccount) => ProxyBindingValue,
  sourceId: string,
  itemId: string,
  groupId?: string,
  selections?: ProxyCatalogSelections,
): { same: number } {
  let same = 0;
  for (const account of accounts) {
    if (isSameBinding(saved(account), sourceId, itemId, groupId, selections)) same += 1;
  }
  return { same };
}

export interface BatchBindResult {
  accountId: string;
  ok: boolean;
  /** Translation key only: backend messages are never shown. */
  errorKey?: string;
}

export function proxyBatchCompletedSuccessfully(results: BatchBindResult[], targetCount: number, cancelled: boolean): boolean {
  return !cancelled && targetCount > 0 && results.length === targetCount && results.every((entry) => entry.ok);
}

export function batchBindSummary(results: BatchBindResult[]): { total: number; success: number; failed: number } {
  const failed = results.filter((entry) => !entry.ok).length;
  return { total: results.length, success: results.length - failed, failed };
}

/** Cancellation stops future writes; an in-flight write must still publish its actual outcome. */
export async function executeProxyBatch(
  targets: BatchBindTarget[],
  options: {
    cancelled: () => boolean;
    bind: (account: CodexAccount) => Promise<CodexAccount>;
    applied: (account: CodexAccount) => void;
    errorKey: (error: unknown) => string;
    progress: (results: BatchBindResult[]) => void;
  },
): Promise<void> {
  const results: BatchBindResult[] = [];
  for (const target of targets) {
    if (options.cancelled()) break;
    let updated: CodexAccount;
    try {
      updated = await options.bind(target.account);
    } catch (error) {
      results.push({ accountId: target.account.id, ok: false, errorKey: options.errorKey(error) });
      options.progress([...results]);
      continue;
    }
    options.applied(updated);
    results.push({ accountId: target.account.id, ok: true });
    options.progress([...results]);
  }
}
