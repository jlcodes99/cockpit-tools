import type { CodexAccount } from '../types/codex';
import { canUseCodexAccountProxy } from './codexAccountProxy';

/** A resource selection is never authority to bind an implicit or ineligible account. */
export function proxyAssignmentAccounts(accounts: CodexAccount[], selectedIds: readonly string[]): CodexAccount[] {
  const selected = new Set(selectedIds);
  return accounts.filter((account) => selected.has(account.id) && canUseCodexAccountProxy(account));
}
