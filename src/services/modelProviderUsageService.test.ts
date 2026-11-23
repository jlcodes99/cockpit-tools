import assert from "node:assert/strict";
import test from "node:test";
import {
  buildUsageBaseUrlCandidates,
  formatModelProviderUsageMoney,
  presentNewApiQuota,
  resolveNewApiQuotaSnapshot,
  resolveModelProviderUsageMode,
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

test("new_api quota recovers legacy billing values ahead of the unlimited key flag", () => {
  const usage = summary({
    mode: "new_api",
    quotaUnlimited: true,
    details: [
      { key: "hardLimitUsd", label: "Hard limit", value: "100" },
      { key: "totalUsage", label: "Total usage", value: "750" },
    ],
  });
  assert.deepEqual(presentNewApiQuota(usage, "Unlimited", "Unknown"), {
    text: "92.50 / 100", percent: 93, expiresAt: null,
  });
});

test("new_api quota never displays the legacy unlimited sentinel as an amount", () => {
  const usage = summary({
    mode: "new_api",
    quotaUnlimited: true,
    details: [
      { key: "hardLimitUsd", label: "Hard limit", value: "100000000" },
      { key: "totalUsage", label: "Total usage", value: "750" },
    ],
  });
  assert.deepEqual(presentNewApiQuota(usage, "Unlimited", "Unknown"), {
    text: "Unlimited", percent: 100, expiresAt: null,
  });
});

test("new_api quota distinguishes missing balance from an explicit zero", () => {
  const usage = summary({ mode: "new_api", quotaUnlimited: true, quotaLimit: 60, unit: "%" });
  assert.deepEqual(presentNewApiQuota(usage, "Unlimited", "Unknown"), {
    text: "Unknown / 60%", percent: null, expiresAt: null,
  });
  assert.deepEqual(presentNewApiQuota({ ...usage, remaining: 0 }, "Unlimited", "Unknown"), {
    text: "0% / 60%", percent: 0, expiresAt: null,
  });
  assert.deepEqual(presentNewApiQuota(summary({ balance: 4, unit: "CNY" }), "Unlimited", "Unknown"), {
    text: "¥4.00", percent: null, expiresAt: null,
  });
});

test("token plan percentages render without currency decimals", () => {
  assert.equal(formatModelProviderUsageMoney(72, "%"), "72%");
});

test("official usage stays a spend summary instead of falling back to a relay balance", () => {
  const usage = summary({ mode: 'openai_official', totalCost: 2, todayRequests: 4 });
  assert.equal(resolveModelProviderUsageMode(usage), 'openai_official');
  assert.equal(usage.balance, undefined);
  assert.equal(usage.quotaRemaining, undefined);
});

test("new-api amounts without a reported unit do not acquire a dollar sign", () => {
  assert.equal(formatModelProviderUsageMoney(45.2, ""), "45.20");
  assert.equal(formatModelProviderUsageMoney(0, ""), "0.00");
  assert.equal(formatModelProviderUsageMoney(45.2, "CNY"), "¥45.20");
  assert.equal(formatModelProviderUsageMoney(45.2, "USD"), "$45.20");
  assert.equal(formatModelProviderUsageMoney(45.2), "$45.20");
});
