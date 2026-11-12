import assert from "node:assert/strict";
import test from "node:test";
import {
  nearestQuotaHistoryPoint, quotaHistoryDateRange, quotaHistorySeries, quotaHistoryTicks, type QuotaHistoryPoint,
} from "./codexQuotaHistoryChart";

test("date filters use local calendar boundaries and navigate across months and years", () => {
  const now = new Date(2026, 0, 1, 18);
  assert.deepEqual(quotaHistoryDateRange("today", -1, now), {
    from: new Date(2025, 11, 31).getTime(), to: new Date(2026, 0, 1).getTime(),
  });
  assert.deepEqual(quotaHistoryDateRange("thisWeek", 0, now), {
    from: new Date(2025, 11, 29).getTime(), to: new Date(2026, 0, 5).getTime(),
  });
  assert.deepEqual(quotaHistoryDateRange("thisMonth", -1, now), {
    from: new Date(2025, 11, 1).getTime(), to: new Date(2026, 0, 1).getTime(),
  });
});

test("week and month show times for a single day and distinct dates across days", () => {
  const start = new Date(2026, 10, 12, 9).getTime();
  const sameDay = new Date(2026, 10, 12, 15).getTime();
  const nextDay = new Date(2026, 10, 13, 9).getTime();
  for (const period of ["thisWeek", "thisMonth"] as const) {
    assert.deepEqual(quotaHistoryTicks(start, sameDay, period, 4, "en-GB").map(({ label }) => label),
      ["09:00", "11:00", "13:00", "15:00"]);
    const days = quotaHistoryTicks(start, nextDay, period, 4, "en-GB");
    assert.deepEqual(days.map(({ value }) => value), [start, nextDay]);
    assert.equal(new Set(days.map(({ label }) => label)).size, 2);
  }
});

test("the chart keeps zeros and resets while leaving unavailable observations and intervals blank", () => {
  const points: QuotaHistoryPoint[] = [
    { bucketStart: 0, observedAt: 100, shortRemaining: 0, weeklyRemaining: 61 },
    { bucketStart: 60_000, observedAt: 70_000, shortRemaining: 100, weeklyRemaining: 61 },
    { bucketStart: 120_000, observedAt: 121_000, shortRemaining: null, weeklyRemaining: 60 },
    { bucketStart: 180_000, observedAt: 185_000, shortRemaining: 95, weeklyRemaining: null },
    { bucketStart: 300_000, observedAt: 312_000, shortRemaining: 94, weeklyRemaining: 60 },
  ];
  const short = quotaHistorySeries(points, "shortRemaining", (x) => x / 1000, (y) => y, 60_000);
  assert.equal(short.path, "M0.10,0.00 L70.00,100.00 M185.00,95.00 M312.00,94.00");
  assert.deepEqual(short.singles, [{ x: 185, y: 95 }, { x: 312, y: 94 }]);
  assert.equal(nearestQuotaHistoryPoint(points, 119_000), 2);
  assert.equal(nearestQuotaHistoryPoint(points, 313_000), 4);
});
