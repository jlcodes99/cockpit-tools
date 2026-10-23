import { getCodexQuotaWindows, type CodexAccount } from '../types/codex';
import type { CodexLocalAccessAccountWindowQuery, CodexLocalAccessAccountWindowStats } from '../types/codexLocalAccess';
import { buildCodexAccountWindowStatQueries, type CodexWindowStats } from './codexWindowStats';

type Window = { id: string; resetTime: number | null; windowMinutes: number | null };
export interface CodexAccountWindowStatsScope {
  key: string;
  accounts: { id: string; windows: Window[] }[];
}
export interface CodexAccountWindowStatsSnapshot {
  scope: string | null;
  ready: boolean;
  byAccountId: Record<string, { primary?: CodexWindowStats; secondary?: CodexWindowStats }>;
}
export const EMPTY_CODEX_ACCOUNT_WINDOW_STATS: CodexAccountWindowStatsSnapshot = {
  scope: null, ready: false, byAccountId: {},
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
  return { key, accounts: members };
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
  let lastStartedScope: string | null = null;

  const schedule = () => {
    if (disposed || options.canRead?.() === false || running || timer !== undefined || !requested || !scope) return;
    const wait = lastStartedScope === scope.key ? Math.max(0, minRefreshMs - (now() - lastStartedAt)) : 0;
    timer = setTimeout(() => { timer = undefined; void run(); }, Math.max(coalesceMs, wait));
  };
  async function run() {
    if (disposed || options.canRead?.() === false || running || !requested || !scope) return;
    const startedScope = scope;
    const startedRevision = revision;
    requested = false;
    running = true;
    lastStartedAt = now();
    lastStartedScope = scope.key;
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
      options.onChange({ scope: startedScope.key, ready: true, byAccountId });
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
      options.onChange({ ...EMPTY_CODEX_ACCOUNT_WINDOW_STATS, scope: next?.key ?? null });
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
