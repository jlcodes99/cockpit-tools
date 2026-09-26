import type { CodexAccount } from '../types/codex';
import type { ProxyCatalog, ProxyCatalogSelections } from '../services/codexProxyCatalogService';
import type { CodexUnifiedProxyView } from '../services/codexUnifiedProxyService';
import { restoreProxySelection, savedRootProxySelections } from './codexProxySelection';
import type { BatchBindResult, BatchBindTarget, ProxyBindingValue } from './codexProxyBatch';

/** One account's exit draft: the catalog identity plus the explicit manual-group choices. */
export interface CodexProxyExitChoice {
  sourceId: string;
  itemId: string;
  groupId: string;
  selections: ProxyCatalogSelections;
}

export type CodexProxyExitFilter = 'all' | 'bound' | 'unbound';

/** Effective exit per account. `stale` means the saved catalog reference no longer resolves. */
export type CodexProxyExitMode = 'independent' | 'unified' | 'default' | 'stale';

export interface CodexProxyCatalogState {
  catalog: ProxyCatalog;
  loading: boolean;
  failed: boolean;
}

export const EMPTY_CODEX_PROXY_EXIT_CHOICE: CodexProxyExitChoice = { sourceId: '', itemId: '', groupId: '', selections: {} };

const exitModeKeys: Record<CodexProxyExitMode, string> = {
  independent: 'codex.proxy.modeIndependent',
  unified: 'codex.proxy.modeUnified',
  default: 'codex.proxy.modeDefault',
  stale: 'codex.proxy.modeStale',
};

export function codexProxyExitModeKey(mode: CodexProxyExitMode): string {
  return exitModeKeys[mode];
}

/** Account label used by the list, the tables and the batch failure rows. */
export type CodexProxyAccountIdentity = Pick<CodexAccount, 'id'> & Partial<Pick<CodexAccount, 'account_name' | 'email'>>;

export function codexProxyAccountName(account: CodexProxyAccountIdentity): string {
  return account.account_name?.trim() || account.email?.trim() || account.id;
}

/** Restore a saved binding into an editable draft. Missing resources never pick a replacement. */
export function restoreExitChoice(catalog: ProxyCatalog, binding: ProxyBindingValue): CodexProxyExitChoice {
  const restored = restoreProxySelection(catalog, binding);
  const source = catalog.sources.find((entry) => entry.id === restored.sourceId);
  return {
    sourceId: restored.sourceId,
    itemId: restored.itemId,
    groupId: restored.groupId,
    selections: source ? savedRootProxySelections(source, binding) : {},
  };
}

function sameSelections(current: ProxyCatalogSelections, next: ProxyCatalogSelections): boolean {
  const keys = Object.keys(current);
  if (keys.length !== Object.keys(next).length) return false;
  return keys.every((key) => current[key] === next[key]);
}

export function sameExitChoice(current: CodexProxyExitChoice, next: CodexProxyExitChoice): boolean {
  return current.sourceId === next.sourceId && current.itemId === next.itemId && current.groupId === next.groupId
    && sameSelections(current.selections, next.selections);
}

/** A prefilled source without a chosen node is browsing state, not a pending write. */
export function hasExitSelection(choice: CodexProxyExitChoice): boolean {
  return Boolean(choice.itemId);
}

export interface CodexProxyDraftState {
  bound: boolean;
  saved: boolean;
  dirty: boolean;
}

/** A saved binding alone never proves an account is usable: a catalog reference can be gone. */
export function exitDraftState(binding: ProxyBindingValue, restored: CodexProxyExitChoice, draft: CodexProxyExitChoice): CodexProxyDraftState {
  const bound = Boolean(binding);
  const saved = bound && sameExitChoice(draft, restored);
  return { bound, saved, dirty: !saved && hasExitSelection(draft) };
}

/**
 * Direct HTTP/SOCKS bindings cannot go stale, and a catalog that is still loading or failed
 * must never be reported as stale.
 */
export function resolveExitMode(binding: ProxyBindingValue, state: CodexProxyCatalogState, followingUnified: boolean): CodexProxyExitMode {
  if (!binding) return followingUnified ? 'unified' : 'default';
  if (!binding.sourceId || state.loading || state.failed) return 'independent';
  const owner = state.catalog.sources.find((entry) => entry.id === binding.sourceId);
  const resolvable = Boolean(owner)
    && (Boolean(owner?.nodes.some((node) => node.id === binding.itemId)) || Boolean(owner?.groups.some((group) => group.id === binding.itemId)));
  return resolvable ? 'independent' : 'stale';
}

/** The shared exit only exists as an applied binding on all eligible accounts. */
export function unifiedProxyActive(unified: CodexUnifiedProxyView | null | undefined): boolean {
  return unified?.mode === 'all_accounts' && Boolean(unified.binding);
}

/** Independent account bindings take precedence over the shared exit. */
export function unifiedFollowingIds(
  accounts: CodexAccount[],
  saved: (account: CodexAccount) => ProxyBindingValue,
  unified: CodexUnifiedProxyView | null | undefined,
): string[] {
  if (!unifiedProxyActive(unified)) return [];
  return accounts.filter((account) => !saved(account)).map((account) => account.id);
}

export function filterProxyAccounts(accounts: CodexAccount[], options: {
  search?: string;
  filter?: CodexProxyExitFilter;
  saved: (account: CodexAccount) => ProxyBindingValue;
}): CodexAccount[] {
  const query = (options.search ?? '').trim().toLowerCase();
  const filter = options.filter ?? 'all';
  return accounts.filter((account) => {
    if (query && !codexProxyAccountName(account).toLowerCase().includes(query)) return false;
    return filter === 'all' || Boolean(options.saved(account)) === (filter === 'bound');
  });
}

/** Batch unbinding only touches accounts that still carry a binding. */
export function batchUnbindTargets(
  accounts: CodexAccount[],
  saved: (account: CodexAccount) => ProxyBindingValue,
): BatchBindTarget[] {
  return accounts.flatMap((account) => saved(account) ? [{ account, willOverwrite: true }] : []);
}

export interface CodexProxyBatchFailure {
  accountId: string;
  name: string;
  errorKey: string;
}

export interface CodexProxyBatchSummary {
  total: number;
  success: number;
  failed: number;
  failures: CodexProxyBatchFailure[];
}

/** Every failure stays addressable by account so a partial batch result can be located in the list. */
export function summarizeProxyBatch(results: BatchBindResult[], accounts: CodexAccount[]): CodexProxyBatchSummary {
  const names = new Map(accounts.map((account) => [account.id, codexProxyAccountName(account)]));
  const failures = results.filter((entry) => !entry.ok).map((entry) => ({
    accountId: entry.accountId,
    name: names.get(entry.accountId) ?? entry.accountId,
    errorKey: entry.errorKey ?? 'codex.proxy.saveFailed',
  }));
  return { total: results.length, success: results.length - failures.length, failed: failures.length, failures };
}
