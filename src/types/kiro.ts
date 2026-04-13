import {
  isRecord,
  getPathValue,
  toNumber,
  toStringValue as _toStringValue,
  firstNumber,
  firstTimestamp,
  normalizeTimestamp,
  clampPercent,
  safeLeft,
  sumDefined,
} from '../utils/dataExtract';

/** Kiro 账号数据（后端原样返回 + 前端兼容字段） */
export interface KiroAccount {
  id: string;
  email: string;
  user_id?: string | null;
  login_provider?: string | null;
  tags?: string[] | null;

  access_token: string;
  refresh_token?: string | null;
  token_type?: string | null;
  expires_at?: number | null;

  idc_region?: string | null;
  issuer_url?: string | null;
  client_id?: string | null;
  scopes?: string | null;
  login_hint?: string | null;

  plan_name?: string | null;
  plan_tier?: string | null;
  credits_total?: number | null;
  credits_used?: number | null;
  bonus_total?: number | null;
  bonus_used?: number | null;
  usage_reset_at?: number | null;
  bonus_expire_days?: number | null;

  kiro_auth_token_raw?: unknown;
  kiro_profile_raw?: unknown;
  kiro_usage_raw?: unknown;
  status?: string | null;
  status_reason?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;

  created_at: number;
  last_used: number;

  // 兼容复制自 Windsurf 页面的字段（可选）
  github_login?: string;
  github_email?: string | null;
  copilot_plan?: string | null;

  // Provider store 统一视图字段
  plan_type?: string;
  quota?: KiroQuota;
}

export interface KiroQuota {
  hourly_percentage: number;
  hourly_reset_time?: number | null;
  weekly_percentage: number;
  weekly_reset_time?: number | null;
  raw_data?: unknown;
}

export type KiroQuotaClass = 'high' | 'medium' | 'low' | 'critical';
export type KiroPlanBadge = 'FREE' | 'INDIVIDUAL' | 'PRO' | 'BUSINESS' | 'ENTERPRISE' | 'UNKNOWN';

export type KiroUsage = {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null;
  remainingCompletions?: number | null;
  remainingChat?: number | null;
  totalCompletions?: number | null;
  totalChat?: number | null;
};

export type KiroCreditsSummary = {
  planName: string | null;
  creditsLeft: number | null;
  promptCreditsLeft: number | null;
  promptCreditsUsed: number | null;
  promptCreditsTotal: number | null;
  addOnCredits: number | null;
  addOnCreditsUsed: number | null;
  addOnCreditsTotal: number | null;
  planStartsAt: number | null;
  planEndsAt: number | null;
  bonusExpireDays: number | null;
};

/* ------------------------------------------------------------------ */
/*  Kiro 特有的工具函数                                                 */
/* ------------------------------------------------------------------ */

function isLikelyEmail(value: string | null | undefined): boolean {
  if (!value) return false;
  const trimmed = value.trim();
  if (!trimmed) return false;
  return /^[^\s@]+@[^\s@]+$/.test(trimmed);
}

function isPlaceholderIdentity(value: string | null | undefined): boolean {
  if (!value) return true;
  const normalized = value.trim().toLowerCase();
  return (
    normalized === '-' ||
    normalized === '--' ||
    normalized === 'unknown' ||
    normalized === 'n/a' ||
    normalized === 'na' ||
    normalized === 'null' ||
    normalized === 'undefined'
  );
}

/**
 * Kiro 版 toStringValue：在通用版基础上增加 placeholder 检查。
 * 用于从 raw 数据中提取有效字符串时过滤占位符。
 */
function toStringValue(value: unknown): string | null {
  const str = _toStringValue(value);
  return str && !isPlaceholderIdentity(str) ? str : null;
}

/**
 * Kiro 版 firstString：使用 Kiro 的 toStringValue（带 placeholder 过滤）。
 */
function firstString(root: unknown, paths: string[][]): string | null {
  for (const path of paths) {
    const value = toStringValue(getPathValue(root, path));
    if (value) return value;
  }
  return null;
}

function resolveRawEmail(account: KiroAccount): string | null {
  const fromUsage = firstString(account.kiro_usage_raw, [
    ['userInfo', 'email'],
    ['email'],
  ]);
  if (isLikelyEmail(fromUsage)) return fromUsage;

  const fromProfile = firstString(account.kiro_profile_raw, [
    ['email'],
    ['user', 'email'],
    ['account', 'email'],
    ['primaryEmail'],
  ]);
  if (isLikelyEmail(fromProfile)) return fromProfile;

  const fromAuth = firstString(account.kiro_auth_token_raw, [
    ['email'],
    ['userEmail'],
    ['login_hint'],
    ['loginHint'],
  ]);
  if (isLikelyEmail(fromAuth)) return fromAuth;
  return null;
}

function resolveRawUserId(account: KiroAccount): string | null {
  const userId = account.user_id?.trim();
  if (userId && !isPlaceholderIdentity(userId)) return userId;

  return (
    firstString(account.kiro_usage_raw, [
      ['userInfo', 'userId'],
      ['userId'],
      ['user_id'],
    ]) ||
    firstString(account.kiro_profile_raw, [
      ['userId'],
      ['user_id'],
      ['id'],
      ['sub'],
      ['account', 'id'],
      ['arn'],
      ['profileArn'],
    ]) ||
    firstString(account.kiro_auth_token_raw, [
      ['userId'],
      ['user_id'],
      ['sub'],
      ['accountId'],
      ['profileArn'],
      ['profile_arn'],
      ['arn'],
    ]) ||
    null
  );
}

function resolveRawProvider(account: KiroAccount): string | null {
  const provider = account.login_provider?.trim();
  if (provider && !isPlaceholderIdentity(provider)) return provider;

  return (
    firstString(account.kiro_usage_raw, [
      ['userInfo', 'provider', 'label'],
      ['userInfo', 'provider', 'name'],
      ['userInfo', 'provider', 'id'],
      ['userInfo', 'providerId'],
      ['provider', 'label'],
      ['provider', 'name'],
      ['provider', 'id'],
    ]) ||
    firstString(account.kiro_profile_raw, [
      ['loginProvider'],
      ['provider'],
      ['authProvider'],
      ['signedInWith'],
      ['name'],
    ]) ||
    firstString(account.kiro_auth_token_raw, [
      ['provider'],
      ['loginProvider'],
      ['login_option'],
      ['authMethod'],
    ]) ||
    null
  );
}

function resolvePlanName(account: KiroAccount): string | null {
  return (
    account.plan_name?.trim() ||
    account.plan_tier?.trim() ||
    account.copilot_plan?.trim() ||
    firstString(account.kiro_usage_raw, [
      ['subscriptionInfo', 'subscriptionName'],
      ['subscriptionInfo', 'subscriptionTitle'],
      ['subscriptionInfo', 'subscriptionType'],
      ['subscriptionInfo', 'type'],
      ['planName'],
      ['currentPlanName'],
      ['plan', 'name'],
      ['plan', 'tier'],
      ['usageBreakdownList', '0', 'displayName'],
      ['usageBreakdownList', '0', 'displayNamePlural'],
      ['usageBreakdownList', '0', 'resourceType'],
      ['usageBreakdowns', 'planName'],
      ['usageBreakdowns', 'tier'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'displayName'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'displayNamePlural'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'type'],
      ['usageState', 'usageBreakdowns', '0', 'displayName'],
      ['usageState', 'usageBreakdowns', '0', 'displayNamePlural'],
      ['usageState', 'usageBreakdowns', '0', 'type'],
    ])
  );
}

function getUsageRoot(account: KiroAccount): unknown {
  return (
    getPathValue(account.kiro_usage_raw, ['kiro.resourceNotifications.usageState']) ||
    getPathValue(account.kiro_usage_raw, ['usageState']) ||
    account.kiro_usage_raw
  );
}

function getUsageBreakdowns(account: KiroAccount): Record<string, unknown>[] {
  const usageRoot = getUsageRoot(account);
  const fromUsageBreakdownList = getPathValue(usageRoot, ['usageBreakdownList']);
  if (Array.isArray(fromUsageBreakdownList)) {
    return fromUsageBreakdownList.filter(isRecord);
  }
  const fromUsageBreakdowns = getPathValue(usageRoot, ['usageBreakdowns']);
  if (Array.isArray(fromUsageBreakdowns)) {
    return fromUsageBreakdowns.filter(isRecord);
  }
  return [];
}

function getBreakdownUsageTotal(breakdown: Record<string, unknown>): number | null {
  return firstNumber(breakdown, [
    ['usageLimitWithPrecision'],
    ['usageLimit'],
    ['limit'],
    ['total'],
    ['totalCredits'],
  ]);
}

function getBreakdownUsageUsed(breakdown: Record<string, unknown>): number | null {
  return firstNumber(breakdown, [
    ['currentUsageWithPrecision'],
    ['currentUsage'],
    ['used'],
    ['usedCredits'],
  ]);
}

function getBreakdownFreeTrialData(
  breakdown: Record<string, unknown>,
): Record<string, unknown> | null {
  const freeTrialUsage = getPathValue(breakdown, ['freeTrialUsage']);
  if (isRecord(freeTrialUsage)) return freeTrialUsage;
  const freeTrialInfo = getPathValue(breakdown, ['freeTrialInfo']);
  if (isRecord(freeTrialInfo)) return freeTrialInfo;
  return null;
}

function getBonusDaysRemaining(raw: unknown): number | null {
  const direct = firstNumber(raw, [['daysRemaining'], ['expiryDays'], ['expireDays']]);
  if (direct != null) return Math.max(0, Math.round(direct));

  const expiryTs = firstTimestamp(raw, [['expiryDate'], ['freeTrialExpiry'], ['expiresAt']]);
  if (expiryTs == null) return null;

  const now = Math.floor(Date.now() / 1000);
  if (expiryTs <= now) return 0;
  return Math.ceil((expiryTs - now) / 86_400);
}



type ParsedBonusUsage = {
  total: number | null;
  used: number | null;
  daysRemaining: number | null;
};

function parseBonusUsage(raw: unknown): ParsedBonusUsage | null {
  if (!isRecord(raw)) return null;
  const total = firstNumber(raw, [['usageLimitWithPrecision'], ['usageLimit'], ['limit'], ['total'], ['totalCredits']]);
  const used = firstNumber(raw, [['currentUsageWithPrecision'], ['currentUsage'], ['used'], ['usedCredits']]);
  const daysRemaining = getBonusDaysRemaining(raw);

  const hasUsageData =
    (typeof total === 'number' && total > 0) ||
    (typeof used === 'number' && used > 0);
  const hasDaysData = typeof daysRemaining === 'number';

  if (!hasUsageData && !hasDaysData) return null;
  if (hasDaysData && (daysRemaining as number) <= 0) return null;

  return { total, used, daysRemaining };
}

function collectActiveBonusUsages(account: KiroAccount): ParsedBonusUsage[] {
  const result: ParsedBonusUsage[] = [];

  getUsageBreakdowns(account).forEach((breakdown) => {
    const freeTrialUsage = getBreakdownFreeTrialData(breakdown);
    const parsedFreeTrial = parseBonusUsage(freeTrialUsage);
    if (parsedFreeTrial) {
      result.push(parsedFreeTrial);
    }

    const bonuses = getPathValue(breakdown, ['bonuses']);
    if (!Array.isArray(bonuses)) return;

    bonuses.forEach((item) => {
      const parsedBonus = parseBonusUsage(item);
      if (parsedBonus) {
        result.push(parsedBonus);
      }
    });
  });

  return result;
}

function resolvePromptTotal(account: KiroAccount): number | null {
  const fromAccount = toNumber(account.credits_total);
  if (fromAccount != null) return fromAccount;

  const aggregated = sumDefined(
    getUsageBreakdowns(account)
      .map((breakdown) => getBreakdownUsageTotal(breakdown))
      .map((value) => (typeof value === 'number' && value > 0 ? value : null)),
  );
  if (aggregated != null) return aggregated;

  return (
    firstNumber(getUsageRoot(account), [
      ['usageBreakdownList', '0', 'usageLimitWithPrecision'],
      ['usageBreakdownList', '0', 'usageLimit'],
      ['estimatedUsage', 'total'],
      ['estimatedUsage', 'creditsTotal'],
      ['usageBreakdowns', 'plan', 'totalCredits'],
      ['usageBreakdowns', 'covered', 'total'],
      ['credits', 'total'],
      ['totalCredits'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'usageLimit'],
      ['usageState', 'usageBreakdowns', '0', 'usageLimit'],
    ])
  );
}

function resolvePromptUsed(account: KiroAccount): number | null {
  const fromAccount = toNumber(account.credits_used);
  if (fromAccount != null) return fromAccount;

  const aggregated = sumDefined(
    getUsageBreakdowns(account).map((breakdown) => {
      const total = getBreakdownUsageTotal(breakdown);
      const used = getBreakdownUsageUsed(breakdown);
      const hasFreeTrial = !!getBreakdownFreeTrialData(breakdown);

      if ((total == null || total <= 0) && hasFreeTrial) return null;
      if (used == null) return null;
      if (total != null && total > 0) return Math.max(0, Math.min(used, total));
      return Math.max(0, used);
    }),
  );
  if (aggregated != null) return aggregated;

  return (
    firstNumber(getUsageRoot(account), [
      ['usageBreakdownList', '0', 'currentUsageWithPrecision'],
      ['usageBreakdownList', '0', 'currentUsage'],
      ['estimatedUsage', 'used'],
      ['estimatedUsage', 'creditsUsed'],
      ['usageBreakdowns', 'plan', 'usedCredits'],
      ['usageBreakdowns', 'covered', 'used'],
      ['credits', 'used'],
      ['usedCredits'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'currentUsage'],
      ['usageState', 'usageBreakdowns', '0', 'currentUsage'],
    ])
  );
}

function resolveAddOnTotal(account: KiroAccount): number | null {
  const fromAccount = toNumber(account.bonus_total);
  if (fromAccount != null) return fromAccount;

  const aggregated = sumDefined(
    collectActiveBonusUsages(account).map((item) => item.total),
  );
  if (aggregated != null) return aggregated;

  return (
    firstNumber(getUsageRoot(account), [
      ['usageBreakdownList', '0', 'freeTrialInfo', 'usageLimitWithPrecision'],
      ['usageBreakdownList', '0', 'freeTrialInfo', 'usageLimit'],
      ['bonusCredits', 'total'],
      ['bonus', 'total'],
      ['usageBreakdowns', 'bonus', 'total'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'usageLimit'],
      ['usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'usageLimit'],
    ])
  );
}

function resolveAddOnUsed(account: KiroAccount): number | null {
  const fromAccount = toNumber(account.bonus_used);
  if (fromAccount != null) return fromAccount;

  const aggregated = sumDefined(
    collectActiveBonusUsages(account).map((item) => {
      if (item.used == null) return null;
      if (item.total != null && item.total > 0) {
        return Math.max(0, Math.min(item.used, item.total));
      }
      return Math.max(0, item.used);
    }),
  );
  if (aggregated != null) return aggregated;

  return (
    firstNumber(getUsageRoot(account), [
      ['usageBreakdownList', '0', 'freeTrialInfo', 'currentUsageWithPrecision'],
      ['usageBreakdownList', '0', 'freeTrialInfo', 'currentUsage'],
      ['bonusCredits', 'used'],
      ['bonus', 'used'],
      ['usageBreakdowns', 'bonus', 'used'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'currentUsage'],
      ['usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'currentUsage'],
    ])
  );
}

function resolvePlanEnd(account: KiroAccount): number | null {
  return (
    normalizeTimestamp(account.usage_reset_at) ??
    firstTimestamp(getUsageRoot(account), [
      ['billingCycle', 'resetDate'],
      ['nextDateReset'],
      ['resetAt'],
      ['resetTime'],
      ['resetOn'],
      ['usageBreakdowns', 'resetAt'],
      ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'resetDate'],
      ['usageState', 'usageBreakdowns', '0', 'resetDate'],
    ])
  );
}

function resolveBonusExpireDays(account: KiroAccount): number | null {
  const direct = toNumber(account.bonus_expire_days);
  if (direct != null) return Math.max(0, Math.round(direct));

  const bonusDays = collectActiveBonusUsages(account)
    .map((item) => item.daysRemaining)
    .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
  if (bonusDays.length > 0) {
    return Math.max(0, Math.round(Math.min(...bonusDays)));
  }

  const usageDays = firstNumber(getUsageRoot(account), [
    ['usageBreakdownList', '0', 'freeTrialInfo', 'daysRemaining'],
    ['bonusCredits', 'expiryDays'],
    ['bonusCredits', 'expireDays'],
    ['bonus', 'expiryDays'],
    ['bonus', 'expireDays'],
    ['freeTrialUsage', 'daysRemaining'],
    ['freeTrialUsage', 'expiryDays'],
    ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'daysRemaining'],
    ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'expiryDays'],
    ['usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'daysRemaining'],
    ['usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'expiryDays'],
  ]);
  if (usageDays != null) return Math.max(0, Math.round(usageDays));

  const expiryTs = firstTimestamp(getUsageRoot(account), [
    ['usageBreakdownList', '0', 'freeTrialInfo', 'freeTrialExpiry'],
    ['freeTrialUsage', 'expiryDate'],
    ['bonusCredits', 'expiryDate'],
    ['bonus', 'expiryDate'],
    ['kiro.resourceNotifications.usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'expiryDate'],
    ['usageState', 'usageBreakdowns', '0', 'freeTrialUsage', 'expiryDate'],
  ]);
  if (expiryTs == null) return null;

  const now = Math.floor(Date.now() / 1000);
  if (expiryTs <= now) return 0;
  return Math.ceil((expiryTs - now) / 86_400);
}

export function getKiroPlanDisplayName(planType?: string | null): string {
  if (!planType) return 'UNKNOWN';
  const upper = planType.toUpperCase();
  if (upper.includes('FREE')) return 'FREE';
  if (upper.includes('STANDALONE')) return 'FREE';
  if (upper.includes('PRO')) return 'PRO';
  if (upper.includes('INDIVIDUAL')) return 'INDIVIDUAL';
  if (upper.includes('BUSINESS') || upper.includes('TEAM')) return 'BUSINESS';
  if (upper.includes('ENTERPRISE')) return 'ENTERPRISE';
  return upper;
}

export function getKiroPlanBadgeClass(planType?: string | null): string {
  const plan = getKiroPlanDisplayName(planType);
  if (plan === 'FREE') return 'free';
  if (plan === 'INDIVIDUAL') return 'individual';
  if (plan === 'PRO') return 'pro';
  if (plan === 'BUSINESS') return 'business';
  if (plan === 'ENTERPRISE') return 'enterprise';
  return 'unknown';
}

export function getKiroPlanBadge(account: KiroAccount): KiroPlanBadge {
  const plan = getKiroPlanDisplayName(resolvePlanName(account));
  if (
    plan === 'FREE' ||
    plan === 'PRO' ||
    plan === 'INDIVIDUAL' ||
    plan === 'BUSINESS' ||
    plan === 'ENTERPRISE'
  ) {
    return plan;
  }
  return 'UNKNOWN';
}

export function getKiroQuotaClass(percentage: number): KiroQuotaClass {
  // 页面展示的是“使用量”：使用越高，风险颜色越高。
  if (percentage <= 20) return 'high';
  if (percentage <= 60) return 'medium';
  if (percentage <= 85) return 'low';
  return 'critical';
}

export function getKiroAccountDisplayEmail(account: KiroAccount): string {
  const email = account.email?.trim();
  if (isLikelyEmail(email)) return email;
  const githubEmail = account.github_email?.trim();
  if (githubEmail && isLikelyEmail(githubEmail)) return githubEmail;
  const rawEmail = resolveRawEmail(account);
  if (rawEmail) return rawEmail;
  const login = account.github_login?.trim();
  if (login && !isPlaceholderIdentity(login)) return login;
  const userId = resolveRawUserId(account);
  if (userId && !isPlaceholderIdentity(userId)) return userId;
  return account.id;
}

export function getKiroAccountDisplayUserId(account: KiroAccount): string {
  const userId = resolveRawUserId(account);
  if (userId && !isPlaceholderIdentity(userId)) return userId;
  if (isLikelyEmail(account.email)) return account.email.trim();
  return account.id;
}

export function getKiroAccountLoginProvider(account: KiroAccount): string | null {
  const provider = resolveRawProvider(account);
  if (!provider) return null;
  const lower = provider.toLowerCase();
  if (lower === 'google') return 'Google';
  if (lower === 'github') return 'GitHub';
  if (lower === 'social') return 'Social';
  return provider;
}

export type KiroAccountStatus = 'normal' | 'banned' | 'error' | 'unknown';

function normalizeStatusText(raw: string | null | undefined): string {
  return raw?.trim().toLowerCase() ?? '';
}

function inferBannedFromReason(raw: string | null | undefined): boolean {
  const reason = normalizeStatusText(raw);
  if (!reason) return false;
  return (
    reason.includes('banned') ||
    reason.includes('forbidden') ||
    reason.includes('suspended') ||
    reason.includes('disabled') ||
    reason.includes('封禁') ||
    reason.includes('禁用')
  );
}

export function getKiroAccountStatus(account: KiroAccount): KiroAccountStatus {
  const status = normalizeStatusText(account.status);
  if (status === 'normal' || status === 'ok' || status === 'active') return 'normal';
  if (status === 'banned' || status === 'ban' || status === 'forbidden') return 'banned';
  if (status === 'error' || status === 'failed' || status === 'invalid') return 'error';

  if (inferBannedFromReason(account.status_reason)) return 'banned';
  if (normalizeStatusText(account.status_reason)) return 'error';
  return 'unknown';
}

export function getKiroAccountStatusReason(account: KiroAccount): string | null {
  const fromField = account.status_reason?.trim();
  if (fromField) return fromField;
  return null;
}

export function hasKiroQuotaData(account: KiroAccount): boolean {
  return account.kiro_usage_raw != null;
}

export function isKiroAccountBanned(account: KiroAccount): boolean {
  return getKiroAccountStatus(account) === 'banned';
}

export function getKiroCreditsSummary(account: KiroAccount): KiroCreditsSummary {
  const promptTotal = resolvePromptTotal(account);
  const promptUsed = resolvePromptUsed(account);
  const promptLeft = safeLeft(promptTotal, promptUsed);

  const addOnTotal = resolveAddOnTotal(account);
  const addOnUsed = resolveAddOnUsed(account);
  const addOnLeft = safeLeft(addOnTotal, addOnUsed);

  const creditsLeft =
    promptLeft != null && addOnLeft != null
      ? promptLeft + addOnLeft
      : promptLeft ?? addOnLeft ?? null;

  return {
    planName: resolvePlanName(account),
    creditsLeft,
    promptCreditsLeft: promptLeft,
    promptCreditsUsed: promptUsed,
    promptCreditsTotal: promptTotal,
    addOnCredits: addOnLeft,
    addOnCreditsUsed: addOnUsed,
    addOnCreditsTotal: addOnTotal,
    planStartsAt: null,
    planEndsAt: resolvePlanEnd(account),
    bonusExpireDays: resolveBonusExpireDays(account),
  };
}

export function getKiroUsage(account: KiroAccount): KiroUsage {
  const summary = getKiroCreditsSummary(account);

  const inlinePct =
    summary.promptCreditsTotal != null && summary.promptCreditsTotal > 0 && summary.promptCreditsUsed != null
      ? clampPercent((summary.promptCreditsUsed / summary.promptCreditsTotal) * 100)
      : null;

  const chatPct =
    summary.addOnCreditsTotal != null && summary.addOnCreditsTotal > 0 && summary.addOnCreditsUsed != null
      ? clampPercent((summary.addOnCreditsUsed / summary.addOnCreditsTotal) * 100)
      : null;

  return {
    inlineSuggestionsUsedPercent: inlinePct,
    chatMessagesUsedPercent: chatPct,
    allowanceResetAt: summary.planEndsAt,
    remainingCompletions: summary.promptCreditsLeft,
    remainingChat: summary.addOnCredits,
    totalCompletions: summary.promptCreditsTotal,
    totalChat: summary.addOnCreditsTotal,
  };
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

export function formatKiroResetTime(resetAt: number | null | undefined, t: Translate): string {
  if (!resetAt || !Number.isFinite(resetAt)) {
    return t('common.shared.credits.planEndsUnknown', {
      defaultValue: '配额周期时间未知',
    });
  }
  const ts = Math.floor(resetAt);
  const now = Math.floor(Date.now() / 1000);
  const diff = ts - now;
  if (diff <= 0) {
    return t('common.shared.quota.resetDone', {
      defaultValue: '已重置',
    });
  }

  const totalMinutes = Math.floor(diff / 60);
  const days = Math.floor(totalMinutes / (60 * 24));
  const hours = Math.floor((totalMinutes % (60 * 24)) / 60);
  const minutes = totalMinutes % 60;

  let relative = t('common.shared.time.lessThanMinute', { defaultValue: '<1m' });
  if (days > 0 && hours > 0) {
    relative = t('common.shared.time.relativeDaysHours', {
      days,
      hours,
      defaultValue: '{{days}}d {{hours}}h',
    });
  } else if (days > 0) {
    relative = t('common.shared.time.relativeDays', {
      days,
      defaultValue: '{{days}}d',
    });
  } else if (hours > 0 && minutes > 0) {
    relative = t('common.shared.time.relativeHoursMinutes', {
      hours,
      minutes,
      defaultValue: '{{hours}}h {{minutes}}m',
    });
  } else if (hours > 0) {
    relative = t('common.shared.time.relativeHours', {
      hours,
      defaultValue: '{{hours}}h',
    });
  } else if (minutes > 0) {
    relative = t('common.shared.time.relativeMinutes', {
      minutes,
      defaultValue: '{{minutes}}m',
    });
  }

  const absolute = formatKiroResetTimeAbsolute(ts);
  return t('common.shared.time.relativeWithAbsolute', {
    relative,
    absolute,
    defaultValue: '{{relative}} ({{absolute}})',
  });
}

function formatKiroResetTimeAbsolute(resetTime: number): string {
  const date = new Date(resetTime * 1000);
  if (Number.isNaN(date.getTime())) return '';
  const pad = (value: number) => String(value).padStart(2, '0');
  const month = pad(date.getMonth() + 1);
  const day = pad(date.getDate());
  const hours = pad(date.getHours());
  const minutes = pad(date.getMinutes());
  return `${month}/${day} ${hours}:${minutes}`;
}
