import test from "node:test";
import assert from "node:assert/strict";
import { getQoderSubscriptionInfo, type QoderAccount } from "./qoder.ts";

test("getQoderSubscriptionInfo handles enterprise orgResourcePackage with cap", () => {
  const account: QoderAccount = {
    id: "qoder_1",
    email: "zt02571897@qoder.alibaba-inc.com",
    created_at: 0,
    last_used: 0,
    auth_credit_usage_raw: {
      userQuota: {
        used: 536,
        total: 6000,
        remaining: 5464,
        percentage: 9,
      },
      orgResourcePackage: {
        available: true,
        cap: 14000,
        percentage: 0,
        remaining: 14000,
        unit: "credits",
        used: 0,
      },
    },
  };

  const info = getQoderSubscriptionInfo(account);
  assert.equal(info.userQuota.total, 6000);
  assert.equal(info.userQuota.used, 536);
  assert.equal(info.addOnQuota.total, 14000, "addOnQuota total should be 14000 from cap");
  assert.equal(info.addOnQuota.used, 0);
  assert.equal(info.addOnQuota.remaining, 14000);
  assert.equal(info.addOnQuota.percentage, 0);
});

test("getQoderSubscriptionInfo handles standard subscription without enterprise cap", () => {
  const account: QoderAccount = {
    id: "qoder_2",
    email: "user@qoder.com",
    created_at: 0,
    last_used: 0,
    auth_credit_usage_raw: {
      userQuota: {
        used: 100,
        total: 3000,
        remaining: 2900,
        percentage: 3,
      },
    },
  };

  const info = getQoderSubscriptionInfo(account);
  assert.equal(info.userQuota.total, 3000);
  assert.equal(info.userQuota.used, 100);
  assert.equal(info.addOnQuota.total, null);
});
