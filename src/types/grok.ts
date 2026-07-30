import type {
  CodebuddyUsage,
  OfficialQuotaResource,
  QuotaCategory,
  QuotaCategoryGroup,
} from "./codebuddy-suite";
import { normalizeTimestamp } from "../utils/dataExtract";

export interface GrokProductUsage {
  /** Original product value returned by xAI. */
  product: string;
  usagePercent?: number | null;
  used?: number | null;
  total?: number | null;
  remaining?: number | null;
}

export interface GrokQuota {
  periodType?: string | null;
  periodStart?: string | null;
  periodEnd?: string | null;
  weeklyLimitPercent?: number | null;
  weeklyUsed?: number | null;
  weeklyTotal?: number | null;
  onDemandUsed?: number | null;
  onDemandCap?: number | null;
  prepaidBalance?: number | null;
  frequentUsage?: number | null;
  frequentLimit?: number | null;
  occasionalUsage?: number | null;
  occasionalLimit?: number | null;
  /** Original subscription tier returned by xAI; never localize this value. */
  subscriptionTier?: string | null;
  subscriptionStatus?: string | null;
  products: GrokProductUsage[];
}

/**
 * Sanitized account DTO returned to the UI. OAuth credentials remain in the
 * Rust backend; `access_token` is always empty for shared-view compatibility.
 */
export type GrokAuthMode = "oauth" | "api_key";

export interface GrokAccount {
  id: string;
  email: string;
  access_token: "";
  auth_mode?: GrokAuthMode | null;
  tags?: string[] | null;
  first_name?: string | null;
  last_name?: string | null;
  user_id?: string | null;
  principal_id?: string | null;
  principal_type?: string | null;
  team_id?: string | null;
  profile_image_asset_id?: string | null;
  coding_data_retention_opt_out?: boolean | null;
  expires_at?: number | null;
  /** OpenAI-compatible endpoint configured for this API-key account. */
  api_base_url?: string | null;
  /** Model identifier configured for this API-key account. */
  api_model?: string | null;
  has_grok_code_access?: boolean | null;
  plan_type?: string;
  quota?: GrokQuota | null;
  status?: string | null;
  status_reason?: string | null;
  quota_query_last_error?: string | null;
  quota_query_last_error_at?: number | null;
  usage_updated_at?: number | null;
  /** Preferred CLI working directory bound to this account. */
  working_dir?: string | null;
  created_at: number;
  last_used: number;
}

export function isGrokApiKeyAccount(account: GrokAccount): boolean {
  return (
    account.auth_mode === "api_key" ||
    (account.plan_type || "").trim().toUpperCase() === "API_KEY"
  );
}

export interface GrokQuotaSummaryItem {
  key: string;
  label: string;
  percentage: number;
  resetAtMs: number | null;
  /** Absolute used amount when API provides it (credits/tasks). */
  used?: number | null;
  /** Absolute total/limit amount when API provides it. */
  total?: number | null;
}

export interface GrokUsage extends CodebuddyUsage {
  totalUsedPercent: number | null;
  exhausted: boolean;
}

function finite(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function clampPercent(value: number): number {
  return Math.max(0, Math.min(100, value));
}

function timestampMs(value: unknown): number | null {
  const seconds = normalizeTimestamp(value);
  return seconds == null ? null : seconds * 1000;
}

function quotaClass(remainPercent: number | null): string {
  if (remainPercent == null) return "high";
  if (remainPercent <= 10) return "critical";
  if (remainPercent <= 30) return "low";
  if (remainPercent <= 60) return "medium";
  return "high";
}

function percentageResource(
  code: string,
  name: string,
  usedPercent: number,
  refreshAt: number | null,
): OfficialQuotaResource {
  const used = clampPercent(usedPercent);
  const remain = 100 - used;
  return {
    packageCode: code,
    packageName: name,
    cycleStartTime: null,
    cycleEndTime: null,
    deductionEndTime: null,
    expiredTime: null,
    total: 100,
    remain,
    used,
    usedPercent: used,
    remainPercent: remain,
    refreshAt,
    expireAt: null,
    isBasePackage: true,
  };
}

function amountResource(
  code: string,
  name: string,
  usedValue: number,
  totalValue: number,
  refreshAt: number | null,
): OfficialQuotaResource {
  const total = Math.max(0, totalValue);
  const used = Math.max(0, usedValue);
  const remain = Math.max(0, total - used);
  const usedPercent = total > 0 ? clampPercent((used / total) * 100) : 0;
  const remainPercent = total > 0 ? clampPercent((remain / total) * 100) : null;
  return {
    packageCode: code,
    packageName: name,
    cycleStartTime: null,
    cycleEndTime: null,
    deductionEndTime: null,
    expiredTime: null,
    total,
    remain,
    used,
    usedPercent,
    remainPercent,
    refreshAt,
    expireAt: null,
    isBasePackage: false,
  };
}

function group(
  category: QuotaCategory,
  label: string,
  items: OfficialQuotaResource[],
): QuotaCategoryGroup {
  const total = items.reduce((sum, item) => sum + item.total, 0);
  const used = items.reduce((sum, item) => sum + item.used, 0);
  const remain = items.reduce((sum, item) => sum + item.remain, 0);
  const usedPercent = total > 0 ? clampPercent((used / total) * 100) : 0;
  const remainPercent = total > 0 ? clampPercent((remain / total) * 100) : null;
  return {
    key: category,
    label,
    used,
    total,
    remain,
    usedPercent,
    remainPercent,
    quotaClass: quotaClass(remainPercent),
    items,
    visible: items.length > 0,
  };
}

export function getGrokAccountDisplayEmail(account: GrokAccount): string {
  const email = account.email?.trim();
  if (email && email !== "unknown@grok.local") return email;
  const fullName = [account.first_name, account.last_name]
    .filter(Boolean)
    .join(" ")
    .trim();
  return (
    fullName ||
    account.principal_id?.trim() ||
    account.user_id?.trim() ||
    email ||
    account.id
  );
}

export function getGrokPlanRawValue(account: GrokAccount): string | null {
  if (isGrokApiKeyAccount(account)) {
    return account.plan_type?.trim() || "API_KEY";
  }
  return (
    account.plan_type?.trim() || account.quota?.subscriptionTier?.trim() || null
  );
}

/**
 * Display badge for a Grok account plan.
 * Missing tier returns empty string — UI should show a localized "none" label
 * (e.g. 暂无). Do not invent Free; a false Free happens when billing tier
 * fields were not resolved.
 */
export function getGrokPlanBadge(account: GrokAccount): string {
  if (isGrokApiKeyAccount(account)) {
    // Plan/tier badges show raw values (project rule).
    return getGrokPlanRawValue(account) || "API_KEY";
  }
  const raw = getGrokPlanRawValue(account);
  // Unknown / unresolved tier: leave empty for the UI "none" fallback.
  if (!raw) return "";
  const normalized = raw
    .trim()
    .toUpperCase()
    .replace(/[\s-]+/g, "_");
  const compact = normalized.replace(/_/g, "");
  // Explicit free / invalid enums only — not missing data.
  if (
    [
      "SUBSCRIPTION_TIER_INVALID",
      "SUBSCRIPTION_TIER_FREE",
      "INVALID",
      "FREE",
    ].includes(normalized)
  ) {
    return "Free";
  }
  if (
    ["SUBSCRIPTION_TIER_GROK_PRO", "GROK_PRO"].includes(normalized) ||
    compact === "GROKPRO" ||
    compact === "SUBSCRIPTIONTIERGROKPRO"
  ) {
    return "Grok Pro";
  }
  // XPremiumPlus 等无下划线别名必须先于 XPremium 精确匹配
  if (
    ["SUBSCRIPTION_TIER_X_PREMIUM_PLUS", "X_PREMIUM_PLUS"].includes(normalized) ||
    compact === "XPREMIUMPLUS" ||
    compact === "SUBSCRIPTIONTIERXPREMIUMPLUS"
  ) {
    return "X Premium+";
  }
  if (
    ["SUBSCRIPTION_TIER_X_PREMIUM", "X_PREMIUM"].includes(normalized) ||
    compact === "XPREMIUM" ||
    compact === "SUBSCRIPTIONTIERXPREMIUM"
  ) {
    return "X Premium";
  }
  if (
    ["SUBSCRIPTION_TIER_X_BASIC", "X_BASIC"].includes(normalized) ||
    compact === "XBASIC" ||
    compact === "SUBSCRIPTIONTIERXBASIC"
  ) {
    return "X Basic";
  }
  if (
    [
      "SUBSCRIPTION_TIER_SUPER_GROK_LITE",
      "SUBSCRIPTION_TIER_SUPERGROK_LITE",
    ].includes(normalized) ||
    compact === "SUPERGROKLITE"
  ) {
    return "SuperGrok Lite";
  }
  if (
    [
      "SUBSCRIPTION_TIER_SUPER_GROK_PRO",
      "SUBSCRIPTION_TIER_SUPERGROK_PRO",
    ].includes(normalized) ||
    compact === "SUPERGROKPRO"
  ) {
    return "SuperGrok Pro";
  }
  if (
    [
      "SUBSCRIPTION_TIER_SUPERGROK_HEAVY",
      "SUBSCRIPTION_TIER_GROK_HEAVY",
    ].includes(normalized) ||
    compact === "SUPERGROKHEAVY" ||
    compact === "GROKHEAVY"
  ) {
    return "SuperGrok Heavy";
  }
  if (normalized === "SUBSCRIPTION_TIER_SUPERGROK" || compact === "SUPERGROK") {
    return "SuperGrok";
  }
  return raw;
}

function grokLabelT(key: string, defaultValue?: string): string {
  return defaultValue ?? key;
}

/**
 * Account health uses the same visible quota buckets as the overview cards
 * (weekly pool / products / tasks / on-demand). Most-constrained bucket wins.
 */
export function getGrokUsage(account: GrokAccount): GrokUsage {
  const usagePercents = getGrokQuotaSummaryItems(account, grokLabelT).map(
    (item) => clampPercent(item.percentage),
  );
  // The most constrained bucket drives account health and recommendations.
  const totalUsedPercent =
    usagePercents.length > 0 ? Math.max(...usagePercents) : null;
  const statusText = [
    account.status,
    account.status_reason,
    account.quota_query_last_error,
  ]
    .filter(Boolean)
    .join(" ");
  const exhausted =
    (totalUsedPercent != null && totalUsedPercent >= 100) ||
    /exhausted|used[\s_-]*up|insufficient|limit[\s_-]*(?:reached|exceeded)/i.test(
      statusText,
    );
  const isNormal =
    !account.quota_query_last_error &&
    !exhausted &&
    !/error|invalid|expired|disabled|unauthorized|forbidden|reauth/i.test(
      statusText,
    );
  const statusCode =
    account.quota_query_last_error ||
    account.status_reason ||
    account.status ||
    (account.quota ? "normal" : undefined);

  return {
    dosageNotifyCode: statusCode ?? undefined,
    dosageNotifyZh: statusCode ?? undefined,
    dosageNotifyEn: statusCode ?? undefined,
    isNormal,
    inlineSuggestionsUsedPercent: totalUsedPercent,
    chatMessagesUsedPercent: totalUsedPercent,
    allowanceResetAt: timestampMs(account.quota?.periodEnd),
    totalUsedPercent,
    exhausted,
  };
}

/** Compatibility adapter for shared suite / presentation; single source is summary items. */
export function getGrokQuotaGroups(
  account: GrokAccount,
  t: (key: string, defaultValue?: string) => string,
): QuotaCategoryGroup[] {
  const summaryItems = getGrokQuotaSummaryItems(account, t);
  const baseItems: OfficialQuotaResource[] = summaryItems.map((item) => {
    if (
      item.used != null &&
      item.total != null &&
      Number.isFinite(item.used) &&
      Number.isFinite(item.total) &&
      item.total > 0
    ) {
      return amountResource(
        item.key,
        item.label,
        item.used,
        item.total,
        item.resetAtMs,
      );
    }
    return percentageResource(
      item.key,
      item.label,
      item.percentage,
      item.resetAtMs,
    );
  });
  return [
    group("base", t("grok.quota.included", "套餐用量"), baseItems),
  ];
}

export function hasGrokQuotaData(account: GrokAccount): boolean {
  return getGrokQuotaSummaryItems(account, grokLabelT).length > 0;
}

export function getGrokQuotaClass(
  usedPercent: number | null | undefined,
): "high" | "medium" | "low" | "critical" {
  if (usedPercent == null || !Number.isFinite(usedPercent)) return "high";
  if (usedPercent >= 90) return "critical";
  if (usedPercent >= 70) return "low";
  if (usedPercent >= 40) return "medium";
  return "high";
}

/**
 * Single source of truth for Grok quota rows (overview, health, presentation).
 * Includes weekly shared pool + productUsage + task/on-demand buckets.
 * Product rows may carry billing period reset; task/on-demand rows do not
 * invent a reset time when the API does not provide one.
 */
export function getGrokQuotaSummaryItems(
  account: GrokAccount,
  t: (key: string, defaultValue?: string) => string,
): GrokQuotaSummaryItem[] {
  if (isGrokApiKeyAccount(account)) {
    // API key accounts bill via xAI API credits, not SuperGrok product quotas.
    return [];
  }
  const quota = account.quota;
  if (!quota) return [];
  const billingResetAtMs = timestampMs(quota.periodEnd);
  const items: GrokQuotaSummaryItem[] = [];

  // 周总池（creditUsagePercent / weeklyCredits）——展示剩余时由调用方 100-used
  const weeklyUsed = finite(quota.weeklyUsed);
  const weeklyTotal = finite(quota.weeklyTotal);
  const weeklyUsedPercent =
    finite(quota.weeklyLimitPercent) ??
    (weeklyUsed != null && weeklyTotal != null && weeklyTotal > 0
      ? (weeklyUsed / weeklyTotal) * 100
      : null);
  if (weeklyUsedPercent != null || (weeklyUsed != null && weeklyTotal != null)) {
    items.push({
      key: "weekly",
      label: t("grok.quota.weekly", "每周用量"),
      percentage: clampPercent(weeklyUsedPercent ?? 0),
      used: weeklyUsed,
      total: weeklyTotal,
      resetAtMs: billingResetAtMs,
    });
  }

  (quota.products ?? []).forEach((product, index) => {
    const used = finite(product.used);
    const total = finite(product.total);
    const remaining = finite(product.remaining);
    const resolvedUsed =
      used ??
      (total != null && remaining != null
        ? Math.max(0, total - remaining)
        : null);
    const resolvedTotal =
      total ??
      (used != null && remaining != null ? used + remaining : null);
    const usagePercent =
      finite(product.usagePercent) ??
      (resolvedUsed != null && resolvedTotal != null && resolvedTotal > 0
        ? (resolvedUsed / resolvedTotal) * 100
        : remaining != null && resolvedTotal != null && resolvedTotal > 0
          ? ((resolvedTotal - remaining) / resolvedTotal) * 100
          : null);
    if (usagePercent == null && resolvedUsed == null && resolvedTotal == null) {
      return;
    }
    items.push({
      key: `product-${index}`,
      label: product.product || t("grok.quota.included", "套餐用量"),
      percentage: clampPercent(usagePercent ?? 0),
      used: resolvedUsed,
      total: resolvedTotal,
      resetAtMs: billingResetAtMs,
    });
  });

  const frequentLimit = finite(quota.frequentLimit);
  const frequentUsage = finite(quota.frequentUsage);
  if (frequentLimit != null && frequentLimit > 0 && frequentUsage != null) {
    items.push({
      key: "frequent",
      label: t("grok.quota.frequent", "高频任务"),
      percentage: clampPercent((frequentUsage / frequentLimit) * 100),
      used: frequentUsage,
      total: frequentLimit,
      // Task usage API does not expose a dedicated reset timestamp.
      resetAtMs: null,
    });
  }

  const occasionalLimit = finite(quota.occasionalLimit);
  const occasionalUsage = finite(quota.occasionalUsage);
  if (
    occasionalLimit != null &&
    occasionalLimit > 0 &&
    occasionalUsage != null
  ) {
    items.push({
      key: "occasional",
      label: t("grok.quota.occasional", "普通任务"),
      percentage: clampPercent((occasionalUsage / occasionalLimit) * 100),
      used: occasionalUsage,
      total: occasionalLimit,
      resetAtMs: null,
    });
  }

  const onDemandCap = finite(quota.onDemandCap);
  const onDemandUsed = finite(quota.onDemandUsed);
  if (onDemandCap != null && onDemandCap > 0 && onDemandUsed != null) {
    items.push({
      key: "on-demand",
      label: t("grok.quota.onDemand", "按量用量"),
      percentage: clampPercent((onDemandUsed / onDemandCap) * 100),
      used: onDemandUsed,
      total: onDemandCap,
      resetAtMs: null,
    });
  }

  return items;
}

export function formatGrokQuotaAmount(
  value: number | null | undefined,
): string {
  if (value == null || !Number.isFinite(value)) return "";
  if (Math.abs(value - Math.round(value)) < 1e-6) {
    return String(Math.round(value));
  }
  return value.toFixed(1).replace(/\.0$/, "");
}

export function formatGrokQuotaUsedTotal(
  used: number | null | undefined,
  total: number | null | undefined,
): string {
  const usedText = formatGrokQuotaAmount(used);
  const totalText = formatGrokQuotaAmount(total);
  if (usedText && totalText) return `${usedText}/${totalText}`;
  if (totalText) return totalText;
  if (usedText) return usedText;
  return "";
}

export function formatGrokQuotaResetTime(
  value: number | null | undefined,
): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return "";
  try {
    return new Date(value).toLocaleString();
  } catch {
    return "";
  }
}
