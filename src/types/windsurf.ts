import {
  getPathValue,
  toNumber as getNumber,
  firstNumber as getNumberFromPaths,
  firstString as getStringFromPaths,
  firstRecord,
  normalizeTimestamp as parseTimestampSeconds,
} from '../utils/dataExtract';

/** Windsurf 账号数据（后端原样返回的结构） */
export interface WindsurfAccount {
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
  windsurf_api_key?: string | null;
  windsurf_api_server_url?: string | null;
  windsurf_auth_token?: string | null;
  windsurf_user_status?: unknown;
  windsurf_plan_status?: unknown;
  windsurf_auth_status_raw?: unknown;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;

  created_at: number;
  last_used: number;

  // ---- 兼容旧 UI（从 Codex 页面复制而来） ----
  // 这些字段不会由后端直接返回，需要在前端做映射/派生。
  email?: string;
  plan_type?: string;
  quota?: WindsurfQuota;
}

export type WindsurfQuotaClass = 'high' | 'medium' | 'low' | 'critical';
export type WindsurfPlanBadge =
  | 'FREE'
  | 'TRIAL'
  | 'INDIVIDUAL'
  | 'PRO'
  | 'PRO_ULTIMATE'
  | 'TEAMS'
  | 'TEAMS_ULTIMATE'
  | 'BUSINESS'
  | 'ENTERPRISE'
  | 'UNKNOWN';

const WINDSURF_TEAMS_TIER_BADGE_MAP: Record<number, WindsurfPlanBadge> = {
  1: 'TEAMS',
  2: 'PRO',
  3: 'ENTERPRISE',
  4: 'ENTERPRISE',
  5: 'ENTERPRISE',
  7: 'TEAMS_ULTIMATE',
  8: 'PRO_ULTIMATE',
  9: 'TRIAL',
  10: 'ENTERPRISE',
  11: 'ENTERPRISE',
  12: 'ENTERPRISE',
  14: 'PRO',
};

function normalizePlanToken(planType?: string | null): string {
  return (planType || '').trim().toUpperCase().replace(/[\s-]+/g, '_');
}

export function getWindsurfPlanDisplayName(planType?: string | null): string {
  const normalized = normalizePlanToken(planType);
  if (!normalized) return 'UNKNOWN';
  if (normalized.includes('TEAMS_ULTIMATE')) return 'TEAMS_ULTIMATE';
  if (normalized.includes('PRO_ULTIMATE')) return 'PRO_ULTIMATE';
  if (
    normalized === 'TRIAL' ||
    normalized === 'FREE_TRIAL' ||
    normalized.includes('_TRIAL')
  ) {
    return 'TRIAL';
  }
  if (normalized.includes('FREE')) return 'FREE';
  if (normalized.includes('INDIVIDUAL_PRO')) return 'PRO';
  if (normalized.endsWith('_PRO')) return 'PRO';
  if (normalized === 'PRO') return 'PRO';
  if (normalized.includes('INDIVIDUAL')) return 'INDIVIDUAL';
  if (normalized.includes('TEAMS')) return 'TEAMS';
  if (normalized.includes('BUSINESS')) return 'BUSINESS';
  if (normalized.includes('ENTERPRISE')) return 'ENTERPRISE';
  return normalized;
}

export function getWindsurfPlanLabel(planType?: string | null): string {
  const normalized = getWindsurfPlanDisplayName(planType);
  switch (normalized) {
    case 'FREE':
      return 'Free';
    case 'TRIAL':
      return 'Trial';
    case 'INDIVIDUAL':
      return 'Individual';
    case 'PRO':
      return 'Pro';
    case 'PRO_ULTIMATE':
      return 'Pro Ultimate';
    case 'TEAMS':
      return 'Teams';
    case 'TEAMS_ULTIMATE':
      return 'Teams Ultimate';
    case 'BUSINESS':
      return 'Business';
    case 'ENTERPRISE':
      return 'Enterprise';
    default: {
      const trimmed = planType?.trim();
      return trimmed || 'UNKNOWN';
    }
  }
}

export function getWindsurfPlanBadgeClass(planType?: string | null): string {
  const normalized = getWindsurfPlanDisplayName(planType);
  switch (normalized) {
    case 'FREE':
      return 'free';
    case 'TRIAL':
      return 'trial';
    case 'INDIVIDUAL':
      return 'individual';
    case 'PRO':
      return 'pro';
    case 'PRO_ULTIMATE':
      return 'pro-ultimate';
    case 'TEAMS':
      return 'teams';
    case 'TEAMS_ULTIMATE':
      return 'teams-ultimate';
    case 'BUSINESS':
      return 'business';
    case 'ENTERPRISE':
      return 'enterprise';
    default:
      return 'unknown';
  }
}

function formatStoredPlanLabel(planType?: string | null): string | null {
  const trimmed = planType?.trim();
  if (!trimmed) return null;

  const looksMachineReadable =
    trimmed.includes('_') ||
    trimmed === trimmed.toUpperCase() ||
    (trimmed === trimmed.toLowerCase() && !/[\s-]/.test(trimmed));

  return looksMachineReadable ? getWindsurfPlanLabel(trimmed) : trimmed;
}

function resolvePlanFromSku(sku: string): WindsurfPlanBadge | null {
  const lower = sku.toLowerCase();
  if (!lower) return null;
  if (lower.includes('teams_ultimate')) return 'TEAMS_ULTIMATE';
  if (lower.includes('pro_ultimate')) return 'PRO_ULTIMATE';
  if (lower.includes('trial')) return 'TRIAL';
  if (lower.includes('free_limited') || lower.includes('no_auth_limited')) return 'FREE';
  if (lower === 'free' || lower === 'windsurf') return 'FREE';
  if (lower.includes('enterprise')) return 'ENTERPRISE';
  if (lower.includes('business')) return 'BUSINESS';
  if (lower.includes('teams')) return 'TEAMS';
  if (lower.includes('individual_pro') || lower === 'pro' || lower.includes('_pro')) return 'PRO';
  if (lower.includes('individual')) return 'INDIVIDUAL';
  return null;
}

function mapPlanNameToBadge(planName?: string | null): WindsurfPlanBadge {
  const normalizedPlan = getWindsurfPlanDisplayName(planName);
  switch (normalizedPlan) {
    case 'FREE':
      return 'FREE';
    case 'TRIAL':
      return 'TRIAL';
    case 'PRO':
      return 'PRO';
    case 'PRO_ULTIMATE':
      return 'PRO_ULTIMATE';
    case 'INDIVIDUAL':
      return 'INDIVIDUAL';
    case 'TEAMS':
      return 'TEAMS';
    case 'TEAMS_ULTIMATE':
      return 'TEAMS_ULTIMATE';
    case 'BUSINESS':
      return 'BUSINESS';
    case 'ENTERPRISE':
      return 'ENTERPRISE';
    default:
      return 'UNKNOWN';
  }
}

function getNormalizedNumberFromPaths(root: unknown, paths: string[][]): number | null {
  return normalizeProtoCreditsValue(getNumberFromPaths(root, paths));
}

function resolveWindsurfPlanStatus(account: WindsurfAccount): Record<string, unknown> | null {
  return firstRecord([
    getPathValue(account.windsurf_plan_status, ['planStatus']),
    account.windsurf_plan_status,
    getPathValue(account.copilot_quota_snapshots, ['windsurfPlanStatus', 'planStatus']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfPlanStatus']),
    getPathValue(account.windsurf_user_status, ['userStatus', 'planStatus']),
    getPathValue(account.windsurf_user_status, ['planStatus']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfUserStatus', 'userStatus', 'planStatus']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfUserStatus', 'planStatus']),
  ]);
}

function resolveWindsurfPlanInfo(
  account: WindsurfAccount,
  planStatus: Record<string, unknown> | null,
): Record<string, unknown> | null {
  return firstRecord([
    getPathValue(planStatus, ['planInfo']),
    getPathValue(account.windsurf_plan_status, ['planStatus', 'planInfo']),
    getPathValue(account.windsurf_plan_status, ['planInfo']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfPlanInfo']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfPlanStatus', 'planStatus', 'planInfo']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfPlanStatus', 'planInfo']),
    getPathValue(account.windsurf_user_status, ['userStatus', 'planInfo']),
    getPathValue(account.windsurf_user_status, ['planInfo']),
    getPathValue(account.copilot_quota_snapshots, ['windsurfUserStatus', 'userStatus', 'planInfo']),
  ]);
}

function resolveWindsurfRemotePlanName(account: WindsurfAccount): string | null {
  const planStatus = resolveWindsurfPlanStatus(account);
  const planInfo = resolveWindsurfPlanInfo(account, planStatus);

  const directPlanName =
    getStringFromPaths(planInfo, [['planName'], ['plan_name'], ['name']]) ??
    getStringFromPaths(planStatus, [['planName'], ['plan_name']]);
  if (directPlanName) {
    return directPlanName.trim();
  }

  const teamsTierLabel = resolvePlanLabelFromTeamsTier(planInfo) ?? resolvePlanLabelFromTeamsTier(planStatus);
  return teamsTierLabel ?? null;
}

export function getWindsurfPlanBadge(account: WindsurfAccount): WindsurfPlanBadge {
  const tokenMap = parseTokenMap(account.copilot_token || '');
  const skuBadge = resolvePlanFromSku(tokenMap['sku'] || '');
  if (skuBadge) return skuBadge;

  const resolvedPlanBadge = mapPlanNameToBadge(getWindsurfResolvedPlanLabel(account));
  if (resolvedPlanBadge !== 'UNKNOWN') return resolvedPlanBadge;

  const directPlanBadge = mapPlanNameToBadge(account.copilot_plan ?? account.plan_type);
  if (directPlanBadge !== 'UNKNOWN') return directPlanBadge;

  return 'UNKNOWN';
}

function resolvePlanLabelFromTeamsTier(root: unknown): string | null {
  const teamsTierString = getStringFromPaths(root, [['teamsTier'], ['teams_tier']]);
  if (teamsTierString) {
    return getWindsurfPlanLabel(teamsTierString);
  }

  const teamsTierNumber = getNumberFromPaths(root, [['teamsTier'], ['teams_tier']]);
  if (teamsTierNumber == null) {
    return null;
  }

  const badge = WINDSURF_TEAMS_TIER_BADGE_MAP[Math.trunc(teamsTierNumber)];
  return badge ? getWindsurfPlanLabel(badge) : null;
}

export function getWindsurfResolvedPlanLabel(account: WindsurfAccount): string | null {
  const protoSummary = parseWindsurfProtoSummary(account);
  const candidates = [
    resolveWindsurfRemotePlanName(account),
    protoSummary?.planName?.trim() ?? null,
    formatStoredPlanLabel(account.copilot_plan),
    formatStoredPlanLabel(account.plan_type),
  ];
  let unknownCandidate: string | null = null;

  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (!trimmed) continue;
    if (getWindsurfPlanDisplayName(trimmed) === 'UNKNOWN') {
      unknownCandidate ??= trimmed;
      continue;
    }
    return getWindsurfPlanLabel(trimmed);
  }

  return unknownCandidate ? getWindsurfPlanLabel(unknownCandidate) : null;
}

export function getWindsurfQuotaClass(percentage: number): WindsurfQuotaClass {
  // Windsurf 页面展示的是“使用量”：使用越高，风险颜色越高。
  if (percentage <= 20) return 'high';
  if (percentage <= 60) return 'medium';
  if (percentage <= 85) return 'low';
  return 'critical';
}

export function getWindsurfAccountDisplayEmail(account: WindsurfAccount): string {
  const githubEmail = account.github_email?.trim();
  if (githubEmail) return githubEmail;

  const protoSummary = parseWindsurfProtoSummary(account);
  const protoEmail = protoSummary?.email?.trim();
  if (protoEmail) return protoEmail;

  const remoteEmail =
    getStringFromPaths(account.windsurf_user_status, [['userStatus', 'email'], ['email']]) ??
    getStringFromPaths(account.windsurf_auth_status_raw, [['email']]) ??
    getStringFromPaths(account.copilot_quota_snapshots, [
      ['windsurfCurrentUser', 'email'],
      ['windsurfUserStatus', 'userStatus', 'email'],
      ['windsurfUserStatus', 'email'],
    ]);

  return remoteEmail || account.github_login;
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

export type WindsurfUsage = {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null; // unix seconds
  remainingCompletions?: number | null;
  remainingChat?: number | null;
  totalCompletions?: number | null;
  totalChat?: number | null;
};

export type WindsurfCreditsSummary = {
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
};

export type WindsurfOfficialUsageMode = 'credits' | 'quota';

export type WindsurfQuotaUsageSummary = {
  dailyUsedPercent: number | null;
  weeklyUsedPercent: number | null;
  dailyResetAt: number | null;
  weeklyResetAt: number | null;
  overageBalanceMicros: number | null;
};

/** 兼容 Codex 风格的 quota 结构（用于复用 UI 组件/样式） */
export interface WindsurfQuota {
  hourly_percentage: number;
  hourly_reset_time?: number | null;
  weekly_percentage: number;
  weekly_reset_time?: number | null;
  raw_data?: unknown;
}

function parseTokenMap(token: string): Record<string, string> {
  const map: Record<string, string> = {};
  // Windsurf synthetic tokens contain ";sku=" and may have colons in values (e.g. rd=timestamp:0),
  // so skip the colon-based prefix split for them to avoid losing sku/source fields.
  const tokenStr = token.includes(';sku=') ? token : (token.split(':')[0] ?? token);
  for (const part of tokenStr.split(';')) {
    const eqIdx = part.indexOf('=');
    if (eqIdx < 0) continue;
    const key = part.substring(0, eqIdx).trim();
    if (!key) continue;
    map[key] = part.substring(eqIdx + 1).trim();
  }
  return map;
}

function isFreeLimitedSku(account: WindsurfAccount, tokenMap: Record<string, string>): boolean {
  const sku = (tokenMap['sku'] || '').toLowerCase();
  if (sku.includes('free_limited')) return true;
  const plan = (account.copilot_plan || '').toLowerCase();
  return plan.includes('free_limited');
}

function getPremiumQuotaSnapshot(account: WindsurfAccount): Record<string, unknown> | null {
  const raw = account.copilot_quota_snapshots as unknown;
  if (!raw || typeof raw !== 'object') return null;
  const snapshots = raw as Record<string, unknown>;

  const premiumInteractions = snapshots['premium_interactions'];
  if (premiumInteractions && typeof premiumInteractions === 'object') {
    return premiumInteractions as Record<string, unknown>;
  }

  const premiumModels = snapshots['premium_models'];
  if (premiumModels && typeof premiumModels === 'object') {
    return premiumModels as Record<string, unknown>;
  }

  return null;
}

// getNumber is now imported from dataExtract as toNumber

type WindsurfProtoSummary = {
  name: string | null;
  email: string | null;
  planName: string | null;
  planStartsAt: number | null;
  planEndsAt: number | null;
  promptCreditsLeft: number | null;
  promptCreditsUsed: number | null;
  promptCreditsTotal: number | null;
  addOnCreditsLeft: number | null;
  addOnCreditsUsed: number | null;
  addOnCreditsTotal: number | null;
  dailyQuotaUsedPercent: number | null;
  weeklyQuotaUsedPercent: number | null;
  overageBalanceMicros: number | null;
  dailyQuotaResetAt: number | null;
  weeklyQuotaResetAt: number | null;
  topUpEnabled: boolean | null;
};

function decodeBase64ToBytes(value: string): Uint8Array | null {
  if (!value) return null;
  try {
    if (typeof atob === 'function') {
      const binary = atob(value);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i += 1) {
        bytes[i] = binary.charCodeAt(i);
      }
      return bytes;
    }
  } catch {
    // fallback below
  }

  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const maybeBuffer = (globalThis as any)?.Buffer;
    if (maybeBuffer?.from) {
      return Uint8Array.from(maybeBuffer.from(value, 'base64'));
    }
  } catch {
    // ignore
  }
  return null;
}

function decodeUtf8(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder('utf-8', { fatal: false }).decode(bytes);
  } catch {
    try {
      return String.fromCharCode(...Array.from(bytes));
    } catch {
      return null;
    }
  }
}

function readProtoVarint(
  bytes: Uint8Array,
  offset: number,
): { value: number; nextOffset: number } | null {
  let result = 0;
  let shift = 0;
  let pos = offset;
  while (pos < bytes.length && shift < 53) {
    const byte = bytes[pos];
    result += (byte & 0x7f) * 2 ** shift;
    pos += 1;
    if ((byte & 0x80) === 0) {
      return { value: result, nextOffset: pos };
    }
    shift += 7;
  }
  return null;
}

function readProtoLengthDelimited(
  bytes: Uint8Array,
  offset: number,
): { value: Uint8Array; nextOffset: number } | null {
  const lengthInfo = readProtoVarint(bytes, offset);
  if (!lengthInfo) return null;
  const length = Math.max(0, Math.floor(lengthInfo.value));
  const start = lengthInfo.nextOffset;
  const end = start + length;
  if (end > bytes.length) return null;
  return {
    value: bytes.slice(start, end),
    nextOffset: end,
  };
}

function skipProtoField(bytes: Uint8Array, wireType: number, offset: number): number | null {
  if (wireType === 0) {
    return readProtoVarint(bytes, offset)?.nextOffset ?? null;
  }
  if (wireType === 1) {
    const next = offset + 8;
    return next <= bytes.length ? next : null;
  }
  if (wireType === 2) {
    return readProtoLengthDelimited(bytes, offset)?.nextOffset ?? null;
  }
  if (wireType === 5) {
    const next = offset + 4;
    return next <= bytes.length ? next : null;
  }
  return null;
}

function parseProtoTimestampSecondsFromMessage(bytes: Uint8Array): number | null {
  let offset = 0;
  while (offset < bytes.length) {
    const tagInfo = readProtoVarint(bytes, offset);
    if (!tagInfo) break;
    const fieldNo = Math.floor(tagInfo.value / 8);
    const wireType = tagInfo.value & 0x7;
    if (fieldNo === 1 && wireType === 0) {
      const value = readProtoVarint(bytes, tagInfo.nextOffset);
      if (!value) return null;
      return Math.floor(value.value);
    }
    const nextOffset = skipProtoField(bytes, wireType, tagInfo.nextOffset);
    if (nextOffset == null) break;
    offset = nextOffset;
  }
  return null;
}

function normalizeProtoCreditsValue(value: number | null): number | null {
  if (value == null || !Number.isFinite(value)) return null;
  if (value >= 1000) return value / 100;
  if (value >= 100 && value % 100 === 0) return value / 100;
  return value;
}

function parseWindsurfProtoSummary(account: WindsurfAccount): WindsurfProtoSummary | null {
  const base64Value = getStringFromPaths(account.windsurf_auth_status_raw, [
    ['userStatusProtoBinaryBase64'],
    ['userStatusProtoBinary'],
  ]);
  if (!base64Value) return null;

  const bytes = decodeBase64ToBytes(base64Value);
  if (!bytes || bytes.length === 0) return null;

  let name: string | null = null;
  let email: string | null = null;
  let planStatusBytes: Uint8Array | null = null;

  let offset = 0;
  while (offset < bytes.length) {
    const tagInfo = readProtoVarint(bytes, offset);
    if (!tagInfo) break;
    const fieldNo = Math.floor(tagInfo.value / 8);
    const wireType = tagInfo.value & 0x7;
    if (wireType === 2) {
      const valueInfo = readProtoLengthDelimited(bytes, tagInfo.nextOffset);
      if (!valueInfo) break;
      if (fieldNo === 3) {
        name = decodeUtf8(valueInfo.value)?.trim() || null;
      } else if (fieldNo === 7) {
        email = decodeUtf8(valueInfo.value)?.trim() || null;
      } else if (fieldNo === 13) {
        planStatusBytes = valueInfo.value;
      }
      offset = valueInfo.nextOffset;
      continue;
    }
    const nextOffset = skipProtoField(bytes, wireType, tagInfo.nextOffset);
    if (nextOffset == null) break;
    offset = nextOffset;
  }

  let planName: string | null = null;
  let planStartsAt: number | null = null;
  let planEndsAt: number | null = null;
  let promptCreditsLeft: number | null = null;
  let promptCreditsUsed: number | null = null;
  let promptCreditsTotal: number | null = null;
  let addOnCreditsLeft: number | null = null;
  let addOnCreditsUsed: number | null = null;
  let addOnCreditsTotal: number | null = null;
  let dailyQuotaUsedPercent: number | null = null;
  let weeklyQuotaUsedPercent: number | null = null;
  let overageBalanceMicros: number | null = null;
  let dailyQuotaResetAt: number | null = null;
  let weeklyQuotaResetAt: number | null = null;
  let topUpEnabled: boolean | null = null;

  if (planStatusBytes) {
    let planInfoBytes: Uint8Array | null = null;
    let topUpStatusBytes: Uint8Array | null = null;
    let promptAvailableFromPlanStatus: number | null = null;
    let promptUsedFromPlanStatus: number | null = null;
    let flexAvailableFromPlanStatus: number | null = null;
    let flexUsedFromPlanStatus: number | null = null;

    let planStatusOffset = 0;
    while (planStatusOffset < planStatusBytes.length) {
      const tagInfo = readProtoVarint(planStatusBytes, planStatusOffset);
      if (!tagInfo) break;
      const fieldNo = Math.floor(tagInfo.value / 8);
      const wireType = tagInfo.value & 0x7;

      if (fieldNo === 8 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        promptAvailableFromPlanStatus = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 6 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        promptUsedFromPlanStatus = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 4 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        flexAvailableFromPlanStatus = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 7 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        flexUsedFromPlanStatus = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 3 && wireType === 2) {
        const valueInfo = readProtoLengthDelimited(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        planEndsAt = parseProtoTimestampSecondsFromMessage(valueInfo.value);
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 2 && wireType === 2) {
        const valueInfo = readProtoLengthDelimited(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        planStartsAt = parseProtoTimestampSecondsFromMessage(valueInfo.value);
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 1 && wireType === 2) {
        const valueInfo = readProtoLengthDelimited(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        planInfoBytes = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 10 && wireType === 2) {
        const valueInfo = readProtoLengthDelimited(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        topUpStatusBytes = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 14 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        dailyQuotaUsedPercent = clampPercent(valueInfo.value);
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 15 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        weeklyQuotaUsedPercent = clampPercent(valueInfo.value);
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 16 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        overageBalanceMicros = valueInfo.value;
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 17 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        dailyQuotaResetAt = Math.floor(valueInfo.value);
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }
      if (fieldNo === 18 && wireType === 0) {
        const valueInfo = readProtoVarint(planStatusBytes, tagInfo.nextOffset);
        if (!valueInfo) break;
        weeklyQuotaResetAt = Math.floor(valueInfo.value);
        planStatusOffset = valueInfo.nextOffset;
        continue;
      }

      const nextOffset = skipProtoField(planStatusBytes, wireType, tagInfo.nextOffset);
      if (nextOffset == null) break;
      planStatusOffset = nextOffset;
    }

    if (topUpStatusBytes) {
      let topUpOffset = 0;
      while (topUpOffset < topUpStatusBytes.length) {
        const tagInfo = readProtoVarint(topUpStatusBytes, topUpOffset);
        if (!tagInfo) break;
        const fieldNo = Math.floor(tagInfo.value / 8);
        const wireType = tagInfo.value & 0x7;

        if (fieldNo === 2 && wireType === 0) {
          const valueInfo = readProtoVarint(topUpStatusBytes, tagInfo.nextOffset);
          if (!valueInfo) break;
          topUpEnabled = valueInfo.value !== 0;
          topUpOffset = valueInfo.nextOffset;
          continue;
        }

        const nextOffset = skipProtoField(topUpStatusBytes, wireType, tagInfo.nextOffset);
        if (nextOffset == null) break;
        topUpOffset = nextOffset;
      }
    }

    let monthlyPromptCreditsFromPlanInfo: number | null = null;
    let monthlyFlexCreditsFromPlanInfo: number | null = null;
    if (planInfoBytes) {
      let planInfoOffset = 0;
      while (planInfoOffset < planInfoBytes.length) {
        const tagInfo = readProtoVarint(planInfoBytes, planInfoOffset);
        if (!tagInfo) break;
        const fieldNo = Math.floor(tagInfo.value / 8);
        const wireType = tagInfo.value & 0x7;

        if (wireType === 2 && fieldNo === 2) {
          const valueInfo = readProtoLengthDelimited(planInfoBytes, tagInfo.nextOffset);
          if (!valueInfo) break;
          planName = decodeUtf8(valueInfo.value)?.trim() || null;
          planInfoOffset = valueInfo.nextOffset;
          continue;
        }
        if (wireType === 0 && fieldNo === 12) {
          const valueInfo = readProtoVarint(planInfoBytes, tagInfo.nextOffset);
          if (!valueInfo) break;
          monthlyPromptCreditsFromPlanInfo = valueInfo.value;
          planInfoOffset = valueInfo.nextOffset;
          continue;
        }
        if (wireType === 0 && fieldNo === 14) {
          const valueInfo = readProtoVarint(planInfoBytes, tagInfo.nextOffset);
          if (!valueInfo) break;
          monthlyFlexCreditsFromPlanInfo = valueInfo.value;
          planInfoOffset = valueInfo.nextOffset;
          continue;
        }

        const nextOffset = skipProtoField(planInfoBytes, wireType, tagInfo.nextOffset);
        if (nextOffset == null) break;
        planInfoOffset = nextOffset;
      }
    }

    const promptAvailable = normalizeProtoCreditsValue(
      promptAvailableFromPlanStatus ?? monthlyPromptCreditsFromPlanInfo,
    );
    promptCreditsUsed = normalizeProtoCreditsValue(promptUsedFromPlanStatus);
    // availablePromptCredits = total monthly quota, NOT remaining
    // total = monthly quota; remaining = total - used
    promptCreditsTotal =
      normalizeProtoCreditsValue(monthlyPromptCreditsFromPlanInfo) ??
      promptAvailable;
    promptCreditsLeft =
      promptCreditsTotal != null && promptCreditsUsed != null
        ? Math.max(0, promptCreditsTotal - promptCreditsUsed)
        : promptAvailable;

    const addOnAvailable = normalizeProtoCreditsValue(
      flexAvailableFromPlanStatus ?? monthlyFlexCreditsFromPlanInfo,
    );
    addOnCreditsUsed = normalizeProtoCreditsValue(flexUsedFromPlanStatus);
    addOnCreditsTotal =
      normalizeProtoCreditsValue(monthlyFlexCreditsFromPlanInfo) ??
      addOnAvailable;
    addOnCreditsLeft =
      addOnCreditsTotal != null && addOnCreditsUsed != null
        ? Math.max(0, addOnCreditsTotal - addOnCreditsUsed)
        : addOnAvailable;
  }

  return {
    name,
    email,
    planName,
    planStartsAt,
    planEndsAt,
    promptCreditsLeft,
    promptCreditsUsed,
    promptCreditsTotal,
    addOnCreditsLeft,
    addOnCreditsUsed,
    addOnCreditsTotal,
    dailyQuotaUsedPercent,
    weeklyQuotaUsedPercent,
    overageBalanceMicros,
    dailyQuotaResetAt,
    weeklyQuotaResetAt,
    topUpEnabled,
  };
}

function getLimitedQuota(account: WindsurfAccount, key: 'chat' | 'completions'): number | null {
  const raw = account.copilot_limited_user_quotas as any;
  if (!raw || typeof raw !== 'object') return null;
  return getNumber(raw[key]);
}

function pickAllowanceResetAt(account: WindsurfAccount): number | null {
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
  // windsurf 版接收 non-null number，直接处理
  if (value < 0) return 0;
  if (value > 100) return 100;
  return Math.round(value);
}

function calcUsedPercentFromRemainingPercent(remainingPercent: number | null): number | null {
  if (remainingPercent == null) return null;
  return clampPercent(100 - remainingPercent);
}

function calcUsedPercent(total: number | null, remaining: number | null): number | null {
  if (total == null || remaining == null) return null;
  if (total <= 0) return null;
  // remaining 可能会大于 total（异常/不同计划），这里做一个宽松处理
  const used = Math.max(0, total - remaining);
  return clampPercent((used / total) * 100);
}

function calcUsedPercentFromPremiumSnapshot(snapshot: Record<string, unknown>): number | null {
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

function calcRemainingFromPremiumSnapshot(snapshot: Record<string, unknown>): number | null {
  const entitlement = getNumber(snapshot['entitlement']);
  const percentRemaining = getNumber(snapshot['percent_remaining']);
  if (entitlement == null || percentRemaining == null || entitlement <= 0) return null;
  return Math.max(0, Math.round((entitlement * percentRemaining) / 100));
}

function sumFiniteNumbers(...values: Array<number | null | undefined>): number | null {
  let total = 0;
  let hasValue = false;
  values.forEach((value) => {
    if (typeof value === 'number' && Number.isFinite(value)) {
      total += value;
      hasValue = true;
    }
  });
  return hasValue ? total : null;
}

export function getWindsurfUsage(account: WindsurfAccount): WindsurfUsage {
  const tokenMap = parseTokenMap(account.copilot_token || '');
  const freeLimited = isFreeLimitedSku(account, tokenMap);

  // 与 VS Code 扩展口径对齐：付费用户优先使用 quota_snapshots.premium_interactions。
  if (!freeLimited) {
    const premiumSnapshot = getPremiumQuotaSnapshot(account);
    if (premiumSnapshot) {
      const usedPercent = calcUsedPercentFromPremiumSnapshot(premiumSnapshot);
      const entitlement = getNumber(premiumSnapshot['entitlement']);
      const remaining = calcRemainingFromPremiumSnapshot(premiumSnapshot);

      return {
        inlineSuggestionsUsedPercent: usedPercent,
        chatMessagesUsedPercent: usedPercent,
        allowanceResetAt: pickAllowanceResetAt(account),
        remainingCompletions: remaining,
        remainingChat: remaining,
        totalCompletions: entitlement,
        totalChat: entitlement,
      };
    }
  }

  const remainingCompletions = getLimitedQuota(account, 'completions');
  const remainingChat = getLimitedQuota(account, 'chat');

  const totalCompletions = getNumber(tokenMap['cq']) ?? (remainingCompletions ?? null);
  // VS Code Windsurf Free Usage 的 chat 口径：
  // free_limited 账号一般按 500 总额度计算已用百分比。
  let totalChat = getNumber(tokenMap['tq']);
  if (totalChat == null) {
    if (freeLimited && remainingChat != null) {
      totalChat = 500;
    } else {
      totalChat = remainingChat ?? null;
    }
  }

  return {
    inlineSuggestionsUsedPercent: calcUsedPercent(totalCompletions, remainingCompletions),
    chatMessagesUsedPercent: calcUsedPercent(totalChat, remainingChat),
    allowanceResetAt: pickAllowanceResetAt(account),
    remainingCompletions,
    remainingChat,
    totalCompletions,
    totalChat,
  };
}

export function getWindsurfCreditsSummary(account: WindsurfAccount): WindsurfCreditsSummary {
  const usage = getWindsurfUsage(account);
  const planStatus = resolveWindsurfPlanStatus(account);
  const planInfo = resolveWindsurfPlanInfo(account, planStatus);
  const protoSummary = parseWindsurfProtoSummary(account);

  const promptCreditsLeft =
    getNormalizedNumberFromPaths(planStatus, [['availablePromptCredits'], ['available_prompt_credits']]) ??
    protoSummary?.promptCreditsLeft ??
    normalizeProtoCreditsValue(usage.remainingCompletions ?? null) ??
    null;
  const usedPromptCredits =
    getNormalizedNumberFromPaths(planStatus, [['usedPromptCredits'], ['used_prompt_credits']]) ??
    protoSummary?.promptCreditsUsed;

  const promptCreditsMonthlyTotal =
    getNormalizedNumberFromPaths(planInfo, [['monthlyPromptCredits'], ['monthly_prompt_credits']]) ??
    protoSummary?.promptCreditsTotal;

  // availablePromptCredits = total monthly quota (same as monthlyPromptCredits), NOT remaining
  // total = monthlyPromptCredits ?? available; remaining = total - used
  let promptCreditsTotal =
    promptCreditsMonthlyTotal ??
    normalizeProtoCreditsValue(usage.totalCompletions ?? null) ??
    promptCreditsLeft ??
    null;
  if (promptCreditsTotal == null && promptCreditsLeft != null) {
    promptCreditsTotal = promptCreditsLeft;
  }
  const promptCreditsUsed =
    usedPromptCredits ??
    (promptCreditsTotal != null && promptCreditsLeft != null
      ? Math.max(0, promptCreditsTotal - promptCreditsLeft)
      : promptCreditsLeft != null
      ? 0
      : null);
  // Compute true remaining = total - used
  const promptCreditsLeftActual =
    promptCreditsTotal != null && promptCreditsUsed != null
      ? Math.max(0, promptCreditsTotal - promptCreditsUsed)
      : promptCreditsLeft;

  const addOnCredits =
    getNormalizedNumberFromPaths(planStatus, [
      ['availableFlexCredits'],
      ['available_flex_credits'],
      ['flexCreditsAvailable'],
      ['flex_credits_available'],
      ['availableAddOnCredits'],
      ['available_add_on_credits'],
      ['addOnCreditsAvailable'],
      ['add_on_credits_available'],
      ['availableTopUpCredits'],
      ['available_top_up_credits'],
      ['topUpCreditsAvailable'],
      ['top_up_credits_available'],
    ]) ??
    protoSummary?.addOnCreditsLeft ??
    0;

  const usedAddOnCredits =
    getNormalizedNumberFromPaths(planStatus, [
      ['usedFlexCredits'],
      ['used_flex_credits'],
      ['usedAddOnCredits'],
      ['used_add_on_credits'],
      ['usedTopUpCredits'],
      ['used_top_up_credits'],
    ]) ?? protoSummary?.addOnCreditsUsed;

  const addOnCreditsMonthlyTotal =
    getNormalizedNumberFromPaths(planInfo, [
      ['monthlyFlexCreditPurchaseAmount'],
      ['monthly_flex_credit_purchase_amount'],
      ['monthlyAddOnCredits'],
      ['monthly_add_on_credits'],
      ['monthlyTopUpCredits'],
      ['monthly_top_up_credits'],
    ]) ?? protoSummary?.addOnCreditsTotal;

  // Same fix for add-on credits
  let addOnCreditsTotal =
    addOnCreditsMonthlyTotal ??
    addOnCredits ??
    null;
  if (addOnCreditsTotal != null && addOnCredits != null && addOnCreditsTotal < addOnCredits) {
    addOnCreditsTotal = addOnCredits;
  }
  const addOnCreditsUsed =
    usedAddOnCredits ??
    (addOnCreditsTotal != null && addOnCredits != null
      ? Math.max(0, addOnCreditsTotal - addOnCredits)
      : addOnCredits != null
      ? 0
      : null);
  const addOnCreditsLeftActual =
    addOnCreditsTotal != null && addOnCreditsUsed != null
      ? Math.max(0, addOnCreditsTotal - addOnCreditsUsed)
      : addOnCredits ?? null;

  const planStartsAt =
    parseTimestampSeconds(getPathValue(planStatus, ['planStart'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['plan_start'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['currentPeriodStart'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['current_period_start'])) ??
    protoSummary?.planStartsAt ??
    null;

  const planEndsAt =
    parseTimestampSeconds(getPathValue(planStatus, ['planEnd'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['plan_end'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['currentPeriodEnd'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['current_period_end'])) ??
    protoSummary?.planEndsAt ??
    parseTimestampSeconds(account.copilot_limited_user_reset_date) ??
    parseTimestampSeconds(account.copilot_quota_reset_date);

  return {
    planName: getWindsurfResolvedPlanLabel(account),
    creditsLeft: sumFiniteNumbers(promptCreditsLeftActual, addOnCreditsLeftActual),
    promptCreditsLeft: promptCreditsLeftActual,
    promptCreditsUsed,
    promptCreditsTotal,
    addOnCredits: addOnCreditsLeftActual,
    addOnCreditsUsed,
    addOnCreditsTotal,
    planStartsAt,
    planEndsAt,
  };
}

export function getWindsurfBillingStrategy(account: WindsurfAccount): string | null {
  const planStatus = resolveWindsurfPlanStatus(account);
  const planInfo = resolveWindsurfPlanInfo(account, planStatus);

  const strategy =
    getStringFromPaths(planStatus, [['billingStrategy'], ['billing_strategy']]) ??
    getStringFromPaths(planInfo, [['billingStrategy'], ['billing_strategy']]) ??
    getStringFromPaths(account.windsurf_plan_status, [
      ['planStatus', 'billingStrategy'],
      ['planStatus', 'billing_strategy'],
      ['billingStrategy'],
      ['billing_strategy'],
    ]) ??
    getStringFromPaths(account.windsurf_user_status, [
      ['userStatus', 'planStatus', 'billingStrategy'],
      ['userStatus', 'planStatus', 'billing_strategy'],
      ['planStatus', 'billingStrategy'],
      ['planStatus', 'billing_strategy'],
      ['billingStrategy'],
      ['billing_strategy'],
    ]) ??
    getStringFromPaths(account.copilot_quota_snapshots, [
      ['windsurfPlanStatus', 'billingStrategy'],
      ['windsurfPlanStatus', 'billing_strategy'],
      ['windsurfPlanStatus', 'planStatus', 'billingStrategy'],
      ['windsurfPlanStatus', 'planStatus', 'billing_strategy'],
      ['windsurfPlanInfo', 'billingStrategy'],
      ['windsurfPlanInfo', 'billing_strategy'],
      ['windsurfUserStatus', 'userStatus', 'planStatus', 'billingStrategy'],
      ['windsurfUserStatus', 'userStatus', 'planStatus', 'billing_strategy'],
    ]);

  const normalizedStrategy = strategy?.trim();
  if (!normalizedStrategy) {
    return null;
  }

  const canonicalStrategy = normalizedStrategy
    .replace(/^billing[_-]?strategy[_-]?/i, '')
    .trim()
    .toLowerCase();

  if (canonicalStrategy === 'quota') {
    return 'quota';
  }
  if (canonicalStrategy.includes('credit')) {
    return 'credits';
  }

  return canonicalStrategy || normalizedStrategy;
}

export function getWindsurfOfficialUsageMode(account: WindsurfAccount): WindsurfOfficialUsageMode {
  const billingStrategy = getWindsurfBillingStrategy(account)?.trim().toLowerCase();
  return billingStrategy === 'quota' ? 'quota' : 'credits';
}

export function getWindsurfQuotaUsageSummary(account: WindsurfAccount): WindsurfQuotaUsageSummary {
  const planStatus = resolveWindsurfPlanStatus(account);
  const isQuotaBilling = getWindsurfBillingStrategy(account)?.trim().toLowerCase() === 'quota';

  // 官方 planStatus 的 RemainingPercent 表示“剩余百分比”，
  // 页面展示需要转换成“已用百分比”。
  // Windsurf 官方 protobuf 解码后这两个字段默认值为 0，所以 quota 模式下
  // 缺失字段也会表现为“剩余 0 / 已用 100”。
  const dailyUsedPercent =
    (() => {
      const remainingPercent = getNumberFromPaths(planStatus, [
        ['dailyQuotaRemainingPercent'],
        ['daily_quota_remaining_percent'],
      ]);
      return calcUsedPercentFromRemainingPercent(remainingPercent);
    })() ??
    (isQuotaBilling ? 100 : null);

  const weeklyUsedPercent =
    (() => {
      const remainingPercent = getNumberFromPaths(planStatus, [
        ['weeklyQuotaRemainingPercent'],
        ['weekly_quota_remaining_percent'],
      ]);
      return calcUsedPercentFromRemainingPercent(remainingPercent);
    })() ??
    (isQuotaBilling ? 100 : null);

  const overageBalanceMicros =
    getNumberFromPaths(planStatus, [['overageBalanceMicros'], ['overage_balance_micros']]) ??
    null;

  const dailyResetAt =
    parseTimestampSeconds(getPathValue(planStatus, ['dailyQuotaResetAtUnix'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['daily_quota_reset_at_unix'])) ??
    null;

  const weeklyResetAt =
    parseTimestampSeconds(getPathValue(planStatus, ['weeklyQuotaResetAtUnix'])) ??
    parseTimestampSeconds(getPathValue(planStatus, ['weekly_quota_reset_at_unix'])) ??
    null;

  return {
    dailyUsedPercent,
    weeklyUsedPercent,
    dailyResetAt,
    weeklyResetAt,
    overageBalanceMicros,
  };
}

export function hasWindsurfQuotaData(account: WindsurfAccount): boolean {
  return (
    account.copilot_quota_snapshots != null ||
    account.copilot_limited_user_quotas != null ||
    account.windsurf_plan_status != null ||
    account.windsurf_user_status != null
  );
}

export function formatUnixSecondsToYmd(seconds: number, locale = 'zh-CN'): string {
  const date = new Date(seconds * 1000);
  if (Number.isNaN(date.getTime())) return '';
  return new Intl.DateTimeFormat(locale, { year: 'numeric', month: '2-digit', day: '2-digit' }).format(date);
}

export function formatWindsurfAllowanceResetLine(
  account: WindsurfAccount,
  t: Translate,
  locale = 'zh-CN',
): string {
  const usage = getWindsurfUsage(account);
  const resetAt = usage.allowanceResetAt;
  if (!resetAt) return t('common.shared.usage.resetUnknown', { defaultValue: 'Allowance resets -' });
  const dateText = formatUnixSecondsToYmd(resetAt, locale);
  if (!dateText) return t('common.shared.usage.resetUnknown', { defaultValue: 'Allowance resets -' });
  return t('common.shared.usage.resetLine', {
    dateText,
    defaultValue: 'Allowance resets {{dateText}}.',
  });
}

export function formatWindsurfResetTime(
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

  const absolute = formatWindsurfResetTimeAbsolute(resetTime);
  return t('common.shared.time.relativeWithAbsolute', {
    relative,
    absolute,
    defaultValue: '{{relative}} ({{absolute}})',
  });
}

export function formatWindsurfResetTimeAbsolute(resetTime: number | null | undefined): string {
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
