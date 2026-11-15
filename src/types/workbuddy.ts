/**
 * WorkBuddy 类型定义
 *
 * 此文件从 codebuddy-suite 导入共享类型并重新导出，
 * 保持向后兼容性。WorkBuddy 使用基础类型（无签到字段）。
 */

// 从共享类型导入基础定义
export {
  PACKAGE_CODE,
  RESOURCE_STATUS,
  ENTERPRISE_ACCOUNT_TYPES,
  WORKBUDDY_PACKAGE_CODE,
  WORKBUDDY_RESOURCE_STATUS,
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
  WorkbuddyAccount,
  CodebuddySuiteAccountBase,
} from './codebuddy-suite';

// 兼容旧类型名称（从导入的类型引用）
import type {
  OfficialQuotaResource,
  OfficialQuotaModel,
} from './codebuddy-suite';
export type WorkbuddyOfficialQuotaResource = OfficialQuotaResource;
export type WorkbuddyOfficialQuotaModel = OfficialQuotaModel;

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
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyAccountDisplayName,
  getWorkbuddyPlanDetail,
  getWorkbuddyPlanBadge,
  getWorkbuddyAiPlanBadge,
  getWorkbuddyUsage,
  getWorkbuddyAccountStatus,
  getWorkbuddyOfficialQuotaModel,
  getWorkbuddyQuotaDisplayItems,
  getWorkbuddyQuotaCategoryGroups,
} from './workbuddy-compat';