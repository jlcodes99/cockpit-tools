import type { OpenCodeGoConnection } from '../types/openCodeGo.ts';

export interface OpenCodeGoConnectionSlot {
  connection: OpenCodeGoConnection | null;
}

export type OpenCodeGoConnectionTier =
  | 'available'
  | 'exhausted'
  | 'error'
  | 'pending';

export interface OpenCodeGoConnectionQuery {
  query?: string;
  tier?: OpenCodeGoConnectionTier | 'all';
  sortBy?: 'name' | 'created_at' | 'remaining';
  sortDirection?: 'asc' | 'desc';
}

/**
 * Maps every stored connection to a renderable slot. Connection ownership is
 * intentionally unbounded; storage and key validation are the real limits.
 */
export function createOpenCodeGoConnectionSlots(
  connections: OpenCodeGoConnection[],
): OpenCodeGoConnectionSlot[] {
  return connections.map((connection) => ({ connection }));
}

/**
 * Empty or whitespace-only names fall back to a stable slot label derived from
 * the connection's position (1-based), matching the Rust store's fallbacks.
 */
export function normalizeOpenCodeGoConnectionName(
  name: string,
  slotIndex: number,
): string {
  const trimmed = name.trim();
  if (trimmed) return trimmed;
  return `Connection ${slotIndex + 1}`;
}

/**
 * Derives the presentation tier from sanitized backend state only:
 * cached quota errors win, then the quota status flag, then "pending".
 */
export function resolveOpenCodeGoConnectionTier(
  connection: OpenCodeGoConnection,
): OpenCodeGoConnectionTier {
  if (connection.enabled === false) return 'pending';
  if (connection.quotaError) return 'error';
  if (!connection.quota) return 'pending';
  return connection.quota.status === 'exhausted' ? 'exhausted' : 'available';
}

/**
 * Sort heuristic. Windows degraded under the partial-window contract carry a
 * null `remainingPercent`; those windows are excluded rather than coerced to
 * a fabricated number. Only fully-unknown quotas score -1.
 */
function averageRemainingPercent(connection: OpenCodeGoConnection): number {
  const known = [
    connection.quota?.rolling,
    connection.quota?.weekly,
    connection.quota?.monthly,
  ]
    .map((window) => window?.remainingPercent)
    .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
  if (known.length === 0) return -1;
  return known.reduce((total, value) => total + value, 0) / known.length;
}

function compareConnections(
  left: OpenCodeGoConnection,
  right: OpenCodeGoConnection,
  sortBy: NonNullable<OpenCodeGoConnectionQuery['sortBy']>,
): number {
  switch (sortBy) {
    case 'name':
      return left.name.localeCompare(right.name);
    case 'created_at':
      return left.createdAt - right.createdAt;
    case 'remaining':
      return averageRemainingPercent(left) - averageRemainingPercent(right);
  }
}

/**
 * Pure query over connection summaries: case-insensitive text match on name
 * and key hint, optional tier filter, then deterministic sort with an id
 * tie-break so equal keys never flicker between renders.
 */
export function filterAndSortOpenCodeGoConnections(
  connections: OpenCodeGoConnection[],
  options: OpenCodeGoConnectionQuery = {},
): OpenCodeGoConnection[] {
  const query = options.query?.trim().toLowerCase() ?? '';
  const tier = options.tier ?? 'all';
  const sortBy = options.sortBy ?? 'name';
  const direction = options.sortDirection === 'desc' ? -1 : 1;

  const filtered = connections.filter((connection) => {
    if (tier !== 'all' && resolveOpenCodeGoConnectionTier(connection) !== tier) {
      return false;
    }
    if (!query) return true;
    return (
      connection.name.toLowerCase().includes(query) ||
      connection.emailHint?.toLowerCase().includes(query) ||
      connection.keyHint.toLowerCase().includes(query)
    );
  });

  return filtered.sort(
    (left, right) =>
      compareConnections(left, right, sortBy) * direction ||
      left.id.localeCompare(right.id),
  );
}
