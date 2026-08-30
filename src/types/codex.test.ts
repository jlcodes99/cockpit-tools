import assert from "node:assert/strict";
import test from "node:test";

import {
  getCodexPlanBadgePresentation,
  getCodexPlanFilterKey,
} from "./codex.ts";
import type { CodexAccount } from "./codex.ts";

function account(partial: Partial<CodexAccount>): CodexAccount {
  return {
    id: "account-1",
    email: "account@example.com",
    tokens: {
      id_token: "id-token",
      access_token: "access-token",
      refresh_token: "refresh-token",
    },
    created_at: Date.now(),
    last_used: Date.now(),
    ...partial,
  };
}

test("expired paid subscriptions are presented and grouped as free", () => {
  const expiredPlus = account({
    plan_type: "plus",
    subscription_active_until: "2020-01-01T00:00:00Z",
  });

  assert.equal(getCodexPlanFilterKey(expiredPlus), "FREE");
  assert.deepEqual(getCodexPlanBadgePresentation(expiredPlus), {
    label: "FREE",
    className: "free",
  });
});

test("expired pro subscriptions no longer retain the pro multiplier badge", () => {
  const expiredPro = account({
    plan_type: "pro",
    subscription_active_until: "2020-01-01T00:00:00Z",
  });

  assert.equal(getCodexPlanFilterKey(expiredPro), "FREE");
  assert.deepEqual(getCodexPlanBadgePresentation(expiredPro), {
    label: "FREE",
    className: "free",
  });
});

test("active paid subscriptions keep their paid plan", () => {
  const activePlus = account({
    plan_type: "plus",
    subscription_active_until: "2100-01-01T00:00:00Z",
  });

  assert.equal(getCodexPlanFilterKey(activePlus), "PLUS");
  assert.deepEqual(getCodexPlanBadgePresentation(activePlus), {
    label: "PLUS",
    className: "plus codex-plus",
  });
});

test("paid subscriptions without a usable expiry are not downgraded", () => {
  const missingExpiry = account({ plan_type: "plus" });
  const invalidExpiry = account({
    plan_type: "plus",
    subscription_active_until: "not-a-date",
  });

  assert.equal(getCodexPlanFilterKey(missingExpiry), "PLUS");
  assert.equal(getCodexPlanFilterKey(invalidExpiry), "PLUS");
});

test("API key and pending accounts keep their special classifications", () => {
  const apiKeyAccount = account({
    auth_mode: "apikey",
    plan_type: "API_KEY",
    subscription_active_until: "2020-01-01T00:00:00Z",
  });
  const pendingAccount = account({
    authorization_status: "pending",
    plan_type: "plus",
    subscription_active_until: "2020-01-01T00:00:00Z",
  });

  assert.equal(getCodexPlanFilterKey(apiKeyAccount), "API_KEY");
  assert.equal(getCodexPlanFilterKey(pendingAccount), "PENDING");
});
