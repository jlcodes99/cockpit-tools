import type { TFunction } from "i18next";
import { getCodexPlanFilterKey, type CodexAccount } from "../../types/codex";
import type {
  CodexLocalAccessAccountHealth,
  CodexLocalAccessScope,
  CodexLocalAccessRoutingStrategy,
  CodexLocalAccessState,
  CodexLocalAccessUsageStats,
} from "../../types/codexLocalAccess";
import {
  formatCodexQuotaPoolPercent,
  formatCodexQuotaPoolWindowLabel,
  summarizeCodexQuotaPool,
  type CodexQuotaPoolItem,
} from "../../utils/codexQuotaPool";
import { isBlockingCodexAccountQuotaError } from "../../utils/codexQuotaError";

/** State-free calculations and display formatting shared by the API service modal. */
export interface AccountPoolHealthSummary {
  total: number;
  available: number;
  abnormal: number;
  cooldown: number;
  missing: number;
  authError: number;
  quotaLimited: number;
}

export interface CustomRoutingDraftRule {
  priority: number;
  weight: number;
  isBackup: boolean;
  isPreferred: boolean;
}

export type AccountUsagePriority = "lowest" | "normal" | "highest";

export interface TestChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  latencyMs?: number | null;
  failureTitle?: string;
  failureDetail?: string;
}

export const CUSTOM_ROUTING_PRIORITY_MIN = 0;
export const CUSTOM_ROUTING_PRIORITY_MAX = 100;
export const CUSTOM_ROUTING_WEIGHT_MIN = 1;
export const CUSTOM_ROUTING_WEIGHT_MAX = 100;
const ABNORMAL_ACCOUNT_FAILURE_CATEGORIES = new Set([
  "auth_unavailable",
  "auth_refresh_failed",
  "account_prepare_failed",
]);

export function normalizeAccessScope(value: string): CodexLocalAccessScope {
  return value === "lan" ? "lan" : "localhost";
}

function clampInteger(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function normalizeCustomRoutingPriority(value: number): number {
  return clampInteger(
    value,
    CUSTOM_ROUTING_PRIORITY_MIN,
    CUSTOM_ROUTING_PRIORITY_MAX,
  );
}

export function normalizeCustomRoutingWeight(value: number): number {
  return clampInteger(
    value,
    CUSTOM_ROUTING_WEIGHT_MIN,
    CUSTOM_ROUTING_WEIGHT_MAX,
  );
}

export function resolveAccountUsagePriority(
  rule?: Pick<CustomRoutingDraftRule, "isBackup" | "isPreferred"> | null,
): AccountUsagePriority {
  if (rule?.isPreferred) return "highest";
  if (rule?.isBackup) return "lowest";
  return "normal";
}

export function formatCompactNumber(value: number): string {
  return new Intl.NumberFormat("en", {
    notation: value >= 1000 ? "compact" : "standard",
    maximumFractionDigits: value >= 1000 ? 1 : 0,
  }).format(value || 0);
}

export function formatLatencyMs(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "--";
  if (value >= 1000) return `${(value / 1000).toFixed(2)}s`;
  return `${Math.round(value)}ms`;
}

export function formatUsdCost(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "$0.00";
  if (value < 0.000001) return "<$0.000001";
  if (value < 0.01) return `$${value.toFixed(6)}`;
  if (value < 1) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(2)}`;
}

export function createTestChatMessage(
  role: TestChatMessage["role"],
  content: string,
  extra: Partial<Omit<TestChatMessage, "id" | "role" | "content">> = {},
): TestChatMessage {
  return {
    id: `${role}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    role,
    content,
    ...extra,
  };
}

export function formatQuotaPoolLabel(
  baseLabel: string,
  pool: CodexQuotaPoolItem,
  weeklyLabel: string,
): string {
  const quotaText = pool.windows
    .map(
      (window) =>
        `${formatCodexQuotaPoolWindowLabel(window.label, weeklyLabel)} ${formatCodexQuotaPoolPercent(window.percentage)}`,
    )
    .join(" · ");
  return quotaText ? `${baseLabel} · ${quotaText}` : baseLabel;
}

export function summarizeQuotaPoolsByPlan(
  accounts: CodexAccount[],
): Record<string, CodexQuotaPoolItem> {
  const accountsByPlan: Record<string, CodexAccount[]> = {};
  accounts.forEach((account) => {
    const planKey = getCodexPlanFilterKey(account);
    (accountsByPlan[planKey] ??= []).push(account);
  });
  return Object.fromEntries(
    Object.entries(accountsByPlan).map(([planKey, planAccounts]) => [
      planKey,
      summarizeCodexQuotaPool(planAccounts).all,
    ]),
  );
}

export function areSetsEqual(left: Set<string>, right: Set<string>): boolean {
  if (left.size !== right.size) return false;
  for (const value of left) {
    if (!right.has(value)) return false;
  }
  return true;
}

function isAbnormalAccountFailure(
  health?: CodexLocalAccessAccountHealth,
): boolean {
  return Boolean(
    health &&
    ((health.schedulerAvailable === false && !health.cooldowns.length) ||
      (health.consecutiveFailures >= 3 &&
        health.lastFailureCategory &&
        ABNORMAL_ACCOUNT_FAILURE_CATEGORIES.has(health.lastFailureCategory))),
  );
}

export function formatRequestResultDetail(
  t: TFunction,
  usage?: CodexLocalAccessUsageStats | null,
): string {
  return t("codex.localAccess.stats.requestsDetail", {
    success: formatCompactNumber(usage?.successCount ?? 0),
    failed: formatCompactNumber(
      Math.max(
        (usage?.failureCount ?? 0) -
          (usage?.clientCanceledCount ?? 0) -
          (usage?.upstreamResponseFailedCount ?? 0) -
          (usage?.streamIncompleteCount ?? 0),
        0,
      ),
    ),
    canceled: formatCompactNumber(usage?.clientCanceledCount ?? 0),
    upstreamFailed: formatCompactNumber(
      usage?.upstreamResponseFailedCount ?? 0,
    ),
    incomplete: formatCompactNumber(usage?.streamIncompleteCount ?? 0),
    defaultValue:
      "成功 {{success}} / 失败 {{failed}} / 取消 {{canceled}} / 上游失败 {{upstreamFailed}} / 流未完成 {{incomplete}}",
  });
}

export function buildLocalAccessSummaryStats(
  t: TFunction,
  selectedTotals: CodexLocalAccessUsageStats | undefined,
  avgLatencyMs: number,
  successRate: number,
) {
  return [
    {
      key: "requests",
      label: t("codex.localAccess.stats.requests", "总请求数"),
      value: formatCompactNumber(selectedTotals?.requestCount ?? 0),
      detail: formatRequestResultDetail(t, selectedTotals),
    },
    {
      key: "tokens",
      label: t("codex.localAccess.stats.tokens", "总 Token 数"),
      value: formatCompactNumber(selectedTotals?.totalTokens ?? 0),
      detail: t("codex.localAccess.stats.tokensDetail", {
        input: formatCompactNumber(selectedTotals?.inputTokens ?? 0),
        output: formatCompactNumber(selectedTotals?.outputTokens ?? 0),
        defaultValue: "输入 {{input}} / 输出 {{output}}",
      }),
    },
    {
      key: "specialTokens",
      label: t("codex.localAccess.stats.specialTokens", "缓存 / 思考"),
      value: formatCompactNumber(
        (selectedTotals?.cachedTokens ?? 0) +
          (selectedTotals?.reasoningTokens ?? 0),
      ),
      detail: t("codex.localAccess.stats.specialTokensDetail", {
        cached: formatCompactNumber(selectedTotals?.cachedTokens ?? 0),
        reasoning: formatCompactNumber(selectedTotals?.reasoningTokens ?? 0),
        defaultValue: "缓存 {{cached}} / 思考 {{reasoning}}",
      }),
    },
    {
      key: "cost",
      label: t("codex.localAccess.stats.estimatedCost", "估算价值"),
      value: formatUsdCost(selectedTotals?.estimatedCostUsd ?? 0),
      detail: t(
        "codex.localAccess.stats.estimatedCostDetail",
        "按当前请求价格快照累计",
      ),
    },
    {
      key: "latency",
      label: t("codex.localAccess.stats.avgLatency", "平均延迟"),
      value: formatLatencyMs(avgLatencyMs),
      detail: t("codex.localAccess.stats.successRate", {
        rate: successRate,
        defaultValue: "成功率 {{rate}}%",
      }),
    },
  ];
}

export function summarizeAccountPoolHealth(
  localAccessAccounts: CodexAccount[],
  accountIds: string[],
  state: Pick<CodexLocalAccessState, "accountHealth" | "accountPoolHealth" | "recoverySuppressedAccountIds"> | null,
): AccountPoolHealthSummary {
  const accountById = new Map(
    localAccessAccounts.map((account) => [account.id, account]),
  );
  const healthById = new Map(
    (state?.accountHealth ?? []).map((health) => [health.accountId, health]),
  );
  // 手动恢复后仍在抑制窗口内的账号：与账号状态弹框保持一致，
  // 不再计入异常，也暂不参与“可用/异常”统计。
  const suppressedAccountIds = new Set(
    (state?.recoverySuppressedAccountIds ?? [])
      .map((accountId) => accountId.trim())
      .filter(Boolean),
  );
  const poolUnavailableAccountIds = new Set<string>();
  (state?.accountPoolHealth ?? []).forEach((pool) => {
    const statuses = (pool.accountStatuses ?? []).filter((member) =>
      member.accountId.trim(),
    );
    if (statuses.length === 0) {
      accountIds.forEach((accountId) =>
        poolUnavailableAccountIds.add(accountId),
      );
      return;
    }
    statuses
      .filter((member) => !member.available)
      .forEach((member) => poolUnavailableAccountIds.add(member.accountId.trim()));
  });
  const summary: AccountPoolHealthSummary = {
    total: accountIds.length,
    available: 0,
    abnormal: 0,
    cooldown: 0,
    missing: 0,
    authError: 0,
    quotaLimited: 0,
  };

  accountIds.forEach((accountId) => {
    if (suppressedAccountIds.has(accountId)) {
      summary.available += 1;
      return;
    }
    const account = accountById.get(accountId);
    const health = healthById.get(accountId);
    if (!account) {
      summary.missing += 1;
      summary.abnormal += 1;
      return;
    }
    if (health?.cooldowns?.length) {
      summary.cooldown += 1;
      return;
    }
    if (isBlockingCodexAccountQuotaError(account)) {
      summary.quotaLimited += 1;
      return;
    }
    if (isAbnormalAccountFailure(health)) {
      summary.authError += 1;
      summary.abnormal += 1;
      return;
    }
    if (health && !health.available) {
      return;
    }
    if (poolUnavailableAccountIds.has(accountId)) {
      return;
    }
    summary.available += 1;
  });

  return summary;
}

export function buildLocalAccessRoutingStrategyOptions(t: TFunction) {
  return [
    {
      value: "auto",
      label: t("codex.localAccess.routingStrategy.auto", "自动（推荐）"),
    },
    {
      value: "quota_high_first",
      label: t(
        "codex.localAccess.routingStrategy.quotaHighFirst",
        "优先高配额",
      ),
    },
    {
      value: "quota_low_first",
      label: t(
        "codex.localAccess.routingStrategy.quotaLowFirst",
        "优先低配额",
      ),
    },
    {
      value: "plan_high_first",
      label: t(
        "codex.localAccess.routingStrategy.planHighFirst",
        "优先高订阅",
      ),
    },
    {
      value: "plan_low_first",
      label: t(
        "codex.localAccess.routingStrategy.planLowFirst",
        "优先低订阅",
      ),
    },
    {
      value: "expiry_soon_first",
      label: t(
        "codex.localAccess.routingStrategy.expirySoonFirst",
        "优先近到期",
      ),
    },
    {
      value: "custom",
      label: t("codex.localAccess.routingStrategy.custom", "自定义"),
    },
  ] satisfies Array<{
    value: CodexLocalAccessRoutingStrategy;
    label: string;
  }>;
}
