import assert from 'node:assert/strict';
import test from 'node:test';

import {
  formatKimiWindowLabel,
  getKimiAccountDisplayEmail,
  getKimiPlanBadge,
  getKimiQuotaClass,
  getKimiQuotaGroups,
  getKimiQuotaSummaryItems,
  hasKimiQuotaData,
  type KimiAccount,
} from './kimi.ts';

function account(overrides: Partial<KimiAccount> = {}): KimiAccount {
  return {
    id: 'kimi-test-1',
    email: 'user@example.com',
    access_token: '',
    created_at: 1,
    last_used: 1,
    ...overrides,
  };
}

test('getKimiAccountDisplayEmail prefers nickname over email', () => {
  assert.equal(
    getKimiAccountDisplayEmail(account({ nickname: 'Coder', email: 'user@example.com' })),
    'Coder',
  );
});

test('getKimiAccountDisplayEmail falls back for kimi.local placeholder', () => {
  assert.equal(
    getKimiAccountDisplayEmail(account({ email: 'unknown@kimi.local', user_id: 'uid-1' })),
    'unknown@kimi.local',
  );
});

test('formatKimiWindowLabel converts 300 minutes to hours in name', () => {
  assert.equal(formatKimiWindowLabel({ name: '300 minutes quota' }), '5 小时 quota');
});

test('hasKimiQuotaData detects weekly and rolling limits', () => {
  assert.equal(hasKimiQuotaData(account({ quota: { weeklyLimit: 100 } })), true);
  assert.equal(
    hasKimiQuotaData(account({ quota: { limits: [{ used: 1, limit: 10 }] } })),
    true,
  );
  assert.equal(hasKimiQuotaData(account({ quota: null })), false);
});

test('getKimiQuotaGroups orders rolling windows before weekly', () => {
  const groups = getKimiQuotaGroups(
    account({
      quota: {
        weeklyUsed: 10,
        weeklyLimit: 100,
        limits: [
          {
            name: '5 hour',
            windowUnit: 'hour',
            windowDuration: 5,
            used: 2,
            limit: 20,
          },
        ],
      },
    }),
  );
  assert.equal(groups.length, 2);
  assert.equal(groups[0].key, 'base');
  assert.equal(groups[1].key, 'extra');
});

test('getKimiQuotaSummaryItems maps used percent for bars', () => {
  const items = getKimiQuotaSummaryItems(
    account({
      quota: {
        limits: [{ used: 25, limit: 100, name: '5 hour', windowUnit: 'hour', windowDuration: 5 }],
      },
    }),
  );
  assert.equal(items.length, 1);
  assert.equal(items[0].percentage, 25);
});

test('getKimiQuotaClass maps used percent to severity', () => {
  assert.equal(getKimiQuotaClass(95), 'critical');
  assert.equal(getKimiQuotaClass(75), 'low');
  assert.equal(getKimiQuotaClass(50), 'medium');
  assert.equal(getKimiQuotaClass(10), 'high');
});

test('getKimiPlanBadge prefers quota user level', () => {
  assert.equal(
    getKimiPlanBadge(account({ quota: { userLevelName: 'ALLEGRO' }, plan_type: 'Kimi Code' })),
    'ALLEGRO',
  );
});
