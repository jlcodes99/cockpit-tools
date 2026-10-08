import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { resolveCodexApiProviderPresetId, CODEX_API_PROVIDER_CUSTOM_ID } from '../../utils/codexProviderPresets';
import { isCodexApiKeyAccount } from '../../types/codex';
import type { CodexAccount } from '../../types/codex';

// Execute the actual save handler with isolated IPC, rather than writing real account files.
const source = readFileSync(new URL('./CodexModelProviderManager.tsx', import.meta.url), 'utf8');
const start = source.indexOf('  const handleSaveApiKeyEdit =');
const end = source.indexOf('  }, [', start);
assert.ok(start > 0 && end > start);
const handler = `${source.slice(start, end)}  }, []); globalThis.save = handleSaveApiKeyEdit;`;

test('editing only a provider key retains each linked account mode and its model settings', async () => {
  const provider = {
    id: 'provider', name: 'Official endpoint', baseUrl: 'https://api.openai.com/v1',
    modelCatalog: ['provider-default'], wireApi: 'responses', supportsWebsockets: true,
    apiKeys: [{ id: 'key', apiKey: 'new-key', name: 'Key' }],
  };
  const account = {
    email: 'api@example.com', tokens: { access_token: '', id_token: '' }, created_at: 1, last_used: 1,
    id: 'old-id', auth_mode: 'apikey', openai_api_key: 'old-key', api_base_url: provider.baseUrl,
    api_provider_mode: 'custom', api_model_catalog: ['account-model'], api_wire_api: 'responses',
    api_sync_model_catalog_to_codex: true, api_supports_vision: true,
    api_model_vision_support: { 'account-model': true }, api_vision_routing_model: 'account-model',
    api_model_context_windows: { 'account-model': 256000 }, api_supports_websockets: false,
    account_name: 'Original',
  } as CodexAccount;
  const updates: unknown[][] = [];
  const context: Record<string, any> = {
    useCallback: (fn: unknown) => fn, saving: false, providers: [provider], accounts: [account],
    editingApiKey: { providerId: 'provider', apiKeyId: 'key', apiKey: 'new-key', originalApiKey: 'old-key' },
    isCodexApiKeyAccount, resolveCodexApiProviderPresetId, CODEX_API_PROVIDER_CUSTOM_ID,
    normalizeCodexModelProviderBaseUrl: (url: string) => url.replace(/\/+$/, '').toLowerCase(),
    resolveProviderWireApi: () => 'responses',
    updateApiKeyOnCodexModelProvider: async () => provider,
    updateCodexApiKeyCredentials: async (...args: unknown[]) => { updates.push(args); },
    reloadProviders: async () => {}, emitAccountsChanged: async () => {},
    setSaving() {}, setNotice() {}, setEditingApiKey() {}, t: (key: string) => key,
  };
  vm.runInNewContext(ts.transpileModule(handler, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  await context.save();
  assert.equal(updates.length, 1);
  assert.equal(updates[0][1], 'new-key');
  assert.equal(updates[0][3], 'custom', 'official URL must not rewrite an existing custom mode');
  assert.deepEqual(updates[0][6], ['account-model']);
  assert.deepEqual(updates[0][8], { 'account-model': true });
  assert.equal(updates[0][9], 'account-model');
  assert.equal(updates[0][11], false);
  assert.equal(updates[0][12], true);
  assert.equal(updates[0][13], 'Original');
  assert.deepEqual(updates[0][14], { 'account-model': 256000 });
  account.api_provider_mode = 'openai_builtin';
  await context.save();
  assert.equal(updates[1][3], 'openai_builtin');
});
