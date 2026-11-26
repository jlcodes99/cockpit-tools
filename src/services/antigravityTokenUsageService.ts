import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';
import type {
  AntigravityRehydrateStatus,
  AntigravityTokenUsageSettings,
  AntigravityUsageRangePreset,
  ConversationDetailResponse,
  DashboardBundleResponse,
  DashboardQueryRequest,
} from '../types/antigravityTokenUsage';

export const ANTIGRAVITY_TOKEN_USAGE_RANGE_STORAGE_KEY =
  'agtools.antigravity.token_usage.range.v1';
const RANGE_CHANGED_EVENT = 'agtools:antigravity-token-usage-range-changed';

const queryBundleCache = new Map<string, DashboardBundleResponse>();
const presetSummaryCache = new Map<
  AntigravityUsageRangePreset,
  DashboardBundleResponse
>();
let lastSyncedAtMs = 0;
let inFlightIncrementalSync: {
  key: string;
  promise: Promise<DashboardBundleResponse>;
} | null = null;

export function normalizeAntigravityUsageRange(
  value: unknown,
): AntigravityUsageRangePreset {
  if (
    value === 'today' ||
    value === '7d' ||
    value === '30d' ||
    value === 'month' ||
    value === 'all'
  ) {
    return value;
  }
  return '30d';
}

export function getSavedAntigravityUsageRange(): AntigravityUsageRangePreset {
  if (typeof window === 'undefined') {
    return '30d';
  }
  try {
    return normalizeAntigravityUsageRange(
      window.localStorage.getItem(ANTIGRAVITY_TOKEN_USAGE_RANGE_STORAGE_KEY),
    );
  } catch {
    return '30d';
  }
}

export function setSavedAntigravityUsageRange(
  next: AntigravityUsageRangePreset,
): void {
  if (typeof window === 'undefined') {
    return;
  }
  const normalized = normalizeAntigravityUsageRange(next);
  try {
    window.localStorage.setItem(
      ANTIGRAVITY_TOKEN_USAGE_RANGE_STORAGE_KEY,
      normalized,
    );
  } catch {
    // Ignore storage write errors in restricted webviews.
  }
  window.dispatchEvent(
    new CustomEvent<AntigravityUsageRangePreset>(RANGE_CHANGED_EVENT, {
      detail: normalized,
    }),
  );
}

export function useAntigravityUsageRange(): [
  AntigravityUsageRangePreset,
  (next: AntigravityUsageRangePreset) => void,
] {
  const [range, setRange] = useState<AntigravityUsageRangePreset>(() =>
    getSavedAntigravityUsageRange(),
  );

  useEffect(() => {
    const handleCustom = (event: Event) => {
      const detail = (event as CustomEvent<AntigravityUsageRangePreset>).detail;
      setRange(normalizeAntigravityUsageRange(detail));
    };
    const handleStorage = (event: StorageEvent) => {
      if (event.key === ANTIGRAVITY_TOKEN_USAGE_RANGE_STORAGE_KEY) {
        setRange(normalizeAntigravityUsageRange(event.newValue));
      }
    };
    window.addEventListener(RANGE_CHANGED_EVENT, handleCustom);
    window.addEventListener('storage', handleStorage);
    return () => {
      window.removeEventListener(RANGE_CHANGED_EVENT, handleCustom);
      window.removeEventListener('storage', handleStorage);
    };
  }, []);

  const updateRange = useCallback((next: AntigravityUsageRangePreset) => {
    const normalized = normalizeAntigravityUsageRange(next);
    setRange(normalized);
    setSavedAntigravityUsageRange(normalized);
  }, []);

  return [range, updateRange];
}

export function currentTzOffsetMinutes(): number {
  return -new Date().getTimezoneOffset();
}

function withDefaultTz(query?: DashboardQueryRequest): DashboardQueryRequest {
  return {
    tzOffsetMinutes: currentTzOffsetMinutes(),
    ...query,
  };
}

function buildQueryKey(q: DashboardQueryRequest): string {
  return JSON.stringify([
    q.preset ?? '30d',
    q.from ?? '',
    q.to ?? '',
    q.granularity ?? 'day',
    q.model ?? '',
    q.workspace ?? '',
    q.kind ?? '',
    q.search ?? '',
    q.tzOffsetMinutes ?? 0,
    q.sort ?? 'lastModifiedTime',
    q.order ?? 'desc',
    q.page ?? 1,
    q.pageSize ?? 20,
  ]);
}

function isUnfilteredPresetQuery(q: DashboardQueryRequest): boolean {
  return (
    !q.from &&
    !q.to &&
    (!q.granularity || q.granularity === 'day') &&
    !q.model &&
    !q.workspace &&
    !q.kind &&
    !q.search
  );
}

function recordBundleInCache(
  q: DashboardQueryRequest,
  bundle: DashboardBundleResponse,
): void {
  const key = buildQueryKey(q);
  if (queryBundleCache.size > 32) {
    queryBundleCache.clear();
  }
  queryBundleCache.set(key, bundle);
  if (isUnfilteredPresetQuery(q)) {
    const preset = normalizeAntigravityUsageRange(q.preset);
    presetSummaryCache.set(preset, bundle);
  }
}

export function peekCachedAntigravityTokenUsage(
  query?: DashboardQueryRequest,
): DashboardBundleResponse | null {
  const effectiveQuery = withDefaultTz(query);
  return queryBundleCache.get(buildQueryKey(effectiveQuery)) ?? null;
}

export function peekCachedAntigravitySummaryByPreset(
  preset: AntigravityUsageRangePreset,
): DashboardBundleResponse | null {
  return presetSummaryCache.get(preset) ?? null;
}

export function hasRecentAntigravityTokenSync(maxAgeMs = 4_000): boolean {
  return lastSyncedAtMs > 0 && Date.now() - lastSyncedAtMs <= maxAgeMs;
}

export async function queryAntigravityTokenUsage(
  query?: DashboardQueryRequest,
): Promise<DashboardBundleResponse> {
  const effectiveQuery = withDefaultTz(query);
  const res = await invoke<DashboardBundleResponse>(
    'antigravity_query_token_usage',
    {
      query: effectiveQuery,
    },
  );
  recordBundleInCache(effectiveQuery, res);
  if (lastSyncedAtMs === 0) {
    lastSyncedAtMs = Date.now();
  }
  return res;
}

export async function syncAntigravityTokenUsage(
  rebuild = false,
  query?: DashboardQueryRequest,
): Promise<DashboardBundleResponse> {
  const effectiveQuery = withDefaultTz(query);
  const key = buildQueryKey(effectiveQuery);

  if (!rebuild && inFlightIncrementalSync) {
    const inflight = inFlightIncrementalSync;
    const shared = await inflight.promise;
    if (inflight.key === key) {
      return shared;
    }
    return await queryAntigravityTokenUsage(effectiveQuery);
  }

  const runPromise = (async () => {
    const res = await invoke<DashboardBundleResponse>(
      'antigravity_sync_token_usage',
      {
        rebuild,
        query: effectiveQuery,
      },
    );
    queryBundleCache.clear();
    presetSummaryCache.clear();
    recordBundleInCache(effectiveQuery, res);
    lastSyncedAtMs = Date.now();
    return res;
  })();

  if (!rebuild) {
    inFlightIncrementalSync = { key, promise: runPromise };
    try {
      return await runPromise;
    } finally {
      if (inFlightIncrementalSync?.promise === runPromise) {
        inFlightIncrementalSync = null;
      }
    }
  }

  return await runPromise;
}

export async function getAntigravityConversationTokenDetail(
  conversationId: string,
): Promise<ConversationDetailResponse> {
  return await invoke('antigravity_get_conversation_token_detail', {
    conversationId,
  });
}

export async function getAntigravityTokenUsageSettings(): Promise<AntigravityTokenUsageSettings> {
  return await invoke('antigravity_get_token_usage_settings');
}

export async function setAntigravityTokenUsageDataDir(
  customDir: string | null,
  query?: DashboardQueryRequest,
): Promise<DashboardBundleResponse> {
  const effectiveQuery = withDefaultTz(query);
  const res = await invoke<DashboardBundleResponse>(
    'antigravity_set_token_usage_data_dir',
    {
      customDir,
      query: effectiveQuery,
    },
  );
  queryBundleCache.clear();
  presetSummaryCache.clear();
  recordBundleInCache(effectiveQuery, res);
  lastSyncedAtMs = Date.now();
  return res;
}

export async function setAntigravityRehydrateEnabled(
  enabled: boolean,
): Promise<AntigravityRehydrateStatus> {
  const status = await invoke<AntigravityRehydrateStatus>(
    'antigravity_set_rehydrate_enabled',
    { enabled },
  );
  queryBundleCache.forEach((bundle) => {
    if (bundle.settings) {
      bundle.settings.rehydrate = status;
    }
  });
  presetSummaryCache.forEach((bundle) => {
    if (bundle.settings) {
      bundle.settings.rehydrate = status;
    }
  });
  return status;
}
