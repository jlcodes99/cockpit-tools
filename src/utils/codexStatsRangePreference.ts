import {
  buildCodexStatsTimeRange,
  parseCodexStatsTimeRange,
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "./codexStatsRange.ts";

type RangePreferenceStorage = Pick<Storage, "getItem" | "setItem">;

export interface CodexStatsRangeSelection {
  key: CodexStatsRangeKey;
  range: CodexStatsTimeRange;
}

export type CodexSessionUsageRange = "7d" | "30d" | "month" | "all";

export const CODEX_SESSION_USAGE_RANGE_STORAGE_KEY =
  "agtools.codex.session_usage.range.v1";

function readPreference(key: string, storage?: RangePreferenceStorage): string | null {
  try {
    return (storage ?? localStorage).getItem(key);
  } catch {
    console.warn("[Codex stats] Unable to read the saved range; using the default.");
    return null;
  }
}

function writePreference(key: string, value: string, storage?: RangePreferenceStorage): boolean {
  try {
    (storage ?? localStorage).setItem(key, value);
    return true;
  } catch {
    console.warn("[Codex stats] Unable to save the range; the current selection remains usable.");
    return false;
  }
}

export function readCodexStatsRangeSelection(
  storageKey: string,
  now = new Date(),
  storage?: RangePreferenceStorage,
): CodexStatsRangeSelection {
  const raw = readPreference(storageKey, storage);
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
  return { key: "rolling7d", range: buildCodexStatsTimeRange("rolling7d", now) };
}

export function persistCodexStatsRangeSelection(
  storageKey: string,
  selection: CodexStatsRangeSelection,
  storage?: RangePreferenceStorage,
): boolean {
  if (selection.key === "custom") {
    const { startInput, endInput } = selection.range;
    if (!parseCodexStatsTimeRange(startInput, endInput)) return false;
    return writePreference(
      storageKey,
      JSON.stringify({ key: "custom", startInput, endInput }),
      storage,
    );
  }
  return writePreference(storageKey, selection.key, storage);
}

export function normalizeCodexSessionUsageRange(value: unknown): CodexSessionUsageRange {
  return value === "30d" || value === "month" || value === "all" ? value : "7d";
}

export function readCodexSessionUsageRange(storage?: RangePreferenceStorage): CodexSessionUsageRange {
  return normalizeCodexSessionUsageRange(readPreference(CODEX_SESSION_USAGE_RANGE_STORAGE_KEY, storage));
}

export function persistCodexSessionUsageRange(
  range: CodexSessionUsageRange,
  storage?: RangePreferenceStorage,
): boolean {
  return writePreference(CODEX_SESSION_USAGE_RANGE_STORAGE_KEY, range, storage);
}
