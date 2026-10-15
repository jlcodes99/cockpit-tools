import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { CodexModelProvider } from './codexModelProviderService';
import type { CodexAccount } from '../types/codex';

function provider(): CodexModelProvider {
  return {
    id: 'provider', name: 'Relay', baseUrl: 'https://relay.example/v1',
    wireApi: 'responses', supportsWebsockets: false,
    apiKeys: [{ id: 'key-1', name: 'First', apiKey: 'sk-first', createdAt: 1, updatedAt: 1 },
      { id: 'key-2', name: 'Second', apiKey: 'sk-second', createdAt: 1, updatedAt: 1 }],
    createdAt: 1, updatedAt: 1,
  };
}
function account(key = 'sk-first'): CodexAccount {
  return {
    id: 'account', auth_mode: 'apikey', email: 'api@example.com', openai_api_key: key,
    api_provider_id: 'provider', api_base_url: 'https://relay.example/v1',
    tokens: { access_token: '', id_token: '' }, created_at: 1, last_used: 1,
  };
}
function harness() {
  let disk = JSON.stringify([provider()]);
  let writeError: Error | null = null;
  let readError: Error | null = null;
  let gate: ReturnType<typeof deferred<void>> | null = null;
  let writes = 0;
  const service = loadHookModule(new URL('./codexModelProviderService.ts', import.meta.url), {
    '@tauri-apps/api/core': { async invoke(command: string, args?: { data: string }) {
      if (command === 'load_codex_model_providers') {
        if (readError) throw readError;
        return disk;
      }
      assert.equal(command, 'save_codex_model_providers');
      writes += 1;
      const currentGate = gate; gate = null;
      if (currentGate) await currentGate.promise;
      if (writeError) throw writeError;
      disk = args!.data;
    } },
    './modelProviderUsageService': { queryModelProviderUsage() { throw new Error('unexpected network'); } },
  }, { URL, TextEncoder }).exports as typeof import('./codexModelProviderService');
  return { service, stored: () => JSON.parse(disk) as CodexModelProvider[], writeCount: () => writes,
    failWrite(error: Error | null) { writeError = error; }, failRead(error: Error | null) { readError = error; },
    gateWrite() { return gate = deferred<void>(); },
  };
}

test('deleted provider keys stay removed after account reconciliation and disk reload', async () => {
  const h = harness();
  await h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  await h.service.mergeCodexModelProviderApiKeysFromAccounts([account()]);
  assert.deepEqual(h.stored()[0].apiKeys.map((key) => key.apiKey), ['sk-second']);
  assert.match(h.stored()[0].excludedApiKeyHashes![0], /^[a-f0-9]{64}$/);
  h.service.invalidateCodexModelProviderCache();
  const reloaded = await h.service.mergeCodexModelProviderApiKeysFromAccounts([account()]);
  assert.deepEqual(Array.from(reloaded[0].apiKeys, (key) => key.apiKey), ['sk-second']);
});

test('explicit re-add clears the exclusion and new account keys still import', async () => {
  const h = harness();
  await h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  await h.service.mergeCodexModelProviderApiKeysFromAccounts([account(), account('sk-third')]);
  assert.deepEqual(h.stored()[0].apiKeys.map((key) => key.apiKey), ['sk-second', 'sk-third']);
  await h.service.addApiKeyToCodexModelProvider('provider', ' sk-first ', 'Restored');
  assert.equal(h.stored()[0].apiKeys.length, 3);
  assert.deepEqual(h.stored()[0].excludedApiKeyHashes, []);
});

test('background reconciliation cannot overwrite a concurrent deletion or metadata edit', async () => {
  const h = harness();
  const gate = h.gateWrite();
  const remove = h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  await settlePromises();
  const merge = h.service.mergeCodexModelProviderApiKeysFromAccounts([account()]);
  const edit = h.service.updateCodexModelProvider('provider', { wireApi: 'chat_completions' });
  gate.resolve();
  await Promise.all([remove, merge, edit]);
  assert.deepEqual(h.stored()[0].apiKeys.map((key) => key.apiKey), ['sk-second']);
  assert.equal(h.stored()[0].wireApi, 'chat_completions');
});

test('concurrent key deletions survive stale account reconciliation and allow restoring only one key', async () => {
  const h = harness();
  const gate = h.gateWrite();
  const first = h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  await settlePromises();
  const second = h.service.removeApiKeyFromCodexModelProvider('provider', 'key-2');
  const merge = h.service.mergeCodexModelProviderApiKeysFromAccounts([account(), account('sk-second')]);
  gate.resolve();
  await Promise.all([first, second, merge]);
  h.service.invalidateCodexModelProviderCache();
  assert.equal((await h.service.listCodexModelProviders())[0].apiKeys.length, 0);
  assert.equal(h.stored()[0].excludedApiKeyHashes?.length, 2);
  await h.service.addApiKeyToCodexModelProvider('provider', 'sk-first');
  await h.service.mergeCodexModelProviderApiKeysFromAccounts([account(), account('sk-second')]);
  assert.deepEqual(h.stored()[0].apiKeys.map((key) => key.apiKey), ['sk-first']);
  assert.equal(h.stored()[0].excludedApiKeyHashes?.length, 1);
});

test('failed deletion leaves cache and disk unchanged and remains retryable', async () => {
  const h = harness();
  h.failWrite(new Error('disk full'));
  await assert.rejects(h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1'), /disk full/);
  assert.equal((await h.service.listCodexModelProviders())[0].apiKeys.length, 2);
  assert.equal(h.stored()[0].apiKeys.length, 2);
  h.failWrite(null);
  await h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  assert.equal(h.stored()[0].apiKeys.length, 1);
});

test('a load failure is surfaced instead of replacing existing providers with empty state', async () => {
  const h = harness();
  h.failRead(new Error('unreadable provider store'));
  await assert.rejects(h.service.listCodexModelProviders(), /unreadable provider store/);
  await assert.rejects(h.service.mergeCodexModelProviderApiKeysFromAccounts([account()]), /unreadable provider store/);
  assert.equal(h.writeCount(), 0);
  h.failRead(null);
  assert.equal((await h.service.listCodexModelProviders())[0].apiKeys.length, 2);
});

test('same URL keys retain independent catalogs, context and vision settings across reload and key edits', async () => {
  const h = harness();
  await h.service.updateCodexModelProvider('provider', {
    modelCatalog: ['legacy-model'], modelContextWindows: { 'legacy-model': 128000 },
  });
  await h.service.upsertCodexModelProviderFromCredential({
    providerId: 'provider', apiBaseUrl: 'https://relay.example/v1', apiKey: 'sk-first',
    modelCatalog: ['model-a'], modelContextWindows: { 'model-a': 256000 },
    supportsVision: true, modelCapabilities: { 'model-a': { supportsVision: true } },
    visionRoutingModel: 'model-a',
  });
  await h.service.upsertCodexModelProviderFromCredential({
    providerId: 'provider', apiBaseUrl: 'https://relay.example/v1', apiKey: 'sk-second',
    modelCatalog: ['model-b'], modelContextWindows: {}, supportsVision: false,
    modelCapabilities: {}, visionRoutingModel: null,
  });
  h.service.invalidateCodexModelProviderCache();
  const saved = (await h.service.listCodexModelProviders())[0];
  assert.deepEqual(Array.from(saved.modelCatalog!), ['legacy-model']);
  assert.deepEqual(Array.from(saved.apiKeys[0].modelCatalog!), ['model-a']);
  assert.equal(saved.apiKeys[0].modelContextWindows?.['model-a'], 256000);
  assert.equal(saved.apiKeys[0].modelCapabilities?.['model-a'].supportsVision, true);
  assert.deepEqual(Array.from(saved.apiKeys[1].modelCatalog!), ['model-b']);
  assert.equal(saved.apiKeys[1].supportsVision, false);
  assert.equal(saved.apiKeys[1].visionRoutingModel, null);
  await h.service.updateApiKeyOnCodexModelProvider('provider', 'key-1', 'sk-first-rotated');
  assert.deepEqual(h.stored()[0].apiKeys[0].modelCatalog, ['model-a']);
  assert.deepEqual(h.stored()[0].apiKeys[1].modelCatalog, ['model-b']);
  saved.apiKeys[0].modelCatalog!.push('mutated');
  saved.apiKeys[0].modelCapabilities!['model-a'].supportsVision = false;
  const cached = (await h.service.listCodexModelProviders())[0];
  assert.deepEqual(Array.from(cached.apiKeys[0].modelCatalog!), ['model-a']);
  assert.equal(cached.apiKeys[0].modelCapabilities!['model-a'].supportsVision, true);
});

test('legacy keys inherit provider settings without an automatic migration; explicit empty catalog persists', async () => {
  const h = harness();
  await h.service.updateCodexModelProvider('provider', { modelCatalog: ['legacy-model'] });
  h.service.invalidateCodexModelProviderCache();
  const legacy = (await h.service.listCodexModelProviders())[0];
  assert.equal(legacy.apiKeys[0].modelCatalog, undefined);
  assert.equal(legacy.apiKeys[1].modelCatalog, undefined);
  await h.service.upsertCodexModelProviderFromCredential({
    providerId: 'provider', apiBaseUrl: 'https://relay.example/v1', apiKey: 'sk-first', modelCatalog: [],
  });
  h.service.invalidateCodexModelProviderCache();
  const saved = (await h.service.listCodexModelProviders())[0];
  assert.deepEqual(Array.from(saved.apiKeys[0].modelCatalog!), []);
  assert.equal(saved.apiKeys[1].modelCatalog, undefined);
  assert.deepEqual(Array.from(saved.modelCatalog!), ['legacy-model']);
});
