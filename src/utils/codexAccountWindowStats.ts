import { getCodexQuotaWindows, type CodexAccount } from '../types/codex';
import type { CodexLocalAccessAccountWindowQuery, CodexLocalAccessAccountWindowStats } from '../types/codexLocalAccess';
import { buildCodexAccountWindowStatQueries, CODEX_WINDOW_FALLBACK_MINUTES, type CodexWindowStats } from './codexWindowStats';

type Window = { id: string; resetTime: number | null; windowMinutes: number | null };
export interface CodexAccountWindowStatsScope {
  key: string;
  nowSeconds: number;
  accounts: { id: string; windows: Window[] }[];
}
export interface CodexAccountWindowStatsSnapshot {
  scope: string | null;
  sourceScope: CodexAccountWindowStatsScope | null;
  ready: boolean;
  byAccountId: Record<string, { primary?: CodexWindowStats; secondary?: CodexWindowStats }>;
}
export const EMPTY_CODEX_ACCOUNT_WINDOW_STATS: CodexAccountWindowStatsSnapshot = {
  scope: null, sourceScope: null, ready: false, byAccountId: {},
};

/** Quota percentages, token refreshes and list ordering do not change a usage window. */
export function buildCodexAccountWindowStatsScope(
  accounts: readonly CodexAccount[], memberIds: readonly string[], nowSeconds: number,
): CodexAccountWindowStatsScope | null {
  const byId = new Map(accounts.map((account) => [account.id, account]));
  const finiteOrNull = (value: number | undefined) => typeof value === 'number' && Number.isFinite(value) ? value : null;
  const members = [...new Set(memberIds)].sort().flatMap((id) => {
    const account = byId.get(id);
    if (!account) return [];
    const windows = getCodexQuotaWindows(account.quota)
      .filter((window) => window.id === 'primary' || window.id === 'secondary')
      .map((window) => ({ id: window.id, resetTime: finiteOrNull(window.resetTime), windowMinutes: finiteOrNull(window.windowMinutes) }));
    return windows.length ? [{ id, windows }] : [];
  });
  if (!members.length) return null;
  // A reset crossing changes startAt even when the server has not sent its next quota snapshot.
  const key = JSON.stringify(members.map((account) => [account.id, account.windows.map((window) => [
    window.id, window.resetTime, window.windowMinutes,
    window.resetTime !== null && window.resetTime <= nowSeconds,
  ])]));
  return { key, nowSeconds, accounts: members };
}

/** Keep confirmed counters during reset-time corrections, but never across a quota reset. */
export function retainCodexAccountWindowStats(
  snapshot: CodexAccountWindowStatsSnapshot, next: CodexAccountWindowStatsScope | null,
): CodexAccountWindowStatsSnapshot {
  if (!next || !snapshot.ready || !snapshot.sourceScope) return EMPTY_CODEX_ACCOUNT_WINDOW_STATS;
  if (snapshot.scope === next.key && snapshot.sourceScope.key === next.key) return snapshot;
  const previous = new Map(snapshot.sourceScope.accounts.map((account) => [account.id, account.windows]));
  const byAccountId: CodexAccountWindowStatsSnapshot['byAccountId'] = {};
  const duration = (window: Window) => Math.max(1, Math.round((window.windowMinutes !== null && window.windowMinutes > 0
    ? window.windowMinutes : CODEX_WINDOW_FALLBACK_MINUTES[window.id as 'primary' | 'secondary']) * 60));
  for (const account of next.accounts) {
    for (const window of account.windows) {
      const id = window.id as 'primary' | 'secondary';
      const stats = snapshot.byAccountId[account.id]?.[id];
      const old = previous.get(account.id)?.find((value) => value.id === id);
      if (!stats || !old || duration(old) !== duration(window)) continue;
      if (old.resetTime === null || window.resetTime === null) {
        if (old.resetTime !== window.resetTime) continue;
      } else {
        const wasReset = old.resetTime <= snapshot.sourceScope.nowSeconds;
        const isReset = window.resetTime <= next.nowSeconds;
        if (wasReset !== isReset) continue;
        if (old.resetTime !== window.resetTime) {
          // Future reset estimates can drift within the same overlapping period.
          // Once the confirmed reset is reached, only a fresh query may supply counters.
          if (wasReset || old.resetTime <= next.nowSeconds ||
              Math.max(old.resetTime - duration(old), window.resetTime - duration(window)) >=
              Math.min(old.resetTime, window.resetTime)) continue;
        }
      }
      (byAccountId[account.id] ??= {})[id] = stats;
    }
  }
  return { ...snapshot, scope: next.key, ready: Object.keys(byAccountId).length > 0, byAccountId };
}

/** One native query at a time; bursts keep only the latest scope and one pending refresh. */
export function createCodexAccountWindowStatsReader(options: {
  read: (queries: CodexLocalAccessAccountWindowQuery[]) => Promise<CodexLocalAccessAccountWindowStats[]>;
  onChange: (snapshot: CodexAccountWindowStatsSnapshot) => void;
  onError?: (error: unknown) => void;
  canRead?: () => boolean;
  now?: () => number;
  coalesceMs?: number;
  minRefreshMs?: number;
}) {
  const now = options.now ?? Date.now;
  const coalesceMs = options.coalesceMs ?? 250;
  const minRefreshMs = options.minRefreshMs ?? 5_000;
  let scope: CodexAccountWindowStatsScope | null = null;
  let revision = 0;
  let requested = false;
  let running = false;
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let lastStartedAt = -Infinity;
  let snapshot = EMPTY_CODEX_ACCOUNT_WINDOW_STATS;
  const publish = (next: CodexAccountWindowStatsSnapshot) => { snapshot = next; options.onChange(next); };

  const schedule = () => {
    if (disposed || options.canRead?.() === false || running || timer !== undefined || !requested || !scope) return;
    const wait = Math.max(0, minRefreshMs - (now() - lastStartedAt));
    timer = setTimeout(() => { timer = undefined; void run(); }, Math.max(coalesceMs, wait));
  };
  async function run() {
    if (disposed || options.canRead?.() === false || running || !requested || !scope) return;
    const startedScope = scope;
    const startedRevision = revision;
    requested = false;
    running = true;
    lastStartedAt = now();
    try {
      const nowSeconds = Math.floor(lastStartedAt / 1000);
      const queries = startedScope.accounts.flatMap((account) => buildCodexAccountWindowStatQueries(account.id, account.windows, nowSeconds));
      const rows = await options.read(queries);
      if (disposed || startedRevision !== revision) return;
      const byAccountId: CodexAccountWindowStatsSnapshot['byAccountId'] = {};
      const allowed = new Set(queries.map((query) => JSON.stringify([query.accountId, query.windowKey])));
      for (const row of rows) {
        if ((row.windowKey !== 'primary' && row.windowKey !== 'secondary') || !allowed.has(JSON.stringify([row.accountId, row.windowKey]))) continue;
        const windows = byAccountId[row.accountId] ??= {};
        windows[row.windowKey] = {
          requestCount: row.requestCount, inputTokens: row.inputTokens, cachedInputTokens: row.cachedTokens,
          outputTokens: row.outputTokens, totalTokens: row.totalTokens, estimatedCostUsd: row.estimatedCostUsd,
        };
      }
      publish({ scope: startedScope.key, sourceScope: startedScope, ready: true, byAccountId });
    } catch (error) {
      // A background failure must not replace confirmed values with false zeroes.
      if (!disposed && startedRevision === revision) options.onError?.(error);
    } finally {
      running = false;
      schedule();
    }
  }
  return {
    setScope(next: CodexAccountWindowStatsScope | null) {
      if (disposed) return;
      const changed = scope?.key !== next?.key;
      scope = next;
      if (!changed) return;
      revision++;
      requested = !!next;
      if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
      publish(retainCodexAccountWindowStats(snapshot, next));
      schedule();
    },
    refresh() { if (!disposed && scope) { requested = true; schedule(); } },
    dispose() {
      disposed = true;
      revision++;
      requested = false;
      if (timer !== undefined) clearTimeout(timer);
    },
  };
}
