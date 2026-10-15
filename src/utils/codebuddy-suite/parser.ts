/**
 * CodeBuddy Suite 共享工具函数
 *
 * 用于解析账号数据、配额信息等的通用工具函数
 */

import { PACKAGE_CODE, RESOURCE_STATUS } from '../../types/codebuddy-suite.ts';
import type { CodebuddySuiteAccountBase } from '../../types/codebuddy-suite.ts';

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
  const numeric = parseNumeric(value);
  if (numeric != null) {
    const absolute = Math.abs(numeric);
    return Math.trunc(absolute > 0 && absolute < 1_000_000_000_000 ? numeric * 1000 : numeric);
  }
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
    parseNumeric(a.CycleCapacitySizePrecise) ?? parseNumeric(a.CycleCapacitySize) ?? parseNumeric(a.CapacitySizePrecise) ?? parseNumeric(a.CapacitySize) ?? 0
  );
}

/**
 * 解析周期配额剩余量
 */
export function parseCycleRemain(a: Record<string, unknown>): number {
  return (
    parseNumeric(a.CycleCapacityRemainPrecise) ?? parseNumeric(a.CycleCapacityRemain) ?? parseNumeric(a.CapacityRemainPrecise) ?? parseNumeric(a.CapacityRemain) ?? 0
  );
}

/**
 * 检查是否为活跃资源
 */
export function isActiveResource(a: Record<string, unknown>): boolean {
  if (a.Unlimited === true || a.unlimited === true) return true;
  const s = parseNumeric(a.Status ?? a.status);
  return s === RESOURCE_STATUS.valid || s === RESOURCE_STATUS.usedUp;
}

/**
 * 检查是否为加量包
 */
export function isExtraPackage(a: Record<string, unknown>): boolean {
  const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
  return code === PACKAGE_CODE.extra || code === PACKAGE_CODE.extraIntl || code === PACKAGE_CODE.extra38;
}

/**
 * 检查是否为试用或免费月包
 */
export function isTrialOrFreeMonPackage(a: Record<string, unknown>): boolean {
  const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
  return code === PACKAGE_CODE.gift || code === PACKAGE_CODE.freeMon || code === PACKAGE_CODE.freeMonIntl || code === PACKAGE_CODE.proTrialMon || code === PACKAGE_CODE.proTrialYear;
}

/**
 * 检查是否为专业版包
 */
export function isProPackage(a: Record<string, unknown>): boolean {
  if (isTrialOrFreeMonPackage(a)) return false;
  const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
  return code === PACKAGE_CODE.proMon || code === PACKAGE_CODE.proMonPlus || code === PACKAGE_CODE.proYear || code === PACKAGE_CODE.youth || code === PACKAGE_CODE.advanced || code === PACKAGE_CODE.flagship;
}

function normalizeCreditsResource(value: unknown): Record<string, unknown> | null {
  const resource = asRecord(value);
  if (!resource) return null;
  const packageCode = resource.commodity_code ?? resource.commodityCode ?? resource.package_code ?? resource.packageCode;
  const packageName = resource.name ?? resource.package_name ?? resource.packageName;
  const total = parseNumeric(resource.total) ?? parseNumeric(resource.limit_num) ?? parseNumeric(resource.limitNum) ?? 0;
  const remain = parseNumeric(resource.remaining) ?? parseNumeric(resource.remain) ?? 0;
  const expireAt = resource.expire_at ?? resource.expireAt;
  return {
    Status: RESOURCE_STATUS.valid,
    PackageCode: typeof packageCode === 'string' ? packageCode : '',
    PackageName: typeof packageName === 'string' ? packageName : '',
    CycleCapacitySizePrecise: String(total),
    CycleCapacityRemainPrecise: String(remain),
    ...(typeof expireAt === 'number' ? { DeductionEndTime: expireAt } : {}),
    ...(typeof expireAt === 'string' ? { ExpiredTime: expireAt } : {}),
  };
}

function normalizeEnterpriseResource(value: unknown): Record<string, unknown> | null {
  const outer = asRecord(value);
  const data = asRecord(outer?.data) ?? outer;
  if (!data) return null;
  const limit = parseNumeric(data.limit_num ?? data.limitNum);
  if (limit == null) return null;
  const used = parseNumeric(data.used_num ?? data.usedNum ?? data.credit) ?? 0;
  const unlimited = limit === -1;
  const remain = unlimited ? -1 : Math.max(0, limit - used);
  return {
    Status: RESOURCE_STATUS.valid,
    PackageCode: PACKAGE_CODE.enterprise,
    PackageName: typeof data.name === 'string' ? data.name : '',
    CycleCapacitySizePrecise: String(limit),
    CycleCapacityRemainPrecise: String(remain),
    CycleCapacityUsedPrecise: String(unlimited ? 0 : used),
    CycleStartTime: data.cycle_start_time ?? data.cycleStartTime ?? null,
    CycleEndTime: data.cycle_end_time ?? data.cycleEndTime ?? null,
    CycleResetTime: data.cycle_reset_time ?? data.cycleResetTime ?? null,
    Unlimited: unlimited,
  };
}

/**
 * 提取资源账号列表
 */
export function extractResourceAccounts(account: CodebuddySuiteAccountBase): Array<Record<string, unknown>> {
  const usageRoot = asRecord(account.usage_raw);
  const quotaRoot = asRecord(account.quota_raw);
  const userResource = asRecord(quotaRoot?.userResource) ?? usageRoot;
  const candidates: Record<string, unknown>[] = [];
  const addCandidate = (value: unknown) => {
    const record = asRecord(value);
    if (record && !candidates.includes(record)) candidates.push(record);
  };
  addCandidate(userResource);
  addCandidate(userResource?.data);
  addCandidate(asRecord(userResource?.data)?.data);
  addCandidate(asRecord(quotaRoot)?.data);
  addCandidate(asRecord(asRecord(quotaRoot)?.data)?.data);

  let legacyList: unknown[] = [];
  let creditsList: Record<string, unknown>[] = [];
  let enterpriseResource: Record<string, unknown> | null = null;
  for (const candidate of candidates) {
    const response = asRecord(candidate.Response ?? candidate.response);
    const payload = asRecord(response?.Data ?? response?.data);
    const accounts = payload?.Accounts ?? payload?.accounts;
    if (legacyList.length === 0 && Array.isArray(accounts)) legacyList = accounts;
    if (creditsList.length === 0 && Array.isArray(candidate.resources)) {
      creditsList = candidate.resources
        .map(normalizeCreditsResource)
        .filter((item): item is Record<string, unknown> => item != null);
    }
    if (!enterpriseResource) enterpriseResource = normalizeEnterpriseResource(candidate);
  }
  const list = legacyList.length > 0 ? legacyList : creditsList.length > 0 ? creditsList : enterpriseResource ? [enterpriseResource] : [];
  return list.filter((a): a is Record<string, unknown> => a != null && typeof a === 'object');
}

/**
 * 获取账号配额更新时间（毫秒）
 */
export function getAccountQuotaUpdatedAtMs(account: CodebuddySuiteAccountBase): number | null {
  const updatedAt = account.usage_updated_at;
  if (typeof updatedAt !== 'number' || !Number.isFinite(updatedAt) || updatedAt <= 0) return null;
  return Math.trunc(updatedAt < 1_000_000_000_000 ? updatedAt * 1000 : updatedAt);
}

/**
 * 聚合周期资源
 */
export function aggregateCycleResources(list: Array<Record<string, unknown>>): Record<string, unknown> | null {
  if (list.length === 0) return null;
  const first = list[0];
  const unlimited = list.some((item) => item.Unlimited === true || item.unlimited === true);
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
    CycleCapacitySizePrecise: String(unlimited ? -1 : totals.total),
    CycleCapacityRemainPrecise: String(unlimited ? -1 : totals.remain),
    Unlimited: unlimited,
  };
}
