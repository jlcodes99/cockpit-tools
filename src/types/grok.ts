export interface GrokQuota {
  monthly_limit?: number | null;
  used?: number | null;
  remaining?: number | null;
  usage_percent?: number | null;
  remaining_percent?: number | null;
  on_demand_cap?: number | null;
  on_demand_used?: number | null;
  prepaid_balance?: number | null;
  billing_period_start?: string | null;
  billing_period_end?: string | null;
  unlimited_or_free?: boolean | null;
  exhausted?: boolean | null;
  exhaust_reason?: string | null;
}

export interface GrokAccount {
  id: string;
  email: string;
  name?: string | null;
  first_name?: string | null;
  last_name?: string | null;
  user_id?: string | null;
  principal_id?: string | null;
  team_id?: string | null;
  profile_image_asset_id?: string | null;
  tier?: number | null;
  plan_type?: string | null;
  plan_label?: string | null;
  access_token: string;
  refresh_token?: string | null;
  scope?: string | null;
  expires_at?: number | null;
  expires_at_raw?: string | null;
  oidc_issuer?: string | null;
  oidc_client_id?: string | null;
  auth_entry_key?: string | null;
  auth_mode_raw?: string | null;
  create_time?: string | null;
  coding_data_retention_opt_out?: boolean | null;
  has_grok_code_access?: boolean | null;
  quota?: GrokQuota | null;
  usage_updated_at?: number | null;
  token_updated_at?: number | null;
  status?: string | null;
  status_reason?: string | null;
  requires_reauth?: boolean | null;
  reauth_reason?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  subscription_query_last_success_at?: number | null;
  tags?: string[] | null;
  account_note?: string | null;
  created_at: number;
  last_used: number;
}

export interface GrokOAuthStartResponse {
  loginId: string;
  verificationUri: string;
  userCode?: string | null;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
}

export function getGrokAccountDisplayEmail(account: GrokAccount): string {
  const email = account.email?.trim();
  if (email) return email;
  const name = account.name?.trim();
  if (name) return name;
  const first = account.first_name?.trim() || '';
  const last = account.last_name?.trim() || '';
  const combined = `${first} ${last}`.trim();
  if (combined) return combined;
  return account.id;
}

export function getGrokPlanBadge(account: GrokAccount): string {
  const raw = (account.plan_type || account.plan_label || '').trim().toUpperCase();
  if (!raw) {
    if (account.tier === 5) return 'HEAVY';
    if (account.tier != null && account.tier >= 3) return 'SUPERGROK';
    if (account.tier != null && account.tier <= 1) return 'FREE';
    return 'UNKNOWN';
  }
  if (raw.includes('HEAVY')) return 'HEAVY';
  if (raw.includes('LITE')) return 'LITE';
  if (raw.includes('SUPER')) return 'SUPERGROK';
  if (raw.includes('FREE')) return 'FREE';
  if (raw.includes('ENTERPRISE')) return 'ENTERPRISE';
  return raw.slice(0, 12);
}

export function getGrokPlanBadgeClass(account: GrokAccount): string {
  const badge = getGrokPlanBadge(account);
  if (badge === 'HEAVY' || badge === 'ENTERPRISE') return 'ultra';
  if (badge === 'SUPERGROK' || badge === 'LITE') return 'pro';
  if (badge === 'FREE') return 'free';
  return 'unknown';
}

export function getGrokUsage(account: GrokAccount): {
  totalPercentUsed: number | null;
  remainingPercent: number | null;
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt: number | null;
} {
  const q = account.quota;
  const used = q?.usage_percent ?? null;
  const remaining =
    q?.remaining_percent ??
    (typeof used === 'number' ? Math.max(0, 100 - used) : null);
  let resetAt: number | null = null;
  if (q?.billing_period_end) {
    const parsed = Date.parse(q.billing_period_end);
    if (!Number.isNaN(parsed)) resetAt = Math.floor(parsed / 1000);
  }
  return {
    totalPercentUsed: used,
    remainingPercent: remaining,
    inlineSuggestionsUsedPercent: used,
    chatMessagesUsedPercent: used,
    allowanceResetAt: resetAt,
  };
}

export function hasGrokQuotaData(account: GrokAccount): boolean {
  return account.quota != null;
}

export function formatGrokQuotaSummary(account: GrokAccount): string {
  const q = account.quota;
  if (!q) return '--';
  if (q.unlimited_or_free) return 'Unlimited / Free';
  if (typeof q.used === 'number' && typeof q.monthly_limit === 'number' && q.monthly_limit > 0) {
    const pct =
      typeof q.usage_percent === 'number'
        ? q.usage_percent
        : (q.used / q.monthly_limit) * 100;
    return `${pct.toFixed(0)}% · ${Math.round(q.used)} / ${Math.round(q.monthly_limit)}`;
  }
  return '--';
}
