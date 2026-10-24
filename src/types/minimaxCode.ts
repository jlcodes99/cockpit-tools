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
  message?: string | null;
  credit?: number | null;
  streakDays?: number | null;
}

export function getMinimaxCodeAccountDisplayEmail(account: MinimaxCodeAccount): string {
  return account.email?.trim() || account.nickname?.trim() || account.id;
}

export function getMinimaxCodePlanBadge(_account: MinimaxCodeAccount): string {
  return 'FREE';
}

/**
 * MiniMax Code 没有 CodeBuddy 那种用量百分比接口，因此 `inlineSuggestionsUsedPercent` /
 * `chatMessagesUsedPercent` 恒为 null；`code` / `zh` / `en` 保留给共享账号总览使用。
 */
export interface MinimaxCodeUsage {
  code: string | null;
  zh: string | null;
  en: string | null;
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
}

export function getMinimaxCodeUsage(_account: MinimaxCodeAccount): MinimaxCodeUsage {
  return {
    code: null,
    zh: null,
    en: null,
    inlineSuggestionsUsedPercent: null,
    chatMessagesUsedPercent: null,
  };
}
