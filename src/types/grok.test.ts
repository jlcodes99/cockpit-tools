import assert from "node:assert/strict";
import test from "node:test";

import {
  getGrokPlanBadge,
  getGrokQuotaSummaryItems,
  type GrokAccount,
} from "./grok.ts";

function account(overrides: Partial<GrokAccount>): GrokAccount {
  return {
    id: "grok-test",
    email: "grok@example.com",
    access_token: "",
    created_at: 0,
    last_used: 0,
    ...overrides,
  };
}

test("recognizes the spaced SuperGrok Heavy tier enum", () => {
  assert.equal(
    getGrokPlanBadge(
      account({ plan_type: "SUBSCRIPTION_TIER_SUPER_GROK_HEAVY" }),
    ),
    "SuperGrok Heavy",
  );
});

test("keeps weekly and Grok Build product usage visible", () => {
  const items = getGrokQuotaSummaryItems(
    account({
      quota: {
        weeklyLimitPercent: 18,
        periodEnd: "2099-01-01T00:00:00Z",
        products: [
          {
            product: "GrokBuild",
            usagePercent: 32,
          },
        ],
      },
    }),
    (_key, defaultValue) => defaultValue ?? "",
  );

  assert.deepEqual(
    items.map((item) => ({ key: item.key, label: item.label, percentage: item.percentage })),
    [
      { key: "weekly", label: "", percentage: 18 },
      { key: "product-0", label: "GrokBuild", percentage: 32 },
    ],
  );
});
