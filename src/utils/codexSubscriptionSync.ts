import type { CodexAccount } from '../types/codex.ts';

export interface CodexSubscriptionUpdate {
  id: string;
  previous_account_id: string | null;
  account_id: string | null;
  organization_id: string | null;
  token_generation: number;
  plan_type: string | null;
  usage_updated_at: number | null;
  subscription_active_until: string | null;
  subscription_query_last_attempt_at: number;
  subscription_query_last_success_at: number | null;
  subscription_query_last_error: string | null;
}

export function mergeCodexSubscriptionUpdate(
  account: CodexAccount,
  update?: CodexSubscriptionUpdate,
): CodexAccount {
  if (!update || account.id !== update.id ||
      (account.organization_id ?? null) !== update.organization_id ||
      (account.token_generation ?? 0) !== update.token_generation ||
      ((account.account_id ?? null) !== update.previous_account_id &&
        (account.account_id ?? null) !== update.account_id) ||
      (account.subscription_query_last_attempt_at ?? 0) >= update.subscription_query_last_attempt_at) {
    return account;
  }
  return {
    ...account,
    account_id: update.account_id ?? undefined,
    plan_type: (account.usage_updated_at ?? 0) > (update.usage_updated_at ?? 0)
      ? account.plan_type : update.plan_type ?? undefined,
    subscription_active_until: update.subscription_active_until ?? undefined,
    subscription_query_last_attempt_at: update.subscription_query_last_attempt_at,
    subscription_query_last_success_at: update.subscription_query_last_success_at ?? undefined,
    subscription_query_next_retry_at: undefined,
    subscription_query_last_error: update.subscription_query_last_error ?? undefined,
  };
}
