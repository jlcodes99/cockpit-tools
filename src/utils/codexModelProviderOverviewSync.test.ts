import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { CodexModelProvider } from '../services/codexModelProviderService';
import { reconcileCodexModelProviderOverview } from './codexModelProviderOverviewSync';

function provider(id: string, secret = `key-${id}`): CodexModelProvider {
  return { id, name: id, baseUrl: `https://${id}.example/v1`, supportsWebsockets: false,
    createdAt: 1, updatedAt: 1,
    apiKeys: [{ id: `key-${id}`, name: 'Primary', apiKey: secret, createdAt: 1, updatedAt: 1 }],
  };
}
function account(id: string, secret: string): CodexAccount {
  return { id, email: '', auth_mode: 'apikey', openai_api_key: secret,
    tokens: { access_token: '', id_token: '' }, created_at: 1, last_used: 1 };
}

test('imports every missing key, preserves existing cards, and is idempotent', async () => {
  const providers = [provider('deepseek'), provider('ainipy'), provider('apikeyfun')];
  providers[0].apiKeys.push({ ...providers[0].apiKeys[0], id: 'second', apiKey: 'second-secret' });
  const existing = account('existing', 'key-apikeyfun');
  let created = 0;
  const operations = {
    createAccount: async (_: CodexModelProvider, key: { apiKey: string }) => account(`new-${++created}`, key.apiKey),
    rememberAccount: async (p: CodexModelProvider, key: { id: string }, id: string) => {
      p.apiKeys.find((item) => item.id === key.id)!.overviewAccountId = id;
    },
  };
  const first = await reconcileCodexModelProviderOverview(providers, [existing], operations);
  assert.equal(first.accounts.length, 4);
  assert.equal(first.accounts[0], existing);
  assert.equal(created, 3);
  const second = await reconcileCodexModelProviderOverview(providers, first.accounts, operations);
  assert.equal(second.accounts.length, 4);
  assert.equal(created, 3);
  assert.deepEqual(first.failedProviders, []);
});

test('does not resurrect a deleted card but still imports a newly added key', async () => {
  const p = provider('deleted');
  p.apiKeys[0].overviewAccountId = 'deleted-card';
  let created = 0;
  const result = await reconcileCodexModelProviderOverview([p, provider('new')], [], {
    createAccount: async (_, key) => account(`new-${++created}`, key.apiKey),
    rememberAccount: async () => {},
  });
  assert.equal(created, 1);
  assert.equal(result.accounts[0].openai_api_key, 'key-new');
});

test('reuses a shared key across providers without overwriting the original account', async () => {
  let created = 0;
  const result = await reconcileCodexModelProviderOverview([provider('one', 'shared'), provider('two', ' shared ')], [], {
    createAccount: async (_, key) => account(`new-${++created}`, key.apiKey),
    rememberAccount: async () => {},
  });
  assert.equal(created, 1);
  assert.equal(result.accounts.length, 1);
});

test('a failed import does not hide OAuth accounts or prevent other providers from importing', async () => {
  const oauth = { ...account('oauth', ''), auth_mode: 'oauth' };
  const result = await reconcileCodexModelProviderOverview([provider('broken'), provider('working')], [oauth], {
    createAccount: async (p, key) => {
      if (p.id === 'broken') throw new Error('backend failure with secret');
      return account('created', key.apiKey);
    },
    rememberAccount: async () => {},
  });
  assert.deepEqual(result.failedProviders, ['broken']);
  assert.equal(result.accounts[0], oauth);
  assert.equal(result.accounts.length, 2);
});

test('retries bookkeeping after a failed save without creating the account again', async () => {
  const p = provider('retry');
  const existing = account('already-created', 'key-retry');
  let saves = 0;
  const operations = {
    createAccount: async () => { throw new Error('must reuse existing account'); },
    rememberAccount: async () => { if (++saves === 1) throw new Error('disk full'); },
  };
  assert.deepEqual((await reconcileCodexModelProviderOverview([p], [existing], operations)).failedProviders, ['retry']);
  assert.deepEqual((await reconcileCodexModelProviderOverview([p], [existing], operations)).failedProviders, []);
});
