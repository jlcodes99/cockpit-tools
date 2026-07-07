/**
 * CodeBuddy CN 类型定义
 *
 * 此文件从 codebuddy-suite 导入共享类型并重新导出，
 * 保持向后兼容性。CodeBuddy CN 有额外的签到相关字段。
 */

// 从共享类型导入基础定义
export {
  PACKAGE_CODE,
  RESOURCE_STATUS,
  ENTERPRISE_ACCOUNT_TYPES,
  CB_PACKAGE_CODE,
  CB_RESOURCE_STATUS,
} from './codebuddy-suite';

// 导出类型（需要使用 export type）
export type {
  CodebuddyPlanBadge,
  CodebuddyPlanDetail,
  OfficialQuotaResource,
  OfficialQuotaModel,
  CodebuddyUsage,
  QuotaDisplayItem,
  QuotaCategory,
  QuotaCategoryGroup,
  CodebuddyCnAccount,
  CodebuddySuiteAccountBase,
  CodebuddyAccount,
} from './codebuddy-suite';

// 兼容旧类型名称（从导入的类型引用）
import type {
  OfficialQuotaResource,
  OfficialQuotaModel,
} from './codebuddy-suite';
export type CodebuddyOfficialQuotaResource = OfficialQuotaResource;
export type CodebuddyOfficialQuotaModel = OfficialQuotaModel;
export type CodebuddyResourceSummary = {
  packageName: string | null;
  cycleStartTime: string | null;
  cycleEndTime: string | null;
  remain: number;
  used: number;
  total: number | null;
  usedPercent: number;
  remainPercent: number | null;
  boundUpdatedAt: number | null;
};
export type CodebuddyExtraCreditSummary = {
  remain: number;
  total: number;
  used: number;
  usedPercent: number;
  remainPercent: number | null;
};

// 从共享工具函数导入
export {
  asRecord,
  parseNumeric,
  parseDateTimeToEpoch,
  parseCycleTotal,
  parseCycleRemain,
  isActiveResource,
  isExtraPackage,
  isTrialOrFreeMonPackage,
  isProPackage,
  extractResourceAccounts,
  getAccountQuotaUpdatedAtMs,
  aggregateCycleResources,
  toOfficialQuotaResource,
  getPlanDetail,
  getPlanBadge,
  getPlanBadgeClass,
  getUsage,
  getAccountStatus,
  getCreditsBalance,
  getAccountDisplayEmail,
  getAccountDisplayName,
  getOfficialQuotaModel,
  getQuotaDisplayItems,
  getQuotaCategoryGroups,
} from '../utils/codebuddy-suite';

// 为了向后兼容，重新导出函数名称
export {
  getCodebuddyAccountDisplayEmail,
  getCodebuddyAccountDisplayName,
  getCodebuddyPlanDetail,
  getCodebuddyPlanBadge,
  getCodebuddyPlanBadgeClass,
  getCodebuddyUsage,
  getCodebuddyAccountStatus,
  getCodebuddyCreditsBalance,
  getCodebuddyOfficialQuotaModel,
  getCodebuddyQuotaDisplayItems,
  getCodebuddyQuotaCategoryGroups,
  getCodebuddyResourceSummary,
  getCodebuddyExtraCreditSummary,
} from './codebuddy-compat';

// 导入类型别名定义
import type { CodebuddyCnAccount } from './codebuddy-suite';

// 签到相关类型（统一契约，供 service / UI 复用）
export interface CheckinStatusResponse {
  today_checked_in: boolean;
  active: boolean;
  streak_days: number;
  daily_credit?: number | null;
  today_credit?: number | null;
  next_streak_day?: number | null;
  is_streak_day?: boolean | null;
  checkin_dates?: string[] | null;
  streak_bonus_days?: number | null;
  streak_bonus_credit?: number | null;
}

export interface CheckinResponse {
  success: boolean;
  message?: string | null;
  reward?: Record<string, unknown> | null;
  credit?: number | null;
  streak_days?: number | null;
  is_streak_day?: boolean | null;
  next_checkin_in?: number | null;
  nextCheckinIn?: number | null;
}

// 签到字段特定函数
export function hasCheckinFields(account: CodebuddyCnAccount): boolean {
  return account.last_checkin_time !== undefined || account.checkin_streak !== undefined;
}

export function getCheckinStreak(account: CodebuddyCnAccount): number | null {
  return account.checkin_streak ?? null;
}

export function getLastCheckinTime(account: CodebuddyCnAccount): number | null {
  return account.last_checkin_time ?? null;
}
