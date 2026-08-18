import assert from "node:assert/strict";
import test from "node:test";

import {
  getCodexAdditionalQuotaWindows,
  getCodexQuotaWindowLabel,
  getCodexMonthlyCreditUsage,
  type CodexQuota,
} from "./codex.ts";

const quota: CodexQuota = {
  hourly_percentage: 75,
  weekly_percentage: 40,
  raw_data: {
    additional_rate_limits: [
      {
        limit_name: "gpt-5.3-codex-spark",
        metered_feature: "codex_spark",
        rate_limit: {
          primary_window: {
            used_percent: 35,
            limit_window_seconds: 18_000,
            reset_at: 1_790_000_000,
          },
          secondary_window: {
            used_percent: 60,
            limit_window_seconds: 604_800,
            reset_at: 1_790_500_000,
          },
        },
      },
    ],
  },
};

test("uses 5h / Weekly / N Week window labels", () => {
  assert.equal(getCodexQuotaWindowLabel(300, "hourly"), "5h");
  assert.equal(getCodexQuotaWindowLabel(10_080, "weekly"), "Weekly");
  assert.equal(getCodexQuotaWindowLabel(50_400, "weekly"), "5 Week");
  assert.equal(getCodexQuotaWindowLabel(undefined, "weekly"), "Weekly");
  assert.equal(getCodexQuotaWindowLabel(undefined, "hourly"), "5h");
});

test("keeps upstream Spark-specific quota windows for the account card", () => {
  assert.deepEqual(getCodexAdditionalQuotaWindows(quota), [
    {
      id: "additional:0:primary",
      sourceIndex: 0,
      windowKind: "primary",
      limitName: "gpt-5.3-codex-spark",
      limitLabel: "GPT 5.3 Codex Spark",
      meteredFeature: "codex_spark",
      allowed: undefined,
      limitReached: undefined,
      label: "5h",
      percentage: 65,
      resetTime: 1_790_000_000,
      windowMinutes: 300,
    },
    {
      id: "additional:0:secondary",
      sourceIndex: 0,
      windowKind: "secondary",
      limitName: "gpt-5.3-codex-spark",
      limitLabel: "GPT 5.3 Codex Spark",
      meteredFeature: "codex_spark",
      allowed: undefined,
      limitReached: undefined,
      label: "Weekly",
      percentage: 40,
      resetTime: 1_790_500_000,
      windowMinutes: 10_080,
    },
  ]);
});

test("reads Business monthly credits from the spend-control payload", () => {
  assert.deepEqual(
    getCodexMonthlyCreditUsage({
      hourly_percentage: 100,
      weekly_percentage: 100,
      raw_data: {
        plan_type: "business",
        spend_control: {
          individual_limit: {
            limit: "25000",
            used: "8000",
            remaining: "17000",
            remaining_percent: 68,
            reset_at: 1_790_000_000,
          },
        },
      },
    }),
    {
      used: 8000,
      total: 25000,
      remaining: 17000,
      remainingPercent: 68,
      resetTime: 1_790_000_000,
    },
  );
});

test("falls back to a balance-only credits payload", () => {
  assert.deepEqual(
    getCodexMonthlyCreditUsage({
      hourly_percentage: 100,
      weekly_percentage: 100,
      raw_data: {
        credits: {
          unlimited: false,
          balance: "42",
        },
      },
    }),
    {
      balance: "42",
      remaining: 42,
      unlimited: false,
    },
  );
});
