import assert from 'node:assert/strict';
import test from 'node:test';
import { customRemoteKeyRange, remoteKeyRange } from './codexRemoteKeyRange';

test('remote ranges use UTC boundaries, including across a month rollover', () => {
  const now=Date.parse('2026-11-01T01:00:00+08:00');
  assert.deepEqual(remoteKeyRange('today',now),{startAt:Date.parse('2026-10-31T00:00:00Z'),endAt:Date.parse('2026-11-01T00:00:00Z')});
  assert.equal(remoteKeyRange('7d',now).startAt,Date.parse('2026-10-25T00:00:00Z'));
  assert.equal(remoteKeyRange('month',now).startAt,Date.parse('2026-10-01T00:00:00Z'));
});
test('custom ranges reject invalid, future and oversized dates, and include the final date', () => {
  const now=Date.parse('2026-10-30T10:00:00Z');
  assert.deepEqual(customRemoteKeyRange('2026-10-29','2026-10-30',now),{startAt:Date.parse('2026-10-29T00:00:00Z'),endAt:Date.parse('2026-10-31T00:00:00Z')});
  for (const [start,end] of [['2026-02-30','2026-03-01'],['2026-10-30','2026-10-29'],['2026-10-30','2026-11-01'],['2026-01-01','2026-10-30'],['','']]) {
    assert.equal(customRemoteKeyRange(start,end,now),null);
  }
});
