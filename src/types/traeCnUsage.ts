import type { TraeAccount } from './trae';

type JsonRecord = Record<string, unknown>;

const TRAE_CN_PRODUCT_TYPE = {
  Free: 0,
  Pro: 1,
  Package: 3,
  ProPlusPack: 5,
  Ultra: 6,
  PayGo: 7,
  Lite: 8,
  SoloInvite: 9,
  CNExpress: 100,
} as const;

export type TraeCnUsageSummary = {
  hasData: boolean;
  planLabel: string | null;
  usageText: string;
  usagePercent: number | null;
  statusText: string;
  statusTone: 'normal' | 'warning' | 'unknown';
  resetAt: number | null;
  fastRequestText: string;
  packageText: string;
  payAsYouGoText: string;
};

function toRecord(value: unknown): JsonRecord | null {
  if (typeof value === 'string') {
    try {
      return toRecord(JSON.parse(value));
    } catch {
      return null;
    }
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as JsonRecord;
}

function toArray(value: unknown): unknown[] | null {
  if (Array.isArray(value)) return value;
  if (typeof value === 'string') {
    try {
      const parsed = JSON.parse(value);
      return Array.isArray(parsed) ? parsed : null;
    } catch {
      return null;
    }
  }
  return null;
}

function toNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value.trim());
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function toBoolean(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return value !== 0;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (normalized === 'true' || normalized === '1') return true;
    if (normalized === 'false' || normalized === '0') return false;
  }
  return null;
}

function toStringValue(value: unknown): string | null {
  if (typeof value === 'string') {
    const trimmed = value.trim();
    return trimmed ? trimmed : null;
  }
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  return null;
}

function nested(record: JsonRecord | null, keys: string[]): JsonRecord | null {
  let current = record;
  for (const key of keys) {
    current = toRecord(current?.[key]);
    if (!current) return null;
  }
  return current;
}

function firstRecord(...values: Array<JsonRecord | null>): JsonRecord | null {
  return values.find((value) => value != null) ?? null;
}

function firstNumber(...values: unknown[]): number | null {
  for (const value of values) {
    const parsed = toNumber(value);
    if (parsed != null) return parsed;
  }
  return null;
}

function firstString(...values: unknown[]): string | null {
  for (const value of values) {
    const parsed = toStringValue(value);
    if (parsed != null) return parsed;
  }
  return null;
}

function hasOwn(record: JsonRecord | null, key: string): boolean {
  return record != null && Object.prototype.hasOwnProperty.call(record, key);
}

function toUnixSeconds(value: number | null): number | null {
  if (value == null || value <= 0) return null;
  return value > 10_000_000_000 ? Math.round(value / 1000) : Math.round(value);
}

function usageRoots(rawUsage: unknown): JsonRecord[] {
  const root = toRecord(rawUsage);
  if (!root) return [];

  return [
    root,
    nested(root, ['data']),
    nested(root, ['Result']),
    nested(root, ['result']),
    nested(root, ['payload']),
    nested(root, ['user_current_entitlement_list']),
    nested(root, ['ide_user_ent_usage']),
  ].filter((item): item is JsonRecord => item != null);
}

function isDashboardEntitlementPayload(rawUsage: unknown): boolean {
  return usageRoots(rawUsage).some(
    (root) =>
      root._cockpit_source === 'user_current_entitlement_list' &&
      toArray(root.user_entitlement_pack_list) != null,
  );
}

function getPacks(rawUsage: unknown): JsonRecord[] {
  for (const root of usageRoots(rawUsage)) {
    const packs = toArray(root.user_entitlement_pack_list);
    if (packs) {
      return packs.map((item) => toRecord(item)).filter((item): item is JsonRecord => item != null);
    }
  }
  return [];
}

function getProductType(pack: JsonRecord): number | null {
  return firstNumber(nested(pack, ['entitlement_base_info'])?.product_type, pack.product_type);
}

function getBestPack(rawUsage: unknown): JsonRecord | null {
  const packs = getPacks(rawUsage).filter((pack) => getProductType(pack) !== TRAE_CN_PRODUCT_TYPE.Package);
  const priority = [
    TRAE_CN_PRODUCT_TYPE.CNExpress,
    TRAE_CN_PRODUCT_TYPE.Ultra,
    TRAE_CN_PRODUCT_TYPE.ProPlusPack,
    4,
    TRAE_CN_PRODUCT_TYPE.Pro,
    TRAE_CN_PRODUCT_TYPE.SoloInvite,
    TRAE_CN_PRODUCT_TYPE.Lite,
    TRAE_CN_PRODUCT_TYPE.Free,
    2,
    TRAE_CN_PRODUCT_TYPE.PayGo,
  ];
  for (const type of priority) {
    const pack = packs.find((item) => getProductType(item) === type);
    if (pack) return pack;
  }
  return packs[0] ?? null;
}

function getPlanLabel(account: TraeAccount, pack: JsonRecord | null): string | null {
  const entitlement = toRecord(account.trae_entitlement_raw);
  const server = toRecord(account.trae_server_raw);
  const serverEntitlement = firstRecord(
    nested(server, ['entitlementInfo']),
    nested(server, ['originPayStatusData']),
  );
  return firstString(
    entitlement?.user_pay_identity_str,
    serverEntitlement?.identityStr,
    serverEntitlement?.user_pay_identity_str,
    pack?.display_desc,
    account.plan_type,
  );
}

function getQuota(account: TraeAccount, pack: JsonRecord | null): JsonRecord | null {
  return firstRecord(
    nested(pack, ['entitlement_base_info', 'quota']),
    nested(pack, ['entitlement_base_info', 'product_extra', 'subscription_extra', 'quota']),
    nested(toRecord(account.trae_entitlement_raw), ['quota']),
    nested(toRecord(account.trae_server_raw), ['entitlementInfo', 'quota']),
  );
}

function getPackQuota(pack: JsonRecord | null): JsonRecord | null {
  return firstRecord(
    nested(pack, ['entitlement_base_info', 'product_extra', 'subscription_extra', 'quota']),
    nested(pack, ['entitlement_base_info', 'product_extra', 'package_extra', 'quota']),
    nested(pack, ['entitlement_base_info', 'quota']),
  );
}

function isVisibleActivePack(pack: JsonRecord): boolean {
  if (toBoolean(pack.is_hide) === true) return false;
  const status = firstNumber(pack.status, pack.entitlement_status);
  return status == null || status === 1;
}

function getFastRequestUsage(rawUsage: unknown): { available: number; limit: number; used: number } | null {
  const packs = getPacks(rawUsage).filter(isVisibleActivePack);
  if (packs.length === 0) return null;

  const dashboardPayload = isDashboardEntitlementPayload(rawUsage);
  const limits = packs
    .map((pack) => firstNumber(getPackQuota(pack)?.premium_model_fast_request_limit))
    .filter((value): value is number => value != null);
  const used = packs.reduce((sum, pack) => {
    const usage = toRecord(pack.usage);
    return sum + (firstNumber(usage?.premium_model_fast_amount) ?? 0);
  }, 0);
  const hasFastEvidence = packs.some((pack) => {
    const usage = toRecord(pack.usage);
    const quota = getPackQuota(pack);
    return hasOwn(usage, 'premium_model_fast_amount') || hasOwn(quota, 'premium_model_fast_request_limit');
  });

  if (!dashboardPayload && !hasFastEvidence) return null;

  const limit =
    limits.length === 0 ? 0 : limits.some((value) => value === -1) ? -1 : limits.reduce((sum, value) => sum + value, 0);
  const available = limit === -1 ? -1 : Math.max(limit - used, 0);
  return { available, limit, used };
}

function formatTimes(value: number): string {
  return value === -1 ? '无限次' : `${value} 次`;
}

function getDetail(account: TraeAccount): JsonRecord | null {
  const entitlement = toRecord(account.trae_entitlement_raw);
  const server = toRecord(account.trae_server_raw);
  return firstRecord(
    nested(server, ['entitlementInfo', 'detail']),
    nested(server, ['originPayStatusData', 'detail']),
    nested(entitlement, ['detail']),
  );
}

export function getTraeCnUsageSummary(account: TraeAccount): TraeCnUsageSummary {
  const pack = getBestPack(account.trae_usage_raw);
  const quota = getQuota(account, pack);
  const detail = getDetail(account);
  const planLabel = getPlanLabel(account, pack);
  const fastUsage = getFastRequestUsage(account.trae_usage_raw);

  const fastRequestPer = firstNumber(detail?.fast_request_per, detail?.fastRequestPer);
  const canGetExpressStatus = firstNumber(
    detail?.can_get_express_status,
    detail?.canGetExpressStatus,
  );
  const noBonusQuota = toBoolean(quota?.no_bonus_quota);
  const parallelLimit = firstNumber(quota?.solo_agent_parallel_limit);
  const resetAt = toUnixSeconds(
    firstNumber(
      nested(pack, ['entitlement_base_info'])?.end_time,
      pack?.expire_time,
      account.plan_reset_at,
    ),
  );

  const knownQuotaValues = [
    fastRequestPer,
    canGetExpressStatus,
    parallelLimit,
    fastUsage?.available,
    noBonusQuota == null ? null : noBonusQuota ? 1 : 0,
  ].filter((value) => value != null);

  const hasData = account.trae_usage_raw != null || account.trae_entitlement_raw != null;
  const usageText = (() => {
    if (!hasData) return '用量：--';
    if (fastUsage) return `速通可用 ${formatTimes(fastUsage.available)}`;
    if (fastRequestPer != null && fastRequestPer > 0) return `快请求/月：${fastRequestPer}`;
    if (planLabel?.toLowerCase() === 'free') return '免费剩余：--';
    return '剩余次数：--';
  })();

  const fastRequestText = (() => {
    if (!hasData) return '快通道: --';
    if (fastRequestPer != null && fastRequestPer > 0) return `快通道: ${fastRequestPer}/月`;
    if (canGetExpressStatus != null) return `快通道: ${canGetExpressStatus}`;
    return '快通道: --';
  })();

  const packageText = (() => {
    if (!hasData) return '权益包: --';
    const enabled = [
      quota?.enable_solo_agent,
      quota?.enable_solo_builder,
      quota?.enable_solo_coder,
      quota?.enable_solo_lite,
      quota?.enable_solo_web,
    ].some((value) => toBoolean(value) === true);
    if (enabled && parallelLimit != null) return `权益包: Solo 并发 ${parallelLimit}`;
    if (enabled) return '权益包: 可用';
    return '权益包: --';
  })();

  return {
    hasData,
    planLabel,
    usageText,
    usagePercent: null,
    statusText: fastUsage ? '状态：已同步' : hasData ? '状态：已同步，剩余额待确认' : '状态：--',
    statusTone: fastUsage ? 'normal' : 'unknown',
    resetAt,
    fastRequestText,
    packageText,
    payAsYouGoText: knownQuotaValues.length > 0 && noBonusQuota === true ? '' : '',
  };
}
