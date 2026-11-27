import { Account, QuotaData } from '../types/account';
import {
  AUTH_RECOMMENDED_LABELS,
  getAntigravityDisplayModelsFromQuota,
  getAntigravityModelDisplayName,
} from './antigravityModels';

export const DISPLAY_MODEL_ORDER = [
  ...AUTH_RECOMMENDED_LABELS.map((label) => ({ ids: [label], label })),
];

const MODEL_MATCH_REPLACEMENTS: Record<string, string> = {
  'gemini-3-pro-high': 'gemini-3.1-pro-high',
  'gemini-3-pro-low': 'gemini-3.1-pro-low',
  'claude-sonnet-4-5': 'claude-sonnet-4-6',
  'claude-sonnet-4-5-thinking': 'claude-sonnet-4-6',
  'claude-opus-4-5-thinking': 'claude-opus-4-6-thinking',
};

const normalizeModelForMatch = (value: string): string => {
  const normalized = value.trim().toLowerCase();
  if (!normalized) return '';
  return MODEL_MATCH_REPLACEMENTS[normalized] || normalized;
};

export function matchModelName(modelName: string, target: string): boolean {
  const left = normalizeModelForMatch(modelName);
  const right = normalizeModelForMatch(target);
  if (!left || !right) return false;
  return left === right || left.startsWith(`${right}-`) || right.startsWith(`${left}-`);
}

export type AntigravitySubscriptionTier = 'ULTRA' | 'PRO' | 'FREE' | 'UNKNOWN';

export interface AntigravityTierBadge {
  tier: AntigravitySubscriptionTier;
  label: string;
  className: string;
  rawTier?: string;
}

const ANTIGRAVITY_TIER_DEFINITIONS: Record<string, Omit<AntigravityTierBadge, 'rawTier'>> = {
  'g1-ultra-lite-tier': { tier: 'ULTRA', label: 'ULTRA LITE', className: 'ultra' },
  'g1-ultra-lite': { tier: 'ULTRA', label: 'ULTRA LITE', className: 'ultra' },
  'ultra-lite': { tier: 'ULTRA', label: 'ULTRA LITE', className: 'ultra' },
  'g1-ultra-tier': { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' },
  'g1-ultra': { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' },
  'gcp-ge-ultra-tier': { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' },
  'ws-ai-ultra-business-tier': { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' },
  'ultra-tier': { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' },
  ultra: { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' },
  'gcp-ge-edu-pro-tier': { tier: 'PRO', label: 'EDU PRO', className: 'pro' },
  'gcp-ge-edu-pro-emerging-tier': { tier: 'PRO', label: 'EDU PRO', className: 'pro' },
  'edu-pro': { tier: 'PRO', label: 'EDU PRO', className: 'pro' },
  'gdp-premium-tier': { tier: 'PRO', label: 'PREMIUM', className: 'pro' },
  'gdp-premium': { tier: 'PRO', label: 'PREMIUM', className: 'pro' },
  premium: { tier: 'PRO', label: 'PREMIUM', className: 'pro' },
  'g1-pro-tier': { tier: 'PRO', label: 'PRO', className: 'pro' },
  'g1-pro': { tier: 'PRO', label: 'PRO', className: 'pro' },
  'pro-tier': { tier: 'PRO', label: 'PRO', className: 'pro' },
  pro: { tier: 'PRO', label: 'PRO', className: 'pro' },
  'g1-plus-tier': { tier: 'PRO', label: 'PLUS', className: 'plus' },
  'g1-plus': { tier: 'PRO', label: 'PLUS', className: 'plus' },
  'gcp-ge-plus-tier': { tier: 'PRO', label: 'PLUS', className: 'plus' },
  'plus-tier': { tier: 'PRO', label: 'PLUS', className: 'plus' },
  plus: { tier: 'PRO', label: 'PLUS', className: 'plus' },
  'free-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  free: { tier: 'FREE', label: 'FREE', className: 'free' },
  'standard-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  standard: { tier: 'FREE', label: 'FREE', className: 'free' },
  'cs-standard-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  'gcp-standard-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  'gcp-ge-standard-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  'gcp-ge-standard-emerging-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  'legacy-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  legacy: { tier: 'FREE', label: 'FREE', className: 'free' },
  'unlicensed-tier': { tier: 'FREE', label: 'FREE', className: 'free' },
  unlicensed: { tier: 'FREE', label: 'FREE', className: 'free' },
  'gcp-enterprise-tier': { tier: 'PRO', label: 'ENTERPRISE', className: 'enterprise' },
  'enterprise-agent-tier': { tier: 'PRO', label: 'ENTERPRISE', className: 'enterprise' },
  'enterprise-tier': { tier: 'PRO', label: 'ENTERPRISE', className: 'enterprise' },
  enterprise: { tier: 'PRO', label: 'ENTERPRISE', className: 'enterprise' },
  'ge-business-tier': { tier: 'PRO', label: 'BUSINESS', className: 'business' },
  'business-tier': { tier: 'PRO', label: 'BUSINESS', className: 'business' },
  business: { tier: 'PRO', label: 'BUSINESS', className: 'business' },
  'ge-business-trial-tier': { tier: 'PRO', label: 'TRIAL', className: 'trial' },
  'trial-tier': { tier: 'PRO', label: 'TRIAL', className: 'trial' },
  trial: { tier: 'PRO', label: 'TRIAL', className: 'trial' },
  'agy-business-paygo-tier': { tier: 'PRO', label: 'PAYGO', className: 'business' },
  'gcp-ge-paygo-tier': { tier: 'PRO', label: 'PAYGO', className: 'business' },
  'paygo-tier': { tier: 'PRO', label: 'PAYGO', className: 'business' },
  paygo: { tier: 'PRO', label: 'PAYGO', className: 'business' },
  'gcp-ge-edu-tier': { tier: 'PRO', label: 'EDU', className: 'individual' },
  'gcp-ge-edu-emerging-tier': { tier: 'PRO', label: 'EDU', className: 'individual' },
  'edu-tier': { tier: 'PRO', label: 'EDU', className: 'individual' },
  edu: { tier: 'PRO', label: 'EDU', className: 'individual' },
  'gcp-ge-frontline-worker-tier': { tier: 'PRO', label: 'FRONTLINE', className: 'business' },
  'gcp-ge-frontline-starter-tier': { tier: 'PRO', label: 'FRONTLINE', className: 'business' },
  'googler-tier': { tier: 'PRO', label: 'GOOGLER', className: 'pro' },
  'googler-dogfood-tier': { tier: 'PRO', label: 'GOOGLER', className: 'pro' },
  'aida-tier': { tier: 'FREE', label: 'AIDA', className: 'free' },
};

const ANTIGRAVITY_TIER_FALLBACK_PATTERNS: Array<[RegExp, Omit<AntigravityTierBadge, 'rawTier'>]> = [
  [/^ultra-lite$|(?:^|-)ultra-lite(?:-|$)/, { tier: 'ULTRA', label: 'ULTRA LITE', className: 'ultra' }],
  [/(?:^|-)ultra(?:-|$)/, { tier: 'ULTRA', label: 'ULTRA', className: 'ultra' }],
  [/(?:^|-)pro(?:-|$)/, { tier: 'PRO', label: 'PRO', className: 'pro' }],
  [/(?:^|-)plus(?:-|$)/, { tier: 'PRO', label: 'PLUS', className: 'plus' }],
  [/(?:^|-)(?:free|standard|legacy|unlicensed)(?:-|$)/, { tier: 'FREE', label: 'FREE', className: 'free' }],
];

function getKnownSubscriptionTier(quota?: Pick<QuotaData, 'subscription_tier'>): string | undefined {
  const rawTier = quota?.subscription_tier?.trim();
  return rawTier && rawTier.toLowerCase() !== 'unknown' ? rawTier : undefined;
}

export function resolveAntigravityTierBadge(
  quota?: Pick<QuotaData, 'subscription_tier'>,
): AntigravityTierBadge {
  const rawTier = getKnownSubscriptionTier(quota);
  if (!rawTier) {
    return { tier: 'UNKNOWN', label: '—', className: 'unknown' };
  }

  const slug = rawTier
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  const stripped = slug
    .replace(/^(?:antigravity|google-one-ai|google-one|google-ai|g1)-/, '')
    .replace(/-tier$/, '');

  const exactMatch =
    ANTIGRAVITY_TIER_DEFINITIONS[slug] ??
    ANTIGRAVITY_TIER_DEFINITIONS[stripped];
  if (exactMatch) {
    return { ...exactMatch, rawTier };
  }

  const patternMatch = ANTIGRAVITY_TIER_FALLBACK_PATTERNS.find(([pattern]) =>
    pattern.test(slug) || pattern.test(stripped),
  );
  if (patternMatch) {
    return { ...patternMatch[1], rawTier };
  }

  return {
    tier: 'UNKNOWN',
    label: rawTier,
    className: 'unknown',
    rawTier,
  };
}

export function getSubscriptionTier(quota?: Pick<QuotaData, 'subscription_tier'>): AntigravitySubscriptionTier {
  return resolveAntigravityTierBadge(quota).tier;
}

export function getSubscriptionTierDisplay(quota?: Pick<QuotaData, 'subscription_tier'>): string {
  return resolveAntigravityTierBadge(quota).label;
}

export function getAntigravityTierBadge(quota?: Pick<QuotaData, 'subscription_tier'>): AntigravityTierBadge {
  return resolveAntigravityTierBadge(quota);
}

/**
 * 判定账号是否属于 GCP ToS 体系（服务端挂在共享 'aicode-consumers' 项目）。
 * quota 上已沉淀的 is_gcp_tos 优先，回退到 token 上同步的标记。
 */
export function isGcpTosAccount(account?: Account | null): boolean {
  if (!account) return false;
  if (account.quota?.is_gcp_tos != null) return account.quota.is_gcp_tos;
  return account.token?.is_gcp_tos === true;
}

/** 取账号的 GCP / Enterprise 项目 ID（quota 优先，回退 token）。 */
export function getAccountProjectId(account?: Account | null): string | undefined {
  if (!account) return undefined;
  return account.quota?.project_id || account.token?.project_id || undefined;
}

export function getQuotaClass(percentage: number): string {
  if (percentage >= 70) return 'high';
  if (percentage >= 30) return 'medium';
  return 'low';
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

export function formatResetTime(resetTime: string, t: Translate): string {
  if (!resetTime) return '';
  try {
    const reset = new Date(resetTime);
    if (Number.isNaN(reset.getTime())) return '';
    const now = new Date();
    const diffMs = reset.getTime() - now.getTime();
    if (diffMs <= 0) return t('common.shared.quota.resetDone');

    const totalMinutes = Math.floor(diffMs / (1000 * 60));
    const days = Math.floor(totalMinutes / (60 * 24));
    const hours = Math.floor((totalMinutes % (60 * 24)) / 60);
    const minutes = totalMinutes % 60;

    let parts = [];
    if (days > 0) parts.push(`${days}d`);
    if (hours > 0) parts.push(`${hours}h`);
    if (minutes > 0) parts.push(`${minutes}m`);
    
    // If less than 1 minute but positive, show 1m or <1m. Let's use 1m for simplicity or <1m
    if (parts.length === 0) return '<1m';
    
    return parts.join(' ');
  } catch {
    return '';
  }
}

export function formatResetTimeAbsolute(resetTime: string): string {
  if (!resetTime) return '';
  const reset = new Date(resetTime);
  if (Number.isNaN(reset.getTime())) return '';
  const pad = (value: number) => String(value).padStart(2, '0');
  const month = pad(reset.getMonth() + 1);
  const day = pad(reset.getDate());
  const hours = pad(reset.getHours());
  const minutes = pad(reset.getMinutes());
  return `${month}/${day} ${hours}:${minutes}`;
}

export function formatResetTimeDisplay(resetTime: string, t: Translate): string {
  const resetDone = t('common.shared.quota.resetDone');
  const relative = formatResetTime(resetTime, t);
  const absolute = formatResetTimeAbsolute(resetTime);
  if (!relative && !absolute) return '';
  if (relative === resetDone) return relative;
  // If we have both, return "relative (absolute)"
  // If only one, return that one
  if (relative && absolute) {
    return `${relative} (${absolute})`;
  }
  return relative || absolute;
}

export function getDisplayModels(quota?: QuotaData) {
  return getAntigravityDisplayModelsFromQuota(quota);
}

export function getModelShortName(name: string): string {
  return getAntigravityModelDisplayName(name);
}
