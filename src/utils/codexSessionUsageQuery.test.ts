import assert from 'node:assert/strict';
import test from 'node:test';
import { buildCodexSessionUsageQuery } from './codexSessionUsageQuery.ts';

test('today starts at local midnight and refresh captures the new cutoff', () => {
  const now = new Date(2026, 9, 3, 23, 41, 22);
  const query = buildCodexSessionUsageQuery('today', 'instance-a', now);
  assert.equal(query.fromTimestamp, new Date(2026, 9, 3).getTime() / 1000);
  assert.equal(query.toTimestamp, now.getTime() / 1000);
  assert.equal(query.instanceId, 'instance-a');
  assert.ok(buildCodexSessionUsageQuery('today', '', new Date(2026, 9, 4, 0, 1)).fromTimestamp! > query.fromTimestamp!);
});
test('calendar ranges span local dates through daylight saving and all stays unbounded', () => {
  const previous = process.env.TZ;
  try {
    process.env.TZ = 'America/New_York';
    const query = buildCodexSessionUsageQuery('7d', '', new Date(2026, 2, 10, 10));
    const start = new Date(query.fromTimestamp! * 1000);
    assert.equal(start.getDate(), 4);
    assert.equal(start.getHours(), 0);
    assert.equal(buildCodexSessionUsageQuery('all').fromTimestamp, null);
  } finally {
    if (previous === undefined) delete process.env.TZ;
    else process.env.TZ = previous;
  }
});
