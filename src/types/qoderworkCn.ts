export interface QoderworkCnAccount {
  id: string;
  email: string;
  user_id?: string | null;
  display_name?: string | null;
  user_type?: string | null;
  credits_used?: number | null;
  credits_total?: number | null;
  credits_remaining?: number | null;
  credits_usage_percent?: number | null;
  is_quota_exceeded?: boolean | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  usage_updated_at?: number | null;
  tags?: string[] | null;
  quota_raw?: unknown;
  session_backup_at?: number | null;
  created_at: number;
  last_used: number;
}

interface UnknownRecord {
  [key: string]: unknown;
}

export interface QoderworkCnUsage {
  usagePercent: number | null;
  creditsUsed: number | null;
  creditsTotal: number | null;
  creditsRemaining: number | null;
}

export interface QoderworkCnQuotaBucket {
  used: number | null;
  total: number | null;
  remaining: number | null;
  percentage: number | null;
}

export interface QoderworkCnSubscriptionInfo {
  planTag: string;
  userType: string | null;
  userQuota: QoderworkCnQuotaBucket;
  addOnQuota: QoderworkCnQuotaBucket;
  orgResourcePackage: QoderworkCnQuotaBucket;
  totalUsagePercentage: number | null;
  expiresAt: number | null;
  isQuotaExceeded: boolean;
}

export interface QoderworkCnUsageOverview {
  planTag: string;
  usagePercent: number | null;
  creditsUsed: number | null;
  creditsTotal: number | null;
  creditsRemaining: number | null;
  unit: string;
  isQuotaExceeded: boolean;
}

const QODERWORK_CN_SENTINEL_EXPIRES_AT_MS = Date.UTC(9999, 11, 31, 0, 0, 0);

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

function toNonEmptyString(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function firstFiniteNumber(...values: unknown[]): number | null {
  for (const value of values) {
    const normalized = toFiniteNumber(value);
    if (normalized != null) return normalized;
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

function firstRecord(...values: unknown[]): UnknownRecord | null {
  for (const value of values) {
    if (isRecord(value)) return value;
  }
  return null;
}

function clampPercent(value: number | null): number | null {
  if (value == null) return null;
  const normalized = value <= 1 ? value * 100 : value;
  return Math.max(0, Math.min(100, normalized));
}

function normalizeTimestampMs(value: unknown): number | null {
  const normalized = toFiniteNumber(value);
  if (normalized == null || normalized <= 0) return null;
  const timestampMs =
    normalized >= 1_000_000_000_000 ? Math.round(normalized) : Math.round(normalized * 1000);
  if (timestampMs >= QODERWORK_CN_SENTINEL_EXPIRES_AT_MS) return null;
  return timestampMs;
}

function parseQuotaBucket(raw: unknown, fallback?: Partial<QoderworkCnQuotaBucket>): QoderworkCnQuotaBucket {
  const used = firstFiniteNumber(
    getNestedValue(raw, ['used']),
    getNestedValue(raw, ['usage']),
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
    fallback?.remaining,
    total != null && used != null ? total - used : null,
  );
  const percentage = clampPercent(
    firstFiniteNumber(
      getNestedValue(raw, ['percentage']),
      getNestedValue(raw, ['usagePercent']),
      fallback?.percentage,
      total != null && used != null && total > 0 ? (used / total) * 100 : null,
    ),
  );

  return { used, total, remaining, percentage };
}

export function getQoderworkCnAccountDisplayEmail(account: QoderworkCnAccount): string {
  // For QoderWork CN, prefer display_name (user's actual name) over email (which may be a UUID)
  return account.display_name || account.email || account.user_id || account.id;
}

export function getQoderworkCnPlanBadge(account: QoderworkCnAccount): string {
  return (
    firstNonEmptyString(
      getNestedValue(account.quota_raw, ['user_type']),
      getNestedValue(account.quota_raw, ['userType']),
      account.user_type,
    ) || 'UNKNOWN'
  );
}

export function getQoderworkCnSubscriptionInfo(account: QoderworkCnAccount): QoderworkCnSubscriptionInfo {
  const planTag = getQoderworkCnPlanBadge(account);
  const userQuota = parseQuotaBucket(
    firstRecord(
      getNestedValue(account.quota_raw, ['userQuota']),
      getNestedValue(account.quota_raw, ['user_quota']),
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
      getNestedValue(account.quota_raw, ['addOnQuota']),
      getNestedValue(account.quota_raw, ['add_on_quota']),
      getNestedValue(account.quota_raw, ['addonQuota']),
    ),
  );
  const orgResourcePackage = parseQuotaBucket(
    firstRecord(
      getNestedValue(account.quota_raw, ['orgResourcePackage']),
      getNestedValue(account.quota_raw, ['org_resource_package']),
    ),
  );
  const totalUsagePercentage = clampPercent(
    firstFiniteNumber(
      getNestedValue(account.quota_raw, ['totalUsagePercentage']),
      getNestedValue(account.quota_raw, ['total_usage_percentage']),
    ),
  );
  const expiresAt = normalizeTimestampMs(
    firstFiniteNumber(
      getNestedValue(account.quota_raw, ['expiresAt']),
      getNestedValue(account.quota_raw, ['expires_at']),
    ),
  );

  return {
    planTag,
    userType: firstNonEmptyString(
      getNestedValue(account.quota_raw, ['userType']),
      getNestedValue(account.quota_raw, ['user_type']),
    ),
    userQuota,
    addOnQuota,
    orgResourcePackage,
    totalUsagePercentage,
    expiresAt,
    isQuotaExceeded:
      account.is_quota_exceeded === true ||
      toBoolean(getNestedValue(account.quota_raw, ['isQuotaExceeded'])) ||
      toBoolean(getNestedValue(account.quota_raw, ['is_quota_exceeded'])),
  };
}

export function getQoderworkCnUsage(account: QoderworkCnAccount): QoderworkCnUsage {
  const subscription = getQoderworkCnSubscriptionInfo(account);
  const percent = subscription.userQuota.percentage ?? subscription.totalUsagePercentage;

  return {
    usagePercent: percent,
    creditsUsed: subscription.userQuota.used,
    creditsTotal: subscription.userQuota.total,
    creditsRemaining: subscription.userQuota.remaining,
  };
}

export function getQoderworkCnUsageOverview(account: QoderworkCnAccount): QoderworkCnUsageOverview {
  const subscription = getQoderworkCnSubscriptionInfo(account);
  const usage = getQoderworkCnUsage(account);

  return {
    planTag: subscription.planTag,
    usagePercent: usage.usagePercent,
    creditsUsed: usage.creditsUsed,
    creditsTotal: usage.creditsTotal,
    creditsRemaining: usage.creditsRemaining,
    unit:
      firstNonEmptyString(getNestedValue(account.quota_raw, ['userQuota', 'unit'])) ||
      firstNonEmptyString(getNestedValue(account.quota_raw, ['user_quota', 'unit'])) ||
      'Credits',
    isQuotaExceeded: subscription.isQuotaExceeded,
  };
}

export function hasQoderworkCnQuotaData(account: QoderworkCnAccount): boolean {
  return account.quota_raw != null;
}
