import {
  buildCodexStatsTimeRange,
  parseCodexStatsTimeRange,
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "./codexStatsRange.ts";

export interface CodexStatsRangeSelection {
  key: CodexStatsRangeKey;
  range: CodexStatsTimeRange;
}

export type CodexSessionUsageRange = "today" | "7d" | "30d" | "month" | "all";

export const CODEX_SESSION_USAGE_RANGE_STORAGE_KEY =
  "agtools.codex.session_usage.range.v1";
export const CODEX_API_STATS_RANGE_STORAGE_KEY = "agtools.codex.api_service.stats_range.v1";
export const CODEX_LOCAL_ACCESS_STATS_RANGE_STORAGE_KEY = "agtools.codex.local_access.stats_range.v1";

function parseSavedStatsRange(
  raw: string | null,
  now = new Date(),
): CodexStatsRangeSelection | null {
  // Keep the existing plain-string preset format, including the previously
  // omitted daily value. Recalculate rolling/calendar ranges for today's date.
  if (raw === "daily" || raw === "rolling7d" || raw === "weekly" || raw === "monthly") {
    return { key: raw, range: buildCodexStatsTimeRange(raw, now) };
  }
  if (raw) {
    try {
      const saved: unknown = JSON.parse(raw);
      if (saved && typeof saved === "object"
        && "key" in saved && saved.key === "custom"
        && "startInput" in saved && typeof saved.startInput === "string"
        && "endInput" in saved && typeof saved.endInput === "string") {
        const range = parseCodexStatsTimeRange(saved.startInput, saved.endInput);
        if (range) return { key: "custom", range };
      }
    } catch {
      // Unknown/legacy custom values have no restorable dates. Keep storage
      // untouched until the user explicitly chooses another range.
    }
  }
  return null;
}

export function isValidCodexStatsRangePreference(raw: string): boolean {
  return parseSavedStatsRange(raw) !== null;
}

export function readCodexStatsRangeSelection(
  raw: string | null,
  now = new Date(),
): CodexStatsRangeSelection {
  return parseSavedStatsRange(raw, now) ?? { key: "rolling7d", range: buildCodexStatsTimeRange("rolling7d", now) };
}

export function serializeCodexStatsRangeSelection(
  selection: CodexStatsRangeSelection,
): string | null {
  if (selection.key === "custom") {
    const { startInput, endInput } = selection.range;
    if (!parseCodexStatsTimeRange(startInput, endInput)) return null;
    return JSON.stringify({ key: "custom", startInput, endInput });
  }
  return selection.key;
}

export function normalizeCodexSessionUsageRange(value: unknown): CodexSessionUsageRange {
  return value === "today" || value === "30d" || value === "month" || value === "all" ? value : "7d";
}
