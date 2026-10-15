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

export function getSubscriptionTier(quota?: QuotaData): string {
  const rawTier = quota?.subscription_tier?.trim();
  if (!rawTier) return 'UNKNOWN';

  const tier = rawTier.toLowerCase();
  // 映射等级名称
  if (tier.includes('ultra')) return 'ULTRA';
  if (tier.includes('pro')) return 'PRO';
  return 'FREE';
}

export function getSubscriptionTierDisplay(quota?: QuotaData): string {
  const rawTier = quota?.subscription_tier?.trim();
  if (rawTier) return rawTier;
  return getSubscriptionTier(quota);
}

export function getAntigravityTierBadge(quota?: QuotaData): {
  tier: string;
  label: string;
  className: string;
} {
  const tier = getSubscriptionTier(quota);
  return {
    tier,
    label: tier,
    className: tier.toLowerCase(),
  };
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
