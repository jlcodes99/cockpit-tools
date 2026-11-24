import { PACKAGE_CODE as code, type WorkbuddyAccount } from '../types/codebuddy-suite.ts';
import {
  asRecord,
  extractResourceAccounts,
  isActiveResource,
  parseCycleRemain,
  parseCycleTotal,
  parseDateTimeToEpoch,
  parseNumeric,
} from './codebuddy-suite/parser.ts';

const buddyOneVip = 'TCACA_code_043_6oIkxT7Z0S';
export const WORKBUDDY_CREDIT_GROUPS = {
  planBase: [
    code.proMon, code.proMonPlus, code.proYear, code.youth, code.advanced, code.flagship,
    buddyOneVip, code.freeMon, code.freeMonIntl, code.proTrialMon, code.proTrialYear,
  ],
  planBonus: [code.bonus28, code.bonusIntl],
  benefit: [code.activity, code.bonus29, code.bonus30],
  addon: [code.extra, code.extra38, code.extraIntl],
} satisfies Record<string, readonly string[]>;

export type WorkbuddyCreditGroup = keyof typeof WORKBUDDY_CREDIT_GROUPS;
const groupOrder: WorkbuddyCreditGroup[] = ['planBase', 'planBonus', 'benefit', 'addon'];
const basePriority: Record<string, number> = {
  [buddyOneVip]: 5,
  [code.flagship]: 4,
  [code.advanced]: 3,
  [code.proMon]: 2,
  [code.proMonPlus]: 2,
  [code.proYear]: 2,
  [code.youth]: 1,
};

function billingTime(value: unknown): number | null {
  const time = parseDateTimeToEpoch(value);
  return time !== null && time > 0 && time <= 8.64e15 ? time : null;
}

function resourceExpiry(raw: Record<string, unknown>): number | null {
  return billingTime(raw.DeductionEndTime) ?? billingTime(raw.ExpiredTime) ?? billingTime(raw.CycleEndTime);
}

function getWorkbuddyResourceSummary(account: WorkbuddyAccount) {
  const userResource = asRecord(asRecord(account.quota_raw)?.userResource) ?? asRecord(account.usage_raw);
  return asRecord(asRecord(userResource?.data)?.resourceSummary);
}

// The international client uses the subscription name from billing when present.
const internationalPlans = [
  { id: 'buddyonevip', label: 'Buddy One VIP', codes: [buddyOneVip] },
  { id: 'flagship', label: 'Ultimate', codes: [code.flagship] },
  { id: 'advanced', label: 'Advanced', codes: [code.advanced] },
  { id: 'youth', label: 'Youth', codes: [code.youth] },
  { id: 'pro', label: 'Pro', codes: [code.proMon, code.proMonPlus, code.proYear] },
  { id: 'pro_trial', label: 'Pro-Trial', codes: [code.proTrialMon, code.proTrialYear] },
];
const freePackageCodes: readonly string[] = [code.free, code.freeMon, code.freeMonIntl, code.gift];

export function getWorkbuddyAiPlan(account: WorkbuddyAccount) {
  const summary = getWorkbuddyResourceSummary(account);
  const name = typeof summary?.SubscriptionPackageName === 'string'
    ? summary.SubscriptionPackageName.trim() : '';
  const accountType = asRecord(account.profile_raw)?.type;
  const packageCode = summary?.SubscriptionPackageCode;
  const codes = new Set([
    ...extractResourceAccounts(account).filter(isActiveResource).map((item) => item.PackageCode),
    ...(Array.isArray(summary?.Packages) ? summary.Packages.map((item) => asRecord(item)?.PackageCode) : []),
  ]);
  const known = internationalPlans.find((plan) => plan.codes.some((code) => code === packageCode));
  const recognizedFree = typeof packageCode === 'string' && freePackageCodes.includes(packageCode);
  const inferred = internationalPlans.find((plan) => plan.codes.some((code) => codes.has(code)));
  const profileEdition = asRecord(account.profile_raw)?.editionType;
  const accountEdition = typeof profileEdition === 'string' && profileEdition.trim()
    ? profileEdition.trim()
    : account.plan_type?.trim();
  const normalizedEdition = accountEdition?.toLowerCase();
  const reportedEdition = normalizedEdition === 'free' ? 'free'
    : normalizedEdition === 'unknown' ? undefined : accountEdition || undefined;
  const namedFree = name.toLowerCase() === 'free';
  const edition = accountType === 'ultimate' || accountType === 'exclusive'
    ? accountType
    : known?.id ?? (recognizedFree ? 'free' : inferred?.id)
      ?? (namedFree ? 'free' : reportedEdition)
      ?? (summary && summary.IsPaidUser !== true && !packageCode ? 'free' : undefined);
  const label = edition === 'ultimate' ? 'Team'
    : edition === 'exclusive' ? 'Enterprise Plan'
    : internationalPlans.find((plan) => plan.id === edition)?.label ?? edition;
  const style = edition === 'free' ? 'free'
    : edition === 'pro_trial' ? 'trial'
    : edition === 'ultimate' || edition === 'exclusive' ? 'enterprise'
    : internationalPlans.some((plan) => plan.id === edition) ? 'pro' : 'unknown';
  return { label: style === 'free' ? 'FREE' : name || label || 'UNKNOWN', style };
}

/** 是否存在即将到期的积分（与积分明细里「包含即将到期的积分」提示一致）。 */
export function hasWorkbuddyExpiringCredits(account: WorkbuddyAccount, now = Date.now()): boolean {
  return getWorkbuddyCredits(account, now).groups.some((group) => group.expiring);
}

export function workbuddyCreditPercent(total: number, amount: number, round: (value: number) => number) {
  return total > 0 ? Math.max(0, Math.min(100, round(amount / total * 100))) : 0;
}

/** 积分包到期提示的默认统计窗口：24 小时 */
export const EXPIRING_CREDITS_HORIZON_MS = 86_400_000;

export interface ExpiringCreditPackage {
  packageCode: string;
  group: WorkbuddyCreditGroup;
  /** 官方发放原因或套餐名，缺失时为空字符串 */
  name: string;
  left: number;
  expireAt: number;
}

export interface ExpiringCredits {
  total: number;
  packages: ExpiringCreditPackage[];
  /**
   * 最近的未来到期时间（可能落在统计窗口之外）。卡片用它安排一次唤醒，
   * 到积分包进入窗口的那一刻再刷新，避免每张卡片都常驻定时器。
   */
  nextExpiryAt: number | null;
}

/** 官方发放原因优先于套餐名，与积分明细里的展示保持一致 */
function resourceGrantName(raw: Record<string, unknown>): string {
  const attributes = Array.isArray(raw.AccountAttributes) ? raw.AccountAttributes : [];
  const reason = asRecord(attributes.find((attribute) => asRecord(attribute)?.Key === 'grantReason'))?.Value;
  const trimmed = typeof reason === 'string' ? reason.trim() : '';
  if (trimmed) return trimmed;
  return typeof raw.PackageName === 'string' ? raw.PackageName.trim() : '';
}

function workbuddyCreditResources(account: WorkbuddyAccount, international: boolean) {
  return extractResourceAccounts(account).filter(isActiveResource).map((raw) => {
    const cycleEnd = billingTime(raw.CycleEndTime);
    return {
      packageCode: typeof raw.PackageCode === 'string' ? raw.PackageCode : '',
      total: Math.max(0, parseCycleTotal(raw)),
      left: Math.max(0, parseCycleRemain(raw)),
      expireAt: international ? billingTime(raw.DeductionEndTime) : resourceExpiry(raw),
      refreshAt: (!international ? billingTime(raw.CycleResetTime) : null) ?? (cycleEnd == null ? null : cycleEnd + 1000),
      name: resourceGrantName(raw),
    };
  });
}

export function getWorkbuddyCredits(account: WorkbuddyAccount, now = Date.now(), international = false) {
  const summary = getWorkbuddyResourceSummary(account);
  const resources = workbuddyCreditResources(account, international);
  const groups = groupOrder.flatMap((kind) => {
    const codes: readonly string[] = international && kind === 'planBase'
      ? WORKBUDDY_CREDIT_GROUPS.planBase.filter((value) => value !== code.freeMon)
      : international && kind === 'benefit'
        ? [...WORKBUDDY_CREDIT_GROUPS.benefit, code.freeMon, code.gift]
        : WORKBUDDY_CREDIT_GROUPS[kind];
    const members = resources.filter((item) => codes.includes(item.packageCode)
      && (kind === 'planBase' || item.expireAt == null || item.expireAt > now));
    if (members.length === 0) return [];
    if (kind === 'planBase') {
      const priority = (packageCode: string) => basePriority[packageCode] == null
        ? Number(international && (packageCode === code.proTrialMon || packageCode === code.proTrialYear))
        : basePriority[packageCode] + (international ? 1 : 0);
      members.sort((a, b) =>
        priority(b.packageCode) - priority(a.packageCode)
        || Number(b.total === 0 || b.left > 0) - Number(a.total === 0 || a.left > 0)
        || b.total - a.total);
      members.length = 1;
    }
    const total = members.reduce((sum, item) => sum + item.total, 0);
    const left = members.reduce((sum, item) => sum + item.left, 0);
    const usesExpiry = kind === 'benefit' || kind === 'addon';
    const dates = members.map((item) => usesExpiry ? item.expireAt : item.refreshAt)
      .filter((date): date is number => date != null);
    return {
      kind, total, left,
      used: members.reduce((sum, item) => sum + Math.max(0, item.total - item.left), 0),
      time: dates.length ? Math.min(...dates) : null,
      usesExpiry,
      expiring: usesExpiry && members.some((item) =>
        item.left > 0 && item.expireAt != null && item.expireAt - now <= 3 * 86_400_000),
    };
  });
  // The summary can arrive without a matching package detail. Keep the
  // usage row when billing gives the base package's real cycle amounts.
  if (international && !groups.some((group) => group.kind === 'planBase') && Array.isArray(summary?.Packages)) {
    const baseCodes: readonly string[] = WORKBUDDY_CREDIT_GROUPS.planBase.filter((value) => value !== code.freeMon);
    const base = summary.Packages.map(asRecord).find((item) =>
      item?.PackageCode === summary.SubscriptionPackageCode && baseCodes.includes(String(item?.PackageCode)),
    );
    const total = parseNumeric(base?.CycleTotalCapacity);
    const left = parseNumeric(base?.CycleRemainCapacity);
    if (total != null && total > 0 && left != null) {
      groups.unshift({
        kind: 'planBase', total, left: Math.max(0, left),
        used: Math.max(0, parseNumeric(base?.CycleUsedCapacity) ?? total - left),
        time: null, usesExpiry: false, expiring: false,
      });
    }
  }
  const left = Array.isArray(summary?.Packages)
    ? summary.Packages.reduce<number>((sum, item) =>
      sum + Math.max(0, parseNumeric(asRecord(item)?.CycleRemainCapacity) ?? 0), 0)
    : groups.length > 0
      ? groups.reduce((sum, group) => sum + group.left, 0)
      : summary ? 0 : null;
  return { left, groups };
}

/**
 * 收集「即将到期」的积分包：只有奖励积分（benefit）和购买积分（addon）会真正
 * 到期，套餐基础积分到期只是刷新周期，因此不参与统计。
 */
export function getWorkbuddyExpiringCredits(
  account: WorkbuddyAccount,
  now = Date.now(),
  horizonMs = EXPIRING_CREDITS_HORIZON_MS,
  international = false,
): ExpiringCredits {
  const benefitCodes: readonly string[] = international
    ? [...WORKBUDDY_CREDIT_GROUPS.benefit, code.freeMon, code.gift]
    : WORKBUDDY_CREDIT_GROUPS.benefit;
  const addonCodes: readonly string[] = WORKBUDDY_CREDIT_GROUPS.addon;
  const packages: ExpiringCreditPackage[] = [];
  let nextExpiryAt: number | null = null;
  for (const item of workbuddyCreditResources(account, international)) {
    const group: WorkbuddyCreditGroup | null = benefitCodes.includes(item.packageCode) ? 'benefit'
      : addonCodes.includes(item.packageCode) ? 'addon' : null;
    if (group == null || item.left <= 0 || item.expireAt == null) continue;
    const remaining = item.expireAt - now;
    if (remaining <= 0) continue;
    nextExpiryAt = nextExpiryAt == null ? item.expireAt : Math.min(nextExpiryAt, item.expireAt);
    if (remaining > horizonMs) continue;
    packages.push({ packageCode: item.packageCode, group, name: item.name, left: item.left, expireAt: item.expireAt });
  }
  packages.sort((a, b) => a.expireAt - b.expireAt || b.left - a.left);
  return { total: packages.reduce((sum, item) => sum + item.left, 0), packages, nextExpiryAt };
}

export function getWorkbuddyAiCredits(account: WorkbuddyAccount, now = Date.now()) {
  const credits = getWorkbuddyCredits(account, now, true);
  const data = asRecord(asRecord(asRecord(account.quota_raw)?.userResource)?.data);
  return { ...credits, hasBreakdown: Array.isArray(data?.resources) };
}

export function readWorkbuddyCreditPackage(raw: Record<string, unknown>, expired = false) {
  const capacity = parseNumeric(raw.CapacitySizePrecise) ?? 0;
  const total = capacity > 0 ? capacity : Math.max(0, parseNumeric(raw.CycleCapacitySizePrecise) ?? 0);
  const usedValue = capacity > 0 ? raw.CapacityUsedPrecise : raw.CycleCapacityUsedPrecise;
  const remainValue = capacity > 0 ? raw.CapacityRemainPrecise : raw.CycleCapacityRemainPrecise;
  const expiredUsed = expired ? parseNumeric(raw.ActualCycleCapacityUsed) : null;
  const used = Math.max(0, expiredUsed ?? parseNumeric(usedValue) ?? total - (parseNumeric(remainValue) ?? 0));
  const attributes = Array.isArray(raw.AccountAttributes) ? raw.AccountAttributes : [];
  const reason = asRecord(attributes.find((attribute) => asRecord(attribute)?.Key === 'grantReason'))?.Value;
  const packageName = typeof raw.PackageName === 'string' ? raw.PackageName : '';
  return {
    id: raw.ResourceId == null ? '' : String(raw.ResourceId),
    name: (typeof reason === 'string' ? reason.trim() : '') || packageName,
    total,
    used,
    left: Math.max(0, total - used),
    expireAt: resourceExpiry(raw),
  };
}
