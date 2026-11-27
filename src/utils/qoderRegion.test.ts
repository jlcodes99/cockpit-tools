import assert from 'node:assert/strict';
import test from 'node:test';
import {
  defaultQoderClient,
  getQoderRegion,
  isQoderClientAvailable,
} from './qoderRegion';
import type { QoderAccount } from '../types/qoder';

// Mirrors `qoder_channel::source_of`: `qoder_cn` is the public CN client, while
// `qodercn` / `dedicated_qodercn` belong to the retired CN IDE adapters and stay
// in the global list.
test('Qoder account region follows the qoder_cn marker, not the display name', () => {
  const account = {
    id: 'test',
    email: 'test@example.invalid',
    created_at: 1,
    last_used: 1,
    tags: ['国际版'],
  } as QoderAccount;
  assert.equal(getQoderRegion({ ...account, auth_user_info_raw: { login_source: 'qoder_cn' } }), 'cn');
  assert.equal(getQoderRegion({ ...account, auth_user_info_raw: { login_source: 'qodercn' } }), 'global');
  assert.equal(
    getQoderRegion({ ...account, auth_user_info_raw: { login_source: 'dedicated_qodercn' } }),
    'global',
  );
  assert.equal(getQoderRegion({ ...account, tags: ['国内 CN'] }), 'global');
  assert.equal(getQoderRegion({ ...account, auth_user_info_raw: { login_source: 'qoder' } }), 'global');
});

test('Qoder client targets follow native adapter platform support', () => {
  assert.equal(defaultQoderClient('cn', true), 'desktop');
  assert.equal(defaultQoderClient('cn', false), 'cli');
  assert.equal(defaultQoderClient('cn', false, true), 'desktop');
  assert.equal(defaultQoderClient('global', true), 'ide');
  assert.equal(defaultQoderClient('global', false), 'ide');

  assert.equal(isQoderClientAvailable('cn', 'desktop', true), true);
  assert.equal(isQoderClientAvailable('cn', 'desktop', false), false);
  assert.equal(isQoderClientAvailable('cn', 'cli', false), true);
  assert.equal(isQoderClientAvailable('cn', 'ide', true), false);
  assert.equal(isQoderClientAvailable('cn', 'desktop', false, true), true);
  assert.equal(isQoderClientAvailable('cn', 'ide', false, true), true);
  assert.equal(isQoderClientAvailable('cn', 'ide', false), false);
  assert.equal(isQoderClientAvailable('global', 'desktop', false, true), true);
  assert.equal(isQoderClientAvailable('global', 'desktop', true), true);
  assert.equal(isQoderClientAvailable('global', 'desktop', false), false);
  assert.equal(isQoderClientAvailable('global', 'cli', true), true);
  assert.equal(isQoderClientAvailable('global', 'cli', false), true);
  assert.equal(isQoderClientAvailable('global', 'ide', false), true);
});
