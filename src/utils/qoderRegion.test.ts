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
  // The backend resolves the marker from both raw payloads; a session stamped
  // only in the plan must not be listed as global and then fail to switch.
  assert.equal(getQoderRegion({ ...account, auth_user_plan_raw: { login_source: 'qoder_cn' } }), 'cn');
  assert.equal(
    getQoderRegion({
      ...account,
      auth_user_info_raw: { login_source: 'qoder' },
      auth_user_plan_raw: { login_source: 'qoder_cn' },
    }),
    'global',
  );
});

test('Qoder client targets follow native adapter platform support', () => {
  assert.equal(defaultQoderClient('cn', true), 'desktop');
  assert.equal(defaultQoderClient('cn', false), 'cli');
  assert.equal(defaultQoderClient('global', true), 'ide');
  assert.equal(defaultQoderClient('global', false), 'ide');

  assert.equal(isQoderClientAvailable('cn', 'desktop', true), true);
  assert.equal(isQoderClientAvailable('cn', 'desktop', false), false);
  assert.equal(isQoderClientAvailable('cn', 'cli', false), true);
  assert.equal(isQoderClientAvailable('cn', 'ide', true), false);
  // The global client got the native desktop and CLI adapters too; desktop stays
  // Windows-only on both realms because its credential file is.
  assert.equal(isQoderClientAvailable('global', 'desktop', true), true);
  assert.equal(isQoderClientAvailable('global', 'desktop', false), false);
  assert.equal(isQoderClientAvailable('global', 'cli', true), true);
  assert.equal(isQoderClientAvailable('global', 'cli', false), true);
  assert.equal(isQoderClientAvailable('global', 'ide', false), true);
});
