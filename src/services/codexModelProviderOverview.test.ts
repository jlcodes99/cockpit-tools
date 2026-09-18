import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import {
  addApiKeyToCodexModelProvider, invalidateCodexModelProviderCache,
  listCodexAccountsWithModelProviders, type CodexModelProvider,
} from './codexModelProviderService';

test('provider overview migration persists, coalesces fetches, and imports subsequently saved keys', async () => {
  let disk: CodexModelProvider[] = [
    { id: 'deepseek', name: 'DeepSeek API', baseUrl: 'https://api.deepseek.com',
      wireApi: 'chat_completions', supportsWebsockets: false, createdAt: 1, updatedAt: 1,
      apiKeys: [{ id: 'key-1', name: 'Primary', apiKey: 'test-deepseek', createdAt: 1, updatedAt: 1 }] },
    { id: 'openai', name: 'OpenAI Official', baseUrl: 'https://api.openai.com/v1',
      wireApi: 'responses', supportsWebsockets: false, createdAt: 1, updatedAt: 1,
      apiKeys: [{ id: 'key-2', name: '', apiKey: 'test-openai', createdAt: 1, updatedAt: 1 }] },
  ];
  let accounts: CodexAccount[] = [];
  const creates: Record<string, unknown>[] = [];
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    __TAURI_INTERNALS__: { invoke: async (command: string, args: Record<string, unknown>) => {
      if (command === 'list_codex_accounts') return accounts.map((item) => ({ ...item }));
      if (command === 'load_codex_model_providers') return JSON.stringify(disk);
      if (command === 'save_codex_model_providers') { disk = JSON.parse(args.data as string); return; }
      if (command === 'add_codex_account_with_api_key') {
        creates.push(args);
        const item: CodexAccount = { id: `account-${creates.length}`, email: '', auth_mode: 'apikey',
          openai_api_key: args.apiKey as string, account_name: args.accountName as string,
          tokens: { access_token: '', id_token: '' }, created_at: 1, last_used: 1 };
        accounts.push(item);
        return item;
      }
      throw new Error(`Unexpected command: ${command}`);
    } },
  } });
  invalidateCodexModelProviderCache();
  try {
    const first = listCodexAccountsWithModelProviders();
    assert.equal(first, listCodexAccountsWithModelProviders());
    assert.equal((await first).accounts.length, 2);
    assert.equal(creates[0].apiWireApi, 'chat_completions');
    assert.equal(creates[0].apiProviderId, 'deepseek');
    assert.match(String(creates[0].accountName), /Primary/);
    assert.equal(creates[1].apiProviderMode, 'openai_builtin');
    assert.ok(disk.every((p) => p.apiKeys[0].overviewAccountId));

    invalidateCodexModelProviderCache();
    await listCodexAccountsWithModelProviders();
    assert.equal(creates.length, 2, 'restart must not duplicate cards');

    accounts = accounts.filter((item) => item.id !== 'account-1');
    invalidateCodexModelProviderCache();
    await listCodexAccountsWithModelProviders();
    assert.equal(creates.length, 2, 'deleted card must stay deleted after restart');

    await addApiKeyToCodexModelProvider('openai', 'test-second-openai', 'Second');
    const refreshed = await listCodexAccountsWithModelProviders();
    assert.equal(creates.length, 3);
    assert.equal(refreshed.accounts.length, 2);
    assert.deepEqual(refreshed.failedProviders, []);
  } finally {
    invalidateCodexModelProviderCache();
    if (descriptor) Object.defineProperty(globalThis, 'window', descriptor);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
