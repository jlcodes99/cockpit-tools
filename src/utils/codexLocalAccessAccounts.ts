import {
  isCodexExplicitFreePlanType,
  type CodexAccount,
} from '../types/codex';

export function isCodexLocalAccessEligibleAccount(
  account: CodexAccount,
  restrictFreeAccounts: boolean,
): boolean {
  if (restrictFreeAccounts && isCodexExplicitFreePlanType(account.plan_type)) {
    return false;
  }
  return true;
}

export function filterCodexLocalAccessAccountIds(
  accountIds: string[],
  accounts: CodexAccount[],
  restrictFreeAccounts: boolean,
): string[] {
  const accountById = new Map(accounts.map((account) => [account.id, account]));
  const seen = new Set<string>();
  const next: string[] = [];

  for (const accountId of accountIds) {
    const account = accountById.get(accountId);
    if (!account || !isCodexLocalAccessEligibleAccount(account, restrictFreeAccounts)) {
      continue;
    }
    if (!seen.has(accountId)) {
      seen.add(accountId);
      next.push(accountId);
    }
  }

  return next;
}
