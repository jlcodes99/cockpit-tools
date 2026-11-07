import type { CodexAccount, CodexQuota } from '../types/codex.ts';

/** Quota-only event: credentials and conversation content never cross this channel. */
export interface CodexQuotaUpdate {
  account_id: string;
  remote_account_id?: string | null;
  organization_id?: string | null;
  token_generation: number;
  quota: CodexQuota;
  plan_type?: string | null;
  usage_updated_at: number;
  sequence: number;
}

export function mergeCodexQuotaUpdate(account: CodexAccount, update?: CodexQuotaUpdate): CodexAccount {
  if (!update || account.id !== update.account_id ||
      (account.account_id ?? null) !== (update.remote_account_id ?? null) ||
      (account.organization_id ?? null) !== (update.organization_id ?? null) ||
      (account.token_generation ?? 0) > update.token_generation ||
      (account.usage_updated_at ?? 0) > update.usage_updated_at) return account;
  return {
    ...account, quota: update.quota, quota_error: undefined,
    usage_updated_at: update.usage_updated_at,
    plan_type: update.plan_type ?? account.plan_type,
  };
}
