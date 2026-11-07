import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { CodexModelProvider } from './codexModelProviderService';
import type { CodexAccount } from '../types/codex';
import { buildCodexModelProviderAccountSnapshot } from '../utils/codexModelProviderAccountSync';

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
function harness(initial: CodexModelProvider | CodexModelProvider[] = provider()) {
  let disk = JSON.stringify(Array.isArray(initial) ? initial : [initial]);
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

test('account reconciliation creates missing official, built-in and imported providers once', async () => {
  const h = harness([]);
  const official = { ...account('official-key'), api_provider_id: 'openai_official', api_provider_mode: 'openai_builtin' as const, api_base_url: undefined };
  const explicit = { ...official, id: 'official-2', openai_api_key: 'other-official-key', api_base_url: 'https://api.openai.com/' };
  const cockpit = { ...account('cockpit-key'), api_provider_id: 'cockpit_api', api_base_url: 'https://chongcodex.cn/v1' };
  const imported = { ...account('imported-key'), api_provider_id: 'missing-provider', api_model_catalog: ['model-a'],
    api_model_context_windows: { 'model-a': 100000 }, api_supports_vision: true, api_model_vision_support: { 'model-a': true } };
  const source = [official, explicit, cockpit, imported];
  const before = structuredClone(source);
  const result = await h.service.mergeCodexModelProviderApiKeysFromAccounts(source);
  assert.equal(result.length, 3);
  const openai = result.find(item => item.baseUrl === 'https://api.openai.com/v1')!;
  assert.equal(openai.name, 'OpenAI Official');
  assert.deepEqual(Array.from(openai.apiKeys, item => item.apiKey), ['official-key', 'other-official-key']);
  const relay = result.find(item => item.baseUrl === imported.api_base_url)!;
  assert.deepEqual(Array.from(relay.apiKeys[0].modelCatalog!), ['model-a']);
  assert.equal(relay.apiKeys[0].modelContextWindows?.['model-a'], 100000);
  assert.equal(relay.apiKeys[0].modelCapabilities?.['model-a'].supportsVision, true);
  const writes = h.writeCount();
  h.service.invalidateCodexModelProviderCache();
  await h.service.mergeCodexModelProviderApiKeysFromAccounts(source);
  assert.equal(h.writeCount(), writes);
  assert.deepEqual(source, before, 'backfill must not rewrite account credentials or routes');
});

test('missing keys and service credential references do not create providers', async () => {
  const h = harness([]);
  const result = await h.service.mergeCodexModelProviderApiKeysFromAccounts([
    { ...account(), auth_mode: 'oauth' },
    { ...account(), openai_api_key: '' },
    { ...account(), api_provider_id: 'cockpit_chatgpt_plan' },
    { ...account(), upstream_grok_account_id: 'source-oauth' },
    { ...account(), api_base_url: '', api_provider_mode: 'custom' },
  ]);
  assert.equal(result.length, 0);
  assert.equal(h.writeCount(), 0);
});

test('a stale provider ID cannot copy a credential to the wrong endpoint', async () => {
  const original = { ...provider(), name: 'Preserve name', boundInstanceId: 'instance-a', supportsWebsockets: true };
  const h = harness(original);
  const result = await h.service.mergeCodexModelProviderApiKeysFromAccounts([
    { ...account('other-endpoint-key'), api_base_url: 'https://other.example/v1' },
  ]);
  const existing = result.find(item => item.id === original.id)!;
  assert.equal(existing.name, original.name);
  assert.equal(existing.boundInstanceId, original.boundInstanceId);
  assert.equal(existing.supportsWebsockets, true);
  assert.deepEqual(Array.from(existing.apiKeys, item => item.apiKey), ['sk-first', 'sk-second']);
  assert.equal(result.find(item => item.baseUrl === 'https://other.example/v1')?.apiKeys[0].apiKey, 'other-endpoint-key');
});

test('provider removal survives refresh and restart without retaining deleted secrets', async () => {
  const h = harness();
  await h.service.deleteCodexModelProvider('provider');
  assert.equal((await h.service.listCodexModelProviders()).length, 0);
  assert.ok(!JSON.stringify(h.stored()).includes('sk-first'));
  assert.ok(!JSON.stringify(h.stored()).includes('sk-second'));
  h.service.invalidateCodexModelProviderCache();
  assert.equal((await h.service.mergeCodexModelProviderApiKeysFromAccounts([account()])).length, 0);
  const restored = await h.service.createCodexModelProvider({ name: 'Restored', baseUrl: 'https://relay.example/v1', initialApiKey: 'explicit-new-key' });
  assert.equal((await h.service.listCodexModelProviders())[0].id, restored.id);
  assert.equal(h.stored().length, 1);
});

test('concurrent removal wins over automatic backfill and failed removal remains retryable', async () => {
  const h = harness();
  h.failWrite(new Error('disk full'));
  await assert.rejects(h.service.deleteCodexModelProvider('provider'), /disk full/);
  assert.equal((await h.service.listCodexModelProviders()).length, 1);
  h.failWrite(null);
  const gate = h.gateWrite();
  const remove = h.service.deleteCodexModelProvider('provider');
  await settlePromises();
  const merge = h.service.mergeCodexModelProviderApiKeysFromAccounts([account()]);
  gate.resolve();
  await Promise.all([remove, merge]);
  assert.equal((await h.service.listCodexModelProviders()).length, 0);
});

test('explicit account imports restore a removed provider while background refresh respects its removal', async () => {
  const h = harness();
  await h.service.deleteCodexModelProvider('provider');
  h.service.invalidateCodexModelProviderCache();
  assert.equal((await h.service.mergeCodexModelProviderApiKeysFromAccounts([account()])).length, 0);
  const restored = await h.service.mergeCodexModelProviderApiKeysFromAccounts([account()], { explicitImport: true });
  assert.equal(restored.length, 1);
  assert.deepEqual(Array.from(restored[0].apiKeys, key => key.apiKey), ['sk-first']);
  assert.equal(h.stored().length, 1);
  assert.equal(h.stored()[0].deletedAt, undefined);
});

test('explicit import restores only the selected removed key and preserves other exclusions', async () => {
  const h = harness();
  await h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  await h.service.removeApiKeyFromCodexModelProvider('provider', 'key-2');
  await h.service.mergeCodexModelProviderApiKeysFromAccounts([account()], { explicitImport: true });
  h.service.invalidateCodexModelProviderCache();
  const result = await h.service.mergeCodexModelProviderApiKeysFromAccounts([account(), account('sk-second')]);
  assert.deepEqual(Array.from(result[0].apiKeys, key => key.apiKey), ['sk-first']);
  assert.equal(h.stored()[0].excludedApiKeyHashes?.length, 1);
});

test('failed provider backfill preserves existing state and can be retried', async () => {
  const h = harness();
  const imported = { ...account('new-key'), api_base_url: 'https://new.example/v1' };
  await h.service.listCodexModelProviders();
  const before = h.stored();
  h.failWrite(new Error('disk full'));
  await assert.rejects(h.service.mergeCodexModelProviderApiKeysFromAccounts([imported]), /disk full/);
  assert.deepEqual(h.stored(), before);
  assert.equal((await h.service.listCodexModelProviders()).length, 1);
  h.failWrite(null);
  assert.equal((await h.service.mergeCodexModelProviderApiKeysFromAccounts([imported])).length, 2);
});

test('editing a provider to an explicitly removed endpoint replaces its removal marker', async () => {
  const other = { ...provider(), id: 'other', baseUrl: 'https://other.example/v1' };
  const h = harness([provider(), other]);
  await h.service.deleteCodexModelProvider('other');
  await h.service.updateCodexModelProvider('provider', { baseUrl: other.baseUrl });
  assert.equal(h.stored().length, 1);
  const restored = await h.service.mergeCodexModelProviderApiKeysFromAccounts([
    { ...account('new-key'), api_base_url: other.baseUrl },
  ]);
  assert.equal(restored.length, 1);
  assert.equal(restored[0].apiKeys.length, 3);
});

test('Anthropic protocol, authentication and manual model catalogs survive save and reload', async () => {
  const h = harness();
  const created = await h.service.createCodexModelProvider({
    name: 'Claude gateway', baseUrl: 'https://anthropic.example.invalid',
    wireApi: 'anthropic', authScheme: 'bearer', supportsWebsockets: true,
    modelCatalog: ['claude-sonnet-4-6', 'claude-opus-4-6'], initialApiKey: 'fixture-anthropic',
  });
  assert.equal(created.wireApi, 'anthropic');
  assert.equal(created.authScheme, 'bearer');
  assert.equal(created.supportsWebsockets, false);
  h.service.invalidateCodexModelProviderCache();
  const reloaded = (await h.service.listCodexModelProviders()).find(item => item.id === created.id)!;
  assert.equal(reloaded.wireApi, 'anthropic');
  assert.equal(reloaded.authScheme, 'bearer');
  assert.deepEqual(Array.from(reloaded.modelCatalog!), ['claude-sonnet-4-6', 'claude-opus-4-6']);
  await h.service.updateCodexModelProvider(created.id, { authScheme: 'x-api-key' });
  assert.equal(h.stored().find(item => item.id === created.id)?.authScheme, 'x-api-key');
});

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

test('plan service references never become provider API keys even when their upstream URL matches', async () => {
  const openai = { ...provider(), baseUrl: 'https://api.openai.com/v1' };
  const h = harness(openai);
  const plan = { ...account('chatgpt-plan-managed'), id: 'chatgpt_plan_reference',
    api_provider_id: 'cockpit_chatgpt_plan', api_base_url: openai.baseUrl };
  const merged = await h.service.mergeCodexModelProviderApiKeysFromAccounts([plan]);
  assert.equal(h.writeCount(), 0);
  assert.deepEqual(Array.from(merged[0].apiKeys, (entry) => entry.apiKey), ['sk-first', 'sk-second']);
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

test('loading capability defaults preserves credential vision policies and is idempotent', async () => {
  const initial = provider();
  initial.modelCatalog = ['Qwen-VL'];
  initial.supportsVision = false;
  initial.modelCapabilities = {};
  initial.apiKeys[0].supportsVision = true;
  initial.apiKeys[1].supportsVision = false;
  initial.apiKeys[1].modelCapabilities = {};
  initial.apiKeys.push({ id: 'legacy', name: 'Legacy', apiKey: 'sk-legacy', createdAt: 1, updatedAt: 1 });
  const h = harness(initial);

  const migrated = (await h.service.listCodexModelProviders())[0];
  const first = buildCodexModelProviderAccountSnapshot(migrated, '', 'sk-first');
  const second = buildCodexModelProviderAccountSnapshot(migrated, '', 'sk-second');
  assert.deepEqual(first.apiModelVisionSupport, { 'qwen-vl': true });
  assert.deepEqual(second.apiModelVisionSupport, { 'Qwen-VL': false });
  assert.deepEqual(h.stored()[0].modelCapabilities, { 'qwen-vl': { supportsVision: false } });
  assert.deepEqual(h.stored()[0].apiKeys[1].modelCapabilities, {});
  assert.equal(h.stored()[0].apiKeys[2].modelCapabilities, undefined);
  assert.equal(h.writeCount(), 1);

  h.service.invalidateCodexModelProviderCache();
  await h.service.listCodexModelProviders();
  assert.equal(h.writeCount(), 1);
  assert.deepEqual(h.stored()[0].apiKeys[0].modelCapabilities, { 'qwen-vl': { supportsVision: true } });
});

test('legacy provider expansion preserves an explicit credential opt-out and model override', async () => {
  const initial = provider();
  initial.modelCatalog = ['Qwen-VL', 'text-only'];
  initial.supportsVision = true;
  initial.apiKeys[0].supportsVision = false;
  initial.apiKeys[0].modelCapabilities = {};
  initial.apiKeys[1].modelCapabilities = { 'Qwen-VL': { supportsVision: false } };
  const h = harness(initial);

  const migrated = (await h.service.listCodexModelProviders())[0];
  assert.deepEqual(buildCodexModelProviderAccountSnapshot(migrated, '', 'sk-first').apiModelVisionSupport,
    { 'Qwen-VL': false, 'text-only': false });
  assert.deepEqual(buildCodexModelProviderAccountSnapshot(migrated, '', 'sk-second').apiModelVisionSupport,
    { 'qwen-vl': false, 'text-only': true });
  assert.deepEqual(h.stored()[0].modelCapabilities,
    { 'qwen-vl': { supportsVision: true }, 'text-only': { supportsVision: true } });
});

test('the shared editor persists a selected key model configuration without overwriting provider defaults or other keys', async () => {
  const original = { ...provider(), modelCatalog: ['shared-model'] };
  const h = harness(original);
  await h.service.updateCodexModelProvider('provider', {
    wireApi: 'anthropic', authScheme: 'bearer',
    keyModelConfig: { apiKeyId: 'key-1', config: { modelCatalog: ['key-model'], modelContextWindows: { 'key-model': 200000 }, supportsVision: false, visionRoutingModel: null } },
  });
  h.service.invalidateCodexModelProviderCache();
  const saved = (await h.service.listCodexModelProviders())[0];
  assert.deepEqual(Array.from(saved.modelCatalog!), ['shared-model']);
  assert.deepEqual(Array.from(saved.apiKeys[0].modelCatalog!), ['key-model']);
  assert.equal(saved.apiKeys[0].modelContextWindows?.['key-model'], 200000);
  assert.equal(saved.apiKeys[0].visionRoutingModel, null);
  assert.equal(saved.apiKeys[1].modelCatalog, undefined);
  assert.equal(saved.authScheme, 'bearer');
});

test('invalid or failed scoped writes leave both provider and key settings unchanged', async () => {
  const h = harness(); await h.service.listCodexModelProviders(); const before = h.stored();
  await assert.rejects(h.service.updateCodexModelProvider('provider', { wireApi: 'anthropic', keyModelConfig: { apiKeyId: 'missing', config: { modelCatalog: ['new'] } } }), /API_KEY_NOT_FOUND/);
  assert.deepEqual(h.stored(), before);
  h.failWrite(new Error('disk full'));
  await assert.rejects(h.service.updateCodexModelProvider('provider', { wireApi: 'anthropic', keyModelConfig: { apiKeyId: 'key-1', config: { modelCatalog: ['new'] } } }), /disk full/);
  assert.deepEqual(h.stored(), before);
  assert.equal((await h.service.listCodexModelProviders())[0].wireApi, 'responses');
});

test('OpenAI query credentials persist independently and never enter account snapshots', async () => {
  const official = { ...provider(), baseUrl: 'https://api.openai.com/v1' };
  const h = harness(official);
  const saved = await h.service.updateCodexOpenAIUsageKey('provider', 'key-1', '  admin-one  ', { apiKey: 'sk-first' });
  assert.equal(saved.apiKeys[0].apiKey, 'sk-first');
  assert.equal(saved.apiKeys[0].usageApiKey, 'admin-one');
  assert.equal(saved.apiKeys[0].usageKeyVersion, 1);
  assert.equal(saved.apiKeys[1].usageApiKey, undefined);
  assert.ok(!JSON.stringify(buildCodexModelProviderAccountSnapshot(saved, '', 'sk-first')).includes('admin-one'));
  h.service.invalidateCodexModelProviderCache();
  const reloaded = (await h.service.listCodexModelProviders())[0];
  assert.equal(reloaded.apiKeys[0].usageApiKey, 'admin-one');
  await h.service.mergeCodexModelProviderApiKeysFromAccounts([{ ...account(), api_base_url: official.baseUrl }]);
  assert.equal(h.stored()[0].apiKeys[0].usageApiKey, 'admin-one');
  await h.service.updateCodexOpenAIUsageKey('provider', 'key-1', '', { apiKey: 'sk-first', usageApiKey: 'admin-one' });
  assert.equal(h.stored()[0].apiKeys[0].usageApiKey, undefined);
  assert.equal(h.stored()[0].apiKeys[0].apiKey, 'sk-first');
  assert.equal(h.stored()[0].apiKeys[0].usageKeyVersion, 2);
});

test('failed and stale query-key saves preserve the current credentials', async () => {
  const h = harness({ ...provider(), baseUrl: 'https://api.openai.com/v1' });
  h.failWrite(new Error('disk full'));
  await assert.rejects(h.service.updateCodexOpenAIUsageKey('provider', 'key-1', 'admin-one', { apiKey: 'sk-first' }), /disk full/);
  assert.equal((await h.service.listCodexModelProviders())[0].apiKeys[0].usageApiKey, undefined);
  h.failWrite(null);
  await h.service.updateCodexOpenAIUsageKey('provider', 'key-1', 'admin-new', { apiKey: 'sk-first' });
  await assert.rejects(h.service.updateCodexOpenAIUsageKey('provider', 'key-1', 'admin-stale', { apiKey: 'sk-first' }), /OPENAI_USAGE_CONFIG_CHANGED/);
  await assert.rejects(h.service.updateCodexOpenAIUsageKey('provider', 'key-1', '', { apiKey: 'old-model-key', usageApiKey: 'admin-new' }), /OPENAI_USAGE_CONFIG_CHANGED/);
  assert.equal(h.stored()[0].apiKeys[0].usageApiKey, 'admin-new');
  await h.service.removeApiKeyFromCodexModelProvider('provider', 'key-1');
  await assert.rejects(h.service.updateCodexOpenAIUsageKey('provider', 'key-1', 'admin-stale', { apiKey: 'sk-first', usageApiKey: 'admin-new' }), /OPENAI_USAGE_CONFIG_CHANGED/);
  assert.equal(h.stored()[0].apiKeys.length, 1);
});

test('creating an official account optionally stores its query key without requiring one', async () => {
  const h = harness([]);
  const saved = await h.service.upsertCodexModelProviderFromCredential({ apiBaseUrl: 'https://api.openai.com/v1', apiKey: 'model-one', usageApiKey: 'admin-one' });
  await h.service.upsertCodexModelProviderFromCredential({ apiBaseUrl: saved.baseUrl, apiKey: 'model-two' });
  const keys = h.stored()[0].apiKeys;
  assert.equal(keys[0].usageApiKey, 'admin-one');
  assert.equal(keys[1].usageApiKey, undefined);
  await assert.rejects(h.service.queryCodexModelProviderUsage({ baseUrl: saved.baseUrl, apiKey: 'model-two' }), /OPENAI_USAGE_NOT_CONFIGURED/);
});

test('concurrent organization queries share one request while other providers retain their model key', async () => {
  const gate = deferred<unknown>();
  const requests: any[] = [];
  const service = loadHookModule(new URL('./codexModelProviderService.ts', import.meta.url), {
    './modelProviderUsageService': { queryModelProviderUsage(input: unknown) { requests.push(input); return gate.promise; } },
  }, { URL, TextEncoder }).exports as typeof import('./codexModelProviderService');
  const a = service.queryCodexModelProviderUsage({ baseUrl: 'https://api.openai.com/v1', apiKey: 'model-a', usageApiKey: 'admin' });
  const b = service.queryCodexModelProviderUsage({ baseUrl: 'https://api.openai.com/v1', apiKey: 'model-b', usageApiKey: 'admin' });
  assert.equal(requests.length, 1);
  const relay = service.queryCodexModelProviderUsage({ baseUrl: 'https://relay.example/v1', apiKey: 'relay-model' });
  assert.equal(requests.length, 2);
  assert.equal(requests[1].apiKey, 'relay-model');
  assert.equal(requests[1].usageApiKey, undefined);
  gate.resolve({}); await Promise.all([a, b, relay]);
});
