import {
  getWindsurfCreditsSummary,
  getWindsurfQuotaUsageSummary,
  getWindsurfUsage,
  type WindsurfAccount,
  type WindsurfCreditsSummary,
  type WindsurfQuotaUsageSummary,
  type WindsurfUsage,
} from '../types/windsurf';

const DAILY_RESET_WINDOW_SEC = 24 * 3600;
const WEEKLY_RESET_WINDOW_SEC = 7 * 24 * 3600;
const PLAN_CYCLE_WINDOW_SEC = 14 * 24 * 3600;
const URGENCY_WEIGHT = 1;

type ComputeWindsurfRecommendScoreStaticOptions = {
  nowSec?: number;
  usage?: WindsurfUsage;
  quotaSummary?: WindsurfQuotaUsageSummary;
  summary?: WindsurfCreditsSummary;
};

function normalizeRemainingQuotaPct(usedPercent: number | null): number {
  if (usedPercent == null || !Number.isFinite(usedPercent)) return 1;
  return Math.max(0, Math.min(1, 1 - usedPercent / 100));
}

function computeTimeUrgency(targetSec: number | null | undefined, nowSec: number, windowSec: number): number {
  if (targetSec == null || targetSec <= nowSec) return 0;
  const timeLeftSec = targetSec - nowSec;
  return Math.max(0, Math.min(1, 1 - timeLeftSec / windowSec));
}

export function computeWindsurfRecommendScoreStatic(
  account: WindsurfAccount,
  options: ComputeWindsurfRecommendScoreStaticOptions = {},
): number {
  const nowSec = options.nowSec ?? Math.floor(Date.now() / 1000);
  const usage = options.usage ?? getWindsurfUsage(account);
  const quotaSummary = options.quotaSummary ?? getWindsurfQuotaUsageSummary(account);
  const summary = options.summary ?? getWindsurfCreditsSummary(account);
  const dailyUsedPercent = quotaSummary.dailyUsedPercent ?? usage.inlineSuggestionsUsedPercent;
  const weeklyUsedPercent = quotaSummary.weeklyUsedPercent ?? usage.chatMessagesUsedPercent;

  const dailyRemainingPct = normalizeRemainingQuotaPct(dailyUsedPercent);
  const weeklyRemainingPct = normalizeRemainingQuotaPct(weeklyUsedPercent);
  const availableQuota = (dailyRemainingPct + weeklyRemainingPct) / 2;

  const urgencyBoost = Math.max(
    computeTimeUrgency(quotaSummary.dailyResetAt ?? usage.allowanceResetAt, nowSec, DAILY_RESET_WINDOW_SEC),
    computeTimeUrgency(quotaSummary.weeklyResetAt, nowSec, WEEKLY_RESET_WINDOW_SEC),
    computeTimeUrgency(summary.planEndsAt, nowSec, PLAN_CYCLE_WINDOW_SEC),
  );

  return availableQuota * (1 + URGENCY_WEIGHT * urgencyBoost);
}
