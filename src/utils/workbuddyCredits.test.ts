import assert from 'node:assert/strict';
import test from 'node:test';
import { PACKAGE_CODE as code, type WorkbuddyAccount } from '../types/codebuddy-suite.ts';
import { getWorkbuddyCredits, readWorkbuddyCreditPackage } from './workbuddyCredits.ts';

const now = Date.parse('2026-11-05T12:00:00+08:00');
const active = (PackageCode: string, total: number, left: number, DeductionEndTime = now + 3600000) => ({
  PackageCode, Status: 0, DeductionEndTime,
  CycleCapacitySizePrecise: String(total), CycleCapacityRemainPrecise: String(left),
});
const account = (resources: object[], resourceSummary?: object) => ({
  usage_raw: { data: { resources, resourceSummary } },
}) as WorkbuddyAccount;

test('WorkBuddy uses official summary and separates plan credits and rewards', () => {
  const credits = getWorkbuddyCredits(account([
    active(code.proMon, 500, 400), active(code.freeMon, 500, 500),
    active(code.activity, 100, 70), active(code.bonus30, 100, 100, now - 1),
    active(code.bonus28, 200, 180), active(code.extra, 50, 45),
  ], { Packages: [{ CycleRemainCapacity: '695.5' }] }), now);
  assert.equal(credits.left, 695.5);
  assert.deepEqual(credits.groups.map(({ kind, left, used }) => [kind, left, used]), [
    ['planBase', 400, 100], ['planBonus', 180, 20], ['benefit', 70, 30], ['addon', 45, 5],
  ]);
  assert.equal(credits.groups[2].expiring, true);
});

test('WorkBuddy preserves cached resource data and distinguishes missing quota from zero', () => {
  const resource = active(code.freeMon, 500, 500);
  const cached = {
    quota_raw: { userResource: { Response: { Data: { Accounts: [resource] } } } },
  } as WorkbuddyAccount;
  assert.equal(getWorkbuddyCredits(cached, now).left, 500);
  assert.equal(getWorkbuddyCredits(account([], { Packages: [] }), now).left, 0);
  assert.equal(getWorkbuddyCredits(account([]), now).left, null);
});

test('WorkBuddy grant details use lifetime capacity before cycle capacity', () => {
  const details = readWorkbuddyCreditPackage({
    ResourceId: 123,
    PackageName: 'Buddy 加油站签到',
    CapacitySizePrecise: '100',
    CapacityUsedPrecise: '12.5',
    CycleCapacitySizePrecise: '50',
    CycleCapacityRemainPrecise: '10',
    ExpiredTime: now,
  });
  assert.deepEqual(details, {
    id: '123', name: 'Buddy 加油站签到', total: 100, used: 12.5, left: 87.5, expireAt: now,
  });
});

test('WorkBuddy expired grants show actual consumed credits and the official grant reason', () => {
  const details = readWorkbuddyCreditPackage({
    PackageName: '平台奖励积分',
    AccountAttributes: [{ Key: 'grantReason', Value: 'Buddy 加油站签到' }],
    CapacitySizePrecise: '100',
    CapacityUsedPrecise: '100',
    ActualCycleCapacityUsed: '0',
  }, true);
  assert.equal(details.name, 'Buddy 加油站签到');
  assert.equal(details.used, 0);
  assert.equal(details.left, 100);
});
