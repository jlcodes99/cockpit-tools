export interface TraeAccount {
  id: string;
  email: string;
  user_id?: string | null;
  nickname?: string | null;
  tags?: string[] | null;

  access_token: string;
  refresh_token?: string | null;
  token_type?: string | null;
  expires_at?: number | null;

  plan_type?: string | null;
  plan_reset_at?: number | null;

  trae_auth_raw?: unknown;
  trae_profile_raw?: unknown;
  trae_entitlement_raw?: unknown;
  trae_usage_raw?: unknown;
  trae_server_raw?: unknown;
  trae_usertag_raw?: string | null;

  status?: string | null;
  status_reason?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;

  created_at: number;
  last_used: number;

  quota?: TraeQuota;
}

export type TraeAccountPlatformId = 'trae' | 'trae_solo' | 'trae_cn' | 'trae_solo_cn';

export interface TraeQuota {
  hourly_percentage: number;
  weekly_percentage: number;
  hourly_reset_time?: number | null;
  weekly_reset_time?: number | null;
  raw_data?: unknown;
}

const TRAE_AUTH_CLIENT_ID = 'ono9krqynydwx5';
const TRAE_SOLO_AUTH_CLIENT_ID = 'en1oxy7wnw8j9n';

export type TraeUsage = {
  usedPercent: number | null;
  spentUsd: number | null;
  totalUsd: number | null;
  resetAt: number | null;
  basicQuota?: number | null;
  basicUsage?: number | null;
  bonusQuota?: number | null;
  bonusUsage?: number | null;
  nextBillingAt?: number | null;
  nextResetDays?: number | null;
  isActive?: boolean | null;
  isCanceled?: boolean | null;
  isBilledYearly?: boolean | null;
  identityStr?: string | null;
  consumingProductType?: number | null;
  hasPackage?: boolean | null;
  payAsYouGoOpen?: boolean | null;
  payAsYouGoUsd?: number | null;
  usageExhausted?: boolean | null;
  /** CN 速通额度模型（premium_model_fast_*），与国际站 USD basic 额度不同 */
  usageModel?: 'usd' | 'fast_request' | 'credits' | 'unknown';
  fastRequestAvailable?: number | null;
  fastRequestLimit?: number | null;
  fastRequestUsed?: number | null;
  /** entitlement detail 中的月度快通道次数（无 pack 速通汇总时的兜底展示） */
  fastRequestPerMonth?: number | null;
  canGetExpressStatus?: number | null;
  soloParallelLimit?: number | null;
  hasSoloPackage?: boolean | null;
  /** CN 积分计费模式（is_credits_billing）：usage_summary 汇总 */
  isCreditsBilling?: boolean | null;
  creditsConsumed?: number | null;
  creditsTotal?: number | null;
};

function toRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value === 'string') {
    try {
      return toRecord(JSON.parse(value));
    } catch {
      return null;
    }
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
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
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function toBoolean(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return value !== 0;
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (!normalized) return null;
    if (normalized === 'true' || normalized === '1' || normalized === 'yes') return true;
    if (normalized === 'false' || normalized === '0' || normalized === 'no') return false;
  }
  return null;
}

function toNonEmptyString(value: unknown): string | null {
  if (typeof value === 'string') {
    const trimmed = value.trim();
    return trimmed ? trimmed : null;
  }
  if (typeof value === 'number' && Number.isFinite(value)) {
    return String(value);
  }
  return null;
}

function toUnixSeconds(value: number | null): number | null {
  if (value == null || !Number.isFinite(value) || value <= 0) return null;
  if (value > 10_000_000_000) return Math.round(value / 1000);
  return Math.round(value);
}

function pickFirstNumber(obj: Record<string, unknown> | null, keys: string[]): number | null {
  if (!obj) return null;
  for (const key of keys) {
    const value = toNumber(obj[key]);
    if (value != null) return value;
  }
  return null;
}

function pickFirstString(obj: Record<string, unknown> | null, keys: string[]): string | null {
  if (!obj) return null;
  for (const key of keys) {
    const value = toNonEmptyString(obj[key]);
    if (value != null) return value;
  }
  return null;
}

function pickNestedObject(
  obj: Record<string, unknown> | null,
  keys: string[],
): Record<string, unknown> | null {
  if (!obj) return null;
  for (const key of keys) {
    const nested = toRecord(obj[key]);
    if (nested) return nested;
  }
  return null;
}

export const TRAE_PRODUCT_TYPE = {
  FREE: 0,
  PRO: 1,
  PACKAGE: 2,
  /** 国际站 promo；CN 侧同值常表示 Package 权益包，选包时仍过滤 */
  PROMO_CODE: 3,
  PRO_PLUS: 4,
  /** CN Pro+ Pack（社区 #1281 product_type=5） */
  PRO_PLUS_PACK: 5,
  ULTRA: 6,
  PAY_GO: 7,
  LITE: 8,
  TRIAL: 9,
  /** Trae CN 专属「CNExpress / 速通」套餐 */
  CN_EXPRESS: 100,
} as const;

const TRAE_EXHAUSTION_TYPES = new Set<number>([
  TRAE_PRODUCT_TYPE.FREE,
  TRAE_PRODUCT_TYPE.PRO,
  TRAE_PRODUCT_TYPE.PACKAGE,
  TRAE_PRODUCT_TYPE.PRO_PLUS,
  TRAE_PRODUCT_TYPE.ULTRA,
  TRAE_PRODUCT_TYPE.LITE,
  TRAE_PRODUCT_TYPE.TRIAL,
]);

const TRAE_BONUS_APPLICABLE_TYPES = new Set<number>([
  TRAE_PRODUCT_TYPE.FREE,
  TRAE_PRODUCT_TYPE.PRO,
  TRAE_PRODUCT_TYPE.PRO_PLUS,
  TRAE_PRODUCT_TYPE.ULTRA,
  TRAE_PRODUCT_TYPE.LITE,
  TRAE_PRODUCT_TYPE.TRIAL,
]);

function identityFromProductType(productType: number | null): string | null {
  switch (productType) {
    case TRAE_PRODUCT_TYPE.CN_EXPRESS:
      return 'CNExpress';
    case TRAE_PRODUCT_TYPE.ULTRA:
      return 'Ultra';
    case TRAE_PRODUCT_TYPE.PRO_PLUS:
    case TRAE_PRODUCT_TYPE.PRO_PLUS_PACK:
      return 'Pro+';
    case TRAE_PRODUCT_TYPE.PRO:
      return 'Pro';
    case TRAE_PRODUCT_TYPE.TRIAL:
      return 'Pro';
    case TRAE_PRODUCT_TYPE.LITE:
      return 'Lite';
    case TRAE_PRODUCT_TYPE.FREE:
      return 'Free';
    default:
      return null;
  }
}

function getPackProductType(pack: Record<string, unknown> | null): number | null {
  if (!pack) return null;
  const entitlementBase = pickNestedObject(pack, ['entitlement_base_info']);
  return (
    pickFirstNumber(entitlementBase, ['product_type']) ??
    pickFirstNumber(pack, ['product_type'])
  );
}

function getPackUsage(pack: Record<string, unknown> | null): Record<string, unknown> | null {
  return pickNestedObject(pack, ['usage']);
}

function getPackQuota(pack: Record<string, unknown> | null): Record<string, unknown> | null {
  if (!pack) return null;
  const entitlementBase = pickNestedObject(pack, ['entitlement_base_info']);
  const productExtra = pickNestedObject(entitlementBase, ['product_extra']);
  const subscriptionExtra = pickNestedObject(productExtra, ['subscription_extra']);
  const packageExtra = pickNestedObject(productExtra, ['package_extra']);
  return (
    pickNestedObject(entitlementBase, ['quota']) ??
    pickNestedObject(subscriptionExtra, ['quota']) ??
    pickNestedObject(packageExtra, ['quota'])
  );
}

function collectUsageRoots(rawUsage: unknown): Record<string, unknown>[] {
  const root = toRecord(rawUsage);
  if (!root) return [];
  const nested = [
    root,
    toRecord(root.data),
    toRecord(root.Result),
    toRecord(root.result),
    toRecord(root.payload),
    toRecord(root.user_current_entitlement_list),
    toRecord(root.ide_user_ent_usage),
  ].filter((item): item is Record<string, unknown> => item != null);
  return nested;
}

function getUsagePacks(rawUsage: unknown): Record<string, unknown>[] {
  for (const root of collectUsageRoots(rawUsage)) {
    const packs = toArray(root.user_entitlement_pack_list);
    if (packs) {
      return packs
        .map((item) => toRecord(item))
        .filter((item): item is Record<string, unknown> => item != null);
    }
  }
  return [];
}

function isVisibleActivePack(pack: Record<string, unknown>): boolean {
  if (toBoolean(pack.is_hide) === true) return false;
  const status = pickFirstNumber(pack, ['status', 'entitlement_status']);
  return status == null || status === 1;
}

function getFastRequestUsage(rawUsage: unknown): {
  available: number;
  limit: number;
  used: number;
} | null {
  const packs = getUsagePacks(rawUsage).filter(isVisibleActivePack);
  if (packs.length === 0) return null;

  const roots = collectUsageRoots(rawUsage);
  const dashboardPayload = roots.some(
    (root) =>
      root._cockpit_source === 'user_current_entitlement_list' &&
      toArray(root.user_entitlement_pack_list) != null,
  );

  const limits = packs
    .map((pack) => pickFirstNumber(getPackQuota(pack), ['premium_model_fast_request_limit']))
    .filter((value): value is number => value != null);
  const used = packs.reduce((sum, pack) => {
    return sum + (pickFirstNumber(getPackUsage(pack), ['premium_model_fast_amount']) ?? 0);
  }, 0);
  const hasFastEvidence = packs.some((pack) => {
    const usage = getPackUsage(pack);
    const quota = getPackQuota(pack);
    return (
      (usage != null && Object.prototype.hasOwnProperty.call(usage, 'premium_model_fast_amount')) ||
      (quota != null &&
        Object.prototype.hasOwnProperty.call(quota, 'premium_model_fast_request_limit'))
    );
  });

  if (!dashboardPayload && !hasFastEvidence) return null;

  const limit =
    limits.length === 0
      ? 0
      : limits.some((value) => value === -1)
        ? -1
        : limits.reduce((sum, value) => sum + value, 0);
  const available = limit === -1 ? -1 : Math.max(limit - used, 0);
  return { available, limit, used };
}

function getPackBasicUsage(pack: Record<string, unknown> | null): number | null {
  return pickFirstNumber(getPackUsage(pack), ['basic_usage_amount']) ?? 0;
}

function getPackBasicQuota(pack: Record<string, unknown> | null): number | null {
  const quota = pickFirstNumber(getPackQuota(pack), ['basic_usage_limit']);
  if (quota == null) return null;
  return quota >= 0 ? quota : null;
}

function getPackBonusUsage(pack: Record<string, unknown> | null): number | null {
  return pickFirstNumber(getPackUsage(pack), ['bonus_usage_amount']) ?? 0;
}

function getPackBonusQuota(pack: Record<string, unknown> | null): number | null {
  const quota = pickFirstNumber(getPackQuota(pack), ['bonus_usage_limit']);
  if (quota == null) return null;
  return quota >= 0 ? quota : null;
}

function isPackExhausted(pack: Record<string, unknown>, withBonus: boolean): boolean {
  const basicQuota = getPackBasicQuota(pack);
  const basicUsage = getPackBasicUsage(pack);
  if (basicQuota == null || basicUsage == null || basicUsage < basicQuota) {
    return false;
  }

  if (!withBonus) {
    return true;
  }

  const bonusQuota = getPackBonusQuota(pack);
  if (bonusQuota == null) {
    return false;
  }
  const bonusUsage = getPackBonusUsage(pack) ?? 0;
  return bonusUsage >= bonusQuota;
}

function getPackResetAt(pack: Record<string, unknown> | null): number | null {
  const entitlementBase = pickNestedObject(pack, ['entitlement_base_info']);
  const endTime = pickFirstNumber(entitlementBase, ['end_time']);
  if (endTime == null || endTime <= 0) return null;
  return endTime + 1;
}

function getPackPayAsYouGoUsd(pack: Record<string, unknown> | null): number {
  return pickFirstNumber(getPackUsage(pack), ['pay_go_amount']) ?? 0;
}

function getPackTimeInfo(pack: Record<string, unknown> | null) {
  if (!pack) {
    return {
      nextBillingAt: null,
      nextResetAt: null,
      nextResetDays: null,
      isActive: null,
      isCanceled: null,
      isBilledYearly: null,
    };
  }

  const entitlementBase = pickNestedObject(pack, ['entitlement_base_info']);
  const productExtra = pickNestedObject(entitlementBase, ['product_extra']);
  const subscriptionExtra = pickNestedObject(productExtra, ['subscription_extra']);

  const endTime = pickFirstNumber(entitlementBase, ['end_time']);
  const nextResetAt = endTime != null && endTime > 0 ? endTime + 1 : null;
  const nextResetDays = (() => {
    if (endTime == null || endTime <= 0) return null;
    const nowSeconds = Math.floor(Date.now() / 1000);
    const diff = endTime - nowSeconds;
    return diff <= 0 ? 0 : Math.floor(diff / (24 * 60 * 60));
  })();

  const status = pickFirstNumber(pack, ['status']);
  const periodType = pickFirstNumber(subscriptionExtra, ['period_type']);

  return {
    nextBillingAt: pickFirstNumber(pack, ['next_billing_time']),
    nextResetAt,
    nextResetDays,
    isActive: status === 1,
    isCanceled: status === 3,
    isBilledYearly: periodType === 2,
  };
}

function getCreditsUsageSummary(rawUsage: unknown): {
  isCreditsBilling: boolean;
  consumed: number | null;
  total: number | null;
} {
  const root = toRecord(rawUsage);
  const nested = root ? toRecord(root.raw) : null;
  const source =
    root != null && root.is_credits_billing != null
      ? root
      : (nested ?? root);
  if (!source) {
    return { isCreditsBilling: false, consumed: null, total: null };
  }

  const isCreditsBilling = toBoolean(source.is_credits_billing) === true;
  const summary = toRecord(source.usage_summary);
  return {
    isCreditsBilling,
    consumed: summary ? toNumber(summary.consumed_amount) : null,
    total: summary ? toNumber(summary.total_amount) : null,
  };
}

function getUsageStatusFromPackList(
  rawUsage: unknown,
  options?: { preferCnSelection?: boolean },
): TraeUsage | null {
  const usageRoot = toRecord(rawUsage);
  if (!usageRoot) return null;

  const apiCode =
    toNumber(usageRoot['code']) ??
    collectUsageRoots(rawUsage)
      .map((root) => toNumber(root.code))
      .find((code) => code != null);
  if (apiCode != null && apiCode !== 0) return null;

  const packs = getUsagePacks(rawUsage).filter(
    (item) => getPackProductType(item) !== TRAE_PRODUCT_TYPE.PROMO_CODE,
  );

  if (packs.length === 0) return null;

  const findPackByType = (productType: number) =>
    packs.find((pack) => getPackProductType(pack) === productType) ?? null;

  const freePack = findPackByType(TRAE_PRODUCT_TYPE.FREE);
  const proPack = findPackByType(TRAE_PRODUCT_TYPE.PRO);
  const proPlusPack = findPackByType(TRAE_PRODUCT_TYPE.PRO_PLUS);
  const proPlusPackCn = findPackByType(TRAE_PRODUCT_TYPE.PRO_PLUS_PACK);
  const ultraPack = findPackByType(TRAE_PRODUCT_TYPE.ULTRA);
  const payGoPack = findPackByType(TRAE_PRODUCT_TYPE.PAY_GO);
  const packagePack = findPackByType(TRAE_PRODUCT_TYPE.PACKAGE);
  const litePack = findPackByType(TRAE_PRODUCT_TYPE.LITE);
  const trialPack = findPackByType(TRAE_PRODUCT_TYPE.TRIAL);
  const cnExpressPack = findPackByType(TRAE_PRODUCT_TYPE.CN_EXPRESS);

  const preferCn = options?.preferCnSelection === true;
  const selectedPack = preferCn
    ? cnExpressPack ??
      ultraPack ??
      proPlusPackCn ??
      proPlusPack ??
      proPack ??
      trialPack ??
      litePack ??
      freePack
    : ultraPack ?? proPlusPack ?? proPack ?? trialPack ?? litePack ?? freePack;
  const selectedProductType = selectedPack
    ? getPackProductType(selectedPack)
    : TRAE_PRODUCT_TYPE.FREE;

  const basicUsage = getPackBasicUsage(selectedPack);
  const basicQuota = getPackBasicQuota(selectedPack);
  const bonusUsage = getPackBonusUsage(selectedPack);
  const bonusQuota = getPackBonusQuota(selectedPack);
  const timeInfo = getPackTimeInfo(selectedPack);
  const spentUsd = getPackBasicUsage(selectedPack) ?? 0;
  const totalUsd = getPackBasicQuota(selectedPack) ?? 0;
  const resetAtRaw = timeInfo.nextResetAt ?? getPackResetAt(selectedPack);

  const consumingProductType = (() => {
    for (const pack of packs) {
      const isFlash = toBoolean(getPackUsage(pack)?.['is_flash_consuming']) ?? false;
      if (isFlash) {
        return getPackProductType(pack);
      }
    }
    return TRAE_PRODUCT_TYPE.FREE;
  })();

  const hasPackage = packagePack != null || proPlusPackCn != null;
  const payAsYouGoOpen = payGoPack != null;
  const payAsYouGoUsd = getPackPayAsYouGoUsd(payGoPack);

  const usageStatusPacks = packs.filter((pack) => {
    const type = getPackProductType(pack);
    return type != null && TRAE_EXHAUSTION_TYPES.has(type);
  });
  const usageExhausted =
    usageStatusPacks.length > 0 &&
    usageStatusPacks.every((pack) => {
      const type = getPackProductType(pack);
      return isPackExhausted(pack, type != null && TRAE_BONUS_APPLICABLE_TYPES.has(type));
    });

  const identityStr =
    (preferCn && selectedPack
      ? toNonEmptyString(selectedPack.display_desc)
      : null) ??
    identityFromProductType(selectedProductType) ??
    'Free';
  const derivedPercent = totalUsd > 0 ? (spentUsd / totalUsd) * 100 : 0;
  const fastUsage = preferCn ? getFastRequestUsage(rawUsage) : null;
  const creditsSummary = preferCn ? getCreditsUsageSummary(rawUsage) : null;
  const isCreditsBilling = creditsSummary?.isCreditsBilling === true;

  let usedPercent: number | null = Math.max(0, Math.min(100, Math.round(derivedPercent)));
  let usageModel: TraeUsage['usageModel'] = 'usd';
  if (preferCn && isCreditsBilling && creditsSummary?.total != null) {
    // 积分计费模式：usage_summary.consumed_amount / total_amount 为权威数据，
    // 优先级高于速通次数与 USD 展示。
    usageModel = 'credits';
    const total = creditsSummary.total;
    if (total > 0) {
      const consumed = creditsSummary.consumed ?? 0;
      usedPercent = Math.max(0, Math.min(100, Math.round((consumed / total) * 100)));
    } else {
      usedPercent = 0;
    }
  } else if (preferCn && fastUsage) {
    usageModel = 'fast_request';
    if (fastUsage.limit === -1) {
      usedPercent = 0;
    } else if (fastUsage.limit > 0) {
      usedPercent = Math.max(
        0,
        Math.min(100, Math.round((fastUsage.used / fastUsage.limit) * 100)),
      );
    } else {
      usedPercent = null;
    }
  } else if (preferCn && (totalUsd == null || totalUsd <= 0)) {
    usageModel = 'unknown';
    usedPercent = null;
  }

  return {
    usedPercent,
    spentUsd: usageModel === 'fast_request' || usageModel === 'credits' ? null : spentUsd,
    totalUsd: usageModel === 'fast_request' || usageModel === 'credits' ? null : totalUsd,
    resetAt: toUnixSeconds(resetAtRaw),
    basicQuota,
    basicUsage,
    bonusQuota,
    bonusUsage,
    nextBillingAt: toUnixSeconds(timeInfo.nextBillingAt),
    nextResetDays: timeInfo.nextResetDays,
    isActive: timeInfo.isActive,
    isCanceled: timeInfo.isCanceled,
    isBilledYearly: timeInfo.isBilledYearly,
    identityStr,
    consumingProductType,
    hasPackage,
    payAsYouGoOpen,
    payAsYouGoUsd,
    usageExhausted: usageModel === 'fast_request'
      ? fastUsage != null && fastUsage.limit !== -1 && fastUsage.available === 0
      : usageExhausted,
    usageModel,
    fastRequestAvailable: fastUsage?.available ?? null,
    fastRequestLimit: fastUsage?.limit ?? null,
    fastRequestUsed: fastUsage?.used ?? null,
    isCreditsBilling,
    creditsConsumed: creditsSummary?.consumed ?? null,
    creditsTotal: creditsSummary?.total ?? null,
  };
}

function getPlanFromEntitlementOrServer(account: TraeAccount): string | null {
  const entitlementRoot = toRecord(account.trae_entitlement_raw);
  const serverRoot = toRecord(account.trae_server_raw);

  const entitlementInfo =
    pickNestedObject(entitlementRoot, ['entitlementInfo']) ??
    pickNestedObject(serverRoot, ['entitlementInfo']);

  const plan =
    pickFirstString(entitlementRoot, ['user_pay_identity_str']) ??
    pickFirstString(entitlementInfo, ['identityStr']) ??
    pickFirstString(serverRoot, ['identityStr']);

  return plan ?? null;
}

function getTraeProfileRoot(account: TraeAccount): Record<string, unknown> | null {
  const profileRaw = toRecord(account.trae_profile_raw);
  if (!profileRaw) return null;
  return toRecord(profileRaw.Result);
}

function getTraeAuthAccountRoot(account: TraeAccount): Record<string, unknown> | null {
  const authRaw = toRecord(account.trae_auth_raw);
  return pickNestedObject(authRaw, ['account']);
}

function normalizeTraeAccountPlatformId(raw: unknown): TraeAccountPlatformId | null {
  const value = toNonEmptyString(raw);
  if (!value) return null;
  const normalized = value.trim().toLowerCase().replace(/[-\s]+/g, '_');
  const compact = normalized.replace(/_/g, '');

  switch (compact) {
    case 'trae':
      return 'trae';
    case 'traesolo':
      return 'trae_solo';
    case 'traecn':
      return 'trae_cn';
    case 'traesolocn':
      return 'trae_solo_cn';
    default:
      return null;
  }
}

function pickStringPath(root: Record<string, unknown> | null, path: string[]): string | null {
  if (!root || path.length === 0) return null;
  let current: unknown = root;
  for (const segment of path) {
    const record = toRecord(current);
    if (!record) return null;
    current = record[segment];
  }
  return toNonEmptyString(current);
}

function pickFirstStringPath(
  roots: Array<Record<string, unknown> | null>,
  paths: string[][],
): string | null {
  for (const root of roots) {
    for (const path of paths) {
      const value = pickStringPath(root, path);
      if (value) return value;
    }
  }
  return null;
}

export function getTraeAccountPlatformId(account: TraeAccount): TraeAccountPlatformId {
  const authRaw = toRecord(account.trae_auth_raw);
  const profileRaw = toRecord(account.trae_profile_raw);
  const profileRoot = getTraeProfileRoot(account);
  const serverRaw = toRecord(account.trae_server_raw);
  const entitlementRaw = toRecord(account.trae_entitlement_raw);
  const usageRaw = toRecord(account.trae_usage_raw);
  const roots = [authRaw, profileRaw, profileRoot, serverRaw, entitlementRaw, usageRaw];

  const explicitPlatform = pickFirstStringPath(roots, [
    ['platformId'],
    ['platform_id'],
    ['platform'],
    ['platform', 'platformId'],
    ['platform', 'platform_id'],
  ]);
  const normalizedPlatform = normalizeTraeAccountPlatformId(explicitPlatform);
  if (normalizedPlatform) return normalizedPlatform;

  const clientId = pickFirstStringPath(roots, [
    ['authClientId'],
    ['clientId'],
    ['ClientID'],
    ['exchangeResponse', 'ClientID'],
    ['exchangeResponse', 'Result', 'ClientID'],
    ['platform', 'authClientId'],
  ]);
  const isSolo = clientId === TRAE_SOLO_AUTH_CLIENT_ID;

  const domainHint = (
    pickFirstStringPath(roots, [
      ['authDomain'],
      ['loginHost'],
      ['apiHost'],
      ['host'],
      ['callbackQuery', 'host'],
      ['platform', 'authDomain'],
      ['Result', 'Host'],
      ['Result', 'AIPayHost'],
      ['Result', 'AIHost'],
    ]) ?? ''
  ).toLowerCase();
  const providerHint = (
    pickFirstStringPath(roots, [
      ['providerCode'],
      ['packageType'],
      ['platformName'],
      ['platform', 'providerCode'],
      ['platform', 'packageType'],
      ['platform', 'platformName'],
    ]) ?? ''
  ).toLowerCase();
  const isCn =
    domainHint.includes('trae.cn') ||
    domainHint.includes('trae.com.cn') ||
    providerHint === 'cn' ||
    providerHint.endsWith('_cn') ||
    providerHint.includes(' cn');

  if (isSolo && isCn) return 'trae_solo_cn';
  if (isSolo) return 'trae_solo';
  if (isCn) return 'trae_cn';
  if (clientId === TRAE_AUTH_CLIENT_ID) return 'trae';
  return 'trae';
}

function normalizeTraeLoginProvider(rawProvider: string | null): string | null {
  if (!rawProvider) return null;
  const normalized = rawProvider.trim().toLowerCase();
  if (!normalized) return null;

  switch (normalized) {
    case 'github':
      return 'GitHub';
    case 'google':
      return 'Google';
    case 'gitlab':
      return 'GitLab';
    case 'apple':
      return 'Apple';
    case 'email':
    case 'password':
      return 'Email';
    default:
      return rawProvider.trim();
  }
}

export function getTraeAccountDisplayEmail(account: TraeAccount): string {
  const profileRoot = getTraeProfileRoot(account);
  const authAccountRoot = getTraeAuthAccountRoot(account);
  return (
    pickFirstString(profileRoot, ['NonPlainTextEmail', 'Email', 'email']) ??
    pickFirstString(authAccountRoot, ['email']) ??
    pickFirstString(toRecord(account.trae_auth_raw), ['email']) ??
    toNonEmptyString(account.email) ??
    'unknown'
  );
}

export function getTraeAccountDisplayName(account: TraeAccount): string {
  const profileRoot = getTraeProfileRoot(account);
  const authAccountRoot = getTraeAuthAccountRoot(account);
  return (
    toNonEmptyString(account.nickname) ??
    pickFirstString(profileRoot, ['ScreenName', 'Nickname', 'nickname', 'Name', 'name', 'displayName']) ??
    pickFirstString(authAccountRoot, ['username', 'nickname', 'name', 'displayName']) ??
    getTraeAccountDisplayEmail(account)
  );
}

export function getTraeLoginProvider(account: TraeAccount): string | null {
  const profileRoot = getTraeProfileRoot(account);
  const rawProvider = pickFirstString(profileRoot, ['LastLoginType']);

  return normalizeTraeLoginProvider(rawProvider);
}

export function isTraeCnAccountPlatform(account: TraeAccount): boolean {
  const platformId = getTraeAccountPlatformId(account);
  return platformId === 'trae_cn' || platformId === 'trae_solo_cn';
}

export function getTraePlanBadge(account: TraeAccount): string {
  const preferCn = isTraeCnAccountPlatform(account);
  const usageFromPackList = getUsageStatusFromPackList(account.trae_usage_raw, {
    preferCnSelection: preferCn,
  });
  const raw =
    usageFromPackList?.identityStr?.trim() ||
    getPlanFromEntitlementOrServer(account);
  if (!raw) return 'UNKNOWN';
  return raw;
}

export function getTraePlanDisplayName(account: TraeAccount): string {
  return getTraePlanBadge(account);
}

export function getTraePlanBadgeClass(planType?: string | null): string {
  const normalized = (planType || '').trim().toLowerCase();
  if (!normalized) return 'unknown';
  if (normalized.includes('free')) return 'free';
  if (
    normalized.includes('pro') ||
    normalized.includes('plus') ||
    normalized.includes('ultra') ||
    normalized.includes('lite') ||
    normalized.includes('trial') ||
    normalized.includes('cnexpress') ||
    normalized.includes('express')
  ) {
    return 'pro';
  }
  if (normalized.includes('enterprise') || normalized.includes('team')) return 'enterprise';
  return 'unknown';
}

function getCnEntitlementDetailFields(account: TraeAccount): {
  fastRequestPerMonth: number | null;
  canGetExpressStatus: number | null;
  soloParallelLimit: number | null;
  hasSoloPackage: boolean;
} {
  const entitlement = toRecord(account.trae_entitlement_raw);
  const server = toRecord(account.trae_server_raw);
  const detail =
    pickNestedObject(entitlement, ['detail']) ??
    pickNestedObject(pickNestedObject(server, ['entitlementInfo']), ['detail']) ??
    pickNestedObject(pickNestedObject(server, ['originPayStatusData']), ['detail']);
  const quota =
    pickNestedObject(entitlement, ['quota']) ??
    pickNestedObject(pickNestedObject(server, ['entitlementInfo']), ['quota']);

  const fastRequestPerMonth =
    pickFirstNumber(detail, ['fast_request_per', 'fastRequestPer']) ?? null;
  const canGetExpressStatus =
    pickFirstNumber(detail, ['can_get_express_status', 'canGetExpressStatus']) ?? null;
  const soloParallelLimit = pickFirstNumber(quota, ['solo_agent_parallel_limit']);
  const hasSoloPackage = [
    'enable_solo_agent',
    'enable_solo_builder',
    'enable_solo_coder',
    'enable_solo_lite',
    'enable_solo_web',
  ].some((key) => toBoolean(quota?.[key]) === true);

  return {
    fastRequestPerMonth,
    canGetExpressStatus,
    soloParallelLimit,
    hasSoloPackage,
  };
}

export function getTraeUsage(account: TraeAccount): TraeUsage {
  const preferCn = isTraeCnAccountPlatform(account);
  const usageFromPackList = getUsageStatusFromPackList(account.trae_usage_raw, {
    preferCnSelection: preferCn,
  });
  const cnDetail = preferCn
    ? getCnEntitlementDetailFields(account)
    : {
        fastRequestPerMonth: null,
        canGetExpressStatus: null,
        soloParallelLimit: null,
        hasSoloPackage: false,
      };

  if (usageFromPackList) {
    return {
      ...usageFromPackList,
      fastRequestPerMonth: cnDetail.fastRequestPerMonth,
      canGetExpressStatus: cnDetail.canGetExpressStatus,
      soloParallelLimit: cnDetail.soloParallelLimit,
      hasSoloPackage: cnDetail.hasSoloPackage || usageFromPackList.hasPackage === true,
    };
  }

  const creditsSummary = preferCn ? getCreditsUsageSummary(account.trae_usage_raw) : null;
  const isCreditsBilling = creditsSummary?.isCreditsBilling === true;
  let fallbackPercent: number | null = null;
  if (isCreditsBilling && creditsSummary?.total != null && creditsSummary.total > 0) {
    fallbackPercent = Math.max(
      0,
      Math.min(100, Math.round(((creditsSummary.consumed ?? 0) / creditsSummary.total) * 100)),
    );
  }

  return {
    usedPercent: fallbackPercent,
    spentUsd: null,
    totalUsd: null,
    resetAt: account.plan_reset_at ?? null,
    identityStr: account.plan_type ?? getPlanFromEntitlementOrServer(account),
    basicQuota: null,
    basicUsage: null,
    bonusQuota: null,
    bonusUsage: null,
    nextBillingAt: null,
    nextResetDays: null,
    isActive: null,
    isCanceled: null,
    isBilledYearly: null,
    hasPackage: cnDetail.hasSoloPackage,
    payAsYouGoOpen: false,
    payAsYouGoUsd: null,
    usageExhausted: false,
    usageModel: isCreditsBilling ? 'credits' : preferCn ? 'unknown' : 'usd',
    fastRequestAvailable: null,
    fastRequestLimit: null,
    fastRequestUsed: null,
    fastRequestPerMonth: cnDetail.fastRequestPerMonth,
    canGetExpressStatus: cnDetail.canGetExpressStatus,
    soloParallelLimit: cnDetail.soloParallelLimit,
    hasSoloPackage: cnDetail.hasSoloPackage,
    isCreditsBilling,
    creditsConsumed: creditsSummary?.consumed ?? null,
    creditsTotal: creditsSummary?.total ?? null,
  };
}

export function hasTraeQuotaData(account: TraeAccount): boolean {
  return account.trae_usage_raw != null;
}
