/**
 * CodeBuddy Suite 共享工具函数
 *
 * 用于解析账号数据、配额信息等的通用工具函数
 */

import { PACKAGE_CODE, RESOURCE_STATUS, CodebuddySuiteAccountBase } from '../../types/codebuddy-suite';

/**
 * 将未知值转换为 Record 对象
 */
export function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
}

/**
 * 解析数值
 */
export function parseNumeric(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

/**
 * 解析日期时间字符串为 Epoch 时间戳
 */
export function parseDateTimeToEpoch(value: unknown): number | null {
  if (typeof value !== 'string') return null;
  const text = value.trim();
  if (!text) return null;
  const isoText = text.includes('T') ? text : text.replace(' ', 'T');
  const parsed = Date.parse(isoText);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * 解析周期配额总量
 */
export function parseCycleTotal(a: Record<string, unknown>): number {
  return (
    parseNumeric(a.CycleCapacitySizePrecise) ??
    parseNumeric(a.CycleCapacitySize) ??
    parseNumeric(a.CapacitySizePrecise) ??
    parseNumeric(a.CapacitySize) ??
    parseNumeric(a.total) ??
    0
  );
}

/**
 * 解析周期配额剩余量
 */
export function parseCycleRemain(a: Record<string, unknown>): number {
  return (
    parseNumeric(a.CycleCapacityRemainPrecise) ??
    parseNumeric(a.CycleCapacityRemain) ??
    parseNumeric(a.CapacityRemainPrecise) ??
    parseNumeric(a.CapacityRemain) ??
    parseNumeric(a.remaining) ??
    parseNumeric(a.remain) ??
    0
  );
}

/**
 * 检查是否为活跃资源
 */
export function isActiveResource(a: Record<string, unknown>): boolean {
  const s = typeof a.Status === 'number' ? a.Status : null;
  if (s === RESOURCE_STATUS.valid || s === RESOURCE_STATUS.usedUp) return true;
  // Official WorkBuddy desktop credits resources may omit Status.
  if (s == null && (parseCycleTotal(a) > 0 || parseCycleRemain(a) > 0)) return true;
  return false;
}

/**
 * 检查是否为加量包
 */
export function isExtraPackage(a: Record<string, unknown>): boolean {
  return typeof a.PackageCode === 'string' && a.PackageCode === PACKAGE_CODE.extra;
}

/**
 * 检查是否为试用或免费月包
 */
export function isTrialOrFreeMonPackage(a: Record<string, unknown>): boolean {
  const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
  return code === PACKAGE_CODE.gift || code === PACKAGE_CODE.freeMon;
}

/**
 * 检查是否为专业版包
 */
export function isProPackage(a: Record<string, unknown>): boolean {
  if (isTrialOrFreeMonPackage(a)) return false;
  const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
  return code === PACKAGE_CODE.proMon || code === PACKAGE_CODE.proYear;
}

function mapDesktopCreditResource(raw: Record<string, unknown>): Record<string, unknown> {
  const remaining = parseNumeric(raw.remaining) ?? parseNumeric(raw.remain) ?? parseCycleRemain(raw);
  const total = parseNumeric(raw.total) ?? parseCycleTotal(raw) ?? remaining;
  const name =
    (typeof raw.name === 'string' && raw.name) ||
    (typeof raw.PackageName === 'string' && raw.PackageName) ||
    null;
  const code =
    (typeof raw.commodity_code === 'string' && raw.commodity_code) ||
    (typeof raw.commodityCode === 'string' && raw.commodityCode) ||
    (typeof raw.PackageCode === 'string' && raw.PackageCode) ||
    null;
  const inferredCode = code || (name?.includes('基础') ? PACKAGE_CODE.free : null);
  const expire = raw.expire_at ?? raw.expireAt ?? raw.ExpiredTime ?? raw.CycleEndTime ?? null;
  const expireText = expire == null ? null : String(expire);
  const status =
    typeof raw.Status === 'number' ? raw.Status : remaining > 0 ? RESOURCE_STATUS.valid : RESOURCE_STATUS.usedUp;

  return {
    ...raw,
    PackageCode: inferredCode,
    PackageName: name,
    CycleCapacitySizePrecise: String(total),
    CycleCapacityRemainPrecise: String(remaining),
    CycleCapacitySize: total,
    CycleCapacityRemain: remaining,
    Status: status,
    ExpiredTime: expireText,
    DeductionEndTime: expire,
    CycleEndTime: expireText,
    __source: 'desktop-credits',
  };
}

/**
 * 提取资源账号列表
 */
export function extractResourceAccounts(account: CodebuddySuiteAccountBase): Array<Record<string, unknown>> {
  const usageRoot = asRecord(account.usage_raw);
  const quotaRoot = asRecord(account.quota_raw);
  const userResource = asRecord(quotaRoot?.userResource) ?? usageRoot;
  const data = asRecord(userResource?.data);
  const response = asRecord(data?.Response);
  const payload = asRecord(response?.Data);
  const list = Array.isArray(payload?.Accounts) ? (payload!.Accounts as unknown[]) : [];
  if (list.length > 0) {
    return list.filter((a): a is Record<string, unknown> => a != null && typeof a === 'object');
  }

  // Official WorkBuddy desktop v5.4.5 billing() returns data.resources Credits rows.
  const resources = Array.isArray(data?.resources)
    ? data!.resources
    : Array.isArray(userResource?.resources)
      ? userResource!.resources
      : [];
  return resources
    .filter((a): a is Record<string, unknown> => a != null && typeof a === 'object')
    .map(mapDesktopCreditResource);
}

/**
 * 获取账号配额更新时间（毫秒）
 */
export function getAccountQuotaUpdatedAtMs(account: CodebuddySuiteAccountBase): number | null {
  const lastUsed = account.last_used;
  if (typeof lastUsed !== 'number' || !Number.isFinite(lastUsed) || lastUsed <= 0) return null;
  return Math.trunc(lastUsed * 1000);
}

/**
 * 聚合周期资源
 */
export function aggregateCycleResources(list: Array<Record<string, unknown>>): Record<string, unknown> | null {
  if (list.length === 0) return null;
  const first = list[0];
  const totals = list.reduce(
    (acc: { total: number; remain: number }, item) => {
      acc.total += parseCycleTotal(item);
      acc.remain += parseCycleRemain(item);
      return acc;
    },
    { total: 0, remain: 0 },
  );
  return {
    ...first,
    CycleCapacitySizePrecise: String(totals.total),
    CycleCapacityRemainPrecise: String(totals.remain),
  };
}