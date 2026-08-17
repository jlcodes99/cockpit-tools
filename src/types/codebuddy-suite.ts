/**
 * CodeBuddy Suite 共享类型定义
 *
 * CodeBuddy CN 和 WorkBuddy 共享相同的账号数据结构，
 * 仅 CodeBuddy CN 有额外的签到相关字段。
 */

/**
 * CodeBuddy Suite 账号基础类型
 * 包含 CodeBuddy CN 和 WorkBuddy 的所有共享字段
 */
export interface CodebuddySuiteAccountBase {
  id: string;
  email: string;
  uid?: string | null;
  nickname?: string | null;
  enterprise_id?: string | null;
  enterprise_name?: string | null;
  tags?: string[] | null;

  access_token: string;
  refresh_token?: string | null;
  token_type?: string | null;
  expires_at?: number | null;
  domain?: string | null;

  plan_type?: string;
  dosage_notify_code?: string;
  dosage_notify_zh?: string;
  dosage_notify_en?: string;
  payment_type?: string;

  quota_raw?: unknown;
  auth_raw?: unknown;
  profile_raw?: unknown;
  usage_raw?: unknown;

  status?: string | null;
  status_reason?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;

  created_at: number;
  last_used: number;
}

/**
 * CodeBuddy CN 账号类型
 * 继承基础类型，添加签到相关字段
 */
export interface CodebuddyCnAccount extends CodebuddySuiteAccountBase {
  last_checkin_time?: number | null;
  checkin_streak?: number;
  checkin_rewards?: Record<string, unknown> | null;
}

/**
 * WorkBuddy 账号类型
 * 使用基础类型（支持签到字段）
 */
export interface WorkbuddyAccount extends CodebuddySuiteAccountBase {
  last_checkin_time?: number | null;
  checkin_streak?: number;
  checkin_rewards?: Record<string, unknown> | null;
}

/**
 * CodeBuddy 账号类型（兼容旧名称）
 * 使用基础类型（与 WorkBuddy 相同）
 */
export type CodebuddyAccount = CodebuddySuiteAccountBase;

/**
 * 套餐徽章类型
 */
export type CodebuddyPlanBadge = 'FREE' | 'PRO' | 'TRIAL' | 'ENTERPRISE' | 'UNKNOWN';

/**
 * 套餐代码常量
 * 与官方 CodeBuddy web client 的 PackageCode enum 对齐
 */
export const PACKAGE_CODE = {
  free: 'TCACA_code_001_PqouKr6QWV',
  proMon: 'TCACA_code_002_AkiJS3ZHF5',
  proYear: 'TCACA_code_003_FAnt7lcmRT',
  gift: 'TCACA_code_006_DbXS0lrypC',
  activity: 'TCACA_code_007_nzdH5h4Nl0',
  freeMon: 'TCACA_code_008_cfWoLwvjU4',
  extra: 'TCACA_code_009_0XmEQc2xOf',
  enterprise: 'TCACA_code_enterprise',
} as const;

/**
 * 资源状态常量
 * 与官方 CodeBuddy web client 的 resource status enum 对齐
 */
export const RESOURCE_STATUS = {
  valid: 0,
  refund: 1,
  expired: 2,
  usedUp: 3,
} as const;

/**
 * 企业账号类型列表
 */
export const ENTERPRISE_ACCOUNT_TYPES = ['ultimate', 'exclusive', 'premise'];

/**
 * 套餐详情
 */
export interface CodebuddyPlanDetail {
  type: 'pro' | 'free';
  isPro: boolean;
  isTrial: boolean;
  badge: string;
  packageCode: string | null;
}

/**
 * 配额资源
 */
export interface OfficialQuotaResource {
  packageCode: string | null;
  packageName: string | null;
  cycleStartTime: string | null;
  cycleEndTime: string | null;
  deductionEndTime: number | null;
  expiredTime: string | null;
  total: number;
  remain: number;
  used: number;
  usedPercent: number;
  remainPercent: number | null;
  refreshAt: number | null;
  expireAt: number | null;
  isBasePackage: boolean;
}

/**
 * 配额模型
 */
export interface OfficialQuotaModel {
  resources: OfficialQuotaResource[];
  extra: OfficialQuotaResource;
  updatedAt: number | null;
}

/**
 * 使用量信息
 */
export interface CodebuddyUsage {
  dosageNotifyCode?: string;
  dosageNotifyZh?: string;
  dosageNotifyEn?: string;
  paymentType?: string;
  isNormal: boolean;
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null;
}

/**
 * 配额显示项
 */
export interface QuotaDisplayItem {
  key: string;
  label: string;
  used: number;
  total: number;
  remain: number;
  usedPercent: number;
  remainPercent: number | null;
  quotaClass: string;
  refreshAt: number | null;
}

/**
 * 配额分组类型
 */
export type QuotaCategory = 'base' | 'activity' | 'extra' | 'other';

/**
 * 配额分组聚合项
 */
export interface QuotaCategoryGroup {
  key: QuotaCategory;
  label: string;
  used: number;
  total: number;
  remain: number;
  usedPercent: number;
  remainPercent: number | null;
  quotaClass: string;
  items: OfficialQuotaResource[];
  visible: boolean;
}

// 兼容旧常量名称
export const CB_PACKAGE_CODE = PACKAGE_CODE;
export const CB_RESOURCE_STATUS = RESOURCE_STATUS;
export const WORKBUDDY_PACKAGE_CODE = PACKAGE_CODE;
export const WORKBUDDY_RESOURCE_STATUS = RESOURCE_STATUS;