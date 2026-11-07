import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider, mergeCodexModelProviderCredentialInput } from '../utils/codexModelProviderAccountSync';
import { contextWindowDraftsFromRecord, parseContextWindowDrafts } from '../utils/codexModelContextWindows';
import { resolveCodexModelProviderForApiKey } from '../utils/codexModelProviderKeyConfig';
import { resolveCodexProviderCapabilityProfile } from '../utils/codexProviderGateway';
import { resolveCodexModelProviderAccountName } from '../utils/codexModelProviderAccountName';

function handler(name: string, state: Record<string, any>) {
  const file = 'useCodexAccountsAccessController.tsx';
  const source = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let declaration: ts.VariableDeclaration | undefined;
  const declarations = new Map<string, ts.VariableDeclaration>();
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node)) {
      declarations.set(node.name.getText(source), node);
      if (node.name.getText(source) === name) declaration = node;
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(declaration);
  const fetchBindings = name === 'handleFetchApiModelCatalog' || name === 'handleFetchEditingApiModelCatalog'
    ? ['apiModelFetchWireApi', 'apiModelFetchAuthScheme', 'apiModelFetchKey', 'modelFetchState'] : [];
  const setup = fetchBindings.map(key => `globalThis.${key} = ${declarations.get(key)!.initializer!.getText(source)};`).join('\n')
    + (fetchBindings.length ? 'modelFetchState.current.addKey = apiModelFetchKey;' : '');
  const code = ts.transpileModule(`${setup}\nglobalThis.run = ${declaration.initializer!.getText(source)}`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  vm.runInNewContext(code, state);
  return state.run as (...args: any[]) => Promise<any>;
}

function harness() {
  const account = { id: 'account', auth_mode: 'apikey', api_provider_id: 'provider',
    api_base_url: 'https://relay.example/v1', openai_api_key: 'sk-test' };
  const provider = { id: 'provider', name: 'Relay', baseUrl: account.api_base_url,
    wireApi: 'responses', supportsWebsockets: true, apiKeys: [{ id: 'key', name: 'Key', apiKey: 'sk-test' }],
    createdAt: 1, updatedAt: 1 };
  const writes: any[] = [];
  const snapshots: any[] = [];
  const state: Record<string, any> = {
    useCallback: (value: any) => value,
    useRef: (value: any) => state.modelFetchState ?? { current: value },
    resolveCodexProviderCapabilityProfile, resolveCodexModelProviderAccountName,
    apiBaseUrlInput: account.api_base_url, apiKeyInput: account.openai_api_key,
    apiProviderPresetId: 'custom', selectedManagedProvider: provider, sponsorApiProviderTemplates: [],
    accounts: [account], editingApiKeyCredentialsId: 'account', editingApiKeyCredentialsValue: 'sk-test',
    editingApiBaseUrlCredentialsValue: account.api_base_url, editingApiProviderPresetId: 'custom',
    editingManagedProviderId: 'provider', editingNewManagedProviderNameInput: 'Relay',
    editingApiModelCatalogDraft: ['model'], editingApiModelContextWindowsInput: {},
    editingApiSyncModelCatalogToCodex: false, savingApiKeyCredentials: false,
    editingApiWireApi: 'chat_completions', editingApiSupportsWebsockets: true,
    selectedEditingManagedProvider: provider, selectedEditingManagedProviderApiKey: provider.apiKeys[0],
    validateApiKeyCredentialInputs: () => ({ ok: true, apiKey: 'sk-test', apiBaseUrl: account.api_base_url }),
    buildApiProviderPayload: () => ({ apiProviderMode: 'custom', apiProviderId: 'provider', apiProviderName: 'Relay',
      apiWireApi: 'responses', apiSupportsWebsockets: true }),
    buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider, mergeCodexModelProviderCredentialInput,
    parseContextWindowDrafts, resolveCodexModelProviderForApiKey, contextWindowDraftsFromRecord, isSameHttpBaseUrl: (a: string, b: string) => a === b,
    isRelayApiProviderTemplateId: () => false, resolveManagedProviderIdForAccount: () => 'provider',
    upsertCodexModelProviderFromCredential: async (input: any) => {
      writes.push(input);
      return { ...provider, wireApi: input.wireApi, supportsWebsockets: input.supportsWebsockets };
    },
    queryCodexModelProviderUsage: async () => ({ mode: 'unknown' }), reloadManagedProviders: async () => {},
    updateApiKeyCredentials: async (...args: any[]) => { snapshots.push(args); return account; },
    codexService: { syncCodexApiKeyProviderAccounts: async (input: any) => { snapshots.push(input); return 1; } },
    emitAccountsChanged: async () => {},
    setEditingApiCredentialsError: (value: unknown) => { state.error = value; },
    setEditingApiKeyCredentialsId: (value: unknown) => { state.modalId = value; },
    setSavingApiKeyCredentials: (value: unknown) => { state.busy = value; },
    setMessage: (value: unknown) => { state.message = value; },
    setApiKeyUsageMap() {}, setEditingApiKeyCredentialsValue() {}, setEditingApiKeyCredentialsVisible() {},
    setEditingApiBaseUrlCredentialsValue() {}, setEditingApiProviderPresetId() {}, setEditingManagedProviderId() {},
    setEditingManagedProviderApiKeyId() {}, setEditingNewManagedProviderNameInput() {}, setEditingApiModelCatalogInput() {},
    setEditingApiSyncModelCatalogToCodex() {}, setEditingApiModelCatalogError() {},
    DEFAULT_CODEX_API_BASE_URL: 'https://api.openai.com/v1', DEFAULT_CODEX_API_PROVIDER_ID: 'custom',
    COCKPIT_API_PROVIDER_ID: 'cockpit', t: (key: string) => key,
    modalId: 'account', error: 'old error',
  };
  return { state, writes, snapshots };
}

test('explicit WebSocket false overrides the canonical provider when changing credentials', () => {
  const h = harness();
  const input = mergeCodexModelProviderCredentialInput(h.state.selectedEditingManagedProvider, {
    apiBaseUrl: 'https://relay.example/v1', apiKey: 'sk-test', wireApi: 'responses', supportsWebsockets: false,
  });
  assert.equal(input.supportsWebsockets, false);
});

for (const scheme of ['bearer', 'x-api-key', undefined] as const) {
  test(`quick switch persists the target authentication (${scheme ?? 'default'})`, async () => {
    const h = harness();
    Object.assign(h.state, {
      quickSwitchAccount: { ...h.state.accounts[0], api_auth_scheme: scheme === 'bearer' ? 'x-api-key' : 'bearer' },
      selectedQuickSwitchProvider: { ...h.state.selectedManagedProvider, wireApi: 'anthropic', authScheme: scheme },
      selectedQuickSwitchApiKey: h.state.selectedManagedProvider.apiKeys[0],
      setQuickSwitchSubmitting() {}, setQuickSwitchError(value: unknown) { h.state.error = value; },
      setQuickSwitchAccountId(value: unknown) { h.state.modalId = value; },
      setQuickSwitchProviderId() {}, setQuickSwitchApiKeyId() {},
    });
    await handler('handleSubmitQuickSwitch', h.state)();
    assert.equal(h.snapshots[0][10], 'anthropic');
    assert.equal(h.snapshots[0][15], scheme ?? 'x-api-key');
    assert.equal(h.state.modalId, null);
    assert.equal(h.state.error, null);
  });
}

for (const entry of ['add'] as const) {
  for (const scheme of ['bearer', 'x-api-key'] as const) {
    test(`${entry} model discovery forwards the Anthropic protocol and ${scheme} authentication`, async () => {
      const h = harness();
      h.state.selectedManagedProvider.wireApi = 'anthropic';
      h.state.selectedManagedProvider.authScheme = scheme;
      h.state.editingApiWireApi = 'anthropic';
      h.state.accounts[0].api_auth_scheme = scheme;
      let request: any;
      let draft = '';
      h.state.listModelProviderModels = async (input: any) => { request = input; return { models: [{ id: 'claude-model' }] }; };
      h.state.parseApiModelCatalogText = (value: string) => value.split('\n');
      h.state.setApiModelCatalogFetching = h.state.setEditingApiModelCatalogFetching = () => {};
      h.state.setApiModelCatalogError = h.state.setEditingApiModelCatalogError = () => {};
      h.state.setApiModelCatalogInput = h.state.setEditingApiModelCatalogInput = (value: string) => { draft = value; };
      await handler('handleFetchApiModelCatalog', h.state)();
      assert.equal(request.wireApi, 'anthropic');
      assert.equal(request.authScheme, scheme);
      assert.equal(draft, 'claude-model');
    });
  }
}
