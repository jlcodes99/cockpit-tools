import assert from 'node:assert/strict';
import test from 'node:test';

import { PACKAGE_CODE } from '../../types/codebuddy-suite.ts';
import type { MinimaxCodeAccount } from '../../types/minimaxCode.ts';
import {
  getMiniMaxCodeQuotaGroups,
  hasMiniMaxCodeQuotaData,
  normalizeMiniMaxCreditDetail,
  wrapMiniMaxCreditsAsResource,
} from './parser.ts';

const EXPIRE_MS = 1_800_000_000_000;

function account(quotaRaw: unknown): MinimaxCodeAccount {
  return {
    id: 'minimax-account',
    email: 'minimax@example.com',
    access_token: 'redacted',
    quota_raw: quotaRaw,
    created_at: 1,
    last_used: 2,
  };
}

test('normalizes a MiniMax credit detail onto the legacy resource shape', () => {
  const resource = normalizeMiniMaxCreditDetail({
    credit_type: 2,
    granted_amount: '1000',
    consumed_amount: '250',
    remaining_amount: '750',
    expire_at_ms: EXPIRE_MS,
  });

  assert.equal(resource.Status, 0);
  assert.equal(resource.PackageCode, PACKAGE_CODE.activity);
  assert.equal(resource.PackageName, '');
  assert.equal(resource.CycleCapacitySizePrecise, '1000');
  assert.equal(resource.CycleCapacityUsedPrecise, '250');
  assert.equal(resource.CycleCapacityRemainPrecise, '750');
  assert.equal(resource.DeductionEndTime, EXPIRE_MS / 1000);
  assert.equal(resource.ExpiredTime, new Date(EXPIRE_MS).toISOString());
});

test('maps purchased credit_type onto a distinct package label', () => {
  const resource = normalizeMiniMaxCreditDetail({
    credit_type: 1,
    granted_amount: 500,
    consumed_amount: 0,
    remaining_amount: 500,
  });

  assert.equal(resource.PackageCode, PACKAGE_CODE.extra);
  assert.equal(resource.CycleCapacitySizePrecise, '500');
  assert.equal(resource.DeductionEndTime, null);
  assert.equal(resource.ExpiredTime, null);
});

test('wraps a credit/details body into the legacy Accounts envelope', () => {
  const wrapped = wrapMiniMaxCreditsAsResource({
    data: {
      details: [
        {
          credit_type: 2,
          granted_amount: '1,000',
          consumed_amount: '100',
          remaining_amount: '900',
        },
        {
          credit_type: 1,
          granted_amount: '200',
          consumed_amount: '50',
          remaining_amount: '150',
        },
      ],
    },
  });

  assert.equal(wrapped.code, 0);
  const accounts = wrapped.data.Response.Data.Accounts;
  assert.equal(accounts.length, 2);
  assert.equal(wrapped.data.Response.Data.TotalCount, 2);
  assert.equal(wrapped.data.Response.Data.TotalDosage, 150);
  assert.equal(accounts[0].CycleCapacitySizePrecise, '1000');
  assert.equal(accounts[1].PackageCode, PACKAGE_CODE.extra);
});

test('reads details from the top level of the real credit/details body', () => {
  // The official client reads `response.data.details` straight off the axios
  // response, so `details` / `total_count` sit at the top level of the body.
  const wrapped = wrapMiniMaxCreditsAsResource({
    base_resp: { status_code: 0, status_msg: 'success' },
    details: [
      { credit_type: 1, granted_amount: '500', consumed_amount: '50', remaining_amount: '450' },
      { credit_type: 2, granted_amount: '1,000', consumed_amount: '200', remaining_amount: '800' },
    ],
    total_count: 2,
  });

  const accounts = wrapped.data.Response.Data.Accounts;
  assert.equal(accounts.length, 2);
  assert.equal(accounts[0].PackageCode, PACKAGE_CODE.extra);
  assert.equal(accounts[0].CycleCapacityRemainPrecise, '450');
  assert.equal(accounts[1].PackageCode, PACKAGE_CODE.activity);
  assert.equal(accounts[1].CycleCapacitySizePrecise, '1000');
  assert.equal(wrapped.data.Response.Data.TotalDosage, 250);
});

test('lands each credit type in a labelled group instead of 其他', () => {
  // Regression: bare 'free'/'purchased' codes fell through to the `其他` group,
  // so the quota card showed one ugly "其他" row.
  const groups = getMiniMaxCodeQuotaGroups(
    account({
      base_resp: { status_code: 0 },
      details: [
        { credit_type: 2, granted_amount: '400', consumed_amount: '0', remaining_amount: '400' },
        { credit_type: 1, granted_amount: '500', consumed_amount: '50', remaining_amount: '450' },
      ],
      total_count: 2,
    }),
    (_key, defaultValue) => defaultValue ?? '',
  );

  const labels = groups.filter((group) => group.visible).map((group) => group.label);
  assert.ok(labels.includes('活动赠送包'), `expected 活动赠送包, got ${JSON.stringify(labels)}`);
  assert.ok(labels.includes('加量包'), `expected 加量包, got ${JSON.stringify(labels)}`);
  assert.ok(!labels.includes('其他'), `其他 should be empty, got ${JSON.stringify(labels)}`);
});

test('drops detail rows the official client would discard', () => {
  const wrapped = wrapMiniMaxCreditsAsResource({
    details: [
      { credit_type: 2, granted_amount: '100', consumed_amount: '10', remaining_amount: '90' },
      // Unknown credit_type and a row with no amounts at all.
      { credit_type: 9, granted_amount: '5', consumed_amount: '1', remaining_amount: '4' },
      { credit_type: 1 },
    ],
    total_count: 3,
  });

  const accounts = wrapped.data.Response.Data.Accounts;
  assert.equal(accounts.length, 1);
  assert.equal(accounts[0].CycleCapacityRemainPrecise, '90');
});

test('tolerates a missing or malformed quota body', () => {
  const wrapped = wrapMiniMaxCreditsAsResource(undefined);
  assert.deepEqual(wrapped.data.Response.Data.Accounts, []);
  assert.equal(wrapped.data.Response.Data.TotalCount, 0);
  assert.equal(wrapped.data.Response.Data.TotalDosage, 0);
});

test('groups wrapped credits through the shared quota pipeline', () => {
  const groups = getMiniMaxCodeQuotaGroups(
    account({
      userResource: {
        data: {
          details: [
            {
              credit_type: 2,
              granted_amount: '1000',
              consumed_amount: '400',
              remaining_amount: '600',
            },
          ],
        },
      },
    }),
    (_key, defaultValue) => defaultValue ?? '',
  );

  assert.equal(groups.length, 4);
  const populated = groups.filter((group) => group.items.length > 0);
  assert.equal(populated.length, 1);
  assert.equal(populated[0].total, 1000);
  assert.equal(populated[0].used, 400);
  assert.equal(populated[0].remain, 600);
  assert.equal(hasMiniMaxCodeQuotaData(account(null), populated), true);
  assert.equal(hasMiniMaxCodeQuotaData(account(null), groups), true);
});

test('reports no quota data when the account has no credit details', () => {
  const groups = getMiniMaxCodeQuotaGroups(account(null), (_key, defaultValue) => defaultValue ?? '');
  assert.equal(hasMiniMaxCodeQuotaData(account(null), groups), false);
});
