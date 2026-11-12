import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { toProviderAccountDisplayCache } from './providerAccountDisplayCache';
import { getTraeAccountDisplayName, getTraeAccountPlatformId, type TraeAccount } from '../types/trae';

test('cache keeps the TRAE edition and display name while excluding its authorization payload', () => {
  const account = {
    id: 'trae-1', email: 'example@test.invalid', created_at: 1, last_used: 2,
    access_token: 'fixture-access', refresh_token: 'fixture-refresh',
    trae_auth_raw: { platformId: 'trae_solo_cn', account: { nickname: 'Example' }, token: 'fixture-auth' },
  } satisfies TraeAccount;
  const cached = toProviderAccountDisplayCache(account, 'trae');
  assert.equal(getTraeAccountPlatformId(cached), 'trae_solo_cn');
  assert.equal(getTraeAccountDisplayName(cached), 'Example');
  assert.doesNotMatch(JSON.stringify(cached), /fixture-/);
  assert.equal(account.access_token, 'fixture-access');
});

test('projection drops API credentials and opaque auth blobs but keeps allowlisted display fields', () => {
  const cache = toProviderAccountDisplayCache({
    id: 'cursor-1', access_token: 'fixture-access', api_key: 'fixture-api', api_extra_env: { KEY: 'fixture-env' },
    cursor_auth_raw: { isEnterprise: true, callback: { token: 'fixture-callback' } },
  }, 'cursor');
  assert.deepEqual(cache.cursor_auth_raw, { isEnterprise: true });
  assert.doesNotMatch(JSON.stringify(cache), /fixture-/);
});

test('a scheduled legacy cache cleanup cannot overwrite newer state', () => {
  let value = 'old';
  const callbacks: Array<() => void> = [];
  const { exports } = loadHookModule(new URL('./providerAccountDisplayCache.ts', import.meta.url), {}, {
    setTimeout: (callback: () => void) => callbacks.push(callback),
    localStorage: { getItem: () => value, setItem: (_key: string, next: string) => { value = next; } },
  });
  exports.replaceLegacyAccountDisplayCache('accounts', 'old', 'clean');
  value = 'newer';
  callbacks.shift()?.();
  assert.equal(value, 'newer');
  exports.replaceLegacyAccountDisplayCache('accounts', 'newer', 'clean');
  callbacks.shift()?.();
  assert.equal(value, 'clean');
});
