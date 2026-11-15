/**
 * WorkBuddy 兼容性函数
 *
 * 提供向后兼容的函数名称（getWorkbuddy* 等旧命名）
 */

import { getWorkbuddyAiPlan } from '../utils/workbuddyCredits';
import type { WorkbuddyAccount, CodebuddyPlanDetail, CodebuddyUsage, OfficialQuotaModel, QuotaDisplayItem, QuotaCategoryGroup } from './codebuddy-suite';
import {
  getAccountDisplayEmail,
  getAccountDisplayName,
  getPlanDetail,
  getPlanBadge,
  getUsage,
  getAccountStatus,
  getOfficialQuotaModel,
  getQuotaDisplayItems,
  getQuotaCategoryGroups,
} from '../utils/codebuddy-suite';

/**
 * @deprecated 使用 getAccountDisplayEmail
 */
export function getWorkbuddyAccountDisplayEmail(account: WorkbuddyAccount): string {
  return getAccountDisplayEmail(account);
}

/**
 * @deprecated 使用 getAccountDisplayName
 */
export function getWorkbuddyAccountDisplayName(account: WorkbuddyAccount): string {
  return getAccountDisplayName(account);
}

/**
 * @deprecated 使用 getPlanDetail
 */
export function getWorkbuddyPlanDetail(account: WorkbuddyAccount): CodebuddyPlanDetail {
  return getPlanDetail(account);
}

/**
 * @deprecated 使用 getPlanBadge
 */
export function getWorkbuddyPlanBadge(account: WorkbuddyAccount): string {
  return getPlanBadge(account);
}

export function getWorkbuddyAiPlanBadge(account: WorkbuddyAccount): string {
  return getWorkbuddyAiPlan(account).label;
}

/**
 * @deprecated 使用 getUsage
 */
export function getWorkbuddyUsage(account: WorkbuddyAccount): CodebuddyUsage {
  return getUsage(account);
}

/**
 * @deprecated 使用 getAccountStatus
 */
export function getWorkbuddyAccountStatus(account: WorkbuddyAccount): string {
  return getAccountStatus(account);
}

/**
 * @deprecated 使用 getOfficialQuotaModel
 */
export function getWorkbuddyOfficialQuotaModel(account: WorkbuddyAccount): OfficialQuotaModel {
  return getOfficialQuotaModel(account);
}

/**
 * @deprecated 使用 getQuotaDisplayItems
 */
export function getWorkbuddyQuotaDisplayItems(account: WorkbuddyAccount): QuotaDisplayItem[] {
  return getQuotaDisplayItems(account);
}

/**
 * @deprecated 使用 getQuotaCategoryGroups
 */
export function getWorkbuddyQuotaCategoryGroups(account: WorkbuddyAccount, t: (key: string, defaultValue?: string) => string): QuotaCategoryGroup[] {
  return getQuotaCategoryGroups(account, t);
}