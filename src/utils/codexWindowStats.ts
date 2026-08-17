export const CODEX_WINDOW_FALLBACK_MINUTES = {
  primary: 5 * 60,
  secondary: 7 * 24 * 60,
} as const;

export interface CodexWindowStats {
  requestCount: number;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  estimatedCostUsd: number;
  /** 下游用户计费；当前 API 服务没有该口径时不展示 U。 */
  userCostUsd?: number | null;
}

/** 从「本窗口满额」算到现在：重置未到则 start = resetAt - 窗口；已重置则 start = resetAt。 */
export function resolveCodexWindowStartSeconds(
  resetAt: number | null | undefined,
  windowMinutes: number | null | undefined,
  nowSeconds: number,
  fallbackMinutes: number,
): number {
  const durationMinutes =
    typeof windowMinutes === "number" && Number.isFinite(windowMinutes) && windowMinutes > 0
      ? windowMinutes
      : fallbackMinutes;
  const durationSeconds = Math.max(1, Math.round(durationMinutes * 60));
  if (typeof resetAt === "number" && Number.isFinite(resetAt)) {
    if (resetAt > nowSeconds) {
      return resetAt - durationSeconds;
    }
    return resetAt;
  }
  return nowSeconds - durationSeconds;
}

/**
 * API 服务 request_logs.timestamp 与统计页一致，用毫秒。
 * 官方额度 resetAt 是 Unix 秒，查询前必须换算。
 */
export function toCodexLocalAccessLogTimestamp(unixSeconds: number): number {
  if (!Number.isFinite(unixSeconds)) return 0;
  const value = Math.trunc(unixSeconds);
  if (Math.abs(value) >= 1_000_000_000_000) {
    return value;
  }
  return value * 1000;
}

export function buildCodexAccountWindowStatQueries(
  accountId: string,
  windows: Array<{
    id: string;
    resetTime?: number | null;
    windowMinutes?: number | null;
  }>,
  nowSeconds: number,
): Array<{
  accountId: string;
  windowKey: string;
  startAt: number;
  endAt: number;
}> {
  return windows.flatMap((window) => {
    if (window.id !== "primary" && window.id !== "secondary") {
      return [];
    }
    const startSeconds = resolveCodexWindowStartSeconds(
      window.resetTime,
      window.windowMinutes,
      nowSeconds,
      window.id === "secondary"
        ? CODEX_WINDOW_FALLBACK_MINUTES.secondary
        : CODEX_WINDOW_FALLBACK_MINUTES.primary,
    );
    return [
      {
        accountId,
        windowKey: window.id,
        startAt: toCodexLocalAccessLogTimestamp(startSeconds),
        endAt: toCodexLocalAccessLogTimestamp(nowSeconds),
      },
    ];
  });
}

export function sessionUsageTotalsToWindowStats(
  totals?: {
    requestCount?: number;
    inputTokens?: number;
    cachedInputTokens?: number;
    outputTokens?: number;
    totalTokens?: number;
    estimatedCostUsd?: number;
  } | null,
): CodexWindowStats {
  return {
    requestCount: totals?.requestCount ?? 0,
    inputTokens: totals?.inputTokens ?? 0,
    cachedInputTokens: totals?.cachedInputTokens ?? 0,
    outputTokens: totals?.outputTokens ?? 0,
    totalTokens: totals?.totalTokens ?? 0,
    estimatedCostUsd: totals?.estimatedCostUsd ?? 0,
  };
}

/** 与 Sub2API formatCompactNumber 一致：K/M/B，1 位小数；请求数最高到 M。 */
export function formatCodexCompactNumber(
  value: number | null | undefined,
  options?: { allowBillions?: boolean },
): string {
  if (value == null || !Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  const allowBillions = options?.allowBillions !== false;
  if (allowBillions && abs >= 1_000_000_000) {
    return `${(value / 1_000_000_000).toFixed(1)}B`;
  }
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(Math.trunc(value));
}

export function formatCodexWindowTokenCount(value: number): string {
  return formatCodexCompactNumber(value);
}

export function formatCodexWindowRequestCount(value: number): string {
  return formatCodexCompactNumber(value, { allowBillions: false });
}

/** Sub2API 费用始终 toFixed(2)，不写 <$0.01。 */
export function formatCodexWindowCostAmount(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0.00";
  return value.toFixed(2);
}

export function formatCodexWindowCostUsd(value: number): string {
  return `$${formatCodexWindowCostAmount(value)}`;
}

export function hasVisibleCodexWindowStats(
  stats?: CodexWindowStats | null,
): boolean {
  if (!stats) return false;
  return stats.requestCount > 0 || stats.totalTokens > 0;
}

export function formatCodexWindowStatsText(stats: CodexWindowStats): string {
  const parts = [
    `${formatCodexWindowRequestCount(stats.requestCount)} req`,
    formatCodexWindowTokenCount(stats.totalTokens),
    `A $${formatCodexWindowCostAmount(stats.estimatedCostUsd)}`,
  ];
  if (stats.userCostUsd != null) {
    parts.push(`U $${formatCodexWindowCostAmount(stats.userCostUsd)}`);
  }
  return parts.join(" · ");
}
