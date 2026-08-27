import assert from "node:assert/strict";
import test from "node:test";
import {
  buildUsageBaseUrlCandidates,
  formatModelProviderUsageMoney,
  resolveNewApiQuotaSnapshot,
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
