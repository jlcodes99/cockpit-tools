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

test("official Pro tiers use numbered names and retain the existing capsule classes", () => {
  for (const [planType, label, className] of [
    ['prolite', 'PRO 100', 'pro codex-pro-lite'],
    ['pro', 'PRO 200', 'pro codex-pro-max'],
    ['promax', 'PRO 500', 'pro codex-pro-500'],
    ['chatgptprolite', 'PRO 100', 'pro codex-pro-lite'],
    ['chatgptpro', 'PRO 200', 'pro codex-pro-max'],
    ['chatgptpromax', 'PRO 500', 'pro codex-pro-500'],
    ['PRO 100', 'PRO 100', 'pro codex-pro-lite'],
    ['PRO 200', 'PRO 200', 'pro codex-pro-max'],
    ['PRO 500', 'PRO 500', 'pro codex-pro-500'],
  ]) {
    const item = account({ plan_type: planType });
    assert.deepEqual(getCodexPlanBadgePresentation(item), { label, className }, planType);
    assert.equal(getCodexPlanFilterKey(item), 'PRO', 'existing Pro filters still include every tier');
  }
});

test("legacy import hints keep their old tier until official usage identifies the plan", () => {
  const legacyLite = account({ plan_type: 'pro', auth_file_plan_type: 'prolite' });
  const legacyMax = account({ plan_type: 'pro', auth_file_plan_type: 'promax' });
  assert.equal(getCodexPlanBadgePresentation(legacyLite).label, 'PRO 100');
  assert.equal(getCodexPlanBadgePresentation(legacyMax).label, 'PRO 200');
  for (const [plan_type, expected] of [['prolite', 'PRO 100'], ['pro', 'PRO 200'], ['promax', 'PRO 500']]) {
    for (const legacy of [legacyLite, legacyMax]) {
      const item = { ...legacy, quota: { hourly_percentage: 90, weekly_percentage: 50, raw_data: { plan_type } } };
      assert.equal(getCodexPlanBadgePresentation(item).label, expected);
    }
  }
  assert.equal(getCodexPlanBadgePresentation({ ...legacyLite, plan_type: 'promax' }).label, 'PRO 500');
  assert.equal(getCodexPlanBadgePresentation({ ...legacyMax, plan_type: 'prolite' }).label, 'PRO 100');
});

test("expired Pro 500 loses its premium badge even with cached official usage", () => {
  const item = account({ plan_type: 'promax', subscription_active_until: '2020-01-01T00:00:00Z',
    quota: { hourly_percentage: 100, weekly_percentage: 100, raw_data: { plan_type: 'promax' } } });
  assert.deepEqual(getCodexPlanBadgePresentation(item), {
    label: 'FREE', className: 'free',
  });
});

test('raw custom labels preserve the original backend value', () => {
  const badge = getCodexPlanBadgePresentation(account({ plan_type: 'Business Custom' }), { preserveRawNonProLabel: true });
  assert.equal(badge.label, 'Business Custom');
  assert.equal(badge.className, 'business');
});
