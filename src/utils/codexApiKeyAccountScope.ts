import type { CodexAccount } from "../types/codex.ts";
import { isCodexLocalAccessEligibleAccount } from "./codexLocalAccessAccounts.ts";

interface SelectCodexApiKeyScopeAccountsOptions<T extends CodexAccount> {
  accounts: T[];
  scopedAccountIds: string[];
}

interface ReconcileCodexApiKeyScopeAccountIdsOptions<
  T extends CodexAccount,
> {
  accounts: T[];
  persistedAccountIds: string[];
  draftAccountIds: string[];
}

interface IsCodexApiKeyScopeAccountActiveOptions {
  accountId: string;
  inheritAccountPool: boolean;
  accountIds: string[];
  inheritedAccountIds: string[];
}

export function selectCodexApiKeyScopeAccounts<T extends CodexAccount>({
  accounts,
  scopedAccountIds,
}: SelectCodexApiKeyScopeAccountsOptions<T>): T[] {
  const scopedIds = new Set(scopedAccountIds);
  const seenIds = new Set<string>();

  return accounts.filter((account) => {
    if (!account.id || seenIds.has(account.id)) return false;
    seenIds.add(account.id);

    if (scopedIds.has(account.id)) return true;
    return isCodexLocalAccessEligibleAccount(account);
  });
}

export function reconcileCodexApiKeyScopeAccountIds<T extends CodexAccount>({
  accounts,
  persistedAccountIds,
  draftAccountIds,
}: ReconcileCodexApiKeyScopeAccountIdsOptions<T>): string[] {
  const selectableAccountIds = new Set(
    selectCodexApiKeyScopeAccounts({
      accounts,
      scopedAccountIds: persistedAccountIds,
    }).map((account) => account.id),
  );
  const loadedAccountIds = new Set(accounts.map((account) => account.id));
  const persistedAccountIdSet = new Set(persistedAccountIds);
  const seenAccountIds = new Set<string>();

  return draftAccountIds.filter((accountId) => {
    if (!accountId || seenAccountIds.has(accountId)) return false;
    seenAccountIds.add(accountId);
    return (
      selectableAccountIds.has(accountId) ||
      (persistedAccountIdSet.has(accountId) && !loadedAccountIds.has(accountId))
    );
  });
}

export function isCodexApiKeyScopeAccountActive({
  accountId,
  inheritAccountPool,
  accountIds,
  inheritedAccountIds,
}: IsCodexApiKeyScopeAccountActiveOptions): boolean {
  return inheritAccountPool
    ? inheritedAccountIds.includes(accountId)
    : accountIds.includes(accountId);
}
