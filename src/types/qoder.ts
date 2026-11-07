export interface QoderAccount {
  id: string;
  email: string;
  user_id?: string | null;
  display_name?: string | null;
  plan_type?: string | null;
  credits_used?: number | null;
  credits_total?: number | null;
  credits_remaining?: number | null;
  credits_usage_percent?: number | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  usage_updated_at?: number | null;
  tags?: string[] | null;
  auth_user_info_raw?: unknown;
  auth_user_plan_raw?: unknown;
  auth_credit_usage_raw?: unknown;
  created_at: number;
  last_used: number;
  last_switched_at?: number | null;
}

interface UnknownRecord {
  [key: string]: unknown;
}

export interface QoderUsage {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  allowanceResetAt?: number | null;
  creditsUsed: number | null;
  creditsTotal: number | null;
  creditsRemaining: number | null;
}

export interface QoderQuotaBucket {
  used: number | null;
  total: number | null;
  remaining: number | null;
  percentage: number | null;
}

export interface QoderSubscriptionInfo {
  planTag: string;
  userType: string | null;
  isPersonalVersion: boolean | null;
  isHighestTier: boolean;
  userQuota: QoderQuotaBucket;
  addOnQuota: QoderQuotaBucket;
  sharedCreditPackageUsed: number | null;
  totalUsagePercentage: number | null;
  expiresAt: number | null;
  detailUrl: string | null;
  upgradeUrl: string | null;
  addCreditsUrl: string | null;
}

export function shouldShowQoderSubscriptionReset(
  subscription: Pick<QoderSubscriptionInfo, 'expiresAt' | 'userType'>,
): boolean {
  return (
    subscription.expiresAt != null &&
    subscription.userType?.trim().toLowerCase() !== 'personal_standard'
  );
}

export interface QoderUsageOverview {
  planTag: string;
  usagePercent: number | null;
  creditsUsed: number | null;
  creditsTotal: number | null;
  creditsRemaining: number | null;
  unit: string;
  detailUrl: string | null;
  upgradeUrl: string | null;
  isQuotaExceeded: boolean;
}

const QODER_SENTINEL_EXPIRES_AT_MS = Date.UTC(9999, 11, 31, 0, 0, 0);

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function getNestedValue(root: unknown, path: string[]): unknown {
  let current: unknown = root;
  for (const key of path) {
    if (!isRecord(current)) return undefined;
    current = current[key];
  }
  return current;
}

function toNonEmptyString(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function toFiniteNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const parsed = Number(trimmed);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function toBoolean(value: unknown): boolean {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    return normalized === '1' || normalized === 'true';
  }
  if (typeof value === 'number') return value === 1;
  return false;
}

function toBooleanOrNull(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (!normalized) return null;
    if (normalized === '1' || normalized === 'true') return true;
    if (normalized === '0' || normalized === 'false') return false;
    return null;
  }
  if (typeof value === 'number') {
    if (value === 1) return true;
    if (value === 0) return false;
  }
  return null;
}

function firstNonEmptyString(...values: unknown[]): string | null {
  for (const value of values) {
    const normalized = toNonEmptyString(value);
    if (normalized) return normalized;
  }
  return null;
}

function firstFiniteNumber(...values: unknown[]): number | null {
  for (const value of values) {
    const normalized = toFiniteNumber(value);
    if (normalized != null) return normalized;
  }
  return null;
}

function firstBoolean(...values: unknown[]): boolean | null {
  for (const value of values) {
    const normalized = toBooleanOrNull(value);
    if (normalized != null) return normalized;
  }
  return null;
}

function firstRecord(...values: unknown[]): UnknownRecord | null {
  for (const value of values) {
    if (isRecord(value)) return value;
  }
  return null;
}

function clampPercent(value: number | null): number | null {
  if (value == null) return null;
  return Math.max(0, Math.min(100, value));
}

function normalizeTimestampMs(value: unknown): number | null {
  const normalized = toFiniteNumber(value);
  if (normalized == null || normalized <= 0) return null;
  const timestampMs =
    normalized >= 1_000_000_000_000 ? Math.round(normalized) : Math.round(normalized * 1000);
  if (timestampMs >= QODER_SENTINEL_EXPIRES_AT_MS) return null;
  return timestampMs;
}

function parseQuotaBucket(raw: unknown, fallback?: Partial<QoderQuotaBucket>): QoderQuotaBucket {
  const used = firstFiniteNumber(
    getNestedValue(raw, ['used']),
    getNestedValue(raw, ['usage']),
    getNestedValue(raw, ['consumed']),
    fallback?.used,
  );
  const total = firstFiniteNumber(
    getNestedValue(raw, ['total']),
    getNestedValue(raw, ['quota']),
    getNestedValue(raw, ['limit']),
    fallback?.total,
  );
  const remaining = firstFiniteNumber(
    getNestedValue(raw, ['remaining']),
    getNestedValue(raw, ['available']),
    getNestedValue(raw, ['left']),
    fallback?.remaining,
    total != null && used != null ? total - used : null,
  );
  const percentage = clampPercent(
    firstFiniteNumber(
      getNestedValue(raw, ['percentage']),
      getNestedValue(raw, ['usagePercent']),
      getNestedValue(raw, ['usage_percentage']),
      fallback?.percentage,
      total != null && used != null && total > 0 ? (used / total) * 100 : null,
    ),
  );

  return {
    used,
    total,
    remaining,
    percentage,
  };
}

function getRawPlanTag(account: QoderAccount): string | null {
  return firstNonEmptyString(
    getNestedValue(account.auth_user_plan_raw, ['plan_tier_name']),
    getNestedValue(account.auth_user_plan_raw, ['tier_name']),
    getNestedValue(account.auth_user_plan_raw, ['tierName']),
    getNestedValue(account.auth_user_plan_raw, ['planTierName']),
    getNestedValue(account.auth_user_plan_raw, ['plan']),
    getNestedValue(account.auth_user_info_raw, ['userTag']),
    getNestedValue(account.auth_user_info_raw, ['user_tag']),
    getNestedValue(resolveQoderCreditUsage(account), ['plan_tier_name']),
    getNestedValue(resolveQoderCreditUsage(account), ['tier_name']),
    getNestedValue(resolveQoderCreditUsage(account), ['tierName']),
    getNestedValue(resolveQoderCreditUsage(account), ['planTierName']),
    account.plan_type,
  );
}

export function getQoderAccountDisplayEmail(account: QoderAccount): string {
  return (
    account.email ||
    account.display_name ||
    account.user_id ||
    account.id
  );
}

export function getQoderPlanBadge(account: QoderAccount): string {
  const raw = getRawPlanTag(account);
  if (raw) return raw;
  return 'UNKNOWN';
}

export function resolveQoderCreditUsage(account: QoderAccount): unknown {
  const raw = account.auth_credit_usage_raw;
  if (!isRecord(raw)) return raw;
  if (raw.displayMode === 'enterprise') return null;
  if (isRecord(raw.qoderUsage)) return raw.qoderUsage;
  if ('displayMode' in raw || 'qoderUsage' in raw || 'enterpriseUsage' in raw) return null;
  return raw;
}

export function getQoderSubscriptionInfo(account: QoderAccount): QoderSubscriptionInfo {
  const planTag = getQoderPlanBadge(account);
  const unavailable = isRecord(account.auth_credit_usage_raw)
    && resolveQoderCreditUsage(account) == null;
  const emptyQuota: QoderQuotaBucket = { used: null, total: null, remaining: null, percentage: null };
  const userQuota = parseQuotaBucket(
    firstRecord(
      getNestedValue(resolveQoderCreditUsage(account), ['userQuota']),
      getNestedValue(account.auth_user_plan_raw, ['userQuota']),
      getNestedValue(account.auth_user_info_raw, ['userQuota']),
    ),
    {
      used: account.credits_used,
      total: account.credits_total,
      remaining: account.credits_remaining,
      percentage: account.credits_usage_percent,
    },
  );
  const addOnQuota = parseQuotaBucket(
    firstRecord(
      getNestedValue(resolveQoderCreditUsage(account), ['addOnQuota']),
      getNestedValue(resolveQoderCreditUsage(account), ['addonQuota']),
      getNestedValue(resolveQoderCreditUsage(account), ['add_on_quota']),
      getNestedValue(account.auth_user_plan_raw, ['addOnQuota']),
      getNestedValue(account.auth_user_plan_raw, ['addonQuota']),
      getNestedValue(account.auth_user_plan_raw, ['add_on_quota']),
    ),
  );
  const sharedCreditPackageRaw = firstRecord(
    getNestedValue(resolveQoderCreditUsage(account), ['orgResourcePackage']),
    getNestedValue(resolveQoderCreditUsage(account), ['organizationResourcePackage']),
    getNestedValue(resolveQoderCreditUsage(account), ['sharedCreditPackage']),
    getNestedValue(resolveQoderCreditUsage(account), ['resourcePackage']),
    getNestedValue(account.auth_user_plan_raw, ['orgResourcePackage']),
    getNestedValue(account.auth_user_plan_raw, ['organizationResourcePackage']),
    getNestedValue(account.auth_user_plan_raw, ['sharedCreditPackage']),
    getNestedValue(account.auth_user_plan_raw, ['resourcePackage']),
  );
  const totalUsagePercentage = clampPercent(
    firstFiniteNumber(
      getNestedValue(resolveQoderCreditUsage(account), ['totalUsagePercentage']),
      getNestedValue(resolveQoderCreditUsage(account), ['total_usage_percentage']),
      getNestedValue(account.auth_user_plan_raw, ['totalUsagePercentage']),
      getNestedValue(account.auth_user_plan_raw, ['total_usage_percentage']),
    ),
  );
  const detailUrl = firstNonEmptyString(
    getNestedValue(resolveQoderCreditUsage(account), ['creditBreakdownUrl']),
    getNestedValue(account.auth_user_plan_raw, ['creditBreakdownUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['usageDetailUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['usageDetailsUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['detailUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['overviewUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['usageUrl']),
  );
  const upgradeUrl = firstNonEmptyString(
    getNestedValue(resolveQoderCreditUsage(account), ['upgradeUrl']),
    getNestedValue(account.auth_user_plan_raw, ['upgradeUrl']),
    getNestedValue(account.auth_user_info_raw, ['upgradeUrl']),
  );
  const addCreditsUrl = firstNonEmptyString(
    getNestedValue(resolveQoderCreditUsage(account), ['addCreditsUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['add_credits_url']),
    getNestedValue(resolveQoderCreditUsage(account), ['topUpUrl']),
    getNestedValue(resolveQoderCreditUsage(account), ['top_up_url']),
    getNestedValue(account.auth_user_plan_raw, ['addCreditsUrl']),
    getNestedValue(account.auth_user_plan_raw, ['add_credits_url']),
    getNestedValue(account.auth_user_plan_raw, ['topUpUrl']),
    getNestedValue(account.auth_user_plan_raw, ['top_up_url']),
  );
  const expiresAt = normalizeTimestampMs(
    firstFiniteNumber(
      getNestedValue(resolveQoderCreditUsage(account), ['expiresAt']),
      getNestedValue(resolveQoderCreditUsage(account), ['expires_at']),
      getNestedValue(resolveQoderCreditUsage(account), ['resetAt']),
      getNestedValue(resolveQoderCreditUsage(account), ['reset_at']),
      getNestedValue(account.auth_user_plan_raw, ['expiresAt']),
      getNestedValue(account.auth_user_plan_raw, ['expires_at']),
      getNestedValue(account.auth_user_plan_raw, ['resetAt']),
      getNestedValue(account.auth_user_plan_raw, ['reset_at']),
      getNestedValue(account.auth_user_info_raw, ['expiresAt']),
      getNestedValue(account.auth_user_info_raw, ['expires_at']),
    ),
  );

  return {
    planTag,
    userType: firstNonEmptyString(
      getNestedValue(resolveQoderCreditUsage(account), ['userType']),
      getNestedValue(resolveQoderCreditUsage(account), ['user_type']),
      getNestedValue(account.auth_user_plan_raw, ['userType']),
      getNestedValue(account.auth_user_plan_raw, ['user_type']),
      getNestedValue(account.auth_user_info_raw, ['userType']),
      getNestedValue(account.auth_user_info_raw, ['user_type']),
    ),
    isPersonalVersion: firstBoolean(
      getNestedValue(resolveQoderCreditUsage(account), ['isPersonalVersion']),
      getNestedValue(resolveQoderCreditUsage(account), ['is_personal_version']),
      getNestedValue(account.auth_user_plan_raw, ['isPersonalVersion']),
      getNestedValue(account.auth_user_plan_raw, ['is_personal_version']),
      getNestedValue(account.auth_user_info_raw, ['isPersonalVersion']),
      getNestedValue(account.auth_user_info_raw, ['is_personal_version']),
    ),
    isHighestTier:
      firstBoolean(
        getNestedValue(resolveQoderCreditUsage(account), ['isHighestTier']),
        getNestedValue(resolveQoderCreditUsage(account), ['is_highest_tier']),
        getNestedValue(account.auth_user_plan_raw, ['isHighestTier']),
        getNestedValue(account.auth_user_plan_raw, ['is_highest_tier']),
        getNestedValue(account.auth_user_info_raw, ['isHighestTier']),
        getNestedValue(account.auth_user_info_raw, ['is_highest_tier']),
      ) ?? false,
    userQuota: unavailable ? emptyQuota : userQuota,
    addOnQuota: unavailable ? emptyQuota : addOnQuota,
    sharedCreditPackageUsed: unavailable ? null : firstFiniteNumber(
      getNestedValue(sharedCreditPackageRaw, ['used']),
      getNestedValue(sharedCreditPackageRaw, ['usage']),
      getNestedValue(sharedCreditPackageRaw, ['consumed']),
      getNestedValue(sharedCreditPackageRaw, ['count']),
    ),
    totalUsagePercentage: unavailable ? null : totalUsagePercentage,
    expiresAt,
    detailUrl,
    upgradeUrl,
    addCreditsUrl,
  };
}

export function getQoderUsage(account: QoderAccount): QoderUsage {
  const subscription = getQoderSubscriptionInfo(account);
  const percent = subscription.userQuota.percentage ?? subscription.totalUsagePercentage;

  return {
    inlineSuggestionsUsedPercent: percent,
    chatMessagesUsedPercent: percent,
    allowanceResetAt: subscription.expiresAt,
    creditsUsed: subscription.userQuota.used,
    creditsTotal: subscription.userQuota.total,
    creditsRemaining: subscription.userQuota.remaining,
  };
}

export function getQoderUsageOverview(account: QoderAccount): QoderUsageOverview {
  const subscription = getQoderSubscriptionInfo(account);
  const usage = getQoderUsage(account);

  return {
    planTag: subscription.planTag,
    usagePercent: usage.inlineSuggestionsUsedPercent,
    creditsUsed: usage.creditsUsed,
    creditsTotal: usage.creditsTotal,
    creditsRemaining: usage.creditsRemaining,
    unit:
      firstNonEmptyString(getNestedValue(resolveQoderCreditUsage(account), ['userQuota', 'unit'])) ||
      'Credits',
    detailUrl: subscription.detailUrl || subscription.upgradeUrl,
    upgradeUrl: subscription.upgradeUrl,
    isQuotaExceeded:
      toBoolean(getNestedValue(resolveQoderCreditUsage(account), ['isQuotaExceeded'])) ||
      toBoolean(getNestedValue(account.auth_user_info_raw, ['isQuotaExceeded'])),
  };
}

export function hasQoderQuotaData(account: QoderAccount): boolean {
  return account.auth_credit_usage_raw != null;
}
