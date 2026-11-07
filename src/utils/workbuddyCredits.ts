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

export function getWorkbuddyCredits(account: WorkbuddyAccount, now = Date.now()) {
  const summary = getWorkbuddyResourceSummary(account);
  const resources = extractResourceAccounts(account).filter(isActiveResource).map((raw) => {
    const cycleEnd = billingTime(raw.CycleEndTime);
    return {
      packageCode: typeof raw.PackageCode === 'string' ? raw.PackageCode : '',
      total: Math.max(0, parseCycleTotal(raw)),
      left: Math.max(0, parseCycleRemain(raw)),
      expireAt: resourceExpiry(raw),
      refreshAt: billingTime(raw.CycleResetTime) ?? (cycleEnd == null ? null : cycleEnd + 1000),
    };
  });
  const groups = groupOrder.flatMap((kind) => {
    const codes: readonly string[] = WORKBUDDY_CREDIT_GROUPS[kind];
    const members = resources.filter((item) => codes.includes(item.packageCode)
      && (kind === 'planBase' || item.expireAt == null || item.expireAt > now));
    if (members.length === 0) return [];
    if (kind === 'planBase') {
      members.sort((a, b) =>
        (basePriority[b.packageCode] ?? 0) - (basePriority[a.packageCode] ?? 0)
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
  const left = Array.isArray(summary?.Packages)
    ? summary.Packages.reduce<number>((sum, item) =>
      sum + Math.max(0, parseNumeric(asRecord(item)?.CycleRemainCapacity) ?? 0), 0)
    : groups.length > 0
      ? groups.reduce((sum, group) => sum + group.left, 0)
      : summary ? 0 : null;
  return { left, groups };
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
