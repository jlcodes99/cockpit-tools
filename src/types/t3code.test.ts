import assert from 'node:assert/strict';
import test from 'node:test';
import {
  getT3codeAccountDisplayEmail,
  getT3codePlanBadge,
  getT3codeQuotaGroups,
  getT3codeQuotaUpdatedAtMs,
  getT3codeUsage,
  hasT3codeResolvedIdentity,
  type T3CodeAccount,
} from './t3code';

const legacy: T3CodeAccount = {
  id: 't3code_test',
  user_id: 'user_test',
  email: 'old@example.test',
  display_name: 'Historical label',
  access_token: '',
  created_at: 1,
  last_used: 2,
  plan_type: 'FREE',
  quota_total: 3,
  usage_updated_at: 30,
};
const translate = (key: string) => key;

test('does not present legacy identity, inferred quota or refresh time as verified', () => {
  assert.equal(hasT3codeResolvedIdentity(legacy), false);
  assert.equal(getT3codeAccountDisplayEmail(legacy), '');
  assert.equal(getT3codePlanBadge(legacy), 'UNKNOWN');
  assert.equal(getT3codeUsage(legacy).isNormal, false);
  assert.equal(getT3codeQuotaUpdatedAtMs(legacy), null);
  assert.deepEqual(getT3codeQuotaGroups({ ...legacy, relay_verified_at: 30 }, translate), []);
});

test('shows the synchronized email, original plan and complete tunnel counts', () => {
  const account = {
    ...legacy,
    email: 'current@example.test',
    identity_verified_at: 25,
    plan_type: 'pro',
    quota_used: 1,
    quota_total: 10,
    relay_verified_at: 30,
  };
  assert.equal(getT3codeAccountDisplayEmail(account), 'current@example.test');
  assert.equal(getT3codePlanBadge(account), 'pro');
  const [group] = getT3codeQuotaGroups(account, translate);
  assert.deepEqual([group.used, group.total], [1, 10]);
  assert.equal(group.items[0].refreshAt, null);
  assert.equal(getT3codeQuotaUpdatedAtMs(account), 30_000);
});

test('keeps zero counts and separates an unlinked plan from an unknown one', () => {
  const account = {
    ...legacy,
    t3chat_linked: false,
    plan_type: undefined,
    quota_used: 0,
    quota_total: 0,
    relay_verified_at: 30,
    quota_query_last_error: 'Relay unavailable',
  };
  const [group] = getT3codeQuotaGroups(account, translate);
  assert.deepEqual([group.used, group.total], [0, 0]);
  assert.equal(getT3codePlanBadge(account, 'Unlinked'), 'Unlinked');
  assert.equal(getT3codePlanBadge({ ...account, t3chat_linked: undefined }, 'Unlinked'), 'UNKNOWN');
  assert.equal(getT3codeUsage(account).isNormal, false);
  assert.equal(getT3codeQuotaUpdatedAtMs(account), 30_000);
});
