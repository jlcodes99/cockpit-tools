/**
 * CodeBuddy Suite 配额模型函数
 *
 * 用于计算和展示配额信息的统一函数
 */

import {
  PACKAGE_CODE,
  ENTERPRISE_ACCOUNT_TYPES,
} from '../../types/codebuddy-suite.ts';
import type {
  CodebuddySuiteAccountBase,
  CodebuddyPlanDetail,
  OfficialQuotaResource,
  OfficialQuotaModel,
  CodebuddyUsage,
  QuotaDisplayItem,
  QuotaCategoryGroup,
} from '../../types/codebuddy-suite.ts';
import {
  asRecord,
  parseDateTimeToEpoch,
  parseCycleTotal,
  parseCycleRemain,
  isActiveResource,
  isExtraPackage,
  isTrialOrFreeMonPackage,
  isProPackage,
  extractResourceAccounts,
  getAccountQuotaUpdatedAtMs,
  aggregateCycleResources,
} from './parser.ts';

/**
 * 将原始资源转换为 OfficialQuotaResource
 */
export function toOfficialQuotaResource(raw: Record<string, unknown>): OfficialQuotaResource {
  const packageCode = typeof raw.PackageCode === 'string' ? raw.PackageCode : null;
  const packageName = typeof raw.PackageName === 'string' ? raw.PackageName : null;
  const scalarDateText = (value: unknown): string | null =>
    typeof value === 'string' || typeof value === 'number' ? String(value) : null;
  const cycleStartTime = scalarDateText(raw.CycleStartTime);
  const cycleEndTime = scalarDateText(raw.CycleEndTime);
  const deductionEndTime = parseDateTimeToEpoch(raw.DeductionEndTime);
  const expiredTime = scalarDateText(raw.ExpiredTime);

  const total = parseCycleTotal(raw);
  const remain = parseCycleRemain(raw);
  const unlimited = raw.Unlimited === true || raw.unlimited === true || total === -1;
  const used = unlimited ? 0 : Math.max(0, total - remain);
  const usedPercent = unlimited ? 0 : total > 0 ? Math.max(0, Math.min(100, (used / total) * 100)) : 0;
  const remainPercent = unlimited ? null : total > 0 ? Math.max(0, Math.min(100, (remain / total) * 100)) : null;

  const cycleEndAt = parseDateTimeToEpoch(raw.CycleEndTime);
  const cycleResetAt = parseDateTimeToEpoch(raw.CycleResetTime);
  const expireAt = deductionEndTime ?? parseDateTimeToEpoch(expiredTime) ?? cycleEndAt;
  const refreshAt = cycleResetAt ?? (cycleEndAt != null && expireAt != null && cycleEndAt !== expireAt ? cycleEndAt + 1000 : null);

  const isBasePackage = packageCode === PACKAGE_CODE.free || packageCode === PACKAGE_CODE.freeMon || packageCode === PACKAGE_CODE.freeMonIntl;

  return {
    packageCode,
    packageName,
    cycleStartTime,
    cycleEndTime,
    deductionEndTime,
    expiredTime,
    total,
    remain,
    used,
    usedPercent,
    remainPercent,
    refreshAt,
    expireAt,
    isBasePackage,
    unlimited,
  };
}

/**
 * 获取套餐详情
 * 遵循官方 CodeBuddy web client 逻辑
 */
export function getPlanDetail(account: CodebuddySuiteAccountBase): CodebuddyPlanDetail {
  const profile = asRecord(account.profile_raw);
  const accountType = typeof profile?.type === 'string' ? profile.type.toLowerCase() : '';

  // 企业账号类型优先
  if (ENTERPRISE_ACCOUNT_TYPES.includes(accountType)) {
    return { type: 'pro', isPro: true, isTrial: false, badge: 'ENTERPRISE', packageCode: null };
  }

  const all = extractResourceAccounts(account);
  const active = all.filter(isActiveResource);

  const proPkg = active.find((a) => {
    const c = typeof a.PackageCode === 'string' ? a.PackageCode : '';
    return c === PACKAGE_CODE.proYear || c === PACKAGE_CODE.proMon || c === PACKAGE_CODE.proMonPlus || c === PACKAGE_CODE.advanced || c === PACKAGE_CODE.flagship || c === PACKAGE_CODE.youth;
  });

  const hasGift = active.some((a) => {
    const c = typeof a.PackageCode === 'string' ? a.PackageCode : '';
    return c === PACKAGE_CODE.gift;
  });

  if (proPkg) {
    const code = typeof proPkg.PackageCode === 'string' ? proPkg.PackageCode : null;
    return { type: 'pro', isPro: true, isTrial: hasGift, badge: 'PRO', packageCode: code };
  }

  if (hasGift) {
    return { type: 'free', isPro: false, isTrial: true, badge: 'TRIAL', packageCode: PACKAGE_CODE.gift };
  }

  if (all.length === 0) {
    return planBadgeFallback(account);
  }

  return { type: 'free', isPro: false, isTrial: false, badge: 'FREE', packageCode: null };
}

/**
 * 套餐徽章回退逻辑
 */
function planBadgeFallback(account: CodebuddySuiteAccountBase): CodebuddyPlanDetail {
  const payment = account.payment_type?.toLowerCase() || '';
  const plan = account.plan_type?.toLowerCase() || '';
  const source = payment || plan;

  if (source.includes('enterprise')) return { type: 'pro', isPro: true, isTrial: false, badge: 'ENTERPRISE', packageCode: null };
  if (source.includes('trial')) return { type: 'free', isPro: false, isTrial: true, badge: 'TRIAL', packageCode: null };
  if (source.includes('pro')) return { type: 'pro', isPro: true, isTrial: false, badge: 'PRO', packageCode: null };
  if (source.includes('free')) return { type: 'free', isPro: false, isTrial: false, badge: 'FREE', packageCode: null };
  if (source) {
    const raw = (account.payment_type || account.plan_type || 'UNKNOWN').toUpperCase();
    return { type: 'free', isPro: false, isTrial: false, badge: raw, packageCode: null };
  }
  return { type: 'free', isPro: false, isTrial: false, badge: 'UNKNOWN', packageCode: null };
}

/**
 * 获取套餐徽章
 */
export function getPlanBadge(account: CodebuddySuiteAccountBase): string {
  return getPlanDetail(account).badge;
}

/**
 * 获取套餐徽章样式类
 */
export function getPlanBadgeClass(badge: string): string {
  switch (badge) {
    case 'FREE':
      return 'plan-badge plan-free';
    case 'PRO':
      return 'plan-badge plan-pro';
    case 'TRIAL':
      return 'plan-badge plan-trial';
    case 'ENTERPRISE':
      return 'plan-badge plan-enterprise';
    default:
      return 'plan-badge plan-unknown';
  }
}

/**
 * 获取使用量信息
 */
export function getUsage(account: CodebuddySuiteAccountBase): CodebuddyUsage {
  const code = account.dosage_notify_code || '';
  return {
    dosageNotifyCode: code,
    dosageNotifyZh: account.dosage_notify_zh || undefined,
    dosageNotifyEn: account.dosage_notify_en || undefined,
    paymentType: account.payment_type || undefined,
    isNormal: !code || code === '0' || code === 'USAGE_NORMAL',
    inlineSuggestionsUsedPercent: null,
    chatMessagesUsedPercent: null,
    allowanceResetAt: null,
  };
}

/**
 * 获取账号状态
 */
export function getAccountStatus(account: CodebuddySuiteAccountBase): string {
  return account.status || 'unknown';
}

/**
 * 获取积分余额
 */
export function getCreditsBalance(account: CodebuddySuiteAccountBase): number | null {
  const active = extractResourceAccounts(account).filter(isActiveResource);
  if (active.length === 0) return null;
  const balance = active.reduce((sum, item) => sum + parseCycleRemain(item), 0);
  if (!Number.isFinite(balance)) return null;
  return Math.max(0, balance);
}

/**
 * 获取账号显示邮箱
 */
export function getAccountDisplayEmail(account: CodebuddySuiteAccountBase): string {
  return account.email || account.nickname || account.uid || account.id;
}

/**
 * 获取账号显示名称
 */
export function getAccountDisplayName(account: CodebuddySuiteAccountBase): string {
  return account.nickname || account.email || account.uid || account.id;
}

/**
 * 获取官方配额模型
 */
export function getOfficialQuotaModel(account: CodebuddySuiteAccountBase): OfficialQuotaModel {
  const updatedAt = getAccountQuotaUpdatedAtMs(account);
  const empty: OfficialQuotaResource = {
    packageCode: PACKAGE_CODE.extra,
    packageName: null,
    cycleStartTime: null,
    cycleEndTime: null,
    deductionEndTime: null,
    expiredTime: null,
    total: 0,
    remain: 0,
    used: 0,
    usedPercent: 0,
    remainPercent: null,
    refreshAt: null,
    expireAt: null,
    isBasePackage: false,
    unlimited: false,
  };

  const all = extractResourceAccounts(account).filter(isActiveResource);
  if (all.length === 0) {
    return { resources: [], extra: empty, updatedAt };
  }

  const pro = all.filter(isProPackage);
  const extras = all.filter(isExtraPackage);
  const trialOrFreeMon = all.filter(isTrialOrFreeMonPackage);
  const free = all.filter((a) => {
    const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
    return code === PACKAGE_CODE.free;
  });
  const activity = all.filter((a) => {
    const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
    return code === PACKAGE_CODE.activity;
  });
  const enterprise = all.filter((a) => {
    const code = typeof a.PackageCode === 'string' ? a.PackageCode : '';
    return code === PACKAGE_CODE.enterprise;
  });

  const mergedTrialOrFreeMon = aggregateCycleResources(trialOrFreeMon);
  const mergedFree = aggregateCycleResources(free);
  const mergedEnterprise = aggregateCycleResources(enterprise);
  const classified = new Set([...pro, ...extras, ...trialOrFreeMon, ...free, ...activity, ...enterprise]);
  const leftovers = all.filter((item) => !classified.has(item));
  const ordered = [mergedTrialOrFreeMon, ...pro, ...activity, mergedEnterprise, mergedFree, ...leftovers].filter(
    (item): item is Record<string, unknown> => item != null && !!(item.PackageCode || item.PackageName),
  );
  const resources = ordered.map(toOfficialQuotaResource);

  const mergedExtra = aggregateCycleResources(extras);
  const extra = mergedExtra ? toOfficialQuotaResource(mergedExtra) : empty;
  return { resources, extra, updatedAt };
}

/**
 * 解析包名称
 */
function resolvePackageName(resource: OfficialQuotaResource): string {
  if (resource.packageName) return resource.packageName;
  if (resource.packageCode === PACKAGE_CODE.enterprise) return '企业版';
  if (resource.packageCode === PACKAGE_CODE.extra) return '加量包';
  if (resource.packageCode === PACKAGE_CODE.activity) return '活动赠送包';
  if (resource.packageCode === PACKAGE_CODE.free || resource.packageCode === PACKAGE_CODE.gift || resource.packageCode === PACKAGE_CODE.freeMon || resource.packageCode === PACKAGE_CODE.freeMonIntl) {
    return '基础体验包';
  }
  if (resource.packageCode === PACKAGE_CODE.proMon || resource.packageCode === PACKAGE_CODE.proMonPlus || resource.packageCode === PACKAGE_CODE.proYear) {
    return '专业版订阅';
  }
  return '基础包';
}

/**
 * 获取配额显示项列表
 */
export function getQuotaDisplayItems(account: CodebuddySuiteAccountBase): QuotaDisplayItem[] {
  const model = getOfficialQuotaModel(account);
  const items: QuotaDisplayItem[] = [];

  for (const resource of model.resources) {
    if (!resource.unlimited && resource.total <= 0 && resource.remain <= 0) continue;

    const remainPercent = resource.remainPercent ?? Math.max(0, 100 - resource.usedPercent);
    const quotaClass = remainPercent <= 10 ? 'low' : remainPercent <= 30 ? 'medium' : 'high';

    items.push({
      key: `base-${resource.packageCode || items.length}`,
      label: resolvePackageName(resource),
      used: resource.used,
      total: resource.total,
      remain: resource.remain,
      usedPercent: resource.usedPercent,
      remainPercent: resource.remainPercent,
      quotaClass,
      refreshAt: resource.refreshAt,
      unlimited: resource.unlimited,
    });
  }

  if (model.extra.unlimited || model.extra.total > 0 || model.extra.remain > 0) {
    const remainPercent = model.extra.remainPercent ?? Math.max(0, 100 - model.extra.usedPercent);
    const quotaClass = remainPercent <= 10 ? 'low' : remainPercent <= 30 ? 'medium' : 'high';

    items.push({
      key: 'extra',
      label: '加量包',
      used: model.extra.used,
      total: model.extra.total,
      remain: model.extra.remain,
      usedPercent: model.extra.usedPercent,
      remainPercent: model.extra.remainPercent,
      quotaClass,
      refreshAt: model.extra.refreshAt,
      unlimited: model.extra.unlimited,
    });
  }

  return items;
}

/**
 * 获取配额分组聚合数据
 */
export function getQuotaCategoryGroups(account: CodebuddySuiteAccountBase, t: (key: string, defaultValue?: string) => string): QuotaCategoryGroup[] {
  const model = getOfficialQuotaModel(account);

  const baseItems: OfficialQuotaResource[] = [];
  const activityItems: OfficialQuotaResource[] = [];
  const extraItems: OfficialQuotaResource[] = [];
  const otherItems: OfficialQuotaResource[] = [];

  for (const resource of model.resources) {
    const code = resource.packageCode;
    const name = resource.packageName || '';
    if (code === PACKAGE_CODE.enterprise) {
      baseItems.push(resource);
    } else if (code === PACKAGE_CODE.free || code === PACKAGE_CODE.gift || code === PACKAGE_CODE.freeMon || code === PACKAGE_CODE.freeMonIntl || code === PACKAGE_CODE.proMon || code === PACKAGE_CODE.proMonPlus || code === PACKAGE_CODE.proYear || code === PACKAGE_CODE.youth || code === PACKAGE_CODE.advanced || code === PACKAGE_CODE.flagship || name.includes('基础')) {
      baseItems.push(resource);
    } else if (code === PACKAGE_CODE.activity || code === PACKAGE_CODE.bonus28 || code === PACKAGE_CODE.bonus29 || code === PACKAGE_CODE.bonus30 || code === PACKAGE_CODE.bonusIntl || name.includes('赠')) {
      activityItems.push(resource);
    } else {
      otherItems.push(resource);
    }
  }

  if (model.extra.unlimited || model.extra.total > 0 || model.extra.remain > 0 || model.extra.used > 0) {
    extraItems.push(model.extra);
  }

  const aggregate = (items: OfficialQuotaResource[]): Omit<QuotaCategoryGroup, 'key' | 'label' | 'items' | 'visible'> => {
    const unlimited = items.some((item) => item.unlimited);
    const total = items.reduce((sum, r) => sum + r.total, 0);
    const remain = items.reduce((sum, r) => sum + r.remain, 0);
    const used = items.reduce((sum, r) => sum + r.used, 0);
    const usedPercent = unlimited ? 0 : total > 0 ? Math.max(0, Math.min(100, (used / total) * 100)) : 0;
    const remainPercent = unlimited ? null : total > 0 ? Math.max(0, Math.min(100, (remain / total) * 100)) : null;
    const quotaClass =
      remainPercent != null ? (remainPercent <= 10 ? 'critical' : remainPercent <= 30 ? 'low' : remainPercent <= 60 ? 'medium' : 'high') : 'high';
    return {
      total: unlimited ? -1 : total,
      remain: unlimited ? -1 : remain,
      used: unlimited ? 0 : used,
      usedPercent,
      remainPercent,
      quotaClass,
      unlimited,
    };
  };

  const baseAgg = aggregate(baseItems);
  const activityAgg = aggregate(activityItems);
  const extraAgg = aggregate(extraItems);
  const otherAgg = aggregate(otherItems);

  return [
    { key: 'base', label: t('codebuddy.quotaCategory.base', '基础体验包'), ...baseAgg, items: baseItems, visible: baseAgg.unlimited || baseAgg.total > 0 },
    { key: 'activity', label: t('codebuddy.quotaCategory.activity', '活动赠送包'), ...activityAgg, items: activityItems, visible: activityAgg.unlimited || activityAgg.total > 0 },
    { key: 'extra', label: t('codebuddy.quotaCategory.extra', '加量包'), ...extraAgg, items: extraItems, visible: extraAgg.unlimited || extraAgg.total > 0 },
    { key: 'other', label: t('codebuddy.quotaCategory.other', '其他'), ...otherAgg, items: otherItems, visible: otherAgg.unlimited || otherAgg.total > 0 },
  ];
}
