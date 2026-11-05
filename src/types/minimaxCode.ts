/**
 * MiniMax Code 类型定义
 *
 * MiniMax Code 与 CodeBuddy CN / WorkBuddy 共享同一套 CodeBuddy Suite 账号结构，
 * 额外的 `region` / `auth_file` 字段用于 OAuth 设备流与客户端凭据注入。
 */

import type { CodebuddySuiteAccountBase } from './codebuddy-suite';

export interface MinimaxCodeAccount extends CodebuddySuiteAccountBase {
  region?: string | null;
  auth_file?: string | null;
}

export interface MinimaxCodeCheckinStatus {
  todayCheckedIn: boolean;
  active: boolean;
  points: number;
  bonusPoints: number;
  streakDays: number;
  dayIndex?: number | null;
  message: string;
}

export interface MinimaxCodeCheckinResult {
  success: boolean;
  alreadyClaimed?: boolean;
  warning?: string | null;
  message?: string | null;
  credit?: number | null;
  streakDays?: number | null;
}

export function getMinimaxCodeAccountDisplayEmail(account: MinimaxCodeAccount): string {
  // The username is the primary identity here: `/v1/api/user/info` frequently
  // returns no email address, and preferring `email` used to surface the raw
  // numeric realUserID (e.g. "570025187764355072") on the card.
  return account.nickname?.trim() || account.email?.trim() || account.id;
}

export function getMinimaxCodePlanBadge(account: MinimaxCodeAccount): string {
  const raw = getMinimaxCodeQuotaSnapshot(account).membership;
  return account.plan_type?.trim() || textValue(raw?.token_plan_tier) || textValue(raw?.plan_name) || '';
}

/**
 * MiniMax Code 没有 CodeBuddy 那种用量百分比接口，因此 `inlineSuggestionsUsedPercent` /
 * `chatMessagesUsedPercent` 恒为 null；`code` / `zh` / `en` 保留给共享账号总览使用。
 */
export interface MinimaxCodeUsage {
  isNormal: boolean;
  code: string | null;
  zh: string | null;
  en: string | null;
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
}

export function getMinimaxCodeUsage(account: MinimaxCodeAccount): MinimaxCodeUsage {
  return {
    isNormal: !account.quota_query_last_error && getMinimaxCodeQuotaSnapshot(account).total !== null,
    code: account.quota_query_last_error || null,
    zh: null,
    en: null,
    inlineSuggestionsUsedPercent: null,
    chatMessagesUsedPercent: null,
  };
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
}
function textValue(value: unknown): string { return typeof value === 'string' ? value.trim() : ''; }
export function minimaxCreditNumber(value: unknown): number | null {
  if (typeof value !== 'number' && typeof value !== 'string') return null;
  if (typeof value === 'string' && !value.trim()) return null;
  const number = typeof value === 'number' ? value : Number(value.replace(/,/g, '').trim());
  return Number.isFinite(number) ? number : null;
}
export function getMinimaxCodeQuotaSnapshot(account: MinimaxCodeAccount) {
  const raw = record(account.quota_raw);
  const membership = record(raw?.membership);
  const workspace = record(raw?.workspace);
  const credits = record(membership?.op_credit_summary);
  return {
    membership,
    total: minimaxCreditNumber(credits?.total_remaining_amount ?? membership?.opcredit_balance ?? workspace?.opcredit_balance),
    purchased: minimaxCreditNumber(credits?.purchased_remaining_amount ?? credits?.purchased_credit),
    gifted: minimaxCreditNumber(credits?.free_remaining_amount ?? credits?.free_credit),
    unsubscribed: membership?.subscription_type === 'none',
    plan: textValue(membership?.token_plan_tier) || textValue(membership?.plan_name),
  };
}
