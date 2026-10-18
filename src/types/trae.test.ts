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
  // status=0 的邀请奖励 500 计入；is_hide 的 500 排除（官方仅过滤 is_hide）
  assert.equal(usage.creditLimit, 4000 + 500 + 200 + 200 + 1000);
  assert.equal(usage.creditUsed, 1500 + 50 + 1000);
  assert.equal(usage.creditAvailable, 2500 + 500 + 150 + 200 + 0);
  assert.equal(usage.usedPercent, Math.round((2550 / 5900) * 100));
  assert.equal(usage.spentUsd, null);
  assert.equal(usage.totalUsd, null);
  assert.equal(usage.usageExhausted, false);
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
  assert.equal(base.total, 4000);
  assert.equal(base.used, 1500);
  assert.equal(base.remain, 2500);
  assert.equal(base.visible, true);

  // 非套餐包全部归入奖励积分：邀请奖励 + 签到包×2（过期包对齐官方不剔除）+ 加量包
  assert.equal(activity.label, '奖励积分');
  assert.equal(activity.total, 500 + 200 + 200 + 1000);
  assert.equal(activity.used, 50 + 1000);

  // is_hide 的包不进入任何分组
  const names = groups.flatMap((group) => group.items.map((item) => item.packageName));
  assert.ok(!names.includes(null));
  assert.equal(activity.items[0].packageName, 'TraeCode 邀请好友奖励');
  assert.equal(activity.items[1].packageName, 'checkin_20260902_123');
});

test('legacy fast-request payload no longer reports fast usage', () => {
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

  const usage = getTraeUsage(account);
  assert.equal(usage.usageModel, 'unknown');
  assert.equal(usage.creditAvailable, null);
  assert.equal(usage.usedPercent, null);
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

test('unlimited credits limit reports -1 availability', () => {
  const account = buildAccount('trae_cn', {
    code: 0,
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
  assert.equal(usage.creditLimit, -1);
  assert.equal(usage.creditAvailable, -1);
  assert.equal(usage.usedPercent, 0);
  assert.equal(usage.usageExhausted, false);

  const groups = getTraeCreditCategoryGroups(account, (_key, defaultValue) => defaultValue ?? _key);
  assert.equal(groups[0].unlimited, true);
  assert.equal(groups[0].visible, true);
});

test('exhausted credits mark usage exhausted', () => {
  const account = buildAccount('trae_cn', {
    code: 0,
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
  assert.equal(usage.creditAvailable, 0);
  assert.equal(usage.usageExhausted, true);
});
