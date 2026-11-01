import assert from 'node:assert/strict';
import test from 'node:test';

import {
  getClaudeRemainingPercentage,
  getClaudeRemainingQuotaClass,
} from './claude.ts';

test('converts Claude used percentages to remaining percentages', () => {
  assert.equal(getClaudeRemainingPercentage(0), 100);
  assert.equal(getClaudeRemainingPercentage(42), 58);
  assert.equal(getClaudeRemainingPercentage(100), 0);
});

test('clamps invalid and out-of-range Claude usage percentages', () => {
  assert.equal(getClaudeRemainingPercentage(-10), 100);
  assert.equal(getClaudeRemainingPercentage(120), 0);
  assert.equal(getClaudeRemainingPercentage(Number.NaN), 0);
});

test('uses healthy colors for high remaining quota and warning colors when low', () => {
  assert.equal(getClaudeRemainingQuotaClass(100), 'high');
  assert.equal(getClaudeRemainingQuotaClass(58), 'medium');
  assert.equal(getClaudeRemainingQuotaClass(30), 'low');
  assert.equal(getClaudeRemainingQuotaClass(10), 'critical');
  assert.equal(getClaudeRemainingQuotaClass(Number.NaN), 'critical');
});

test('preserves server-authored desktop subscription labels', async () => {
  const { getClaudePlanBadge } = await import('./claude.ts');
  const account = { id: 'desktop', email: '', auth_mode: 'desktop_oauth' as const, created_at: 0, last_used: 0 };
  assert.equal(getClaudePlanBadge({ ...account, plan_type: 'Free' }), 'Free');
  assert.equal(getClaudePlanBadge({ ...account, plan_type: 'Enterprise Custom' }), 'Enterprise Custom');
  assert.equal(getClaudePlanBadge({ ...account, plan_type: 'claude_pro' }), 'claude_pro');
  assert.equal(getClaudePlanBadge(account), '');
});
