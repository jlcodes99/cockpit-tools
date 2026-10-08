import type { CodexSessionUsageQuery } from '../types/codex';
import type { CodexSessionUsageRange } from './codexStatsRangePreference';

export function buildCodexSessionUsageQuery(
  range: CodexSessionUsageRange, instanceId = '', now = new Date(),
): CodexSessionUsageQuery {
  let start: Date | null = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (range === '7d') start.setDate(start.getDate() - 6);
  else if (range === '30d') start.setDate(start.getDate() - 29);
  else if (range === 'month') start.setDate(1);
  else if (range === 'all') start = null;
  return {
    fromTimestamp: start ? Math.floor(start.getTime() / 1000) : null,
    toTimestamp: Math.floor(now.getTime() / 1000),
    instanceId: instanceId || null,
  };
}
