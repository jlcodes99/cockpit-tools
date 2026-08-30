import assert from "node:assert/strict";
import test from "node:test";

import {
  buildCodexAccountWindowStatQueries,
  formatCodexCompactNumber,
  formatCodexWindowCostAmount,
  formatCodexWindowStatsText,
  hasVisibleCodexWindowStats,
  resolveCodexWindowStartSeconds,
  toCodexLocalAccessLogTimestamp,
} from "./codexWindowStats.ts";

test("uses resetAt minus window when reset is still in the future", () => {
  const now = 1_800_000_000;
  const resetAt = now + 3_600;
  assert.equal(
    resolveCodexWindowStartSeconds(resetAt, 300, now, 300),
    resetAt - 300 * 60,
  );
});

test("uses resetAt as start after the window has already reset", () => {
  const now = 1_800_000_000;
  const resetAt = now - 10;
  assert.equal(
    resolveCodexWindowStartSeconds(resetAt, 300, now, 300),
    resetAt,
  );
});

test("uses fallback minutes when window minutes are missing", () => {
  const now = 1_800_000_000;
  assert.equal(
    resolveCodexWindowStartSeconds(undefined, undefined, now, 120),
    now - 120 * 60,
  );
});

test("converts official reset seconds into request_log milliseconds", () => {
  assert.equal(toCodexLocalAccessLogTimestamp(1_800_000_000), 1_800_000_000_000);
  assert.equal(
    toCodexLocalAccessLogTimestamp(1_800_000_000_000),
    1_800_000_000_000,
  );
});

test("builds 5h/7d queries from last full window to now in log milliseconds", () => {
  const now = 1_800_000_000;
  const resetAt = now + 3_600;
  const queries = buildCodexAccountWindowStatQueries(
    "acct-1",
    [
      { id: "primary", resetTime: resetAt, windowMinutes: 300 },
      { id: "secondary", resetTime: resetAt + 86_400, windowMinutes: 10_080 },
    ],
    now,
  );
  assert.deepEqual(queries, [
    {
      accountId: "acct-1",
      windowKey: "primary",
      startAt: (resetAt - 300 * 60) * 1000,
      endAt: now * 1000,
    },
    {
      accountId: "acct-1",
      windowKey: "secondary",
      startAt: (resetAt + 86_400 - 10_080 * 60) * 1000,
      endAt: now * 1000,
    },
  ]);
});

test("formats compact numbers like Sub2API", () => {
  assert.equal(formatCodexCompactNumber(0), "0");
  assert.equal(formatCodexCompactNumber(999), "999");
  assert.equal(formatCodexCompactNumber(1000), "1.0K");
  assert.equal(formatCodexCompactNumber(999_999), "1000.0K");
  assert.equal(formatCodexCompactNumber(1_000_000), "1.0M");
  assert.equal(formatCodexCompactNumber(1_000_000_000), "1.0B");
  assert.equal(
    formatCodexCompactNumber(1_000_000_000, { allowBillions: false }),
    "1000.0M",
  );
  assert.equal(formatCodexWindowCostAmount(0), "0.00");
  assert.equal(formatCodexWindowCostAmount(0.004), "0.00");
  assert.equal(formatCodexWindowCostAmount(47.78), "47.78");
});

test("renders Sub2API window_stats fields and hides empty usage", () => {
  const stats = {
    requestCount: 422,
    inputTokens: 100,
    cachedInputTokens: 0,
    outputTokens: 20,
    totalTokens: 61_800_000,
    estimatedCostUsd: 47.78,
  };
  assert.equal(hasVisibleCodexWindowStats(stats), true);
  assert.equal(formatCodexWindowStatsText(stats), "422 req · 61.8M · A $47.78");
  assert.equal(
    formatCodexWindowStatsText({ ...stats, userCostUsd: 1.52 }),
    "422 req · 61.8M · A $47.78 · U $1.52",
  );
  assert.equal(
    hasVisibleCodexWindowStats({
      requestCount: 0,
      inputTokens: 0,
      cachedInputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      estimatedCostUsd: 0,
    }),
    false,
  );
});
