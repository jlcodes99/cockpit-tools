import { invoke } from '@tauri-apps/api/core';

export type ModelProviderUsageIntegrationType = 'sub2api' | 'new_api';
export type ModelProviderUsageMode =
  | ModelProviderUsageIntegrationType
  | 'deepseek'
  | 'opencode_go'
  | 'token_plan';

export interface ModelProviderModel {
  id: string;
  displayName?: string | null;
}

export interface ModelProviderModelsResult {
  models: ModelProviderModel[];
  latencyMs: number;
}

export interface ModelProviderUsageSummary {
  mode?: string | null;
  isValid?: boolean | null;
  status?: string | null;
  planName?: string | null;
  remaining?: number | null;
  balance?: number | null;
  unit?: string | null;
  quotaUnlimited?: boolean | null;
  quotaLimit?: number | null;
  quotaUsed?: number | null;
  quotaRemaining?: number | null;
  todayRequests?: number | null;
  todayTotalTokens?: number | null;
  todayCost?: number | null;
  totalRequests?: number | null;
  totalTotalTokens?: number | null;
  totalCost?: number | null;
  modelStatsCount: number;
  latencyMs: number;
  details?: Array<{
    key: string;
    label: string;
    value: string;
  }>;
}

export interface NewApiQuotaSnapshot {
  granted: number | null;
  available: number | null;
  expiresAt: number | null;
}

export interface OpenCodeGoQuotaWindowSnapshot {
  usedPercent: number | null;
  remainingPercent: number | null;
  resetsAt: number | null;
}

export interface OpenCodeGoQuotaSnapshot {
  rolling: OpenCodeGoQuotaWindowSnapshot;
  weekly: OpenCodeGoQuotaWindowSnapshot;
  monthly: OpenCodeGoQuotaWindowSnapshot;
}

function finiteUsageNumber(value: unknown): number | null {
  if (typeof value === 'number') {
    return Number.isFinite(value) ? value : null;
  }
  if (typeof value !== 'string' || !value.trim()) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function usageDetailNumber(
  summary: ModelProviderUsageSummary | undefined,
  key: string,
): number | null {
  return finiteUsageNumber(summary?.details?.find((item) => item.key === key)?.value);
}

export function resolveNewApiQuotaSnapshot(
  summary?: ModelProviderUsageSummary,
): NewApiQuotaSnapshot {
  const used =
    finiteUsageNumber(summary?.quotaUsed) ??
    finiteUsageNumber(summary?.totalCost);
  const granted =
    usageDetailNumber(summary, 'totalGranted') ??
    finiteUsageNumber(summary?.quotaLimit) ??
    usageDetailNumber(summary, 'hardLimitUsd') ??
    usageDetailNumber(summary, 'softLimitUsd') ??
    usageDetailNumber(summary, 'systemHardLimitUsd');
  const available =
    usageDetailNumber(summary, 'totalAvailable') ??
    finiteUsageNumber(summary?.quotaRemaining) ??
    (granted != null && used != null
      ? Math.max(0, granted - used)
      : null);
  const expiresAt =
    usageDetailNumber(summary, 'expiresAt') ??
    usageDetailNumber(summary, 'accessUntil');

  return { granted, available, expiresAt };
}

export function resolveOpenCodeGoQuotaSnapshot(
  summary?: ModelProviderUsageSummary,
): OpenCodeGoQuotaSnapshot {
  const resolveWindow = (key: 'rolling' | 'weekly' | 'monthly') => ({
    usedPercent: usageDetailNumber(summary, `${key}UsedPercent`),
    remainingPercent: usageDetailNumber(summary, `${key}RemainingPercent`),
    resetsAt: usageDetailNumber(summary, `${key}ResetsAt`),
  });
  return {
    rolling: resolveWindow('rolling'),
    weekly: resolveWindow('weekly'),
    monthly: resolveWindow('monthly'),
  };
}

export function buildUsageBaseUrlCandidates(baseUrl: string): string[] {
  const trimmed = baseUrl.trim();
  if (!trimmed) return [];
  const candidates = [trimmed];
  try {
    const parsed = new URL(trimmed);
    const path = parsed.pathname.replace(/\/+$/, '');
    if (path === '' || path === '/') {
      // Sub2API-compatible services may expose /usage at either the host root
      // or under /v1. Try the user's URL first, then the conventional prefix.
      const usageUrl = `${parsed.origin}/v1`;
      if (!candidates.includes(usageUrl)) candidates.push(usageUrl);
    }
  } catch {
    // keep the original value and let the backend return the validation error
  }
  return candidates;
}

export async function queryModelProviderUsage(input: {
  baseUrl: string;
  apiKey: string;
  integrationType?: ModelProviderUsageIntegrationType | null;
}): Promise<ModelProviderUsageSummary> {
  const candidates = buildUsageBaseUrlCandidates(input.baseUrl);
  let lastError: unknown = null;
  for (const baseUrl of candidates) {
    try {
      return await invoke('codex_query_model_provider_usage', {
        baseUrl,
        apiKey: input.apiKey,
        integrationType: input.integrationType ?? null,
      });
    } catch (error) {
      lastError = error;
      if (!isModelProviderUsageUnavailableError(error)) {
        throw error;
      }
    }
  }
  throw lastError ?? new Error('PROVIDER_BASE_URL_INVALID');
}

export async function listModelProviderModels(input: {
  baseUrl: string;
  apiKey: string;
}): Promise<ModelProviderModelsResult> {
  return await invoke('codex_list_model_provider_models', {
    baseUrl: input.baseUrl,
    apiKey: input.apiKey,
  });
}

export function isModelProviderUsageUnavailableError(error: unknown): boolean {
  const message = String(error).replace(/^Error:\s*/, '');
  return (
    message.includes('PROVIDER_USAGE_DETECT_FAILED') ||
    message.includes('PROVIDER_USAGE_HTTP_404') ||
    message.includes('PROVIDER_USAGE_TYPE_UNSUPPORTED')
  );
}

export function resolveModelProviderUsageMode(
  summary?: ModelProviderUsageSummary,
): ModelProviderUsageMode | null {
  if (!summary) return null;
  if (
    summary.mode === 'new_api' ||
    summary.mode === 'sub2api' ||
    summary.mode === 'deepseek' ||
    summary.mode === 'opencode_go' ||
    summary.mode === 'token_plan'
  ) {
    return summary.mode;
  }
  if (
    typeof summary.todayRequests === 'number' ||
    typeof summary.todayTotalTokens === 'number'
  ) {
    return 'sub2api';
  }
  const detailKeys = new Set((summary.details ?? []).map((item) => item.key));
  // OpenCode Go may report quota windows partially (e.g. only the 5-hour
  // rolling window before the weekly/monthly counters are provisioned).
  // Classify from any recognized window key instead of demanding all three.
  if (
    detailKeys.has('rollingRemainingPercent') ||
    detailKeys.has('weeklyRemainingPercent') ||
    detailKeys.has('monthlyRemainingPercent')
  ) {
    return 'opencode_go';
  }
  if (
    detailKeys.has('todayRequests') ||
    detailKeys.has('todayTokens') ||
    detailKeys.has('remaining')
  ) {
    return 'sub2api';
  }
  if (
    detailKeys.has('totalGranted') ||
    detailKeys.has('totalAvailable') ||
    detailKeys.has('expiresAt')
  ) {
    return 'new_api';
  }
  return null;
}

/** Human "resets in …" countdown for a unix-seconds reset timestamp. */
export function formatModelProviderUsageResetCountdown(
  resetsAt?: number | null,
): string {
  if (typeof resetsAt !== 'number' || !Number.isFinite(resetsAt)) return '-';
  const remainingMs = resetsAt * 1000 - Date.now();
  if (remainingMs <= 0) return 'resetting';
  const totalMinutes = Math.floor(remainingMs / 60_000);
  const days = Math.floor(totalMinutes / 1440);
  const hours = Math.floor((totalMinutes % 1440) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

export function formatModelProviderUsageMoney(
  value?: number | null,
  unit?: string | null,
): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '-';
  const normalizedUnit = unit?.trim() || 'USD';
  if (normalizedUnit === '%') {
    return `${Math.round(value)}%`;
  }
  const formatted = value.toFixed(value >= 100 ? 0 : 2);
  if (normalizedUnit === 'USD') return `$${formatted}`;
  if (normalizedUnit === 'CNY') return `¥${formatted}`;
  return `${formatted} ${normalizedUnit}`;
}

export function formatModelProviderUsageInteger(value?: number | null): string {
  const normalized =
    typeof value === 'number' && Number.isFinite(value) ? Math.max(0, value) : 0;
  return new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 }).format(
    normalized,
  );
}

export function formatModelProviderUsageTokenCount(value?: number | null): string {
  const normalized =
    typeof value === 'number' && Number.isFinite(value) ? Math.max(0, value) : 0;
  if (normalized >= 100_000_000) {
    return `${(normalized / 100_000_000)
      .toFixed(normalized >= 1_000_000_000 ? 1 : 2)
      .replace(/\.?0+$/, '')}亿`;
  }
  if (normalized >= 10_000) {
    return `${(normalized / 10_000)
      .toFixed(normalized >= 100_000 ? 1 : 2)
      .replace(/\.?0+$/, '')}万`;
  }
  return new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 }).format(
    normalized,
  );
}
