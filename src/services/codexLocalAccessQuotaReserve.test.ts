import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

test('OAuth binding sends reserve thresholds and protection to the host and returns its authoritative state', async () => {
  const calls: unknown[] = [];
  const state = { collection: { boundOauthAccountId: 'oauth-1' } };
  const h = loadHookModule(new URL('./codexLocalAccessService.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (...args: unknown[]) => { calls.push(args); return state; } },
  });
  const reserve = { hourlyPercent: 30, weeklyPercent: 10, strict: true, safetyMarginPercent: 3 };
  assert.equal(await h.exports.updateCodexLocalAccessBoundOAuthAccount('oauth-1', reserve), state);
  assert.equal((calls[0] as any)[0], 'codex_local_access_update_bound_oauth_account');
  assert.equal((calls[0] as any)[1].boundOauthAccountId, 'oauth-1');
  assert.equal((calls[0] as any)[1].boundOauthQuotaReserve, reserve);
  await h.exports.updateCodexLocalAccessBoundOAuthAccount('oauth-1', null);
  await h.exports.updateCodexLocalAccessBoundOAuthAccount(null);
  assert.equal((calls[1] as any)[1].boundOauthQuotaReserve, null);
  assert.equal((calls[2] as any)[1].boundOauthAccountId, null);
  assert.equal((calls[2] as any)[1].boundOauthQuotaReserve, null);
});

test('OAuth reauthorization retains custom reserve and safety margin until the binding retry', () => {
  const dispatched: any[] = [];
  const h = loadHookModule(new URL('../utils/codexAddAccountRequest.ts', import.meta.url), {}, {
    window: { dispatchEvent: (event: unknown) => dispatched.push(event) },
    CustomEvent: class { constructor(public type: string, public detail?: unknown) {} },
  });
  const reserve = { hourlyPercent: 25, weeklyPercent: 15, strict: true, safetyMarginPercent: 5 };
  h.exports.requestCodexOpenAddAccount({
    tab: 'oauth', targetAccountId: ' oauth-1 ',
    retryOAuthBinding: { targetKind: 'local_access', quotaReserve: reserve },
  });
  assert.equal(dispatched.length, 2);
  const pending = h.exports.takePendingCodexOpenAddAccountRequest();
  assert.equal(pending.targetAccountId, 'oauth-1');
  assert.deepEqual(pending.retryOAuthBinding.quotaReserve, reserve);
  assert.equal(h.exports.takePendingCodexOpenAddAccountRequest(), null);
});
