import type { Account } from "../types/account";
import type {
  CodebuddyAccount,
  CodebuddyOfficialQuotaResource,
} from "../types/codebuddy";
import type { CodexAccount } from "../types/codex";
import type { GitHubCopilotAccount } from "../types/githubCopilot";
import type { WindsurfAccount } from "../types/windsurf";
import type { CursorAccount } from "../types/cursor";
import type { GeminiAccount } from "../types/gemini";
import type { KiroAccount, KiroAccountStatus } from "../types/kiro";
import type { QoderAccount, QoderSubscriptionInfo } from "../types/qoder";
import type { TraeAccount } from "../types/trae";
import type {
  WorkbuddyAccount,
  WorkbuddyOfficialQuotaResource,
} from "../types/workbuddy";
import type { ZedAccount } from "../types/zed";
import {
  formatResetTimeDisplay,
  getAntigravityTierBadge,
  getDisplayModels,
  getModelShortName,
  getQuotaClass as getAntigravityQuotaClass,
  matchModelName,
} from "../utils/account";
import {
  CB_PACKAGE_CODE,
  getCodebuddyAccountDisplayEmail,
  getCodebuddyOfficialQuotaModel,
  getCodebuddyPlanBadge,
  getCodebuddyUsage,
} from "../types/codebuddy";
import {
  formatCodexResetTime,
  getCodexCodeReviewQuotaMetric,
  getCodexEffectiveQuotaPercentages,
  getCodexPlanBadgeClass,
  getCodexPlanBadgeLabel,
  getCodexQuotaClass,
  getCodexQuotaWindows,
  isCodexApiKeyAccount,
  isCodexNewApiAccount,
} from "../types/codex";
import {
  formatGitHubCopilotResetTime,
  getGitHubCopilotPlanBadge,
  getGitHubCopilotPlanBadgeClass,
  getGitHubCopilotPlanBadgeLabel,
  getGitHubCopilotQuotaClass,
  getGitHubCopilotUsage,
} from "../types/githubCopilot";
import {
  formatWindsurfResetTime,
  getWindsurfAccountDisplayEmail,
  getWindsurfOfficialUsageMode,
  getWindsurfPlanBadgeClass,
  getWindsurfCreditsSummary,
  getWindsurfPlanDisplayName,
  getWindsurfQuotaUsageSummary,
  getWindsurfResolvedPlanLabel,
  getWindsurfQuotaClass,
} from "../types/windsurf";
import {
  formatCursorUsageDollars,
  getCursorAccountDisplayEmail,
  getCursorOnDemandSummary,
  getCursorPlanDisplayName,
  getCursorPlanBadgeClass,
  getCursorUsage,
  isCursorAccountBanned,
} from "../types/cursor";
import {
  getGeminiAccountDisplayEmail,
  getGeminiPlanDisplayName,
  getGeminiPlanBadgeClass,
  getGeminiTierQuotaSummary,
} from "../types/gemini";
import {
  formatKiroResetTime,
  getKiroAccountDisplayEmail,
  getKiroAccountDisplayUserId,
  getKiroAccountLoginProvider,
  getKiroAccountStatus,
  getKiroAccountStatusReason,
  getKiroCreditsSummary,
  getKiroPlanBadgeClass,
  getKiroPlanDisplayName,
  getKiroQuotaClass,
} from "../types/kiro";
import {
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  getQoderSubscriptionInfo,
  shouldShowQoderSubscriptionReset,
} from "../types/qoder";
import {
  getTraeAccountDisplayName,
  getTraePlanBadge,
  getTraePlanBadgeClass,
  getTraeUsage,
} from "../types/trae";
import {
  WORKBUDDY_PACKAGE_CODE,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyOfficialQuotaModel,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
} from "../types/workbuddy";
import {
  getZedAccountDisplayEmail,
  getZedEditPredictionsMetrics,
  getZedEditPredictionsLabel,
  getZedPlanBadge,
  getZedUsage,
} from "../types/zed";
import type { DisplayGroup, GroupSettings } from "../services/groupService";
import { calculateGroupQuota } from "../services/groupService";

type Translate = {
  (key: string): string;
  (key: string, defaultValue: string): string;
  (key: string, options: Record<string, unknown>): string;
  (key: string, defaultValue: string, options: Record<string, unknown>): string;
};

export interface UnifiedQuotaMetric {
  key: string;
  label: string;
  percentage: number;
  quotaClass: string;
  valueText: string;
  resetText?: string;
  progressPercent?: number;
  showProgress?: boolean;
  resetAt?: string | number | null;
  used?: number;
  total?: number;
  left?: number;
  hintText?: string;
}

export interface UnifiedAccountPresentation {
  id: string;
  displayName: string;
  planLabel: string;
  planClass: string;
  quotaItems: UnifiedQuotaMetric[];
  cycleText?: string;
  sublineText?: string;
  sublineClass?: string;
}

export interface KiroAccountPresentation extends UnifiedAccountPresentation {
  userIdText: string;
  signedInWithText: string;
  addOnExpiryText: string;
  accountStatus: KiroAccountStatus;
  accountStatusReason: string | null;
  isBanned: boolean;
  hasStatusError: boolean;
}

export interface QuotaPreviewLine {
  key: string;
  label: string;
  percentage: number;
  quotaClass: string;
  text: string;
  title: string;
}

type AgQuotaDisplayItem = {
  key: string;
  label: string;
  percentage: number;
  resetTime: string;
};

export type CreditMetrics = {
  usedPercent: number;
  used: number;
  total: number;
  left: number;
};

function toFiniteNumber(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function toJsonRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function readNumber(
  value: Record<string, unknown> | null,
  key: string,
): number | null {
  const raw = value?.[key];
  return typeof raw === "number" && Number.isFinite(raw) ? raw : null;
}

function readString(
  value: Record<string, unknown> | null,
  key: string,
): string {
  const raw = value?.[key];
  return typeof raw === "string" ? raw.trim() : "";
}

function readBoolean(
  value: Record<string, unknown> | null,
  key: string,
): boolean {
  return value?.[key] === true;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  if (value <= 0) return 0;
  if (value >= 100) return 100;
  return Math.round(value);
}

function normalizeUnixSeconds(
  value: number | null | undefined,
): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return undefined;
  }
  if (value > 10_000_000_000) {
    return Math.floor(value / 1000);
  }
  return Math.floor(value);
}

function formatQuotaNumber(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "0";
  }
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(
    Math.max(0, value),
  );
}

function formatRequestCount(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "0";
  }
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: Number.isInteger(value) ? 0 : 1,
  }).format(Math.max(0, value));
}

function formatUsdCurrency(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "$0.00";
  }
  return `$${value.toFixed(2)}`;
}

function formatMicrosUsd(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "$0.00";
  }
  return formatUsdCurrency(value / 1_000_000);
}

function resolveSimplePlanClass(planLabel: string | null | undefined): string {
  const normalized = (planLabel || "").trim().toLowerCase();
  if (!normalized) return "unknown";
  if (normalized.includes("enterprise") || normalized.includes("team"))
    return "enterprise";
  if (normalized.includes("trial")) return "trial";
  if (
    normalized.includes("pro") ||
    normalized.includes("plus") ||
    normalized.includes("ultra") ||
    normalized.includes("ultimate")
  ) {
    return "pro";
  }
  if (normalized.includes("free")) return "free";
  return "unknown";
}

function getRemainingQuotaClass(remainPercent: number | null): string {
  if (remainPercent == null || !Number.isFinite(remainPercent)) return "high";
  if (remainPercent <= 10) return "low";
  if (remainPercent <= 30) return "medium";
  return "high";
}

function formatMetricResetText(
  resetTime: number | null | undefined,
  t: Translate,
): string {
  const normalized = normalizeUnixSeconds(resetTime);
  return normalized ? formatCodexResetTime(normalized, t) : "";
}

function buildUsageStatusSubline(
  isNormal: boolean,
  t: Translate,
  normalKey: string,
  abnormalKey: string,
): Pick<UnifiedAccountPresentation, "sublineText" | "sublineClass"> {
  return {
    sublineText: isNormal ? t(normalKey, "正常") : t(abnormalKey, "异常"),
    sublineClass: isNormal ? "high" : "critical",
  };
}

function resolveCodebuddyResourceLabel(
  resource: CodebuddyOfficialQuotaResource,
  t: Translate,
): string {
  if (resource.packageCode === CB_PACKAGE_CODE.extra) {
    return t("codebuddy.extraCredit.title", "加量包");
  }
  if (resource.packageCode === CB_PACKAGE_CODE.activity) {
    return t("codebuddy.quotaQuery.packageTitle.activity", "活动赠送包");
  }
  if (
    resource.packageCode === CB_PACKAGE_CODE.free ||
    resource.packageCode === CB_PACKAGE_CODE.gift ||
    resource.packageCode === CB_PACKAGE_CODE.freeMon
  ) {
    return t("codebuddy.quotaQuery.packageTitle.base", "基础体验包");
  }
  if (
    resource.packageCode === CB_PACKAGE_CODE.proMon ||
    resource.packageCode === CB_PACKAGE_CODE.proYear
  ) {
    return t("codebuddy.quotaQuery.packageTitle.pro", "专业版订阅");
  }
  return (
    resource.packageName ||
    t("codebuddy.quotaQuery.packageUnknown", "套餐信息未知")
  );
}

function resolveWorkbuddyResourceLabel(
  resource: WorkbuddyOfficialQuotaResource,
  t: Translate,
): string {
  if (resource.packageCode === WORKBUDDY_PACKAGE_CODE.extra) {
    return t("workbuddy.extraCredit.title", "加量包");
  }
  if (resource.packageCode === WORKBUDDY_PACKAGE_CODE.activity) {
    return t("workbuddy.quotaQuery.packageTitle.activity", "活动赠送包");
  }
  if (
    resource.packageCode === WORKBUDDY_PACKAGE_CODE.free ||
    resource.packageCode === WORKBUDDY_PACKAGE_CODE.gift ||
    resource.packageCode === WORKBUDDY_PACKAGE_CODE.freeMon
  ) {
    return t("workbuddy.quotaQuery.packageTitle.base", "基础体验包");
  }
  if (
    resource.packageCode === WORKBUDDY_PACKAGE_CODE.proMon ||
    resource.packageCode === WORKBUDDY_PACKAGE_CODE.proYear
  ) {
    return resource.packageName || "PRO";
  }
  return (
    resource.packageName ||
    t("workbuddy.quotaQuery.packageUnknown", "套餐信息未知")
  );
}

function resolveResourceTimeText(
  resource: Pick<
    CodebuddyOfficialQuotaResource | WorkbuddyOfficialQuotaResource,
    "isBasePackage" | "refreshAt" | "expireAt"
  >,
  t: Translate,
  updatedAtKey: string,
  expireAtKey: string,
): string {
  const primaryTime = resource.isBasePackage
    ? resource.refreshAt
    : resource.expireAt;
  const fallbackTime = resource.isBasePackage
    ? resource.expireAt
    : resource.refreshAt;
  const primaryText = formatMetricResetText(primaryTime, t);
  if (primaryText) {
    return resource.isBasePackage
      ? t(updatedAtKey, {
          time: primaryText,
          defaultValue: "下次刷新时间：{{time}}",
        })
      : t(expireAtKey, {
          time: primaryText,
          defaultValue: "到期时间：{{time}}",
        });
  }
  const fallbackText = formatMetricResetText(fallbackTime, t);
  if (fallbackText) {
    return resource.isBasePackage
      ? t(expireAtKey, {
          time: fallbackText,
          defaultValue: "到期时间：{{time}}",
        })
      : t(updatedAtKey, {
          time: fallbackText,
          defaultValue: "下次刷新时间：{{time}}",
        });
  }
  return "";
}

export function buildCreditMetrics(
  used: number | null | undefined,
  total: number | null | undefined,
  left: number | null | undefined,
): CreditMetrics {
  const safeUsed = toFiniteNumber(used);
  const safeTotal = toFiniteNumber(total);
  const safeLeft = toFiniteNumber(left);

  let usedPercent = 0;
  if (safeTotal != null && safeTotal > 0) {
    if (safeUsed != null) {
      usedPercent = clampPercent((safeUsed / safeTotal) * 100);
    } else if (safeLeft != null) {
      usedPercent = clampPercent(((safeTotal - safeLeft) / safeTotal) * 100);
    }
  }

  return {
    usedPercent,
    used: safeUsed ?? 0,
    total: safeTotal ?? 0,
    left: safeLeft ?? 0,
  };
}

function getAgAccountQuotas(account: Account): Record<string, number> {
  const quotas: Record<string, number> = {};
  if (!account.quota?.models) {
    return quotas;
  }
  for (const model of account.quota.models) {
    quotas[model.name] = model.percentage;
  }
  return quotas;
}

function buildAgDisplayGroupSettings(groups: DisplayGroup[]): GroupSettings {
  const settings: GroupSettings = {
    groupMappings: {},
    groupNames: {},
    groupOrder: groups.map((group) => group.id),
    updatedAt: 0,
    updatedBy: "desktop",
  };

  for (const group of groups) {
    settings.groupNames[group.id] = group.name;
    for (const modelId of group.models) {
      settings.groupMappings[modelId] = group.id;
    }
  }
  return settings;
}

export function getAntigravityGroupResetTimestamp(
  account: Account,
  group: DisplayGroup,
): number | null {
  if (!account.quota?.models?.length) {
    return null;
  }

  let earliest: number | null = null;
  for (const model of account.quota.models) {
    const belongsToGroup = group.models.some((groupModelId) =>
      matchModelName(model.name, groupModelId),
    );
    if (!belongsToGroup) {
      continue;
    }
    const parsed = new Date(model.reset_time);
    if (Number.isNaN(parsed.getTime())) {
      continue;
    }
    const timestamp = parsed.getTime();
    if (earliest === null || timestamp < earliest) {
      earliest = timestamp;
    }
  }
  return earliest;
}

export function getAntigravityQuotaDisplayItems(
  account: Account,
  displayGroups: DisplayGroup[],
): AgQuotaDisplayItem[] {
  const rawDisplayModels = getDisplayModels(account.quota);
  if (rawDisplayModels.length === 0) {
    return [];
  }

  if (displayGroups.length === 0) {
    return rawDisplayModels.map((model) => ({
      key: model.name,
      label: getModelShortName(model.name),
      percentage: model.percentage,
      resetTime: model.reset_time,
    }));
  }

  const quotas = getAgAccountQuotas(account);
  const settings = buildAgDisplayGroupSettings(displayGroups);
  const groupedItems: AgQuotaDisplayItem[] = [];

  for (const group of displayGroups) {
    const percentage = calculateGroupQuota(group.id, quotas, settings);
    if (percentage === null) continue;

    const resetTimestamp = getAntigravityGroupResetTimestamp(account, group);
    groupedItems.push({
      key: `group:${group.id}`,
      label: group.name,
      percentage,
      resetTime: resetTimestamp ? new Date(resetTimestamp).toISOString() : "",
    });
  }

  if (groupedItems.length > 0) {
    return groupedItems;
  }

  return rawDisplayModels.map((model) => ({
    key: model.name,
    label: getModelShortName(model.name),
    percentage: model.percentage,
    resetTime: model.reset_time,
  }));
}

export function buildAntigravityAccountPresentation(
  account: Account,
  displayGroups: DisplayGroup[],
  t: Translate,
): UnifiedAccountPresentation {
  const tierBadge = getAntigravityTierBadge(account.quota);
  const quotaItems = getAntigravityQuotaDisplayItems(
    account,
    displayGroups,
  ).map((item) => ({
    key: item.key,
    label: item.label,
    percentage: item.percentage,
    quotaClass: getAntigravityQuotaClass(item.percentage),
    valueText: `${item.percentage}%`,
    resetText: item.resetTime ? formatResetTimeDisplay(item.resetTime, t) : "",
    resetAt: item.resetTime,
  }));

  return {
    id: account.id,
    displayName: account.email,
    planLabel: tierBadge.label,
    planClass: tierBadge.className,
    quotaItems,
  };
}

function buildCodexNewApiQuotaItems(
  account: CodexAccount,
  t: Translate,
): UnifiedQuotaMetric[] {
  const raw = toJsonRecord(account.quota?.raw_data);
  const provider = readString(raw, "provider");
  if (provider !== "cockpit-api" && provider !== "new-api") {
    return [];
  }
  const profile = toJsonRecord(raw?.profile);
  const usage = toJsonRecord(raw?.usage) ?? toJsonRecord(profile?.usage);
  const total =
    readNumber(usage, "total_granted") ?? readNumber(raw, "total_granted") ?? 0;
  const used =
    readNumber(usage, "total_used") ?? readNumber(raw, "total_used") ?? 0;
  const available =
    readNumber(usage, "total_available") ??
    readNumber(raw, "total_available") ??
    0;
  const unlimited =
    readBoolean(usage, "unlimited_quota") ||
    readBoolean(raw, "unlimited_quota");
  const percentage =
    unlimited || total <= 0
      ? unlimited
        ? 100
        : 0
      : clampPercent((available / total) * 100);
  const expiresAt = readNumber(usage, "expires_at");
  const valueText = unlimited
    ? t("codex.newApi.quota.unlimited", "不限量")
    : readString(usage, "summary_display") ||
      `${formatQuotaNumber(available)} / ${formatQuotaNumber(total)}`;

  return [
    {
      key: "new_api_quota",
      label: t("codex.newApi.quota.available", "额度"),
      percentage,
      quotaClass: getCodexQuotaClass(percentage),
      valueText,
      resetText: formatMetricResetText(expiresAt, t),
      resetAt: expiresAt,
      used,
      total,
      left: available,
      hintText: t("codex.newApi.quota.usedHint", {
        used: formatQuotaNumber(used),
        defaultValue: "已用 {{used}}",
      }),
    },
  ];
}

export function buildCodexAccountPresentation(
  account: CodexAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const apiKeyDisplayName = account.account_name?.trim();
  const displayName =
    isCodexApiKeyAccount(account) && apiKeyDisplayName
      ? apiKeyDisplayName
      : isCodexNewApiAccount(account)
        ? "Codex API"
      : account.email;
  const effectiveQuota = getCodexEffectiveQuotaPercentages(account.quota);
  const weeklyBlocksHourlyHint = effectiveQuota.weeklyBlocksHourly
    ? t("codex.quota.weeklyBlocksHourly", "周额度为 0，5小时额度已不可用")
    : "";
  const newApiQuotaItems = isCodexNewApiAccount(account)
    ? buildCodexNewApiQuotaItems(account, t)
    : [];
  const quotaItems: UnifiedQuotaMetric[] =
    newApiQuotaItems.length > 0
      ? newApiQuotaItems
      : getCodexQuotaWindows(account.quota).map((window) => ({
          key: window.id,
          label: window.label,
          percentage: window.percentage,
          quotaClass: getCodexQuotaClass(window.percentage),
          valueText: `${window.percentage}%`,
          resetText: window.resetTime
            ? formatCodexResetTime(window.resetTime, t)
            : "",
          resetAt: window.resetTime,
          hintText:
            window.id === "primary" && weeklyBlocksHourlyHint
              ? weeklyBlocksHourlyHint
              : undefined,
        }));
  const codeReviewMetric = getCodexCodeReviewQuotaMetric(account.quota);
  if (codeReviewMetric) {
    quotaItems.push({
      key: "code_review",
      label: "Code Review",
      percentage: codeReviewMetric.percentage,
      quotaClass: getCodexQuotaClass(codeReviewMetric.percentage),
      valueText: `${codeReviewMetric.percentage}%`,
      resetText: codeReviewMetric.resetTime
        ? formatCodexResetTime(codeReviewMetric.resetTime, t)
        : "",
      resetAt: codeReviewMetric.resetTime,
    });
  }

  return {
    id: account.id,
    displayName,
    planLabel: getCodexPlanBadgeLabel(account),
    planClass: getCodexPlanBadgeClass(account),
    quotaItems,
  };
}

function buildCopilotMetric(
  percentage: number | null | undefined,
  included: boolean | undefined,
  quotaClassGetter: (value: number) => string,
  includedText: string,
  usageText?: string,
) {
  if (included) {
    return {
      valueText: includedText,
      percentage: 100,
      quotaClass: quotaClassGetter(0),
    };
  }
  if (typeof percentage !== "number" || !Number.isFinite(percentage)) {
    return {
      valueText: "-",
      percentage: 0,
      quotaClass: quotaClassGetter(0),
    };
  }
  const normalized = Math.max(0, Math.min(100, Math.round(percentage)));
  return {
    valueText: usageText || `${normalized}%`,
    percentage: normalized,
    quotaClass: quotaClassGetter(normalized),
  };
}

export function buildGitHubCopilotAccountPresentation(
  account: GitHubCopilotAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const displayName =
    account.email ?? account.github_email ?? account.github_login;
  const planBadge = getGitHubCopilotPlanBadge(account);
  const usage = getGitHubCopilotUsage(account);
  const includedText = t("githubCopilot.usage.included", "Included");
  const premiumUsageText =
    usage.usedPremiumRequests != null &&
    usage.totalPremiumRequests != null &&
    usage.totalPremiumRequests > 0
      ? t("githubCopilot.usage.usedOfTotal", {
          used: formatRequestCount(usage.usedPremiumRequests),
          total: formatRequestCount(usage.totalPremiumRequests),
          defaultValue: "{{used}} / {{total}}",
        })
      : undefined;

  const inline = buildCopilotMetric(
    usage.inlineSuggestionsUsedPercent,
    usage.inlineIncluded,
    getGitHubCopilotQuotaClass,
    includedText,
  );
  const chat = buildCopilotMetric(
    usage.chatMessagesUsedPercent,
    usage.chatIncluded,
    getGitHubCopilotQuotaClass,
    includedText,
  );
  const premium = buildCopilotMetric(
    usage.premiumRequestsUsedPercent,
    usage.premiumIncluded,
    getGitHubCopilotQuotaClass,
    includedText,
    premiumUsageText,
  );

  const inlineReset =
    account.quota?.hourly_reset_time ?? usage.allowanceResetAt ?? null;
  const chatReset =
    account.quota?.weekly_reset_time ?? usage.allowanceResetAt ?? null;

  return {
    id: account.id,
    displayName,
    planLabel: getGitHubCopilotPlanBadgeLabel(planBadge),
    planClass: getGitHubCopilotPlanBadgeClass(planBadge),
    quotaItems: [
      {
        key: "inline",
        label: t("common.shared.quota.hourly", "Inline Suggestions"),
        percentage: inline.percentage,
        quotaClass: inline.quotaClass,
        valueText: inline.valueText,
        resetText: inlineReset
          ? formatGitHubCopilotResetTime(inlineReset, t)
          : "",
        resetAt: inlineReset,
      },
      {
        key: "chat",
        label: t("common.shared.quota.weekly", "Chat messages"),
        percentage: chat.percentage,
        quotaClass: chat.quotaClass,
        valueText: chat.valueText,
        resetText: chatReset ? formatGitHubCopilotResetTime(chatReset, t) : "",
        resetAt: chatReset,
      },
      {
        key: "premium",
        label: t("githubCopilot.columns.premium", "Premium requests"),
        percentage: premium.percentage,
        quotaClass: premium.quotaClass,
        valueText: premium.valueText,
      },
    ],
  };
}

export function buildWindsurfAccountPresentation(
  account: WindsurfAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const usageMode = getWindsurfOfficialUsageMode(account);
  const credits = getWindsurfCreditsSummary(account);
  const quotaSummary = getWindsurfQuotaUsageSummary(account);
  const rawPlan =
    getWindsurfResolvedPlanLabel(account) ?? credits.planName?.trim() ?? null;
  const normalizedPlan = getWindsurfPlanDisplayName(
    rawPlan ?? account.plan_type ?? null,
  );
  const quotaItems: UnifiedQuotaMetric[] = [];
  const cycleText = credits.planEndsAt
    ? formatWindsurfResetTime(credits.planEndsAt, t)
    : t("common.shared.credits.planEndsUnknown", "配额周期时间未知");

  if (usageMode === "quota") {
    const dailyUsedPercent =
      quotaSummary.dailyUsedPercent == null
        ? null
        : clampPercent(quotaSummary.dailyUsedPercent);
    const weeklyUsedPercent =
      quotaSummary.weeklyUsedPercent == null
        ? null
        : clampPercent(quotaSummary.weeklyUsedPercent);

    quotaItems.push({
      key: "daily_quota",
      label: t("windsurf.usageSummary.dailyQuota", "Daily quota usage"),
      percentage: dailyUsedPercent ?? 0,
      progressPercent: dailyUsedPercent ?? 0,
      quotaClass: getWindsurfQuotaClass(dailyUsedPercent ?? 0),
      valueText: dailyUsedPercent == null ? "--" : `${dailyUsedPercent}%`,
      resetText: quotaSummary.dailyResetAt
        ? formatWindsurfResetTime(quotaSummary.dailyResetAt, t)
        : "",
      resetAt: quotaSummary.dailyResetAt,
      showProgress: true,
    });
    quotaItems.push({
      key: "weekly_quota",
      label: t("windsurf.usageSummary.weeklyQuota", "Weekly quota usage"),
      percentage: weeklyUsedPercent ?? 0,
      progressPercent: weeklyUsedPercent ?? 0,
      quotaClass: getWindsurfQuotaClass(weeklyUsedPercent ?? 0),
      valueText: weeklyUsedPercent == null ? "--" : `${weeklyUsedPercent}%`,
      resetText: quotaSummary.weeklyResetAt
        ? formatWindsurfResetTime(quotaSummary.weeklyResetAt, t)
        : "",
      resetAt: quotaSummary.weeklyResetAt,
      showProgress: true,
    });
    quotaItems.push({
      key: "extra_usage_balance",
      label: t(
        "windsurf.usageSummary.extraUsageBalance",
        "Extra usage balance",
      ),
      percentage: 0,
      progressPercent: 0,
      quotaClass: "high",
      valueText: formatMicrosUsd(quotaSummary.overageBalanceMicros),
      showProgress: false,
    });
  } else {
    const promptMetrics = buildCreditMetrics(
      credits.promptCreditsUsed,
      credits.promptCreditsTotal,
      credits.promptCreditsLeft,
    );
    const addOnMetrics = buildCreditMetrics(
      credits.addOnCreditsUsed,
      credits.addOnCreditsTotal,
      credits.addOnCredits,
    );
    const totalCreditsLeft = credits.creditsLeft;

    quotaItems.push({
      key: "credits_left",
      label: t("windsurf.credits.title", "Plan"),
      percentage: 0,
      progressPercent: 0,
      quotaClass: "high",
      valueText:
        totalCreditsLeft != null
          ? t("windsurf.credits.left", {
              value: formatQuotaNumber(totalCreditsLeft),
              defaultValue: "{{value}} credits left",
            })
          : t("windsurf.credits.leftUnknown", "Credits left -"),
      showProgress: false,
    });

    quotaItems.push({
      key: "prompt",
      label: t(
        "windsurf.credits.promptCreditsLeftLabel",
        "prompt credits left",
      ),
      percentage: promptMetrics.usedPercent,
      progressPercent: promptMetrics.usedPercent,
      quotaClass: getWindsurfQuotaClass(promptMetrics.usedPercent),
      valueText:
        promptMetrics.total > 0
          ? t("windsurf.credits.promptLeft", {
              remaining: formatQuotaNumber(promptMetrics.left),
              total: formatQuotaNumber(promptMetrics.total),
              defaultValue: "{{remaining}}/{{total}} prompt credits left",
            })
          : promptMetrics.left > 0
            ? t("windsurf.credits.promptLeftNoTotal", {
                remaining: formatQuotaNumber(promptMetrics.left),
                defaultValue: "{{remaining}} prompt credits left",
              })
            : t("windsurf.credits.promptLeftUnknown", "Prompt credits left -"),
      resetText: cycleText,
      used: promptMetrics.used,
      total: promptMetrics.total,
      left: promptMetrics.left,
      showProgress: true,
    });
    quotaItems.push({
      key: "addon",
      label: t(
        "windsurf.credits.addOnCreditsAvailableLabel",
        "add-on credits available",
      ),
      percentage: addOnMetrics.usedPercent,
      progressPercent: addOnMetrics.usedPercent,
      quotaClass: getWindsurfQuotaClass(addOnMetrics.usedPercent),
      valueText: t("windsurf.credits.addOnAvailable", {
        count: formatQuotaNumber(addOnMetrics.left),
        defaultValue: "{{count}} add-on credits available",
      }),
      resetText: cycleText,
      used: addOnMetrics.used,
      total: addOnMetrics.total,
      left: addOnMetrics.left,
      showProgress: true,
    });
  }

  return {
    id: account.id,
    displayName:
      account.email?.trim() || getWindsurfAccountDisplayEmail(account),
    planLabel: rawPlan || normalizedPlan,
    planClass: getWindsurfPlanBadgeClass(rawPlan ?? account.plan_type ?? null),
    cycleText: usageMode === "quota" ? "" : cycleText,
    quotaItems,
  };
}

export function buildCodebuddyAccountPresentation(
  account: CodebuddyAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const planLabel = getCodebuddyPlanBadge(account);
  const usage = getCodebuddyUsage(account);
  const model = getCodebuddyOfficialQuotaModel(account);
  const quotaItems: UnifiedQuotaMetric[] = [];
  const allResources = [...model.resources];
  if (model.extra.total > 0 || model.extra.remain > 0 || model.extra.used > 0) {
    allResources.push(model.extra);
  }

  allResources.forEach((resource, index) => {
    if (resource.total <= 0 && resource.remain <= 0) {
      return;
    }
    const remainPercent =
      resource.remainPercent ?? Math.max(0, 100 - resource.usedPercent);
    quotaItems.push({
      key: `resource_${index}`,
      label: resolveCodebuddyResourceLabel(resource, t),
      percentage: clampPercent(resource.usedPercent),
      progressPercent: clampPercent(resource.usedPercent),
      quotaClass: getRemainingQuotaClass(remainPercent),
      valueText: t("codebuddy.quota.usedOfTotal", {
        used: formatQuotaNumber(resource.used),
        total: formatQuotaNumber(resource.total),
        defaultValue: "{{used}} / {{total}}",
      }),
      resetText: resolveResourceTimeText(
        resource,
        t,
        "codebuddy.quotaQuery.updatedAt",
        "codebuddy.quotaQuery.expireAt",
      ),
      resetAt: resource.refreshAt ?? resource.expireAt,
      used: resource.used,
      total: resource.total,
      left: resource.remain,
      showProgress: true,
    });
  });

  return {
    id: account.id,
    displayName: getCodebuddyAccountDisplayEmail(account),
    planLabel,
    planClass: resolveSimplePlanClass(planLabel),
    quotaItems,
    ...buildUsageStatusSubline(
      usage.isNormal,
      t,
      "codebuddy.usageNormal",
      "codebuddy.usageAbnormal",
    ),
  };
}

export function buildWorkbuddyAccountPresentation(
  account: WorkbuddyAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const planLabel = getWorkbuddyPlanBadge(account);
  const usage = getWorkbuddyUsage(account);
  const model = getWorkbuddyOfficialQuotaModel(account);
  const quotaItems: UnifiedQuotaMetric[] = [];
  const allResources = [...model.resources];
  if (model.extra.total > 0 || model.extra.remain > 0 || model.extra.used > 0) {
    allResources.push(model.extra);
  }

  allResources.forEach((resource, index) => {
    if (resource.total <= 0 && resource.remain <= 0) {
      return;
    }
    const remainPercent =
      resource.remainPercent ?? Math.max(0, 100 - resource.usedPercent);
    quotaItems.push({
      key: `resource_${index}`,
      label: resolveWorkbuddyResourceLabel(resource, t),
      percentage: clampPercent(resource.usedPercent),
      progressPercent: clampPercent(resource.usedPercent),
      quotaClass: getRemainingQuotaClass(remainPercent),
      valueText: t("workbuddy.quota.usedOfTotal", {
        used: formatQuotaNumber(resource.used),
        total: formatQuotaNumber(resource.total),
        defaultValue: "{{used}} / {{total}}",
      }),
      resetText: resolveResourceTimeText(
        resource,
        t,
        "workbuddy.quotaQuery.updatedAt",
        "workbuddy.quotaQuery.expireAt",
      ),
      resetAt: resource.refreshAt ?? resource.expireAt,
      used: resource.used,
      total: resource.total,
      left: resource.remain,
      showProgress: true,
    });
  });

  return {
    id: account.id,
    displayName: getWorkbuddyAccountDisplayEmail(account),
    planLabel,
    planClass: resolveSimplePlanClass(planLabel),
    quotaItems,
    ...buildUsageStatusSubline(
      usage.isNormal,
      t,
      "workbuddy.usageNormal",
      "workbuddy.usageAbnormal",
    ),
  };
}

export function buildQoderAccountPresentation(
  account: QoderAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const subscription: QoderSubscriptionInfo = getQoderSubscriptionInfo(account);
  const planLabel = getQoderPlanBadge(account);
  const userRemainingPercent =
    subscription.totalUsagePercentage != null
      ? clampPercent(100 - subscription.totalUsagePercentage)
      : subscription.userQuota.remaining != null &&
          subscription.userQuota.total != null &&
          subscription.userQuota.total > 0
        ? clampPercent(
            (subscription.userQuota.remaining / subscription.userQuota.total) *
              100,
          )
        : null;
  const userUsedPercent =
    userRemainingPercent == null
      ? null
      : clampPercent(100 - userRemainingPercent);
  const quotaItems: UnifiedQuotaMetric[] = [];

  if (
    subscription.userQuota.total != null ||
    subscription.userQuota.used != null ||
    subscription.userQuota.remaining != null ||
    userRemainingPercent != null
  ) {
    quotaItems.push({
      key: "included",
      label: t("qoder.usageOverview.includedCredits", "套餐内 Credits"),
      percentage: userRemainingPercent ?? 0,
      progressPercent: userRemainingPercent ?? 0,
      quotaClass: getCursorUsageQuotaClass(userUsedPercent ?? 0),
      valueText:
        userRemainingPercent == null
          ? "--"
          : t("common.shared.remaining", {
              value: `${userRemainingPercent}%`,
              defaultValue: "剩余 {{value}}",
            }),
      resetText:
        subscription.userQuota.used != null ||
        subscription.userQuota.total != null
          ? t("qoder.usageOverview.usedOfTotal", {
              used: formatQuotaNumber(subscription.userQuota.used),
              total: formatQuotaNumber(subscription.userQuota.total),
              defaultValue: "{{used}} / {{total}}",
            })
          : "",
      showProgress: true,
      used: subscription.userQuota.used ?? 0,
      total: subscription.userQuota.total ?? 0,
      left: subscription.userQuota.remaining ?? 0,
    });
  }

  if (
    (subscription.addOnQuota.total ?? 0) > 0 ||
    (subscription.addOnQuota.remaining ?? 0) > 0
  ) {
    const addOnRemainingPercent =
      subscription.addOnQuota.remaining != null &&
      subscription.addOnQuota.total != null &&
      subscription.addOnQuota.total > 0
        ? clampPercent(
            (subscription.addOnQuota.remaining /
              subscription.addOnQuota.total) *
              100,
          )
        : 0;
    quotaItems.push({
      key: "credit_package",
      label: t("common.shared.columns.creditPackage", "Credit Package"),
      percentage: addOnRemainingPercent,
      progressPercent: addOnRemainingPercent,
      quotaClass: getCursorUsageQuotaClass(
        clampPercent(100 - addOnRemainingPercent),
      ),
      valueText: t("qoder.usageOverview.usedOfTotal", {
        used: formatQuotaNumber(subscription.addOnQuota.remaining),
        total: formatQuotaNumber(subscription.addOnQuota.total),
        defaultValue: "{{used}} / {{total}}",
      }),
      showProgress: true,
      used: subscription.addOnQuota.used ?? 0,
      total: subscription.addOnQuota.total ?? 0,
      left: subscription.addOnQuota.remaining ?? 0,
    });
  }

  if (subscription.sharedCreditPackageUsed != null) {
    quotaItems.push({
      key: "shared_credit_package",
      label: t(
        "common.shared.columns.sharedCreditPackage",
        "Shared Credit Package",
      ),
      percentage: 0,
      progressPercent: 0,
      quotaClass: "high",
      valueText: formatQuotaNumber(subscription.sharedCreditPackageUsed),
      showProgress: false,
    });
  }

  return {
    id: account.id,
    displayName: getQoderAccountDisplayEmail(account),
    planLabel,
    planClass: resolveSimplePlanClass(planLabel),
    quotaItems,
    cycleText: shouldShowQoderSubscriptionReset(subscription)
      ? formatMetricResetText(subscription.expiresAt, t)
      : "",
  };
}

export function buildTraeAccountPresentation(
  account: TraeAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const usage = getTraeUsage(account);
  const planLabel = getTraePlanBadge(account);
  const usedPercent =
    typeof usage.usedPercent === "number" && Number.isFinite(usage.usedPercent)
      ? clampPercent(usage.usedPercent)
      : null;
  const remainingPercent =
    usedPercent == null ? null : clampPercent(100 - usedPercent);
  const quotaItems: UnifiedQuotaMetric[] = [];

  if (
    remainingPercent != null ||
    usage.spentUsd != null ||
    usage.totalUsd != null ||
    usage.resetAt != null
  ) {
    quotaItems.push({
      key: "usage",
      label: t("trae.columns.usage", "Usage"),
      percentage: remainingPercent ?? 0,
      progressPercent: remainingPercent ?? 0,
      quotaClass: getCursorUsageQuotaClass(usedPercent ?? 0),
      valueText:
        remainingPercent == null
          ? "--"
          : t("common.shared.remaining", {
              value: `${remainingPercent}%`,
              defaultValue: "剩余 {{value}}",
            }),
      resetText:
        usage.spentUsd != null && usage.totalUsd != null
          ? t("trae.quota.usedOfTotal", {
              used: formatQuotaNumber(usage.spentUsd),
              total: formatQuotaNumber(usage.totalUsd),
              defaultValue: "${{used}} / ${{total}}",
            })
          : formatMetricResetText(usage.resetAt, t),
      showProgress: true,
    });
  }

  if (usage.payAsYouGoOpen != null) {
    quotaItems.push({
      key: "pay_as_you_go",
      label: t("trae.quota.payAsYouGoLabel", "On-Demand Usage"),
      percentage: 0,
      progressPercent: 0,
      quotaClass: usage.payAsYouGoOpen ? "high" : "medium",
      valueText:
        usage.payAsYouGoUsd != null
          ? formatUsdCurrency(usage.payAsYouGoUsd)
          : usage.payAsYouGoOpen
            ? t("common.enabled", "Enabled")
            : t("common.disabled", "Disabled"),
      showProgress: false,
    });
  }

  return {
    id: account.id,
    displayName: getTraeAccountDisplayName(account),
    planLabel,
    planClass: getTraePlanBadgeClass(planLabel),
    quotaItems,
    cycleText: formatMetricResetText(usage.nextBillingAt ?? usage.resetAt, t),
  };
}

function shouldShowKiroAddOn(
  addOnMetrics: CreditMetrics,
  bonusExpireDays: number | null | undefined,
): boolean {
  return (
    addOnMetrics.left > 0 ||
    addOnMetrics.used > 0 ||
    addOnMetrics.total > 0 ||
    (typeof bonusExpireDays === "number" &&
      Number.isFinite(bonusExpireDays) &&
      bonusExpireDays > 0)
  );
}

export function buildKiroAccountPresentation(
  account: KiroAccount,
  t: Translate,
): KiroAccountPresentation {
  const credits = getKiroCreditsSummary(account);
  const rawPlan =
    account.plan_name?.trim() ||
    account.plan_tier?.trim() ||
    credits.planName?.trim() ||
    "";
  const normalizedPlan = getKiroPlanDisplayName(
    rawPlan || account.plan_type || null,
  );
  const promptMetrics = buildCreditMetrics(
    credits.promptCreditsUsed,
    credits.promptCreditsTotal,
    credits.promptCreditsLeft,
  );
  const addOnMetrics = buildCreditMetrics(
    credits.addOnCreditsUsed,
    credits.addOnCreditsTotal,
    credits.addOnCredits,
  );
  const showAddOn = shouldShowKiroAddOn(addOnMetrics, credits.bonusExpireDays);
  const accountStatus = getKiroAccountStatus(account);
  const accountStatusReason = getKiroAccountStatusReason(account);
  const provider = getKiroAccountLoginProvider(account);
  const signedInWithText = provider
    ? t("kiro.account.signedInWithProvider", {
        provider,
        defaultValue: "Signed in with {{provider}}",
      })
    : t("kiro.account.signedInWithUnknown", "Signed in with unknown");

  const addOnExpiryText =
    typeof credits.bonusExpireDays === "number" &&
    Number.isFinite(credits.bonusExpireDays)
      ? t("kiro.credits.expiryDays", {
          days: Math.max(0, Math.round(credits.bonusExpireDays)),
          defaultValue: "{{days}} days",
        })
      : t("kiro.credits.expiryUnknown", "—");
  const cycleText = credits.planEndsAt
    ? formatKiroResetTime(credits.planEndsAt, t)
    : t("common.shared.credits.planEndsUnknown", "配额周期时间未知");

  const quotaItems: UnifiedQuotaMetric[] = [
    {
      key: "prompt",
      label: t("common.shared.columns.promptCredits", "User Prompt credits"),
      percentage: promptMetrics.usedPercent,
      quotaClass: getKiroQuotaClass(promptMetrics.usedPercent),
      valueText: `${promptMetrics.usedPercent}%`,
      resetText: cycleText,
      used: promptMetrics.used,
      total: promptMetrics.total,
      left: promptMetrics.left,
    },
  ];

  if (showAddOn) {
    quotaItems.push({
      key: "addon",
      label: t(
        "common.shared.columns.addOnPromptCredits",
        "Add-on prompt credits",
      ),
      percentage: addOnMetrics.usedPercent,
      quotaClass: getKiroQuotaClass(addOnMetrics.usedPercent),
      valueText: `${addOnMetrics.usedPercent}%`,
      resetText: cycleText,
      used: addOnMetrics.used,
      total: addOnMetrics.total,
      left: addOnMetrics.left,
    });
  }

  return {
    id: account.id,
    displayName: getKiroAccountDisplayEmail(account),
    userIdText: getKiroAccountDisplayUserId(account),
    signedInWithText,
    addOnExpiryText,
    planLabel: rawPlan || normalizedPlan,
    planClass: getKiroPlanBadgeClass(rawPlan || normalizedPlan),
    accountStatus,
    accountStatusReason,
    isBanned: accountStatus === "banned",
    hasStatusError: accountStatus === "error",
    cycleText,
    quotaItems,
  };
}

export interface CursorAccountPresentation extends UnifiedAccountPresentation {
  isBanned: boolean;
}

export interface GeminiAccountPresentation extends UnifiedAccountPresentation {
  isBanned: boolean;
}

function normalizeCursorUsagePercent(
  raw: number | null | undefined,
): number | null {
  if (raw == null || !Number.isFinite(raw)) {
    return null;
  }
  const base = raw > 0 && raw < 1 ? 1 : raw;
  return clampPercent(base);
}

function getCursorUsageQuotaClass(usedPercent: number): string {
  if (usedPercent >= 90) return "low";
  if (usedPercent >= 70) return "medium";
  return "high";
}

export function buildCursorAccountPresentation(
  account: CursorAccount,
  t: Translate,
): CursorAccountPresentation {
  const planLabel = getCursorPlanDisplayName(account);
  const usage = getCursorUsage(account);
  const ratioPercent =
    usage.planUsedCents != null &&
    usage.planLimitCents != null &&
    usage.planLimitCents > 0
      ? (usage.planUsedCents / usage.planLimitCents) * 100
      : null;
  const totalPercent = normalizeCursorUsagePercent(
    usage.totalPercentUsed ?? ratioPercent,
  );
  const autoPercent = normalizeCursorUsagePercent(usage.autoPercentUsed);
  const apiPercent = normalizeCursorUsagePercent(usage.apiPercentUsed);
  const quotaItems: UnifiedQuotaMetric[] = [];

  if (totalPercent != null) {
    quotaItems.push({
      key: "total",
      label: "Total Usage",
      percentage: totalPercent,
      quotaClass: getCursorUsageQuotaClass(totalPercent),
      valueText: `${totalPercent}%`,
      resetAt: usage.allowanceResetAt,
      resetText: usage.allowanceResetAt
        ? formatCodexResetTime(usage.allowanceResetAt, t)
        : "",
    });
  }

  if (autoPercent != null) {
    quotaItems.push({
      key: "auto",
      label: "Auto + Composer",
      percentage: autoPercent,
      quotaClass: getCursorUsageQuotaClass(autoPercent),
      valueText: `${autoPercent}%`,
    });
  }

  if (apiPercent != null) {
    quotaItems.push({
      key: "api",
      label: "API Usage",
      percentage: apiPercent,
      quotaClass: getCursorUsageQuotaClass(apiPercent),
      valueText: `${apiPercent}%`,
    });
  }

  const onDemand = getCursorOnDemandSummary(usage);

  if (onDemand.hasFixedLimit && onDemand.limitCents != null) {
    const rawPercent = (onDemand.usedCents / onDemand.limitCents) * 100;
    const fixedPercent = normalizeCursorUsagePercent(rawPercent) ?? 0;
    quotaItems.push({
      key: "on_demand",
      label: t("cursor.quota.onDemand", "On-Demand"),
      percentage: fixedPercent,
      quotaClass: getCursorUsageQuotaClass(fixedPercent),
      valueText: `${fixedPercent}%`,
      resetText: `${formatCursorUsageDollars(onDemand.usedCents)} / ${formatCursorUsageDollars(onDemand.limitCents)}`,
    });
  } else if (onDemand.isUnlimited) {
    quotaItems.push({
      key: "on_demand",
      label: t("cursor.quota.onDemand", "On-Demand"),
      percentage: 0,
      quotaClass: "high",
      valueText: "Unlimited",
      resetText: formatCursorUsageDollars(onDemand.usedCents),
    });
  } else if (usage.onDemandEnabled != null || usage.onDemandLimitType != null) {
    quotaItems.push({
      key: "on_demand",
      label: t("cursor.quota.onDemand", "On-Demand"),
      percentage: 0,
      quotaClass: "medium",
      valueText: t("common.disabled", "Disabled"),
    });
  }

  return {
    id: account.id,
    displayName: getCursorAccountDisplayEmail(account),
    planLabel,
    planClass: getCursorPlanBadgeClass(account.membership_type, account),
    isBanned: isCursorAccountBanned(account),
    quotaItems,
  };
}

export function buildGeminiAccountPresentation(
  account: GeminiAccount,
  t: Translate,
): GeminiAccountPresentation {
  const tierSummary = getGeminiTierQuotaSummary(account);
  const planLabel = getGeminiPlanDisplayName(account);
  const quotaItems: UnifiedQuotaMetric[] = [];

  [tierSummary.pro, tierSummary.flash].forEach((tier) => {
    const remaining =
      tier.remainingPercent == null
        ? null
        : clampPercent(tier.remainingPercent);
    const usedPercent = remaining == null ? 100 : 100 - remaining;
    quotaItems.push({
      key: tier.key,
      label: t(`gemini.quota.${tier.key}`, tier.label),
      percentage: remaining ?? 0,
      progressPercent: remaining ?? 0,
      quotaClass: getCursorUsageQuotaClass(usedPercent),
      valueText:
        remaining == null
          ? "--"
          : t("gemini.quota.left", "{{value}}% left", { value: remaining }),
      resetText: formatMetricResetText(tier.resetAt, t),
      resetAt: tier.resetAt,
      showProgress: true,
    });
  });

  return {
    id: account.id,
    displayName: getGeminiAccountDisplayEmail(account),
    planLabel,
    planClass: getGeminiPlanBadgeClass(undefined, account),
    isBanned: false,
    quotaItems,
  };
}

export function buildZedAccountPresentation(
  account: ZedAccount,
  t: Translate,
): UnifiedAccountPresentation {
  const planLabel = getZedPlanBadge(account);
  const usage = getZedUsage(account);
  const editUsedPercent =
    usage.chatMessagesUsedPercent == null
      ? null
      : clampPercent(usage.chatMessagesUsedPercent);
  const editRemainingPercent =
    editUsedPercent == null ? null : clampPercent(100 - editUsedPercent);
  const hasEditPredictions =
    account.edit_predictions_used != null ||
    Boolean(account.edit_predictions_limit_raw?.trim());
  const editMetrics = hasEditPredictions
    ? getZedEditPredictionsMetrics(account)
    : null;
  const quotaItems: UnifiedQuotaMetric[] = [];

  if (editMetrics) {
    quotaItems.push({
      key: "edit_predictions",
      label: "Edit Predictions",
      percentage: editUsedPercent ?? 0,
      progressPercent: editUsedPercent ?? 0,
      quotaClass: getRemainingQuotaClass(editRemainingPercent),
      valueText: getZedEditPredictionsLabel(account),
      used: editMetrics.used,
      total: editMetrics.total,
      left: editMetrics.left,
      showProgress: true,
    });
  }

  if (account.has_overdue_invoices != null) {
    quotaItems.push({
      key: "overdue_invoices",
      label: t("zed.page.overdueField", "是否欠费"),
      percentage: 0,
      progressPercent: 0,
      quotaClass: account.has_overdue_invoices ? "low" : "high",
      valueText: account.has_overdue_invoices
        ? t("zed.page.overdueYes", "是")
        : t("zed.page.overdueNo", "否"),
      showProgress: false,
    });
  }

  return {
    id: account.id,
    displayName: getZedAccountDisplayEmail(account),
    planLabel,
    planClass: resolveSimplePlanClass(planLabel),
    quotaItems,
    sublineText: account.subscription_status?.trim() || undefined,
  };
}

export function buildQuotaPreviewLines(
  quotaItems: UnifiedQuotaMetric[],
  limit = 3,
): QuotaPreviewLine[] {
  return quotaItems.slice(0, Math.max(0, limit)).map((item) => ({
    key: item.key,
    label: item.label,
    percentage: item.percentage,
    quotaClass: item.quotaClass,
    text: `${item.label} ${item.valueText}`,
    title: item.hintText
      ? `${item.label} ${item.valueText} · ${item.hintText}`
      : `${item.label} ${item.valueText}`,
  }));
}
