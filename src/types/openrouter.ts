export interface OpenRouterAccount {
  id: string;
  email: string;
  /** Display-only truncated API key label from /key response, or the last 4 chars */
  label?: string | null;
  /** Key type determined from /key response or user selection */
  key_type: 'api' | 'management' | 'provisioning';
  is_free_tier: boolean;
  /** Total USD spent */
  usage: number | null;
  /** Daily usage in USD */
  usage_daily: number | null;
  /** Weekly usage in USD */
  usage_weekly: number | null;
  /** Monthly usage in USD */
  usage_monthly: number | null;
  /** Credit limit in USD */
  limit: number | null;
  /** Remaining limit in USD */
  limit_remaining: number | null;
  /** Total purchased credits (management key only) */
  total_credits: number | null;
  /** Total usage from /credits (management key only) */
  total_usage: number | null;
  /** Rate limit requests per interval */
  rate_limit_requests: number | null;
  /** Rate limit interval (e.g., "1h") */
  rate_limit_interval: string | null;
  /** Label from /key response */
  key_label: string | null;
  /** Status of the key */
  status: string | null;
  /** Reason for status */
  status_reason: string | null;
  created_at: number;
  last_used: number;
  tags?: string[] | null;
  usage_updated_at?: number | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  /** Raw /key response for debugging */
  auth_key_raw?: unknown;
  /** Raw /credits response (management key only) */
  auth_credits_raw?: unknown;
}

export interface OpenRouterModel {
  id: string;
  name: string;
  pricing: {
    prompt: string;
    completion: string;
    image?: string;
    audio?: string;
    web_search?: string;
  };
  context_length: number;
  top_provider: string;
  /** Whether this is a free model (pricing is all "0") */
  is_free: boolean;
  supported_parameters: string[];
}

export interface OpenRouterUsage {
  used: number | null;
  limit: number | null;
  remaining: number | null;
  percentage: number | null;
  daily: number | null;
  weekly: number | null;
  monthly: number | null;
}

export interface OpenRouterCreditsInfo {
  total_credits: number | null;
  total_usage: number | null;
  total_paid: number | null;
}

/** Get display email for an OpenRouter account - uses label or first 8 chars of ID */
export function getOpenRouterAccountDisplayEmail(account: OpenRouterAccount): string {
  return account.label
    || account.key_label
    || account.email
    || account.id.slice(0, 8);
}

/** Get plan badge text: "FREE", "PAID", or the key type label */
export function getOpenRouterPlanBadge(account: OpenRouterAccount): string {
  if (account.is_free_tier) return 'FREE';
  if (account.key_type === 'management') return 'MANAGEMENT';
  if (account.key_type === 'provisioning') return 'PROVISIONING';
  return 'PAID';
}

export function getOpenRouterPlanBadgeClass(account: OpenRouterAccount): string {
  if (account.is_free_tier) return 'badge-free';
  if (account.key_type === 'management') return 'badge-management';
  if (account.key_type === 'provisioning') return 'badge-provisioning';
  return 'badge-paid';
}

export function isOpenRouterFreeTier(account: OpenRouterAccount): boolean {
  return account.is_free_tier;
}

export function isOpenRouterManagementKey(account: OpenRouterAccount): boolean {
  return account.key_type === 'management';
}

/** Format USD cents for display */
export function formatOpenRouterCredits(cents: number | null): string {
  if (cents == null) return '—';
  return `$${cents.toFixed(2)}`;
}

/** Calculate usage as percentage of limit */
export function getOpenRouterUsagePercent(account: OpenRouterAccount): number | null {
  if (account.usage == null || account.limit == null || account.limit <= 0) return null;
  const pct = (account.usage / account.limit) * 100;
  return Math.max(0, Math.min(100, pct));
}

/** Get all usage info for the account - compatible with ProviderUsage */
export function getOpenRouterUsage(account: OpenRouterAccount): {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  creditsUsed: number | null;
  creditsTotal: number | null;
  creditsRemaining: number | null;
} {
  const percentage = getOpenRouterUsagePercent(account);
  return {
    inlineSuggestionsUsedPercent: percentage != null ? Math.round(percentage) : null,
    chatMessagesUsedPercent: percentage != null ? Math.round(percentage) : null,
    creditsUsed: account.usage,
    creditsTotal: account.limit,
    creditsRemaining: account.limit_remaining,
  };
}

/** Get credits info (management key only) */
export function getOpenRouterCreditsInfo(
  account: OpenRouterAccount,
): OpenRouterCreditsInfo | null {
  if (account.key_type !== 'management') return null;
  return {
    total_credits: account.total_credits,
    total_usage: account.total_usage,
    total_paid:
      account.total_credits != null && account.total_usage != null
        ? account.total_credits - account.total_usage
        : null,
  };
}
