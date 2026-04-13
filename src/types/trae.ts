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

export interface TraeQuota {
  hourly_percentage: number;
  weekly_percentage: number;
  hourly_reset_time?: number | null;
  weekly_reset_time?: number | null;
  raw_data?: unknown;
}

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
  PROMO_CODE: 3,
  PRO_PLUS: 4,
  ULTRA: 6,
  PAY_GO: 7,
  LITE: 8,
  TRIAL: 9,
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
    case TRAE_PRODUCT_TYPE.ULTRA:
      return 'Ultra';
    case TRAE_PRODUCT_TYPE.PRO_PLUS:
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
  const entitlementBase = pickNestedObject(pack, ['entitlement_base_info']);
  return pickNestedObject(entitlementBase, ['quota']);
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

function getUsageStatusFromPackList(rawUsage: unknown): TraeUsage | null {
  const usageRoot = toRecord(rawUsage);
  if (!usageRoot) return null;

  const apiCode = toNumber(usageRoot['code']);
  if (apiCode != null && apiCode !== 0) return null;

  const packs = (toArray(usageRoot.user_entitlement_pack_list) ?? [])
    .map((item) => toRecord(item))
    .filter((item): item is Record<string, unknown> => item != null)
    .filter((item) => getPackProductType(item) !== TRAE_PRODUCT_TYPE.PROMO_CODE);

  if (packs.length === 0) return null;

  const findPackByType = (productType: number) =>
    packs.find((pack) => getPackProductType(pack) === productType) ?? null;

  const freePack = findPackByType(TRAE_PRODUCT_TYPE.FREE);
  const proPack = findPackByType(TRAE_PRODUCT_TYPE.PRO);
  const proPlusPack = findPackByType(TRAE_PRODUCT_TYPE.PRO_PLUS);
  const ultraPack = findPackByType(TRAE_PRODUCT_TYPE.ULTRA);
  const payGoPack = findPackByType(TRAE_PRODUCT_TYPE.PAY_GO);
  const packagePack = findPackByType(TRAE_PRODUCT_TYPE.PACKAGE);
  const litePack = findPackByType(TRAE_PRODUCT_TYPE.LITE);
  const trialPack = findPackByType(TRAE_PRODUCT_TYPE.TRIAL);

  const selectedPack = ultraPack ?? proPlusPack ?? proPack ?? trialPack ?? litePack ?? freePack;
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

  const hasPackage = packagePack != null;
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

  const identityStr = identityFromProductType(selectedProductType) ?? 'Free';
  const derivedPercent = totalUsd > 0 ? (spentUsd / totalUsd) * 100 : 0;

  return {
    usedPercent: Math.max(0, Math.min(100, Math.round(derivedPercent))),
    spentUsd,
    totalUsd,
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
    usageExhausted,
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

export function getTraePlanBadge(account: TraeAccount): string {
  const usageFromPackList = getUsageStatusFromPackList(account.trae_usage_raw);
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
    normalized.includes('trial')
  ) {
    return 'pro';
  }
  if (normalized.includes('enterprise') || normalized.includes('team')) return 'enterprise';
  return 'unknown';
}

export function getTraeUsage(account: TraeAccount): TraeUsage {
  const usageFromPackList = getUsageStatusFromPackList(account.trae_usage_raw);
  if (usageFromPackList) {
    return usageFromPackList;
  }

  return {
    usedPercent: null,
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
    hasPackage: false,
    payAsYouGoOpen: false,
    payAsYouGoUsd: null,
    usageExhausted: false,
  };
}

export function hasTraeQuotaData(account: TraeAccount): boolean {
  return account.trae_usage_raw != null;
}
