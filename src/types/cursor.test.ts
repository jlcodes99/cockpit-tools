import assert from 'node:assert/strict';
import test from 'node:test';
import { getCursorPlanBadge, getCursorPlanBadgeClass, getCursorPlanDisplayName, getCursorUsage, type CursorAccount } from './cursor';

const account = (extra: Partial<CursorAccount> = {}): CursorAccount => ({
  id: 'test', email: 'test@example.com', access_token: '', created_at: 0, last_used: 0, ...extra,
});

test('Cursor displays unified, legacy and future membership values verbatim', () => {
  for (const membership_type of ['Plus', 'Premium', 'Super', 'Ultra', 'Teams', 'pro_plus', 'pro_student', 'Future Tier']) {
    assert.equal(getCursorPlanDisplayName(account({ membership_type })), membership_type);
  }
  assert.equal(getCursorPlanDisplayName(account({ plan_type: 'Premium' })), 'Premium');
  assert.equal(getCursorPlanDisplayName(account({ membership_type: ' ', plan_type: 'Super' })), 'Super');
  assert.equal(getCursorPlanBadge(account({ plan_type: 'Premium' })), 'PREMIUM');
});

test('new tier styling preserves existing legacy filter categories', () => {
  assert.equal(getCursorPlanBadgeClass('Plus'), 'plus');
  assert.equal(getCursorPlanBadgeClass('Premium'), 'pro');
  assert.equal(getCursorPlanBadgeClass('Super'), 'ultra');
  assert.equal(getCursorPlanBadge(account({ membership_type: 'pro_student' })), 'PRO');
});

test('legacy Cursor usage keeps plan usage separate from paid additional usage', () => {
  const usage = getCursorUsage(account({ membership_type: 'Premium', cursor_usage_raw: {
    billingCycleEnd: '2026-11-12T00:00:00Z',
    individualUsage: { plan: { used: 500, limit: 2000, totalPercentUsed: 25 }, onDemand: { used: 800, limit: 1000, enabled: true } },
  } }));
  assert.equal(usage.inlineSuggestionsUsedPercent, 25);
  assert.equal(usage.onDemandUsedCents, 800);
  assert.equal(usage.allowanceResetAt, Date.parse('2026-11-12T00:00:00Z') / 1000);
});
