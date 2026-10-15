import assert from 'node:assert/strict';
import test from 'node:test';

import type { CodebuddySuiteAccountBase } from '../../types/codebuddy-suite.ts';
import { PACKAGE_CODE } from '../../types/codebuddy-suite.ts';
import { extractResourceAccounts, getAccountQuotaUpdatedAtMs } from './parser.ts';
import { getOfficialQuotaModel } from './quota-model.ts';

function account(usageRaw: unknown): CodebuddySuiteAccountBase {
  return {
    id: 'test-account',
    email: 'test@example.com',
    access_token: 'redacted',
    usage_raw: usageRaw,
    created_at: 1,
    last_used: 2,
  };
}

test('normalizes latest data.resources payload and keeps new package codes', () => {
  const model = getOfficialQuotaModel(account({
    code: 0,
    data: {
      resources: [{
        commodity_code: PACKAGE_CODE.freeMonIntl,
        name: 'Free monthly credits',
        remaining: 75,
        total: 100,
        expire_at: 1_800_000_000,
      }],
    },
  }));

  assert.equal(model.resources.length, 1);
  assert.equal(model.resources[0].packageCode, PACKAGE_CODE.freeMonIntl);
  assert.equal(model.resources[0].remain, 75);
  assert.equal(model.resources[0].total, 100);
  assert.equal(model.resources[0].expireAt, 1_800_000_000_000);
});

test('keeps legacy Response.Data.Accounts payload compatible', () => {
  const resources = extractResourceAccounts(account({
    data: {
      Response: {
        Data: {
          Accounts: [{
            Status: 0,
            PackageCode: PACKAGE_CODE.proMon,
            CycleCapacitySizePrecise: '200',
            CycleCapacityRemainPrecise: '150',
          }],
        },
      },
    },
  }));

  assert.equal(resources.length, 1);
  assert.equal(resources[0].PackageCode, PACKAGE_CODE.proMon);
});

test('normalizes snake_case enterprise usage and unlimited sentinel', () => {
  const model = getOfficialQuotaModel(account({
    code: 0,
    data: {
      limit_num: -1,
      used_num: 321,
      cycle_reset_time: '2026-09-01 00:00:00',
    },
  }));

  assert.equal(model.resources.length, 1);
  assert.equal(model.resources[0].packageCode, PACKAGE_CODE.enterprise);
  assert.equal(model.resources[0].unlimited, true);
  assert.equal(model.resources[0].total, -1);
  assert.equal(model.resources[0].remain, -1);
  assert.equal(model.resources[0].refreshAt, Date.parse('2026-09-01T00:00:00'));
});

test('accepts nested and root enterprise response shapes', () => {
  const nested = extractResourceAccounts(account({ data: { data: { limitNum: 1000, credit: 250 } } }));
  const root = extractResourceAccounts(account({ limitNum: 500, usedNum: 125 }));

  assert.equal(nested[0].CycleCapacityRemainPrecise, '750');
  assert.equal(root[0].CycleCapacityRemainPrecise, '375');
});

test('uses usage_updated_at instead of last_used for quota freshness', () => {
  const value = account(null);
  value.last_used = 9_999_999_999;
  value.usage_updated_at = 1_800_000_000;
  assert.equal(getAccountQuotaUpdatedAtMs(value), 1_800_000_000_000);
});

test('keeps unknown official package name and numeric cycle reset time', () => {
  const model = getOfficialQuotaModel(account({
    data: {
      Response: {
        Data: {
          Accounts: [{
            Status: '0',
            PackageCode: 'TCACA_code_future',
            PackageName: 'Future Package',
            CycleCapacitySizePrecise: '80',
            CycleCapacityRemainPrecise: '30',
            CycleResetTime: '1800000000',
          }],
        },
      },
    },
  }));

  assert.equal(model.resources.length, 1);
  assert.equal(model.resources[0].packageCode, 'TCACA_code_future');
  assert.equal(model.resources[0].packageName, 'Future Package');
  assert.equal(model.resources[0].refreshAt, 1_800_000_000_000);
});
