import assert from "node:assert/strict";
import test from "node:test";
import {
  buildUsageBaseUrlCandidates,
  formatModelProviderUsageMoney,
  formatModelProviderUsageResetCountdown,
  resolveModelProviderUsageMode,
  resolveNewApiQuotaSnapshot,
  resolveOpenCodeGoQuotaSnapshot,
  type ModelProviderUsageSummary,
} from "./modelProviderUsageService.ts";

function summary(
  partial: Partial<ModelProviderUsageSummary>,
): ModelProviderUsageSummary {
  return {
    modelStatsCount: 0,
    latencyMs: 0,
    ...partial,
  };
}

test("usage lookup tries a root provider URL before its /v1 fallback", () => {
  assert.deepEqual(
    buildUsageBaseUrlCandidates("https://sub2api.example.com/"),
    ["https://sub2api.example.com/", "https://sub2api.example.com/v1"],
  );
});

test("usage lookup does not rewrite providers with an explicit path", () => {
  assert.deepEqual(
    buildUsageBaseUrlCandidates("https://sub2api.example.com/api"),
    ["https://sub2api.example.com/api"],
  );
});

test("new_api quota uses token allocation details when available", () => {
  const snapshot = resolveNewApiQuotaSnapshot(
    summary({
      mode: "new_api",
      quotaLimit: 100,
      quotaRemaining: 80,
      details: [
        { key: "totalGranted", label: "Granted", value: "250" },
        { key: "totalAvailable", label: "Available", value: "175.5" },
        { key: "expiresAt", label: "Expires", value: "1800000000" },
      ],
    }),
  );

  assert.deepEqual(snapshot, {
    granted: 250,
    available: 175.5,
    expiresAt: 1800000000,
  });
});

test("new_api quota falls back to billing limits when token allocation is absent", () => {
  const snapshot = resolveNewApiQuotaSnapshot(
    summary({
      mode: "new_api",
      quotaLimit: 1849,
      quotaRemaining: 1610,
      details: [
        { key: "hardLimitUsd", label: "Hard Limit", value: "1849" },
        { key: "accessUntil", label: "Access Until", value: "1815609561" },
        { key: "totalUsage", label: "Total Usage", value: "23900" },
      ],
    }),
  );

  assert.deepEqual(snapshot, {
    granted: 1849,
    available: 1610,
    expiresAt: 1815609561,
  });
});

test("new_api quota ignores malformed numeric details", () => {
  const snapshot = resolveNewApiQuotaSnapshot(
    summary({
      mode: "new_api",
      quotaLimit: 75,
      quotaRemaining: 25,
      details: [
        { key: "totalGranted", label: "Granted", value: "unlimited" },
        { key: "totalAvailable", label: "Available", value: "" },
        { key: "expiresAt", label: "Expires", value: "never" },
      ],
    }),
  );

  assert.deepEqual(snapshot, {
    granted: 75,
    available: 25,
    expiresAt: null,
  });
});

test("token plan percentages render without currency decimals", () => {
  assert.equal(formatModelProviderUsageMoney(72, "%"), "72%");
});

test("OpenCode Go quota resolves all usage windows", () => {
  const usage = summary({
    mode: "opencode_go",
    details: [
      { key: "rollingUsedPercent", label: "5-Hour Used %", value: "25.5" },
      { key: "rollingRemainingPercent", label: "5-Hour Remaining %", value: "74.5" },
      { key: "rollingResetsAt", label: "5-Hour Reset", value: "1786766400" },
      { key: "weeklyUsedPercent", label: "Weekly Used %", value: "61" },
      { key: "weeklyRemainingPercent", label: "Weekly Remaining %", value: "39" },
      { key: "weeklyResetsAt", label: "Weekly Reset", value: "1787011200" },
      { key: "monthlyUsedPercent", label: "Monthly Used %", value: "40" },
      { key: "monthlyRemainingPercent", label: "Monthly Remaining %", value: "60" },
      { key: "monthlyResetsAt", label: "Monthly Reset", value: "1788220800" },
    ],
  });

  assert.equal(resolveModelProviderUsageMode(usage), "opencode_go");
  assert.deepEqual(resolveOpenCodeGoQuotaSnapshot(usage), {
    rolling: { usedPercent: 25.5, remainingPercent: 74.5, resetsAt: 1786766400 },
    weekly: { usedPercent: 61, remainingPercent: 39, resetsAt: 1787011200 },
    monthly: { usedPercent: 40, remainingPercent: 60, resetsAt: 1788220800 },
  });
});

test("OpenCode Go mode is detected from a partial rolling-only window", () => {
  const usage = summary({
    mode: null,
    details: [
      { key: "rollingUsedPercent", label: "5-Hour Used %", value: "12" },
      { key: "rollingRemainingPercent", label: "5-Hour Remaining %", value: "88" },
      { key: "rollingResetsAt", label: "5-Hour Reset", value: "1786766400" },
    ],
  });

  assert.equal(resolveModelProviderUsageMode(usage), "opencode_go");
  const snapshot = resolveOpenCodeGoQuotaSnapshot(usage);
  assert.deepEqual(snapshot.rolling, {
    usedPercent: 12,
    remainingPercent: 88,
    resetsAt: 1786766400,
  });
  assert.equal(snapshot.weekly.remainingPercent, null);
  assert.equal(snapshot.monthly.resetsAt, null);
});

test("reset countdown formats relative windows and past timestamps", () => {
  const fixedNow = Date.parse("2026-08-25T14:00:00Z");
  const realNow = Date.now;
  Date.now = () => fixedNow;
  try {
    // 2 days 3 hours ahead
    assert.equal(
      formatModelProviderUsageResetCountdown(
        (fixedNow + 2 * 86_400_000 + 3 * 3_600_000) / 1000,
      ),
      "2d 3h",
    );
    // 95 minutes ahead
    assert.equal(
      formatModelProviderUsageResetCountdown((fixedNow + 95 * 60_000) / 1000),
      "1h 35m",
    );
    // already elapsed
    assert.equal(formatModelProviderUsageResetCountdown(fixedNow / 1000 - 60), "resetting");
    assert.equal(formatModelProviderUsageResetCountdown(null), "-");
  } finally {
    Date.now = realNow;
  }
});
