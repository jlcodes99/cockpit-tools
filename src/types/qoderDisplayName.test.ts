import assert from 'node:assert/strict';
import test from 'node:test';
import { getQoderAccountDisplayEmail, type QoderAccount } from './qoder';
import { buildQoderAccountPresentation } from '../presentation/platformAccountPresentation';

const base: QoderAccount = {
  id: 'fixture-account', email: 'unknown@qoder.local',
  display_name: '已有昵称', user_id: 'fixture-user', created_at: 0, last_used: 0,
};

test('CN accounts without an email show the saved nickname in account and dashboard views', () => {
  const original = structuredClone(base);
  assert.equal(getQoderAccountDisplayEmail(base), '已有昵称');
  assert.equal(buildQoderAccountPresentation(base, key => key).displayName, '已有昵称');
  assert.deepEqual(base, original);
});

test('real email addresses keep their existing display priority', () => {
  assert.equal(getQoderAccountDisplayEmail({ ...base, email: ' member@example.test ' }), 'member@example.test');
});

test('blank and case-varied placeholder emails do not suppress a nickname', () => {
  for (const email of ['', '  ', ' Unknown@Qoder.Local ']) {
    assert.equal(getQoderAccountDisplayEmail({ ...base, email }), '已有昵称');
  }
});

test('older records can use the native name before falling back to stable IDs', () => {
  const account = { ...base, display_name: null, auth_user_info_raw: { name: '客户端昵称' } };
  assert.equal(getQoderAccountDisplayEmail(account), '客户端昵称');
  assert.equal(getQoderAccountDisplayEmail({ ...account, auth_user_info_raw: undefined }), 'fixture-user');
  assert.equal(getQoderAccountDisplayEmail({ ...account, auth_user_info_raw: undefined, user_id: null }), 'fixture-account');
});
