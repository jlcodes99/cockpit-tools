import assert from 'node:assert/strict';
import test from 'node:test';
import { PACKAGE_CODE as code, type WorkbuddyAccount } from '../types/codebuddy-suite.ts';
import { getWorkbuddyAiCredits, getWorkbuddyAiPlan, getWorkbuddyCredits, getWorkbuddyExpiringCredits, readWorkbuddyCreditPackage, workbuddyCreditPercent } from './workbuddyCredits.ts';

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

test('WorkBuddy AI uses the Codex FREE badge for the official free plan', () => {
  const cycleEnd = now + 3600000;
  const free = account([
    { ...active(code.freeMonIntl, 100, 98.06), CycleEndTime: new Date(cycleEnd).toISOString() },
  ], { SubscriptionPackageCode: code.freeMonIntl, SubscriptionPackageName: 'Free', Packages: [{ CycleRemainCapacity: '98.06' }] });
  const credits = getWorkbuddyAiCredits(free, now);
  assert.deepEqual(getWorkbuddyAiPlan(free), { label: 'FREE', style: 'free' });
  assert.equal(credits.left, 98.06);
  const [base] = credits.groups;
  assert.equal(base.kind, 'planBase');
  assert.equal(base.time, cycleEnd + 1000);
  assert.equal(base.used.toFixed(2), '1.94');
  assert.equal(workbuddyCreditPercent(base.total, base.left, Math.floor), 98);
  assert.equal(workbuddyCreditPercent(base.total, base.used, Math.round), 2);
});

test('WorkBuddy AI keeps the gray FREE badge when only the billing name or cached label is available', () => {
  const billing = account([], { SubscriptionPackageName: 'Free', SubscriptionPackageCode: 'new-free-code' });
  const display = { label: 'FREE', style: 'free' };
  assert.deepEqual(getWorkbuddyAiPlan(billing), display);
  assert.deepEqual(getWorkbuddyAiPlan({ ...account([]), plan_type: display.label }), display);
  assert.equal(getWorkbuddyAiPlan(account([], { SubscriptionPackageName: 'New Plan', IsPaidUser: true })).style, 'unknown');
});

test('WorkBuddy AI uses the billing subscription name and counts older free grants as rewards', () => {
  const paid = account([
    active(code.proMon, 500, 300),
    active(code.freeMon, 100, 75),
    active(code.gift, 10, 5),
  ], { SubscriptionPackageCode: code.proMon, SubscriptionPackageName: 'Pro Monthly', Packages: [
    { CycleRemainCapacity: '300' }, { CycleRemainCapacity: '80' },
  ] });
  assert.deepEqual(getWorkbuddyAiPlan(paid), { label: 'Pro Monthly', style: 'pro' });
  assert.deepEqual(getWorkbuddyAiCredits(paid, now).groups.map(({ kind, left }) => [kind, left]), [
    ['planBase', 300], ['benefit', 80],
  ]);
  assert.equal(getWorkbuddyAiPlan(account([])).label, 'UNKNOWN');
});

test('WorkBuddy AI keeps the usage row for a fully unused plan without package details', () => {
  const summary = {
    SubscriptionPackageCode: code.freeMonIntl,
    Packages: [{ PackageCode: code.freeMonIntl, CycleTotalCapacity: '100', CycleRemainCapacity: '100', CycleUsedCapacity: '0' }],
  };
  const [base] = getWorkbuddyAiCredits(account([], summary), now).groups;
  assert.equal(base.kind, 'planBase');
  assert.equal(base.used, 0);
  assert.equal(base.total, 100);
  assert.equal(workbuddyCreditPercent(base.total, base.left, Math.floor), 100);
  assert.equal(getWorkbuddyAiCredits(account([], { Packages: [] }), now).groups.length, 0);
});

test('WorkBuddy expiring badge only counts reward and addon packages inside the horizon', () => {
  const expiring = getWorkbuddyExpiringCredits(account([
    active(code.proMon, 500, 400, now + 3600000),
    active(code.activity, 100, 70, now + 3 * 3600000),
    active(code.extra, 50, 45, now + 20 * 3600000),
    active(code.bonus30, 100, 100, now - 1),
    active(code.bonus29, 80, 0, now + 3600000),
    active(code.bonus28, 200, 180, now + 40 * 3600000),
  ]), now);
  assert.deepEqual(expiring.packages.map(({ group, left }) => [group, left]), [
    ['benefit', 70], ['addon', 45],
  ]);
  assert.equal(expiring.total, 115);
});

test('WorkBuddy expiring badge sorts by expiry, keeps the horizon edge and prefers the grant reason', () => {
  const expiring = getWorkbuddyExpiringCredits(account([
    { ...active(code.extra, 20, 20, now + 5 * 3600000), AccountAttributes: [{ Key: 'grantReason', Value: 'Buddy 加油站签到' }] },
    active(code.activity, 10, 10, now + 3600000),
    active(code.bonus29, 10, 10, now + 24 * 3600000),
    active(code.activity, 10, 10, now + 24 * 3600000 + 1),
  ]), now);
  assert.deepEqual(expiring.packages.map(({ packageCode, name, expireAt }) => [packageCode, name, expireAt - now]), [
    [code.activity, '', 3600000],
    [code.extra, 'Buddy 加油站签到', 5 * 3600000],
    [code.bonus29, '', 24 * 3600000],
  ]);
  assert.equal(expiring.total, 40);
});

test('WorkBuddy AI treats free grants as expiring rewards', () => {
  const ai = getWorkbuddyExpiringCredits(account([
    active(code.freeMon, 100, 75, now + 3600000),
    active(code.gift, 10, 5, now + 7200000),
    active(code.proMon, 500, 300, now + 3600000),
  ]), now, 86400000, true);
  assert.deepEqual(ai.packages.map(({ group, left }) => [group, left]), [['benefit', 75], ['benefit', 5]]);
  assert.equal(ai.total, 80);
  assert.equal(getWorkbuddyExpiringCredits(account([active(code.freeMon, 100, 75, now + 3600000)]), now).packages.length, 0);
});

test('WorkBuddy expiring badge schedules the next wake-up outside the horizon', () => {
  const later = getWorkbuddyExpiringCredits(account([
    active(code.activity, 100, 70, now + 30 * 3600000),
    active(code.extra, 20, 20, now + 40 * 3600000),
    active(code.proMon, 500, 400, now + 3600000),
  ]), now);
  assert.equal(later.packages.length, 0);
  assert.equal(later.nextExpiryAt, now + 30 * 3600000);
  assert.equal(getWorkbuddyExpiringCredits(account([active(code.activity, 100, 70, now - 1)]), now).nextExpiryAt, null);
  assert.equal(getWorkbuddyExpiringCredits(account([active(code.bonus29, 10, 0, now + 3600000)]), now).nextExpiryAt, null);
});
