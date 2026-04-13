/** GitHub Copilot 账号数据（后端原样返回的结构） */
export interface GitHubCopilotAccount {
  id: string;
  github_login: string;
  github_id: number;
  github_name?: string | null;
  github_email?: string | null;
  tags?: string[] | null;

  // 注意：这里包含敏感信息。前端不应打印/上报。
  github_access_token: string;
  github_token_type?: string | null;
  github_scope?: string | null;
  copilot_token: string;

  copilot_plan?: string | null;
  copilot_chat_enabled?: boolean | null;
  copilot_expires_at?: number | null;
  copilot_refresh_in?: number | null;
  copilot_quota_snapshots?: unknown;
  copilot_quota_reset_date?: string | null;
  copilot_limited_user_quotas?: unknown;
  copilot_limited_user_reset_date?: number | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;

  created_at: number;
  last_used: number;

  // ---- 兼容旧 UI（从 Codex 页面复制而来） ----
  // 这些字段不会由后端直接返回，需要在前端做映射/派生。
  email?: string;
  plan_type?: string;
  quota?: GitHubCopilotQuota;
}

export type GitHubCopilotQuotaClass = 'high' | 'medium' | 'low' | 'critical';
export type GitHubCopilotPlanBadge = 'FREE' | 'INDIVIDUAL' | 'PRO' | 'BUSINESS' | 'ENTERPRISE' | 'UNKNOWN';

export function getGitHubCopilotPlanDisplayName(planType?: string | null): string {
  if (!planType) return 'UNKNOWN';
  const upper = planType.toUpperCase();
  if (upper.includes('FREE')) return 'FREE';
  if (upper.includes('INDIVIDUAL_PRO')) return 'PRO';
  if (upper === 'PRO') return 'PRO';
  // 与 VS Code 对齐：copilot_plan=individual 归为 Pro。
  if (upper.includes('INDIVIDUAL')) return 'PRO';
  if (upper.includes('BUSINESS')) return 'BUSINESS';
  if (upper.includes('ENTERPRISE')) return 'ENTERPRISE';
  return upper;
}

function resolvePlanFromSku(sku: string): GitHubCopilotPlanBadge | null {
  const lower = sku.toLowerCase();
  if (!lower) return null;
  if (lower.includes('free_limited') || lower.includes('no_auth_limited')) return 'FREE';
  if (lower.includes('enterprise')) return 'ENTERPRISE';
  if (lower.includes('business')) return 'BUSINESS';
  if (lower.includes('individual_pro') || lower === 'pro' || lower.includes('_pro')) return 'PRO';
  if (lower.includes('individual')) return 'PRO';
  return null;
}

export function getGitHubCopilotPlanBadge(account: GitHubCopilotAccount): GitHubCopilotPlanBadge {
  const tokenMap = parseTokenMap(account.copilot_token || '');
  const skuBadge = resolvePlanFromSku(tokenMap['sku'] || '');
  if (skuBadge) return skuBadge;

  const normalizedPlan = getGitHubCopilotPlanDisplayName(account.copilot_plan);
  switch (normalizedPlan) {
    case 'FREE':
      return 'FREE';
    case 'PRO':
      return 'PRO';
    case 'INDIVIDUAL':
      return 'PRO';
    case 'BUSINESS':
      return 'BUSINESS';
    case 'ENTERPRISE':
      return 'ENTERPRISE';
    default:
      return 'UNKNOWN';
  }
}

export function getGitHubCopilotQuotaClass(percentage: number): GitHubCopilotQuotaClass {
  // GitHub Copilot 页面展示的是“使用量”：使用越高，风险颜色越高。
  if (percentage <= 20) return 'high';
  if (percentage <= 60) return 'medium';
  if (percentage <= 85) return 'low';
  return 'critical';
}

export function getGitHubCopilotAccountDisplayEmail(account: GitHubCopilotAccount): string {
  return account.github_email?.trim() || account.github_login;
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

export type GitHubCopilotUsage = {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  premiumRequestsUsedPercent?: number | null;
  inlineIncluded?: boolean;
  chatIncluded?: boolean;
  premiumIncluded?: boolean;
  allowanceResetAt?: number | null; // unix seconds
  remainingCompletions?: number | null;
  remainingChat?: number | null;
  totalCompletions?: number | null;
  totalChat?: number | null;
};

/** 兼容 Codex 风格的 quota 结构（用于复用 UI 组件/样式） */
export interface GitHubCopilotQuota {
  hourly_percentage: number;
  hourly_reset_time?: number | null;
  weekly_percentage: number;
  weekly_reset_time?: number | null;
  raw_data?: unknown;
}

function parseTokenMap(token: string): Record<string, string> {
  const map: Record<string, string> = {};
  const prefix = token.split(':')[0] ?? token;
  for (const part of prefix.split(';')) {
    const [k, v] = part.split('=');
    const key = (k || '').trim();
    if (!key) continue;
    map[key] = (v || '').trim();
  }
  return map;
}

function isFreeLimitedSku(account: GitHubCopilotAccount, tokenMap: Record<string, string>): boolean {
  const sku = (tokenMap['sku'] || '').toLowerCase();
  if (sku.includes('free_limited')) return true;
  const plan = (account.copilot_plan || '').toLowerCase();
  return plan.includes('free_limited');
}

function getQuotaSnapshot(
  account: GitHubCopilotAccount,
  key: 'chat' | 'completions' | 'premium_interactions',
): Record<string, unknown> | null {
  const raw = account.copilot_quota_snapshots as unknown;
  if (!raw || typeof raw !== 'object') return null;
  const snapshots = raw as Record<string, unknown>;
  const snapshot = snapshots[key];
  if (snapshot && typeof snapshot === 'object') {
    return snapshot as Record<string, unknown>;
  }
  return null;
}

function getNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  }
  return null;
}

function getLimitedQuota(account: GitHubCopilotAccount, key: 'chat' | 'completions'): number | null {
  const raw = account.copilot_limited_user_quotas as any;
  if (!raw || typeof raw !== 'object') return null;
  return getNumber(raw[key]);
}

function pickAllowanceResetAt(account: GitHubCopilotAccount): number | null {
  if (typeof account.copilot_limited_user_reset_date === 'number') {
    return account.copilot_limited_user_reset_date;
  }
  if (typeof account.copilot_quota_reset_date === 'string' && account.copilot_quota_reset_date.trim()) {
    const parsed = Date.parse(account.copilot_quota_reset_date);
    if (Number.isFinite(parsed)) {
      return Math.floor(parsed / 1000);
    }
  }
  const tokenMap = parseTokenMap(account.copilot_token || '');
  const rd = tokenMap['rd'];
  if (rd) {
    const head = rd.split(':')[0];
    const n = Number(head);
    if (Number.isFinite(n)) return n;
  }
  return null;
}

function clampPercent(value: number): number {
  if (value < 0) return 0;
  if (value > 100) return 100;
  return Math.round(value);
}

function calcUsedPercent(total: number | null, remaining: number | null): number | null {
  if (total == null || remaining == null) return null;
  if (total <= 0) return null;
  // remaining 可能会大于 total（异常/不同计划），这里做一个宽松处理
  const used = Math.max(0, total - remaining);
  return clampPercent((used / total) * 100);
}

function calcUsedPercentFromSnapshot(snapshot: Record<string, unknown>): number | null {
  const unlimited = snapshot['unlimited'] === true;
  if (unlimited) return 0;

  const entitlement = getNumber(snapshot['entitlement']);
  if (entitlement != null && entitlement < 0) {
    return 0;
  }

  const percentRemaining = getNumber(snapshot['percent_remaining']);
  if (percentRemaining != null) {
    return clampPercent(100 - percentRemaining);
  }

  return null;
}

function isIncludedFromSnapshot(snapshot: Record<string, unknown> | null): boolean {
  if (!snapshot) return false;
  if (snapshot['unlimited'] === true) return true;
  const entitlement = getNumber(snapshot['entitlement']);
  return entitlement != null && entitlement < 0;
}

function calcRemainingFromSnapshot(snapshot: Record<string, unknown>): number | null {
  const remaining = getNumber(snapshot['remaining']);
  if (remaining != null) return remaining;

  const entitlement = getNumber(snapshot['entitlement']);
  const percentRemaining = getNumber(snapshot['percent_remaining']);
  if (entitlement == null || percentRemaining == null || entitlement <= 0) return null;
  return Math.max(0, Math.round((entitlement * percentRemaining) / 100));
}

export function getGitHubCopilotUsage(account: GitHubCopilotAccount): GitHubCopilotUsage {
  const tokenMap = parseTokenMap(account.copilot_token || '');
  const freeLimited = isFreeLimitedSku(account, tokenMap);

  const completionsSnapshot = getQuotaSnapshot(account, 'completions');
  const chatSnapshot = getQuotaSnapshot(account, 'chat');
  const premiumSnapshot = getQuotaSnapshot(account, 'premium_interactions');

  const snapshotInlineUsed =
    completionsSnapshot ? calcUsedPercentFromSnapshot(completionsSnapshot) : null;
  const snapshotChatUsed =
    chatSnapshot ? calcUsedPercentFromSnapshot(chatSnapshot) : null;
  const snapshotPremiumUsed =
    premiumSnapshot ? calcUsedPercentFromSnapshot(premiumSnapshot) : null;

  const inlineIncluded = isIncludedFromSnapshot(completionsSnapshot);
  const chatIncluded = isIncludedFromSnapshot(chatSnapshot);
  const premiumIncluded = isIncludedFromSnapshot(premiumSnapshot);

  const remainingCompletionsFromSnapshot = completionsSnapshot
    ? calcRemainingFromSnapshot(completionsSnapshot)
    : null;
  const remainingChatFromSnapshot = chatSnapshot
    ? calcRemainingFromSnapshot(chatSnapshot)
    : null;
  const remainingPremiumFromSnapshot = premiumSnapshot
    ? calcRemainingFromSnapshot(premiumSnapshot)
    : null;

  const remainingCompletions = remainingCompletionsFromSnapshot ?? getLimitedQuota(account, 'completions');
  const remainingChat = remainingChatFromSnapshot ?? getLimitedQuota(account, 'chat');
  const remainingPremium = remainingPremiumFromSnapshot;

  const totalCompletions =
    (completionsSnapshot ? getNumber(completionsSnapshot['entitlement']) : null) ??
    getNumber(tokenMap['cq']) ??
    (remainingCompletions ?? null);

  let totalChat =
    (chatSnapshot ? getNumber(chatSnapshot['entitlement']) : null) ??
    getNumber(tokenMap['tq']);
  // VS Code Copilot Free Usage 的 chat 口径：
  // free_limited 账号一般按 500 总额度计算已用百分比。
  if (totalChat == null) {
    if (freeLimited && remainingChat != null) {
      totalChat = 500;
    } else {
      totalChat = remainingChat ?? null;
    }
  }
  const totalPremium =
    (premiumSnapshot ? getNumber(premiumSnapshot['entitlement']) : null) ??
    (remainingPremium ?? null);

  const inlineUsedPercent =
    snapshotInlineUsed ?? calcUsedPercent(totalCompletions, remainingCompletions);
  const chatUsedPercent =
    snapshotChatUsed ?? calcUsedPercent(totalChat, remainingChat);
  const premiumUsedPercent =
    snapshotPremiumUsed ?? calcUsedPercent(totalPremium, remainingPremium);

  return {
    inlineSuggestionsUsedPercent: inlineUsedPercent,
    chatMessagesUsedPercent: chatUsedPercent,
    premiumRequestsUsedPercent: premiumUsedPercent,
    inlineIncluded,
    chatIncluded,
    premiumIncluded,
    allowanceResetAt: pickAllowanceResetAt(account),
    remainingCompletions,
    remainingChat,
    totalCompletions,
    totalChat,
  };
}

export function hasGitHubCopilotQuotaData(account: GitHubCopilotAccount): boolean {
  const usage = getGitHubCopilotUsage(account);
  return (
    account.copilot_quota_snapshots != null ||
    account.copilot_limited_user_quotas != null ||
    usage.inlineSuggestionsUsedPercent != null ||
    usage.chatMessagesUsedPercent != null ||
    usage.premiumRequestsUsedPercent != null ||
    usage.remainingCompletions != null ||
    usage.remainingChat != null ||
    usage.totalCompletions != null ||
    usage.totalChat != null
  );
}

export function formatUnixSecondsToYmd(seconds: number, locale = 'zh-CN'): string {
  const date = new Date(seconds * 1000);
  if (Number.isNaN(date.getTime())) return '';
  return new Intl.DateTimeFormat(locale, { year: 'numeric', month: '2-digit', day: '2-digit' }).format(date);
}

export function formatGitHubCopilotAllowanceResetLine(
  account: GitHubCopilotAccount,
  t: Translate,
  locale = 'zh-CN',
): string {
  const usage = getGitHubCopilotUsage(account);
  const resetAt = usage.allowanceResetAt;
  if (!resetAt) return t('common.shared.usage.resetUnknown', { defaultValue: 'Allowance resets -' });
  const dateText = formatUnixSecondsToYmd(resetAt, locale);
  if (!dateText) return t('common.shared.usage.resetUnknown', { defaultValue: 'Allowance resets -' });
  return t('common.shared.usage.resetLine', {
    dateText,
    defaultValue: 'Allowance resets {{dateText}}.',
  });
}

export function formatGitHubCopilotResetTime(
  resetTime: number | null | undefined,
  t: Translate,
): string {
  if (!resetTime) return '';
  const now = Math.floor(Date.now() / 1000);
  const diff = resetTime - now;
  if (diff <= 0) return t('common.shared.quota.resetDone', { defaultValue: '已重置' });

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

  const absolute = formatGitHubCopilotResetTimeAbsolute(resetTime);
  return t('common.shared.time.relativeWithAbsolute', {
    relative,
    absolute,
    defaultValue: '{{relative}} ({{absolute}})',
  });
}

export function formatGitHubCopilotResetTimeAbsolute(resetTime: number | null | undefined): string {
  if (!resetTime) return '';
  const date = new Date(resetTime * 1000);
  if (Number.isNaN(date.getTime())) return '';
  const pad = (value: number) => String(value).padStart(2, '0');
  const month = pad(date.getMonth() + 1);
  const day = pad(date.getDate());
  const hours = pad(date.getHours());
  const minutes = pad(date.getMinutes());
  return `${month}/${day} ${hours}:${minutes}`;
}
