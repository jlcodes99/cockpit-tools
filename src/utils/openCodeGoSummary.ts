import type { OpenCodeGoConnection } from '../types/openCodeGo';

export interface OpenCodeGoSummaryConnection {
  id: string;
  name: string;
  keyHint: string;
  quota: OpenCodeGoConnection['quota'];
  quotaError: OpenCodeGoConnection['quotaError'];
}

export interface OpenCodeGoSummary {
  connectionCount: number;
  healthyConnectionCount: number;
  connection: OpenCodeGoSummaryConnection | null;
}

/**
 * Safety score for summary selection. Under the partial-window contract a
 * window may carry a null `remainingPercent` (unknown), so only finite
 * values participate; a connection with no known window ranks last instead
 * of fabricating a number.
 */
function minimumRemaining(connection: OpenCodeGoConnection): number {
  const known = [
    connection.quota?.rolling,
    connection.quota?.weekly,
    connection.quota?.monthly,
  ]
    .map((window) => window?.remainingPercent)
    .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
  if (!connection.quota || known.length === 0) return Number.NEGATIVE_INFINITY;
  return Math.min(...known);
}

export function pickOpenCodeGoSummaryConnection(
  connections: readonly OpenCodeGoConnection[],
): OpenCodeGoConnection | null {
  return [...connections].sort((left, right) => {
    const safety = minimumRemaining(right) - minimumRemaining(left);
    if (safety !== 0) return safety;
    const freshness = (right.quota?.queriedAt ?? right.updatedAt) -
      (left.quota?.queriedAt ?? left.updatedAt);
    return freshness || left.id.localeCompare(right.id);
  })[0] ?? null;
}

export function buildOpenCodeGoSummary(
  connections: readonly OpenCodeGoConnection[],
): OpenCodeGoSummary {
  const selected = pickOpenCodeGoSummaryConnection(connections);
  return {
    connectionCount: connections.length,
    healthyConnectionCount: connections.filter((connection) => connection.quota !== undefined).length,
    connection: selected
      ? {
          id: selected.id,
          name: selected.name,
          keyHint: selected.keyHint,
          quota: selected.quota,
          quotaError: selected.quotaError,
        }
      : null,
  };
}

export function formatOpenCodeGoResetCountdown(
  resetsAt: number | null | undefined,
  nowSeconds = Date.now() / 1000,
): string | null {
  if (resetsAt == null || !Number.isFinite(resetsAt)) return null;
  const normalizedSeconds =
    resetsAt > 10_000_000_000 || (resetsAt > 10_000_000 && resetsAt < 1_000_000_000)
      ? resetsAt / 1000
      : resetsAt;
  const seconds = Math.max(0, Math.floor(normalizedSeconds - nowSeconds));
  if (seconds <= 0) return 'resetting';
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${Math.max(1, minutes)}m`;
}
