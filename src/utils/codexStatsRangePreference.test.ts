import assert from "node:assert/strict";
import test from "node:test";

import {
  buildCodexStatsTimeRange,
  parseCodexStatsTimeRange,
} from "./codexStatsRange.ts";
import {
  CODEX_SESSION_USAGE_RANGE_STORAGE_KEY,
  persistCodexSessionUsageRange,
  persistCodexStatsRangeSelection,
  readCodexSessionUsageRange,
  readCodexStatsRangeSelection,
} from "./codexStatsRangePreference.ts";

class MemoryStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

const API_RANGE_KEY = "test.codex.api.range";
const NOW = new Date(2026, 8, 6, 13, 20, 0);

test("defaults to rolling seven days when no range was saved", () => {
  const storage = new MemoryStorage();
  const selection = readCodexStatsRangeSelection(API_RANGE_KEY, NOW, storage);

  assert.equal(selection.key, "rolling7d");
  assert.deepEqual(selection.range, buildCodexStatsTimeRange("rolling7d", NOW));
});

test("restores every preset instead of treating daily as the default", () => {
  const storage = new MemoryStorage();
  for (const key of ["daily", "rolling7d", "weekly", "monthly"] as const) {
    storage.setItem(API_RANGE_KEY, key);
    const selection = readCodexStatsRangeSelection(API_RANGE_KEY, NOW, storage);
    assert.equal(selection.key, key);
    assert.deepEqual(selection.range, buildCodexStatsTimeRange(key, NOW));
  }
});

test("round-trips custom start and end dates", () => {
  const storage = new MemoryStorage();
  const range = parseCodexStatsTimeRange("2026-08-01", "2026-08-17");
  assert.ok(range);

  assert.equal(
    persistCodexStatsRangeSelection(API_RANGE_KEY, { key: "custom", range }, storage),
    true,
  );
  const restored = readCodexStatsRangeSelection(API_RANGE_KEY, NOW, storage);
  assert.equal(restored.key, "custom");
  assert.equal(restored.range.startInput, "2026-08-01");
  assert.equal(restored.range.endInput, "2026-08-17");
});

test("falls back safely when a saved custom range is invalid", () => {
  const storage = new MemoryStorage();
  storage.setItem(
    API_RANGE_KEY,
    JSON.stringify({ key: "custom", startInput: "2026-02-31", endInput: "2026-03-02" }),
  );
  assert.equal(readCodexStatsRangeSelection(API_RANGE_KEY, NOW, storage).key, "rolling7d");
});

test("persists the session usage range independently", () => {
  const storage = new MemoryStorage();
  assert.equal(persistCodexSessionUsageRange("month", storage), true);
  assert.equal(readCodexSessionUsageRange(storage), "month");
  assert.equal(storage.getItem(CODEX_SESSION_USAGE_RANGE_STORAGE_KEY), "month");
});
