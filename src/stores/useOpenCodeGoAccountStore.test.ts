import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createOpenCodeGoAccountStore,
  type OpenCodeGoAccountClient,
} from './useOpenCodeGoAccountStore.ts';
import type {
  OpenCodeGoConnection,
  OpenCodeGoQuotaSnapshot,
} from '../types/openCodeGo.ts';

const quota: OpenCodeGoQuotaSnapshot = {
  rolling: { status: 'available', usedPercent: 20, remainingPercent: 80, resetsAt: 100 },
  weekly: { status: 'available', usedPercent: 30, remainingPercent: 70, resetsAt: 200 },
  monthly: { status: 'available', usedPercent: 40, remainingPercent: 60, resetsAt: 300 },
  status: 'available',
  queriedAt: 50,
};

function account(
  id: string,
  overrides: Partial<OpenCodeGoConnection> = {},
): OpenCodeGoConnection {
  return {
    id,
    name: `Connection ${id}`,
    keyHint: 'sk-…last',
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function client(overrides: Partial<OpenCodeGoAccountClient> = {}): OpenCodeGoAccountClient {
  return {
    listAccounts: async () => [],
    refreshQuota: async (accountId) => ({
      connection: account(accountId, { quota }),
      quota,
    }),
    refreshAllQuotas: async () => ({ connections: [] }),
    ...overrides,
  };
}

test('fetchAccounts loads sanitized OpenCode Go connection summaries', async () => {
  const expected = [account('first'), account('second')];
  const store = createOpenCodeGoAccountStore(
    client({ listAccounts: async () => expected }),
  );

  await store.getState().fetchAccounts();

  assert.deepEqual(store.getState().accounts, expected);
  assert.equal(store.getState().loading, false);
  assert.equal(store.getState().error, null);
});

test('refreshQuota replaces only the matching connection summary', async () => {
  const refreshed = account('first', { quota, updatedAt: 2 });
  const store = createOpenCodeGoAccountStore(
    client({
      listAccounts: async () => [account('first'), account('second')],
      refreshQuota: async () => ({ connection: refreshed, quota }),
    }),
  );

  await store.getState().fetchAccounts();
  await store.getState().refreshQuota('first');

  assert.deepEqual(store.getState().accounts, [refreshed, account('second')]);
  assert.equal(store.getState().loading, false);
});

test('refreshAllQuotas publishes cached failures without discarding connections', async () => {
  const failed = account('second', {
    quotaError: { kind: 'network', occurredAt: 99 },
  });
  const store = createOpenCodeGoAccountStore(
    client({ refreshAllQuotas: async () => ({ connections: [account('first', { quota }), failed] }) }),
  );

  await store.getState().refreshAllQuotas();

  assert.deepEqual(store.getState().accounts, [account('first', { quota }), failed]);
  assert.equal(store.getState().error, null);
});

test('refreshQuota reconciles the stored account before rethrowing a provider error', async () => {
  const failed = account('target', {
    quotaError: { kind: 'authentication', occurredAt: 75 },
    updatedAt: 3,
  });
  let listCall = 0;
  const store = createOpenCodeGoAccountStore(
    client({
      listAccounts: async () => (++listCall === 1 ? [account('target')] : [failed]),
      refreshQuota: async () => {
        throw new Error('OPENCODE_GO_USAGE_AUTHENTICATION');
      },
    }),
  );
  await store.getState().fetchAccounts();

  await assert.rejects(
    store.getState().refreshQuota('target'),
    /OPENCODE_GO_USAGE_AUTHENTICATION/,
  );

  assert.deepEqual(store.getState().accounts, [failed]);
});
