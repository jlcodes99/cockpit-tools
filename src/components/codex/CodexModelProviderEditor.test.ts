import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import { resolveCodexModelProviderForApiKey } from '../../utils/codexModelProviderKeyConfig';
import { buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider, resolveCodexModelProviderForAccount } from '../../utils/codexModelProviderAccountSync';
import { contextWindowDraftsFromRecord, parseContextWindowDrafts } from '../../utils/codexModelContextWindows';
import { buildProviderModelVisionCapabilities } from '../../utils/codexModelProviderVision';
import { resolveCodexApiProviderPresetId } from '../../utils/codexProviderPresets';
import { isValidProviderApiKeyUrl } from '../../utils/codexProviderApiKeyUrl';
import { resolveCodexProviderCapabilityProfile } from '../../utils/codexProviderGateway';
import { isCodexApiKeyAccount } from '../../types/codex';

const file = ts.createSourceFile('manager.tsx', readFileSync(new URL('./CodexModelProviderManager.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
function bind(name: string, state: Record<string, any>) {
  let expression = '';
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(file) === name) expression = node.initializer!.getText(file);
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) expression = node.getText(file);
    ts.forEachChild(node, visit);
  }
  visit(file); assert.ok(expression, name);
  const code = ts.transpileModule(`globalThis.${name} = ${expression}`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  vm.runInNewContext(code, state);
  return state[name];
}
function harness() {
  let provider: any = { id: 'provider', name: 'Relay', baseUrl: 'https://relay.invalid/v1', wireApi: 'responses', authScheme: 'bearer',
    supportsWebsockets: true, modelCatalog: ['default'], supportsVision: false, createdAt: 1, updatedAt: 1,
    apiKeys: [ { id: 'a', apiKey: 'fixture-a', name: 'A', modelCatalog: ['model-a'], modelContextWindows: { 'model-a': 1000 } },
      { id: 'b', apiKey: 'fixture-b', name: 'B', modelCatalog: ['model-b'], supportsVision: true } ] };
  const accounts = provider.apiKeys.map((key: any) => ({ id: key.id, email: `${key.id}@example.invalid`, auth_mode: 'apikey',
    api_provider_id: provider.id, api_base_url: provider.baseUrl, openai_api_key: key.apiKey, api_sync_model_catalog_to_codex: false }));
  const calls = { patches: [] as any[], bindings: [] as any[][], syncs: [] as any[], creates: 0, closed: 0, queries: 0, reloadMerges: [] as any[] };
  let failSync = false, failWrite = false;
  const state: Record<string, any> = {
    useCallback: (value: any) => value, accounts, providers: [provider], currentEditingProvider: provider,
    resolveCodexModelProviderForApiKey, buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider, resolveCodexModelProviderForAccount,
    contextWindowDraftsFromRecord, parseContextWindowDrafts, buildProviderModelVisionCapabilities,
    resolveCodexApiProviderPresetId, isValidProviderApiKeyUrl, resolveCodexProviderCapabilityProfile,
    normalizeCodexModelProviderBaseUrl: (url: string) => url.trim().replace(/\/+$/, '').toLowerCase(),
    resolveProviderWireApi: (item: any) => item.wireApi ?? 'responses',
    resolveEnableModePreferenceForWireApi: () => 'direct',
    selectedSponsorTemplate: null, accountEditor: null, editorModelKeyId: null, saving: false, showModal: true,
    t: (key: string, args?: any) => args?.error ? `${key}: ${args.error}` : key, parseServiceError: String,
    setSaving(value: boolean) { state.saving = value; }, setShowModal(value: boolean) { state.showModal = value; },
    setFormError(value: any) { state.error = value; }, setNotice() {}, setExistingApiKeySearchQuery() {}, setEditingApiKey() {},
    setSelectedPresetId() {}, setSelectedSponsorTemplateId() {},
    setAccountEditor(value: any) { state.accountEditor = typeof value === 'function' ? value(state.accountEditor) : value; },
    setEditorModelKeyId(value: any) { state.editorModelKeyId = value; },
    setForm(value: any) { state.form = typeof value === 'function' ? value(state.form) : value; },
    setProviders(value: any) { state.providers = typeof value === 'function' ? value(state.providers) : value; state.currentEditingProvider = state.providers.find((p: any) => p.id === state.form.providerId); },
    async updateCodexModelProvider(_id: string, patch: any) {
      if (failWrite) throw new Error('disk full');
      calls.patches.push(patch); const { keyModelConfig, ...shared } = patch; provider = { ...provider, ...shared };
      if (keyModelConfig) provider.apiKeys = provider.apiKeys.map((key: any) => key.id === keyModelConfig.apiKeyId ? { ...key, ...keyModelConfig.config } : key);
      return provider;
    },
    async addApiKeyToCodexModelProvider(_id: string, apiKey: string, name: string) {
      if (!provider.apiKeys.some((key: any) => key.apiKey === apiKey)) provider.apiKeys.push({ id: 'added', apiKey, name });
      return provider;
    },
    async createCodexModelProvider(input: any) {
      calls.creates += 1; provider = { ...input, id: 'created', apiKeys: [{ id: 'created-key', apiKey: input.initialApiKey, name: input.initialApiKeyName }] };
      return provider;
    },
    async updateCodexApiKeyCredentials(...args: any[]) { if (failSync) throw new Error('account save failed'); calls.bindings.push(args); },
    async syncCodexApiKeyProviderAccounts(input: any) { calls.syncs.push(input); return 1; },
    async emitAccountsChanged() {}, async fetchAccounts() {}, async reloadProviders(merge: boolean) { calls.reloadMerges.push(merge); },
    onAccountEditorClose() { calls.closed += 1; },
    async queryCodexModelProviderUsage() { calls.queries += 1; throw new Error('unexpected network'); },
  };
  for (const name of ['EMPTY_FORM', 'parseModelCatalogText', 'visionModelTextFromCapabilities', 'visionModelStatesFromCapabilities']) bind(name, state);
  bind('openEditModal', state)(provider);
  return { state, calls, provider: () => provider, failSync(value: boolean) { failSync = value; }, failWrite(value: boolean) { failWrite = value; },
    openAccount(index = 0) { bind('openEditModal', state)(provider, accounts[index]); },
    openSelectedKey(id: string) { bind('openEditModal', state)(provider, undefined, id); },
    save: () => bind('handleSaveProvider', state)(),
  };
}

test('account entry uses the full provider editor and scopes model changes to its current key', async () => {
  const h = harness(); h.openAccount();
  assert.equal(h.state.form.authScheme, 'bearer'); assert.equal(h.state.editorModelKeyId, 'a');
  h.state.form.modelCatalogText = 'updated-a'; h.state.form.wireApi = 'anthropic'; h.state.form.authScheme = 'x-api-key';
  h.state.form.supportsWebsockets = false;
  await h.save();
  assert.equal(h.calls.patches[0].modelCatalog, undefined);
  assert.deepEqual(Array.from(h.provider().apiKeys[0].modelCatalog), ['updated-a']);
  assert.deepEqual(h.provider().apiKeys[1].modelCatalog, ['model-b']);
  assert.equal(h.calls.bindings[0][10], 'anthropic'); assert.equal(h.calls.bindings[0][15], 'x-api-key');
  assert.deepEqual(Array.from(h.calls.syncs[0].apiModelCatalog), ['model-b']);
  assert.equal(h.calls.closed, 1); assert.equal(h.calls.queries, 0);
  assert.deepEqual(h.calls.reloadMerges, [false], "saving must not re-import stale account snapshots");
});

test('a legacy account catalog survives editing, reopening and enabling its provider key', async () => {
  const h = harness();
  const provider = h.provider();
  const key = provider.apiKeys[0];
  const account = h.state.accounts[0];
  const visionModel = 'deepseek-v4-flash-vision-exp';
  delete key.modelCatalog;
  delete key.modelContextWindows;
  provider.modelCatalog = ['deepseek-v4-pro', 'deepseek-v4-flash'];
  Object.assign(account, {
    api_model_catalog: [visionModel, 'gpt-image-2'],
    api_model_context_windows: { [visionModel]: 128000 },
    api_supports_vision: true,
    api_model_vision_support: { [visionModel]: true },
    api_vision_routing_model: visionModel,
  });
  const enables: any[][] = [];
  const prefetched: unknown[] = [];
  Object.assign(h.state, {
    CODEX_API_PROVIDER_CUSTOM_ID: 'custom',
    isCodexApiKeyAccount,
    enablingProviderId: null,
    setEnablingProviderId() {}, setProviderLaunchPreview() {},
    async addCodexAccountWithApiKey(...args: any[]) { enables.push(args); return account; },
    prefetchCodexLaunchModels(savedAccount: unknown) { prefetched.push(savedAccount); },
    async updateCodexApiKeyBoundOAuthAccount() {}, async reloadCurrentAccount() {},
    async reloadLocalAccessState() {}, async reloadCodexInstances() {},
  });
  await bind('handleEnableProvider', h.state)(provider, key, 'instance', 'Test instance');
  assert.deepEqual(enables[0][5], [visionModel, 'gpt-image-2']);
  assert.deepEqual(enables[0][13], { [visionModel]: 128000 });
  assert.equal(enables[0][8], visionModel);
  assert.deepEqual(prefetched, [account]);

  h.openAccount();
  assert.equal(h.state.form.modelCatalogText, `${visionModel}\ngpt-image-2`);
  h.state.form.modelCatalogText += '\nrelay-extra';
  await h.save();
  assert.deepEqual(Array.from(h.calls.bindings[0][6]), [visionModel, 'gpt-image-2', 'relay-extra']);
  h.openAccount();
  assert.equal(h.state.form.modelCatalogText, `${visionModel}\ngpt-image-2\nrelay-extra`);
});

test('account catalog recovery respects an explicit empty key catalog and changed endpoint', () => {
  const h = harness();
  const provider = h.provider();
  provider.modelCatalog = [];
  provider.apiKeys[0].modelCatalog = [];
  h.state.accounts[0].api_model_catalog = ['legacy-only'];
  h.openAccount();
  assert.equal(h.state.form.modelCatalogText, '');

  delete provider.apiKeys[0].modelCatalog;
  h.state.accounts[0].api_base_url = 'https://other-relay.invalid/v1';
  h.openAccount();
  assert.equal(h.state.form.modelCatalogText, '');
});

test('provider entry saves shared defaults while explicit key catalogs stay independent', async () => {
  const h = harness(); h.state.form.modelCatalogText = 'new-default'; await h.save();
  assert.deepEqual(Array.from(h.provider().modelCatalog), ['new-default']);
  assert.deepEqual(h.provider().apiKeys.map((key: any) => key.modelCatalog), [['model-a'], ['model-b']]);
  assert.equal(h.calls.bindings.length, 0); assert.equal(h.calls.syncs.length, 2); assert.equal(h.calls.queries, 0);
});

test('editing the selected provider key opens and saves that key’s own catalog', async () => {
  const h = harness();
  h.openSelectedKey('b');
  assert.equal(h.state.editorModelKeyId, 'b');
  assert.equal(h.state.form.modelCatalogText, 'model-b');
  assert.equal(h.state.accountEditor, null);
  h.state.form.modelCatalogText = 'updated-b';
  await h.save();
  assert.deepEqual(h.provider().modelCatalog, ['default']);
  assert.deepEqual(h.provider().apiKeys[0].modelCatalog, ['model-a']);
  assert.deepEqual(Array.from(h.provider().apiKeys[1].modelCatalog), ['updated-b']);
});

test('an existing third-party API key can switch to Chat Completions and retain it on reopen', async () => {
  const h = harness();
  h.openAccount();
  assert.equal(h.state.form.wireApi, 'responses');
  Object.assign(h.state.form, {
    wireApi: 'chat_completions',
    supportsWebsockets: false,
    enableModePreference: 'gateway',
  });
  await h.save();

  assert.equal(h.provider().wireApi, 'chat_completions');
  assert.equal(h.calls.bindings[0][10], 'chat_completions');
  h.openAccount();
  assert.equal(h.state.form.wireApi, 'chat_completions');
  assert.equal(h.state.form.supportsWebsockets, false);
});

test('account binding changes use the selected key and retain its Codex model-sync option', async () => {
  const h = harness(); h.openAccount();
  h.state.accountEditor.apiKeyId = 'b'; h.state.accountEditor.syncModels = true;
  bind('selectEditorModelKey', h.state)('b'); await h.save();
  assert.equal(h.calls.bindings[0][1], 'fixture-b'); assert.equal(h.calls.bindings[0][12], true);
  assert.deepEqual(Array.from(h.calls.bindings[0][6]), ['model-b']);
});

test('validation and persistence errors keep the editor open and preserve the draft', async () => {
  const h = harness(); h.openAccount(); h.state.form.name = '';
  await h.save(); assert.match(h.state.error, /nameRequired/); assert.equal(h.calls.patches.length, 0);
  h.state.form.name = 'Edited'; h.failWrite(true); await h.save();
  assert.match(h.state.error, /disk full/); assert.equal(h.state.form.name, 'Edited'); assert.equal(h.state.showModal, true);
  assert.equal(h.calls.closed, 0); assert.equal(h.state.saving, false);
});

test('retry after account persistence failure reuses a newly created provider', async () => {
  const h = harness(); h.openAccount(); h.state.providers = []; h.state.currentEditingProvider = null;
  h.state.form.providerId = null; h.state.form.newApiKey = 'fixture-new'; h.state.accountEditor.apiKeyId = '__new__';
  h.failSync(true); await h.save();
  assert.equal(h.calls.creates, 1); assert.equal(h.state.form.providerId, 'created'); assert.equal(h.state.showModal, true);
  assert.match(h.state.error, /account save failed/);
  h.failSync(false); await h.save();
  assert.equal(h.calls.creates, 1); assert.equal(h.calls.bindings[0][1], 'fixture-new'); assert.equal(h.state.showModal, false);
});

test('a provider backfilled while the account editor is open is reused without replacing its other keys', async () => {
  const h = harness(); h.openAccount(); h.state.currentEditingProvider = null; h.state.form.providerId = null;
  h.state.form.newApiKey = 'fixture-a'; h.state.accountEditor.apiKeyId = '__new__'; h.state.editorModelKeyId = '__new__';
  h.state.form.modelCatalogText = 'updated-a'; await h.save();
  assert.equal(h.calls.creates, 0); assert.equal(h.provider().apiKeys.length, 2);
  assert.deepEqual(h.provider().apiKeys[1].modelCatalog, ['model-b']); assert.equal(h.calls.bindings[0][1], 'fixture-a');
});

test('a missing provider key preserves the account credential instead of selecting another key', () => {
  const h = harness();
  bind('openEditModal', h.state)(h.provider(), { ...h.state.accounts[0], openai_api_key: 'unlisted-key', api_model_catalog: ['private-model'] });
  assert.equal(h.state.accountEditor.apiKeyId, '__new__'); assert.equal(h.state.form.newApiKey, 'unlisted-key');
  assert.equal(h.state.form.modelCatalogText, 'private-model');
});
