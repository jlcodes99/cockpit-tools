import { invoke } from '@tauri-apps/api/core';

export type ModelProviderUsageIntegrationType = 'sub2api' | 'new_api';
export type ModelProviderUsageMode = ModelProviderUsageIntegrationType | 'deepseek';

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

function buildUsageBaseUrlCandidates(baseUrl: string): string[] {
  const trimmed = baseUrl.trim();
  if (!trimmed) return [];
  const candidates = [trimmed];
  try {
    const parsed = new URL(trimmed);
    const host = parsed.hostname.toLowerCase();
    const path = parsed.pathname.replace(/\/+$/, '');
    if (
      (host === 'api.apikey.fun' || host === 'slb.apikey.fun') &&
      (path === '' || path === '/')
    ) {
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
    summary.mode === 'deepseek'
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

export function formatModelProviderUsageMoney(
  value?: number | null,
  unit?: string | null,
): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '-';
  const normalizedUnit = unit?.trim() || 'USD';
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
