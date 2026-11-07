import assert from 'node:assert/strict';
import test from 'node:test';
import { getResetCreditExpiry, RESET_CREDIT_URGENT_SECONDS } from './codexResetCreditExpiry.ts';

const now = 1_800_000_000;
test('strictly less than twelve hours triggers urgency', () => {
  for (const [seconds, urgent] of [[43201, false], [RESET_CREDIT_URGENT_SECONDS, false], [43199, true], [1, true]] as const) {
    assert.equal(getResetCreditExpiry({ credits: [], next_expires_at: now + seconds }, now)?.urgent, urgent);
  }
});
test('selects earliest unconsumed future credit and advances when it expires', () => {
  const snapshot = { credits: [
    { status: 'used', expires_at: now + 1 },
    { status: 'expired', expires_at: now + 2 },
    { expires_at: now + 100 }, { expires_at: now + 200 },
  ], next_expires_at: now + 100 };
  assert.equal(getResetCreditExpiry(snapshot, now)?.expiresAt, now + 100);
  assert.equal(getResetCreditExpiry(snapshot, now + 100)?.expiresAt, now + 200);
  assert.equal(getResetCreditExpiry(snapshot, now + 200)?.expired, true);
});
test('unknown, invalid and zero-count data do not invent an expiry', () => {
  assert.equal(getResetCreditExpiry({ credits: [] }, now), null);
  assert.equal(getResetCreditExpiry({ credits: [], next_expires_at: NaN }, now), null);
  assert.equal(getResetCreditExpiry({ credits: [], next_expires_at: -1 }, now), null);
  assert.equal(getResetCreditExpiry({ credits: [{ expires_at: now + 1 }], available_count: 0 }, now), null);
});
test('expired summary is explicitly expired, never an urgent available credit', () => {
  const result = getResetCreditExpiry({ credits: [], next_expires_at: now }, now);
  assert.equal(result?.expired, true);
  assert.equal(result?.urgent, false);
});
