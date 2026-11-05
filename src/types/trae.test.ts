import assert from 'node:assert/strict';
import test from 'node:test';

import {
  getTraeAccountPlatformId,
  getTraeCreditCategoryGroups,
  getTraeUsage,
  type TraeAccount,
} from './trae.ts';

const NOW_SECONDS = Math.floor(Date.now() / 1000);
const FUTURE = NOW_SECONDS + 20 * 24 * 60 * 60;
const EXPIRED = NOW_SECONDS - 10 * 24 * 60 * 60;

function buildAccount(platformId: string, usageRaw: unknown): TraeAccount {
  return {
    id: 'test-account',
    email: 'user@example.com',
    access_token: 'token',
    created_at: NOW_SECONDS * 1000,
    last_used: NOW_SECONDS * 1000,
    trae_auth_raw: { platformId },
    trae_usage_raw: usageRaw,
  };
}

const creditsPayload = {
  code: 0,
  is_credits_billing: true,
  user_entitlement_pack_list: [
    {
      display_desc: 'Pro',
      entitlement_base_info: {
        product_type: 1,
        end_time: FUTURE,
        product_extra: {
          subscription_extra: { quota: { credits_limit: 4000, solo_agent_parallel_limit: 10 } },
        },
      },
      usage: { credits_amount: 1500 },
      status: 1,
    },
    {
      // 活动赠送包：status=0 也计入积分总额（对齐官方，仅过滤 is_hide）
      display_desc: 'TraeCode 邀请好友奖励',
      entitlement_base_info: {
        product_type: 2,
        end_time: FUTURE,
        quota: { credits_limit: 500 },
      },
      usage: {},
      status: 0,
    },
    {
      entitlement_id: 'checkin_20260902_123',
      entitlement_base_info: {
        product_type: 42,
        end_time: FUTURE,
        quota: { credits_limit: 200 },
      },
      usage: { credits_amount: 50 },
    },
    {
      entitlement_id: 'checkin_20260801_123',
      entitlement_base_info: {
        product_type: 42,
        end_time: EXPIRED,
        quota: { credits_limit: 200 },
      },
      usage: { credits_amount: 0 },
    },
    {
      display_desc: '积分加量包',
      entitlement_base_info: {
        product_type: 2,
        end_time: FUTURE,
        product_extra: { package_extra: { quota: { credits_limit: 1000 } } },
      },
      usage: { credits_amount: 1000 },
    },
    {
      display_desc: '隐藏积分包',
      is_hide: true,
      entitlement_base_info: { product_type: 1, quota: { credits_limit: 500 } },
      usage: { credits_amount: 0 },
    },
  ],
};

test('CN credits payload parses into credits usage model', () => {
  const account = buildAccount('trae_cn', creditsPayload);
  assert.equal(getTraeAccountPlatformId(account), 'trae_cn');

  const usage = getTraeUsage(account);
  assert.equal(usage.usageModel, 'credits');
  // 上游 getTraeCreditsUsage 不过滤 pack：status=0 与 is_hide 的包均计入总额
  assert.equal(usage.creditsLimit, 4000 + 500 + 200 + 200 + 1000 + 500);
  assert.equal(usage.creditsUsed, 1500 + 50 + 1000);
  assert.equal(usage.creditsRemaining, 2500 + 500 + 150 + 200 + 0 + 500);
  assert.equal(usage.usedPercent, (2550 / 6400) * 100);
  assert.equal(usage.spentUsd, null);
  assert.equal(usage.totalUsd, null);
  assert.equal(usage.usageExhausted, false);
  assert.equal(usage.creditsUnlimited, false);
});

test('CN credits groups split into member / bonus categories', () => {
  const account = buildAccount('trae_solo_cn', creditsPayload);
  const t = (_key: string, defaultValue?: string) => defaultValue ?? _key;
  const groups = getTraeCreditCategoryGroups(account, t);

  assert.deepEqual(
    groups.map((group) => group.key),
    ['base', 'activity'],
  );

  const [base, activity] = groups;
  assert.equal(base.label, '会员积分');
  // is_hide 的 500 与 status=0 的邀请奖励均计入（与上游 getTraeCreditsUsage 汇总口径一致）
  assert.equal(base.total, 4000 + 500);
  assert.equal(base.used, 1500);
  assert.equal(base.remain, 2500 + 500);
  assert.equal(base.visible, true);
  assert.equal(base.items[1].packageName, '隐藏积分包');

  // 非套餐包全部归入奖励积分：邀请奖励 + 签到包×2（过期包对齐官方不剔除）+ 加量包
  assert.equal(activity.label, '奖励积分');
  assert.equal(activity.total, 500 + 200 + 200 + 1000);
  assert.equal(activity.used, 50 + 1000);

  const names = groups.flatMap((group) => group.items.map((item) => item.packageName));
  assert.ok(!names.includes(null));
  assert.equal(activity.items[0].packageName, 'TraeCode 邀请好友奖励');
  assert.equal(activity.items[1].packageName, 'checkin_20260902_123');
});

test('legacy fast-request payload still reports fast usage', () => {
  const account = buildAccount('trae_cn', {
    code: 0,
    user_entitlement_pack_list: [
      {
        entitlement_base_info: {
          product_type: 1,
          end_time: FUTURE,
          quota: { premium_model_fast_request_limit: 500 },
        },
        usage: { premium_model_fast_amount: 245 },
        status: 1,
      },
    ],
  });

  // 无 is_credits_billing 标志时回落到 fast_request 模型（上游保留双模型）
  const usage = getTraeUsage(account);
  assert.equal(usage.usageModel, 'fast_request');
  assert.equal(usage.fastRequestLimit, 500);
  assert.equal(usage.fastRequestUsed, 245);
  assert.equal(usage.fastRequestAvailable, 255);
  assert.equal(usage.usedPercent, 49);
  assert.equal(usage.spentUsd, null);
  assert.equal(usage.totalUsd, null);
});

test('intl USD payload keeps usd usage model', () => {
  const account = buildAccount('trae', {
    code: 0,
    user_entitlement_pack_list: [
      {
        entitlement_base_info: {
          product_type: 1,
          end_time: FUTURE,
          quota: { basic_usage_limit: 10 },
        },
        usage: { basic_usage_amount: 5 },
        status: 1,
      },
    ],
  });

  const usage = getTraeUsage(account);
  assert.equal(usage.usageModel, 'usd');
  assert.equal(usage.spentUsd, 5);
  assert.equal(usage.totalUsd, 10);
  assert.equal(usage.usedPercent, 50);
});

test('unlimited credits limit reports unlimited semantics', () => {
  const account = buildAccount('trae_cn', {
    code: 0,
    is_credits_billing: true,
    user_entitlement_pack_list: [
      {
        entitlement_base_info: {
          product_type: 1,
          end_time: FUTURE,
          quota: { credits_limit: -1 },
        },
        usage: { credits_amount: 300 },
        status: 1,
      },
    ],
  });

  const usage = getTraeUsage(account);
  assert.equal(usage.usageModel, 'credits');
  assert.equal(usage.creditsLimit, null);
  assert.equal(usage.creditsRemaining, null);
  assert.equal(usage.creditsUnlimited, true);
  assert.equal(usage.usedPercent, null);
  assert.equal(usage.usageExhausted, false);

  const groups = getTraeCreditCategoryGroups(account, (_key, defaultValue) => defaultValue ?? _key);
  assert.equal(groups[0].unlimited, true);
  assert.equal(groups[0].visible, true);
});

test('exhausted credits mark usage exhausted', () => {
  const account = buildAccount('trae_cn', {
    code: 0,
    is_credits_billing: true,
    user_entitlement_pack_list: [
      {
        entitlement_base_info: {
          product_type: 1,
          end_time: FUTURE,
          quota: { credits_limit: 400 },
        },
        usage: { credits_amount: 400 },
        status: 1,
      },
    ],
  });

  const usage = getTraeUsage(account);
  assert.equal(usage.creditsRemaining, 0);
  assert.equal(usage.usageExhausted, true);
});
