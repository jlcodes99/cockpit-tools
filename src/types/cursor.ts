export interface CursorAccount {
  id: string;
  email: string;
  auth_id?: string | null;
  name?: string | null;
  tags?: string[] | null;

  access_token: string;
  refresh_token?: string | null;

  membership_type?: string | null;
  subscription_status?: string | null;
  sign_up_type?: string | null;

  cursor_auth_raw?: unknown;
  cursor_usage_raw?: unknown;

  status?: string | null;
  status_reason?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;

  created_at: number;
  last_used: number;

  plan_type?: string;
  quota?: CursorQuota;
}

export interface CursorQuota {
  hourly_percentage: number;
  hourly_reset_time?: number | null;
  weekly_percentage: number;
  weekly_reset_time?: number | null;
  raw_data?: unknown;
}

export type CursorPlanBadge =
  | 'FREE'
  | 'PRO'
  | 'PRO_PLUS'
  | 'ENTERPRISE'
  | 'FREE_TRIAL'
  | 'ULTRA'
  | 'UNKNOWN';

function normalizeCursorMembershipType(membershipType?: string | null): string {
  const normalized = (membershipType || '').toLowerCase().trim();
  if (!normalized) return '';
  if (normalized === 'business' || normalized === 'team') return 'enterprise';
  return normalized;
}

function getCursorAuthRawObject(account: CursorAccount): Record<string, unknown> | null {
  if (!account.cursor_auth_raw || typeof account.cursor_auth_raw !== 'object') {
    return null;
  }
  return account.cursor_auth_raw as Record<string, unknown>;
}

function getCursorAuthRawString(
  account: CursorAccount,
  ...keys: string[]
): string | null {
  const raw = getCursorAuthRawObject(account);
  if (!raw) return null;
  for (const key of keys) {
    const value = raw[key];
    if (typeof value === 'string') {
      const trimmed = value.trim();
      if (trimmed) return trimmed;
    }
  }
  return null;
}

function parseBoolLike(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    const normalized = value.toLowerCase().trim();
    if (normalized === 'true') return true;
    if (normalized === 'false') return false;
  }
  return null;
}

function getCursorAuthRawBool(
  account: CursorAccount,
  ...keys: string[]
): boolean | null {
  const raw = getCursorAuthRawObject(account);
  if (!raw) return null;
  for (const key of keys) {
    const parsed = parseBoolLike(raw[key]);
    if (parsed !== null) return parsed;
  }
  return null;
}

function isCursorEnterpriseAccount(account: CursorAccount): boolean {
  const explicitEnterprise = getCursorAuthRawBool(
    account,
    'isEnterprise',
    'is_enterprise',
  );
  if (explicitEnterprise !== null) {
    return explicitEnterprise;
  }

  const teamMembershipType = getCursorAuthRawString(
    account,
    'teamMembershipType',
    'team_membership_type',
  );
  if (teamMembershipType) {
    const normalizedTeam = teamMembershipType.toLowerCase();
    if (normalizedTeam.includes('enterprise')) {
      return true;
    }
    if (
      normalizedTeam.includes('self_serve') ||
      normalizedTeam.includes('selfserve')
    ) {
      return false;
    }
  }

  const isTeamMember = getCursorAuthRawBool(
    account,
    'isTeamMember',
    'is_team_member',
  );
  if (isTeamMember !== null) {
    return !isTeamMember;
  }

  return false;
}

function resolveCursorPlanLabel(account: CursorAccount): string {
  const plan = getCursorPlanBadge(account);
  const subscriptionStatus = (account.subscription_status || '')
    .toLowerCase()
    .trim();
  const isTrialing = subscriptionStatus === 'trialing';

  switch (plan) {
    case 'ENTERPRISE':
      return isCursorEnterpriseAccount(account) ? 'Enterprise' : 'Team';
    case 'ULTRA':
      return 'Ultra';
    case 'PRO_PLUS':
      return isTrialing ? 'Pro+ Trial' : 'Pro+';
    case 'PRO':
      return isTrialing ? 'Pro Trial' : 'Pro';
    case 'FREE_TRIAL':
      return 'Pro Trial';
    case 'FREE':
      return 'Free';
    default:
      return 'Unknown';
  }
}

export function getCursorPlanBadge(account: CursorAccount): CursorPlanBadge {
  const membership = normalizeCursorMembershipType(account.membership_type);
  switch (membership) {
    case 'free':
      return 'FREE';
    case 'pro':
      return 'PRO';
    case 'pro_plus':
      return 'PRO_PLUS';
    case 'enterprise':
      return 'ENTERPRISE';
    case 'free_trial':
      return 'FREE_TRIAL';
    case 'ultra':
      return 'ULTRA';
    default:
      return membership ? (membership.toUpperCase() as CursorPlanBadge) : 'UNKNOWN';
  }
}

export function getCursorPlanDisplayName(account: CursorAccount): string {
  return resolveCursorPlanLabel(account);
}

export function getCursorPlanBadgeClass(
  planType?: string | null,
  account?: CursorAccount,
): string {
  const normalized = normalizeCursorMembershipType(planType);
  switch (normalized) {
    case 'ultra':
      return 'ultra';
    case 'enterprise':
      return account && !isCursorEnterpriseAccount(account) ? 'team' : 'enterprise';
    case 'pro_plus':
      return 'plus';
    case 'pro':
    case 'free_trial':
      return 'pro';
    case 'free':
      return 'free';
    default:
      return 'unknown';
  }
}

export function getCursorAccountDisplayEmail(account: CursorAccount): string {
  const email = account.email?.trim();
  if (email) return email;
  const name = account.name?.trim();
  if (name) return name;
  return account.id;
}

export type CursorUsage = {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null;
  planUsedCents?: number | null;
  planLimitCents?: number | null;
  totalPercentUsed?: number | null;
  autoPercentUsed?: number | null;
  apiPercentUsed?: number | null;
  onDemandUsedCents?: number | null;
  onDemandLimitCents?: number | null;
  teamOnDemandUsedCents?: number | null;
  teamOnDemandLimitCents?: number | null;
  onDemandEnabled?: boolean | null;
  onDemandLimitType?: string | null;
  isUnlimited?: boolean;
};

export type CursorOnDemandSummary = {
  isTeamLimit: boolean;
  usedCents: number;
  limitCents: number | null;
  hasFixedLimit: boolean;
  isUnlimited: boolean;
  isDisabled: boolean;
};

function getPath(obj: unknown, ...keys: string[]): unknown {
  let cur: unknown = obj;
  for (const k of keys) {
    if (cur == null || typeof cur !== 'object') return undefined;
    cur = (cur as Record<string, unknown>)[k];
  }
  return cur;
}

/** 从对象中取数字，支持多 key（camelCase + snake_case），API 可能返回任一种 */
function pickNumber(obj: unknown, ...candidateKeys: string[]): number | null {
  if (obj == null || typeof obj !== 'object') return null;
  const o = obj as Record<string, unknown>;
  for (const key of candidateKeys) {
    const val = o[key];
    if (val !== undefined && val !== null) {
      const n = typeof val === 'number' ? val : Number(val);
      if (Number.isFinite(n)) return n;
    }
  }
  return null;
}

function pickBoolean(obj: unknown, ...candidateKeys: string[]): boolean | null {
  if (obj == null || typeof obj !== 'object') return null;
  const o = obj as Record<string, unknown>;
  for (const key of candidateKeys) {
    const val = o[key];
    if (typeof val === 'boolean') return val;
    if (typeof val === 'string') {
      const normalized = val.toLowerCase().trim();
      if (normalized === 'true') return true;
      if (normalized === 'false') return false;
    }
  }
  return null;
}

export function getCursorUsage(account: CursorAccount): CursorUsage {
  const raw = account.cursor_usage_raw;
  if (!raw || typeof raw !== 'object') {
    return { inlineSuggestionsUsedPercent: null, chatMessagesUsedPercent: null };
  }

  const plan =
    getPath(raw, 'individualUsage', 'plan') ??
    getPath(raw, 'individual_usage', 'plan') ??
    getPath(raw, 'planUsage') ??
    getPath(raw, 'plan_usage');
  const individualOnDemand =
    getPath(raw, 'individualUsage', 'onDemand') ??
    getPath(raw, 'individual_usage', 'onDemand');
  const teamOnDemand =
    getPath(raw, 'teamUsage', 'onDemand') ??
    getPath(raw, 'team_usage', 'onDemand');
  const spendLimitUsage =
    getPath(raw, 'spendLimitUsage') ??
    getPath(raw, 'spend_limit_usage');
  const onDemand = individualOnDemand ?? spendLimitUsage;

  const totalPct = pickNumber(plan, 'totalPercentUsed', 'total_percent_used');
  const autoPct = pickNumber(plan, 'autoPercentUsed', 'auto_percent_used');
  const apiPct = pickNumber(plan, 'apiPercentUsed', 'api_percent_used');
  const planUsed = pickNumber(plan, 'used', 'totalSpend', 'total_spend');
  const planLimit = pickNumber(plan, 'limit');
  const odUsed = pickNumber(
    onDemand,
    'used',
    'totalSpend',
    'total_spend',
    'individualUsed',
    'individual_used',
  );
  const odLimit = pickNumber(
    onDemand,
    'limit',
    'individualLimit',
    'individual_limit',
    'pooledLimit',
    'pooled_limit',
  );
  const teamOdUsed =
    pickNumber(teamOnDemand, 'used') ??
    pickNumber(
      spendLimitUsage,
      'pooledUsed',
      'pooled_used',
      'overallUsed',
      'overall_used',
    );
  const teamOdLimit =
    pickNumber(teamOnDemand, 'limit') ??
    pickNumber(
      spendLimitUsage,
      'pooledLimit',
      'pooled_limit',
      'overallLimit',
      'overall_limit',
    );
  const odEnabled = pickBoolean(individualOnDemand, 'enabled');
  const rawObj = raw as Record<string, unknown>;
  const isUnlimited =
    rawObj.isUnlimited === true || rawObj.is_unlimited === true;
  const limitTypeRaw =
    rawObj.limitType ??
    rawObj.limit_type ??
    (spendLimitUsage && typeof spendLimitUsage === 'object'
      ? (spendLimitUsage as Record<string, unknown>).limitType ??
        (spendLimitUsage as Record<string, unknown>).limit_type
      : undefined);
  const onDemandLimitType =
    typeof limitTypeRaw === 'string' && limitTypeRaw.trim()
      ? limitTypeRaw.trim().toLowerCase()
      : null;
  const billingEndRaw =
    rawObj.billingCycleEnd ?? rawObj.billing_cycle_end;
  let resetAt: number | null = null;
  if (typeof billingEndRaw === 'string' && billingEndRaw) {
    const ts = new Date(billingEndRaw).getTime();
    if (Number.isFinite(ts)) resetAt = Math.floor(ts / 1000);
  }

  const ratioPct =
    planUsed != null && planLimit != null && planLimit > 0
      ? (planUsed / planLimit) * 100
      : null;
  const totalBase = totalPct ?? ratioPct;
  const usedPct =
    totalBase == null
      ? null
      : totalBase > 0 && totalBase < 1
        ? 1
        : Math.min(100, Math.max(0, totalBase));

  return {
    inlineSuggestionsUsedPercent: usedPct,
    chatMessagesUsedPercent: null,
    allowanceResetAt: resetAt,
    planUsedCents: planUsed,
    planLimitCents: planLimit,
    totalPercentUsed: totalPct,
    autoPercentUsed: autoPct,
    apiPercentUsed: apiPct,
    onDemandUsedCents: odUsed,
    onDemandLimitCents: odLimit,
    teamOnDemandUsedCents: teamOdUsed,
    teamOnDemandLimitCents: teamOdLimit,
    onDemandEnabled: odEnabled,
    onDemandLimitType,
    isUnlimited,
  };
}

export function getCursorOnDemandSummary(usage: CursorUsage): CursorOnDemandSummary {
  const limitType = (usage.onDemandLimitType || '').toLowerCase();
  const isTeamLimit = limitType === 'team';
  // Team accounts must stay on one quota scope. Prefer team metrics and only
  // fall back to the normalized shared fields when the team-specific field is absent.
  const usedCents = isTeamLimit
    ? (usage.teamOnDemandUsedCents ?? usage.onDemandUsedCents ?? 0)
    : (usage.onDemandUsedCents ?? 0);
  const limitCents = isTeamLimit
    ? (usage.teamOnDemandLimitCents ?? usage.onDemandLimitCents ?? null)
    : (usage.onDemandLimitCents ?? null);
  const hasFixedLimit = limitCents != null && limitCents > 0;
  const isUnlimited = !hasFixedLimit && usage.onDemandEnabled === true && !isTeamLimit;
  const isDisabled = !hasFixedLimit && !isUnlimited;

  return {
    isTeamLimit,
    usedCents,
    limitCents,
    hasFixedLimit,
    isUnlimited,
    isDisabled,
  };
}

export function formatCursorUsageDollars(cents: number | null | undefined): string {
  if (cents == null) return '—';
  return `$${(cents / 100).toFixed(2)}`;
}

export function isCursorAccountBanned(account: CursorAccount): boolean {
  const status = (account.status || '').toLowerCase();
  const reason = (account.status_reason || '').toLowerCase();
  return status === 'banned' || status === 'forbidden' ||
    reason.includes('banned') || reason.includes('suspended') || reason.includes('disabled');
}

export function hasCursorQuotaData(account: CursorAccount): boolean {
  return account.cursor_usage_raw != null;
}
