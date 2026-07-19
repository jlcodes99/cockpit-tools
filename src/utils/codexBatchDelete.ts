interface CodexBatchDeleteAccountRef {
  id: string;
}

interface ResolveCodexBatchDeleteRefreshOptions<T extends CodexBatchDeleteAccountRef> {
  accounts: T[];
  currentAccount: T | null;
  removeIds: ReadonlySet<string>;
}

export interface CodexBatchDeleteRefreshOptions {
  allowEmptyAccounts: boolean;
  allowEmptyCurrent: boolean;
}

/**
 * Empty account/current-account responses are normally treated as transient and
 * ignored. A completed deletion is the exception when it removed every cached
 * account or the cached current account.
 */
export function resolveCodexBatchDeleteRefreshOptions<
  T extends CodexBatchDeleteAccountRef,
>({
  accounts,
  currentAccount,
  removeIds,
}: ResolveCodexBatchDeleteRefreshOptions<T>): CodexBatchDeleteRefreshOptions {
  return {
    allowEmptyAccounts:
      accounts.length > 0 &&
      accounts.every((account) => removeIds.has(account.id)),
    allowEmptyCurrent:
      currentAccount !== null && removeIds.has(currentAccount.id),
  };
}
