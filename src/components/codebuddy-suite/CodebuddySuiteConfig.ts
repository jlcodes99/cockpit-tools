/**
 * CodeBuddy Suite 平台配置
 *
 * 用于 CodeBuddy CN 和 WorkBuddy 页面的差异化配置
 */

import type { CodebuddySuiteAccountBase } from '../../types/codebuddy-suite';

/**
 * 平台配置接口
 */
export interface CodebuddySuitePlatformConfig<
  TAccount extends CodebuddySuiteAccountBase = CodebuddySuiteAccountBase
> {
  /** 平台标识 */
  platformKey: 'CodeBuddy CN' | 'WorkBuddy' | 'MiniMax Code';
  /** 平台标识符（用于路由、存储等） */
  platformId: 'codebuddy_cn' | 'workbuddy' | 'minimaxcode';
  /** OAuth 日志前缀 */
  oauthLogPrefix: string;
  /** localStorage key 前缀 */
  storageKeyPrefix: string;
  /** 导出文件前缀 */
  exportFilePrefix: string;
  /** CSS 类名前缀 */
  cssClassPrefix: string;

  /** 是否有签到功能 */
  hasCheckin: boolean;
  /** 同步目标平台（如果有） */
  syncTarget?: {
    platformKey: string;
    label: string;
    syncFunction: () => Promise<number>;
  };

  /** 获取显示邮箱 */
  getDisplayEmail: (account: TAccount) => string;
  /** 获取计划徽章 */
  getPlanBadge: (account: TAccount) => string;
  /** 获取用量状态 */
  getUsage: (account: TAccount) => {
    code: string | null;
    zh: string | null;
    en: string | null;
  };
}

/**
 * CodeBuddy CN 平台配置
 */
export const CODEBUDDY_CN_CONFIG: CodebuddySuitePlatformConfig = {
  platformKey: 'CodeBuddy CN',
  platformId: 'codebuddy_cn',
  oauthLogPrefix: 'CodebuddyCnOAuth',
  storageKeyPrefix: 'codebuddycn',
  exportFilePrefix: 'codebuddy_cn_accounts',
  cssClassPrefix: 'codebuddy',
  hasCheckin: true,
  // syncTarget 将在运行时注入
  getDisplayEmail: () => '', // 运行时注入
  getPlanBadge: () => '', // 运行时注入
  getUsage: () => ({ code: null, zh: null, en: null }), // 运行时注入
};

/**
 * WorkBuddy 平台配置
 */
export const WORKBUDDY_CONFIG: CodebuddySuitePlatformConfig = {
  platformKey: 'WorkBuddy',
  platformId: 'workbuddy',
  oauthLogPrefix: 'WorkbuddyOAuth',
  storageKeyPrefix: 'workbuddy',
  exportFilePrefix: 'workbuddy_accounts',
  cssClassPrefix: 'workbuddy',
  hasCheckin: false,
  // syncTarget 将在运行时注入
  getDisplayEmail: () => '', // 运行时注入
  getPlanBadge: () => '', // 运行时注入
  getUsage: () => ({ code: null, zh: null, en: null }), // 运行时注入
};

/**
 * MiniMax Code 平台配置
 */
export const MINIMAX_CODE_CONFIG: CodebuddySuitePlatformConfig = {
  platformKey: 'MiniMax Code',
  platformId: 'minimaxcode',
  oauthLogPrefix: 'MinimaxCodeOAuth',
  storageKeyPrefix: 'minimax_code',
  exportFilePrefix: 'minimax_code_accounts',
  cssClassPrefix: 'minimax-code',
  hasCheckin: true,
  // syncTarget 将在运行时注入
  getDisplayEmail: () => '', // 运行时注入
  getPlanBadge: () => '', // 运行时注入
  getUsage: () => ({ code: null, zh: null, en: null }), // 运行时注入
};

/**
 * 已知计划筛选器
 */
export const KNOWN_PLAN_FILTERS = ['FREE', 'TRIAL', 'PRO', 'ENTERPRISE'] as const;
export type KnownPlanFilter = typeof KNOWN_PLAN_FILTERS[number];