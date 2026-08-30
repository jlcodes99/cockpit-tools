import assert from "node:assert/strict";
import test from "node:test";

import { placeCodexMonthlyCreditsLast } from "./codexQuotaItemOrder.ts";

test("places monthly credits after every Codex quota item", () => {
  const items = [
    { key: "primary" },
    { key: "secondary" },
    { key: "monthly_credits" },
    { key: "additional:0:primary" },
    { key: "additional:0:secondary" },
    { key: "code_review" },
  ];

  assert.deepEqual(
    placeCodexMonthlyCreditsLast(items).map((item) => item.key),
    [
      "primary",
      "secondary",
      "additional:0:primary",
      "additional:0:secondary",
      "code_review",
      "monthly_credits",
    ],
  );
});
