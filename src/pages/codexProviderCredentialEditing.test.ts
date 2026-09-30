import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider, mergeCodexModelProviderCredentialInput } from '../utils/codexModelProviderAccountSync';
import { parseContextWindowDrafts } from '../utils/codexModelContextWindows';

function handler(name: string, state: Record<string, any>) {
  const file = 'useCodexAccountsAccessController.tsx';
  const source = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let declaration: ts.VariableDeclaration | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name) declaration = node;
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(declaration);
  const code = ts.transpileModule(`globalThis.run = ${declaration.initializer!.getText(source)}`, {
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
    accounts: [account], editingApiKeyCredentialsId: 'account', editingApiKeyCredentialsValue: 'sk-test',
    editingApiBaseUrlCredentialsValue: account.api_base_url, editingApiProviderPresetId: 'custom',
    editingManagedProviderId: 'provider', editingNewManagedProviderNameInput: 'Relay',
    editingApiModelCatalogDraft: ['model'], editingApiModelContextWindowsInput: {},
    editingApiSyncModelCatalogToCodex: false, editingApiSyncModelCatalogToApiService: true, savingApiKeyCredentials: false,
    editingApiWireApi: 'chat_completions', editingApiSupportsWebsockets: true,
    selectedEditingManagedProvider: provider, selectedEditingManagedProviderApiKey: provider.apiKeys[0],
    validateApiKeyCredentialInputs: () => ({ ok: true, apiKey: 'sk-test', apiBaseUrl: account.api_base_url }),
    buildApiProviderPayload: () => ({ apiProviderMode: 'custom', apiProviderId: 'provider', apiProviderName: 'Relay',
      apiWireApi: 'responses', apiSupportsWebsockets: true }),
    buildCodexModelProviderAccountSnapshot, findCodexAccountsReferencingModelProvider, mergeCodexModelProviderCredentialInput,
    parseContextWindowDrafts, isSameHttpBaseUrl: (a: string, b: string) => a === b,
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
    setEditingApiSyncModelCatalogToCodex() {}, setEditingApiSyncModelCatalogToApiService() {}, setEditingApiModelCatalogError() {},
    DEFAULT_CODEX_API_BASE_URL: 'https://api.openai.com/v1', DEFAULT_CODEX_API_PROVIDER_ID: 'custom',
    COCKPIT_API_PROVIDER_ID: 'cockpit', t: (key: string) => key,
    modalId: 'account', error: 'old error',
  };
  return { state, writes, snapshots };
}

test('Codex catalog availability does not reset API service synchronization', () => {
  const file = 'useCodexAccountsOAuthController.ts';
  const source = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true);
  let callback: ts.Expression | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect' &&
      node.arguments[1]?.getText(source) === '[editingApiModelCatalogSyncAvailable]') {
      callback = node.arguments[0];
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(callback);
  for (const available of [false, true]) {
    const state = {
      editingApiModelCatalogSyncAvailable: available,
      codexSync: true, apiServiceSync: true,
      setEditingApiSyncModelCatalogToCodex(value: boolean) { state.codexSync = value; },
      setEditingApiSyncModelCatalogToApiService(value: boolean) { state.apiServiceSync = value; },
    };
    const code = ts.transpileModule(`(${callback.getText(source)})()`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText;
    vm.runInNewContext(code, state);
    assert.equal(state.apiServiceSync, true);
    assert.equal(state.codexSync, available);
  }
});

test('credential editor saves an explicit protocol change and disables incompatible WebSocket transport', async () => {
  const h = harness();
  await handler('handleSubmitApiKeyCredentials', h.state)();
  assert.equal(h.writes[0].wireApi, 'chat_completions');
  assert.equal(h.writes[0].supportsWebsockets, false);
  assert.equal(h.snapshots[0][10], 'chat_completions');
  assert.equal(h.snapshots[0][11], false);
  assert.equal(h.snapshots[0][15], true);
  assert.equal(h.snapshots[1].apiWireApi, 'chat_completions');
  assert.equal(h.snapshots[1].apiSupportsWebsockets, false);
  assert.equal(h.state.modalId, null);
  assert.equal(h.state.error, null);
});

test('credentials validation and persistence failures stay visible in the open editor', async () => {
  const invalid = harness();
  invalid.state.validateApiKeyCredentialInputs = () => ({ ok: false, message: 'invalid key' });
  await handler('handleSubmitApiKeyCredentials', invalid.state)();
  assert.equal(invalid.state.error, 'invalid key');
  assert.equal(invalid.state.modalId, 'account');
  assert.equal(invalid.writes.length, 0);
  const failed = harness();
  failed.state.upsertCodexModelProviderFromCredential = async () => { throw new Error('disk full'); };
  await handler('handleSubmitApiKeyCredentials', failed.state)();
  assert.match(failed.state.error, /disk full/);
  assert.equal(failed.state.modalId, 'account');
  assert.equal(failed.state.busy, false);
  assert.equal(failed.state.message, undefined);
});

test('explicit WebSocket false overrides the canonical provider when changing credentials', () => {
  const h = harness();
  const input = mergeCodexModelProviderCredentialInput(h.state.selectedEditingManagedProvider, {
    apiBaseUrl: 'https://relay.example/v1', apiKey: 'sk-test', wireApi: 'responses', supportsWebsockets: false,
  });
  assert.equal(input.supportsWebsockets, false);
});
