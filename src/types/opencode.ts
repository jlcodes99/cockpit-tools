export interface OpenCodeAccount {
  id: string;
  email: string;
  name?: string | null;
  tags?: string[] | null;

  access_token: string;

  tier: 'go' | 'zen' | 'free';
  plan_name?: string | null;
  plan_type?: string | null;
  subscription_status?: string | null;

  usage_raw?: unknown;

  status?: string | null;
  status_reason?: string | null;

  created_at: number;
  last_used: number;

  quota?: OpenCodeQuota;
}

export interface OpenCodeGoQuota {
  usage_5h_dollars: number;
  usage_weekly_dollars: number;
  usage_monthly_dollars: number;
  limit_5h: number;
  limit_weekly: number;
  limit_monthly: number;
  reset_times?: {
    reset_5h?: number | null;
    reset_weekly?: number | null;
    reset_monthly?: number | null;
  } | null;
}

export interface OpenCodeZenQuota {
  balance_dollars: number;
  auto_reload_enabled: boolean;
  monthly_spend_limit?: number | null;
}

export type OpenCodeQuota = OpenCodeGoQuota | OpenCodeZenQuota;

export type OpenCodeTier = 'go' | 'zen' | 'free';

export const OPENCODE_GO_BASE_URL = 'https://opencode.ai/zen/go/v1/';

export const OPENCODE_ZEN_BASE_URL = 'https://opencode.ai/zen/v1/';

export const OPENCODE_GO_MODELS: string[] = [
  'opencode-go/glm-5.1',
  'opencode-go/glm-5',
  'opencode-go/kimi-k2.5',
  'opencode-go/kimi-k2.6',
  'opencode-go/mimo-v2.5',
  'opencode-go/mimo-v2.5-pro',
  'opencode-go/deepseek-v4-pro',
  'opencode-go/deepseek-v4-flash',
  'opencode-go/qwen3.5-plus',
  'opencode-go/qwen3.6-plus',
  'opencode-go/minimax-m2.5',
  'opencode-go/minimax-m2.7',
];

export const OPENCODE_ZEN_MODELS: string[] = [
  'opencode/gpt-4o',
  'opencode/gpt-4o-mini',
  'opencode/gpt-4.1',
  'opencode/gpt-4.1-mini',
  'opencode/gpt-4.1-nano',
  'opencode/claude-sonnet-4',
  'opencode/claude-sonnet-4.5',
  'opencode/claude-haiku-4.5',
  'opencode/gemini-2.5-flash',
  'opencode/gemini-2.5-pro',
  'opencode/gemini-2.0-flash',
  'opencode/qwen3-plus',
  'opencode/qwen3-max',
  'opencode/glm-5',
  'opencode/glm-5.1',
  'opencode/kimi-k2.5',
  'opencode/kimi-k2.6',
  'opencode/mimo-v2.5',
  'opencode/mimo-v2.5-pro',
  'opencode/deepseek-v4-pro',
  'opencode/deepseek-v4-flash',
  'opencode/minimax-m2.5',
  'opencode/minimax-m2.7',
];

export const OPENCODE_FREE_MODELS: string[] = [
  'opencode/big-pickle',
  'opencode/deepseek-v4-flash-free',
  'opencode/minimax-m2.5-free',
  'opencode/ring-2.6-1t-free',
  'opencode/nemotron-3-super-free',
];

export function isOpenCodeGo(account: OpenCodeAccount): boolean {
  return account.tier === 'go';
}

export function isOpenCodeZen(account: OpenCodeAccount): boolean {
  return account.tier === 'zen';
}

export function isOpenCodeFree(account: OpenCodeAccount): boolean {
  return account.tier === 'free';
}

function resolveOpencodeTier(tier: string): 'go' | 'zen' | 'free' | 'unknown' {
  const lower = tier.trim().toLowerCase();
  if (lower === 'go') return 'go';
  if (lower === 'zen') return 'zen';
  if (lower === 'free') return 'free';
  return 'unknown';
}

export function getOpenCodePlanBadge(account: OpenCodeAccount): string {
  const tier = account.tier || '';
  const resolved = resolveOpencodeTier(tier);
  if (resolved === 'go') return 'GO';
  if (resolved === 'zen') return 'ZEN';
  if (resolved === 'free') return 'FREE';
  return 'UNKNOWN';
}

export function getOpenCodePlanDisplayName(account: OpenCodeAccount): string {
  return getOpenCodePlanBadge(account);
}

export function getOpenCodePlanBadgeClass(account: OpenCodeAccount): string {
  const tier = account.tier || '';
  const resolved = resolveOpencodeTier(tier);
  if (resolved === 'go') return 'go';
  if (resolved === 'zen') return 'zen';
  if (resolved === 'free') return 'free';
  return 'unknown';
}

export function getOpenCodeAccountDisplayEmail(account: OpenCodeAccount): string {
  const email = account.email?.trim();
  if (email) return email;
  const name = account.name?.trim();
  if (name) return name;
  return account.id;
}

export function getOpenCodeGoUsage(account: OpenCodeAccount): {
  usage5hDollars: number;
  usageWeeklyDollars: number;
  usageMonthlyDollars: number;
  limit5h: number;
  limitWeekly: number;
  limitMonthly: number;
  totalPercentUsed: number | null;
} {
  const raw = account.usage_raw;
  if (!raw || typeof raw !== 'object') {
    return {
      usage5hDollars: 0,
      usageWeeklyDollars: 0,
      usageMonthlyDollars: 0,
      limit5h: 12,
      limitWeekly: 30,
      limitMonthly: 60,
      totalPercentUsed: null,
    };
  }
  const data = raw as Record<string, unknown>;
  const usage5h = toNumber(data.usage_5h_dollars) ?? 0;
  const usageWeekly = toNumber(data.usage_weekly_dollars) ?? 0;
  const usageMonthly = toNumber(data.usage_monthly_dollars) ?? 0;
  const limit5h = toNumber(data.limit_5h) ?? 12;
  const limitWeekly = toNumber(data.limit_weekly) ?? 30;
  const limitMonthly = toNumber(data.limit_monthly) ?? 60;

  const monthlyPct = limitMonthly > 0 ? (usageMonthly / limitMonthly) * 100 : 0;

  return {
    usage5hDollars: usage5h,
    usageWeeklyDollars: usageWeekly,
    usageMonthlyDollars: usageMonthly,
    limit5h,
    limitWeekly,
    limitMonthly,
    totalPercentUsed: clampPercent(monthlyPct),
  };
}

export function getOpenCodeZenUsage(account: OpenCodeAccount): {
  balanceDollars: number;
  autoReloadEnabled: boolean;
  monthlySpendLimit: number | null;
} {
  const raw = account.usage_raw;
  if (!raw || typeof raw !== 'object') {
    return {
      balanceDollars: 0,
      autoReloadEnabled: false,
      monthlySpendLimit: null,
    };
  }
  const data = raw as Record<string, unknown>;
  return {
    balanceDollars: toNumber(data.balance_dollars) ?? 0,
    autoReloadEnabled: data.auto_reload_enabled === true,
    monthlySpendLimit: toNumber(data.monthly_spend_limit) ?? null,
  };
}

export function formatOpenCodeUsageDollars(value: number | null | undefined): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return '$0.00';
  }
  return `$${Math.max(value, 0).toFixed(2)}`;
}

function toNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number.parseFloat(value.trim());
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}
