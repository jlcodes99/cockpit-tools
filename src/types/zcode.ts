export interface ZcodeQuotaItem {
  name: string;
  total?: number | null;
  used?: number | null;
  remaining?: number | null;
  percent_used?: number | null;
  unit: string;
  window: string;
  period_end?: string | null;
}

export interface ZcodeAccount {
  id: string;
  name: string;
  provider?: string | null;
  email?: string | null;
  display_name?: string | null;
  plan_tier?: string | null;
  plan_expire?: string | null;
  quota_total?: number | null;
  quota_used?: number | null;
  quota_remaining?: number | null;
  quota_percent_used?: number | null;
  quota_items?: ZcodeQuotaItem[] | null;
  quota_source?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  usage_updated_at?: number | null;
  tags?: string[] | null;
  created_at: number;
  last_used: number;
}

type ProviderUsage = {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null;
  remainingCompletions?: number | null;
  totalCompletions?: number | null;
};

export function getZcodeAccountDisplayName(account: ZcodeAccount): string {
  return (
    account.display_name?.trim() ||
    account.name?.trim() ||
    account.email ||
    account.id
  );
}

export function getZcodePlanBadge(account: ZcodeAccount): string {
  const raw = account.plan_tier?.trim();
  return raw ? raw.toUpperCase() : 'UNKNOWN';
}

export function getZcodeUsage(account: ZcodeAccount): ProviderUsage {
  return {
    inlineSuggestionsUsedPercent: account.quota_percent_used ?? null,
    chatMessagesUsedPercent: null,
    remainingCompletions: account.quota_remaining ?? null,
    totalCompletions: account.quota_total ?? null,
  };
}

export function hasZcodeQuotaData(account: ZcodeAccount): boolean {
  return (
    account.quota_percent_used != null ||
    account.quota_total != null ||
    (account.quota_items?.length ?? 0) > 0
  );
}

export function formatZcodeQuotaCount(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return '--';
  const abs = Math.abs(value);
  if (abs >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (abs >= 10_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(Math.round(value * 100) / 100);
}
