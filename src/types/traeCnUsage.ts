import type { TraeAccount } from './trae';

type JsonRecord = Record<string, unknown>;

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

function toUnixSeconds(value: number | null): number | null {
  if (value == null || value <= 0) return null;
  return value > 10_000_000_000 ? Math.round(value / 1000) : Math.round(value);
}

function getPacks(rawUsage: unknown): JsonRecord[] {
  const root = toRecord(rawUsage);
  return (toArray(root?.user_entitlement_pack_list) ?? [])
    .map((item) => toRecord(item))
    .filter((item): item is JsonRecord => item != null);
}

function getProductType(pack: JsonRecord): number | null {
  return firstNumber(nested(pack, ['entitlement_base_info'])?.product_type, pack.product_type);
}

function getBestPack(rawUsage: unknown): JsonRecord | null {
  const packs = getPacks(rawUsage).filter((pack) => getProductType(pack) !== 3);
  const priority = [6, 4, 1, 9, 8, 0, 2, 7];
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
    noBonusQuota == null ? null : noBonusQuota ? 1 : 0,
  ].filter((value) => value != null);

  const hasData = account.trae_usage_raw != null || account.trae_entitlement_raw != null;
  const usageText = (() => {
    if (!hasData) return '用量：--';
    if (fastRequestPer != null && fastRequestPer > 0) return `快请求/月：${fastRequestPer}`;
    if (planLabel?.toLowerCase() === 'free') return '免费额度：官方未返回剩余次数';
    return '额度：官方未返回剩余次数';
  })();

  const fastRequestText = (() => {
    if (!hasData) return '快通道: --';
    if (fastRequestPer != null && fastRequestPer > 0) return `快通道: ${fastRequestPer}/月`;
    if (canGetExpressStatus != null) return `快通道状态: ${canGetExpressStatus}`;
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
    return '权益包: 官方未返回次数';
  })();

  return {
    hasData,
    planLabel,
    usageText,
    usagePercent: null,
    statusText: hasData ? '状态：已同步，次数余额待官方字段确认' : '状态：--',
    statusTone: hasData ? 'unknown' : 'unknown',
    resetAt,
    fastRequestText,
    packageText,
    payAsYouGoText:
      knownQuotaValues.length > 0 && noBonusQuota === true
        ? 'Bonus: 不单独展示'
        : 'Bonus: --',
  };
}
