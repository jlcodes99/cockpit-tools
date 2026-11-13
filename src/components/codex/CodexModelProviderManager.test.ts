import { resolveAccountUsageBaseUrl } from "../../utils/openaiOfficialUsage";
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { resolveCodexApiProviderPresetId, CODEX_API_PROVIDER_CUSTOM_ID } from '../../utils/codexProviderPresets';
import { isCodexApiKeyAccount } from '../../types/codex';
import type { CodexAccount } from '../../types/codex';
import {
  compareCodexModelProviderChatTestRecords,
  type CodexModelProviderChatTestRecord,
} from '../../services/codexModelProviderService';

// Execute the actual save handler with isolated IPC, rather than writing real account files.
const source = readFileSync(new URL('./CodexModelProviderManager.tsx', import.meta.url), 'utf8');
const start = source.indexOf('  const handleSaveApiKeyEdit =');
const end = source.indexOf('  }, [', start);
assert.ok(start > 0 && end > start);
const handler = `${source.slice(start, end)}  }, []); globalThis.save = handleSaveApiKeyEdit;`;

test('provider test order prefers successful providers with shorter total duration', () => {
  const fixture = {
    providerName: 'Fixture', wireApi: 'responses', accessMode: 'gateway',
    prompt: 'ping', timestamp: 0,
  } as const;
  const records: CodexModelProviderChatTestRecord[] = [
    { ...fixture, providerId: 'failed-first', success: false, durationMs: 1 },
    { ...fixture, providerId: 'slow', success: true, firstTextMs: 5, durationMs: 100 },
    { ...fixture, providerId: 'unknown', success: true },
    { ...fixture, providerId: 'fast', success: true, firstTextMs: 10, durationMs: 30 },
    { ...fixture, providerId: 'failed-second', success: false },
  ];
  assert.deepEqual(records.sort(compareCodexModelProviderChatTestRecords).map((item) => item.providerId),
    ['fast', 'slow', 'unknown', 'failed-first', 'failed-second']);
});

test('testing from a provider card selects only that provider', () => {
  const start = source.indexOf('  const openBatchTestModal =');
  const end = source.indexOf('  const markBatchTestCancelled =', start);
  assert.ok(start > 0 && end > start);
  const first = { id: 'first' };
  const second = { id: 'second' };
  let selected: string[] = [];
  const set = () => {};
  const open = vm.runInNewContext(ts.transpileModule(
    `${source.slice(start, end)}globalThis.open = openBatchTestModal;`,
    { compilerOptions: { target: ts.ScriptTarget.ES2022 } },
  ).outputText + '\nglobalThis.open;', {
    useCallback: (fn: unknown) => fn,
    filteredProviders: [first, second],
    selectedProviderIds: new Set([second.id]),
    getSelectedProviderApiKey: () => ({ id: 'key' }),
    batchTestSession: null,
    setBatchTestSelectedProviderIds: (ids: Set<string>) => { selected = [...ids]; },
    setBatchTestResultSelectedProviderIds: set, setBatchTestSearchQuery: set, setBatchTestFilter: set,
    setBatchTestModelId: set, setBatchTestModelCustom: set, setBatchTestError: set,
    setBatchTestCancelling: set, setBatchTestSession: set, setBatchTestStep: set, setBatchTestModalOpen: set,
  });
  open(first);
  assert.deepEqual(selected, [first.id]);
  open();
  assert.deepEqual(selected, [second.id]);
});

test('opening a provider’s instance configuration forwards the chosen ID and surfaces opener errors', async () => {
  const file = ts.createSourceFile('manager.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let action: ts.Expression | undefined;
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(file) === 'handleOpenInstanceConfig') action = node.initializer;
    ts.forEachChild(node, visit);
  }
  visit(file);
  assert.ok(action);
  const opened: string[] = [];
  const notices: Array<{ tone: string; text: string } | null> = [];
  const context = {
    useCallback: (fn: unknown) => fn,
    setNotice(value: (typeof notices)[number]) { notices.push(value); },
    async openCodexInstanceConfigToml(id: string) {
      opened.push(id);
      if (id === 'missing') throw new Error('config missing');
    },
    parseServiceError: (error: Error) => error.message,
    t: (key: string, args: { error: string }) => `${key} ${args.error}`,
  };
  const run = vm.runInNewContext(ts.transpileModule(`(${action.getText(file)})`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, context);
  await run('missing');
  assert.equal(notices[notices.length - 1]?.tone, 'error');
  assert.equal(notices[notices.length - 1]?.text, 'instances.form.codexQuickConfig.openFailed config missing');
  await run('separate-profile');
  assert.deepEqual(opened, ['missing', 'separate-profile']);
  assert.equal(notices[notices.length - 1], null);
});

test('provider dragging persists only the final order, even if mouse-up is handled twice', () => {
  const from = source.indexOf('  const stopProviderCustomSortDragging =');
  const to = source.indexOf('  const resetProviderCustomSortOrder =', from);
  assert.ok(from > 0 && to > from);
  const saved: string[][] = [];
  const context = {
    providerCustomSortProviderIds: ['p1', 'p2', 'p3'],
    draggedProviderCustomSortId: 'p1',
    dragOrderRef: { current: null },
    useCallback: (fn: unknown) => fn,
    saveProviderOrder: (order: string[]) => saved.push([...order]),
    setDraggedProviderCustomSortId() {},
    setProviderCustomSortDropTargetId() {},
    setDragOrder(order: string[] | null) {
      if (order) context.providerCustomSortProviderIds = order;
    },
    move: (_target: string) => {},
    stop: () => {},
  };
  const handlers = source.slice(from, to)
    + 'globalThis.move = handleProviderCustomSortDragMove; globalThis.stop = stopProviderCustomSortDragging;';
  vm.runInNewContext(ts.transpileModule(handlers, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  context.move('p2');
  context.move('p3');
  assert.equal(saved.length, 0);
  context.stop();
  context.stop();
  assert.deepEqual(saved, [['p2', 'p3', 'p1']]);
});

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
    isCodexApiKeyAccount, resolveCodexApiProviderPresetId, CODEX_API_PROVIDER_CUSTOM_ID, resolveAccountUsageBaseUrl,
    normalizeCodexModelProviderBaseUrl: (url: string) => url.replace(/\/+$/, '').toLowerCase(),
    resolveProviderWireApi: () => 'responses',
    updateApiKeyOnCodexModelProvider: async () => provider,
    updateCodexApiKeyCredentials: async (...args: unknown[]) => { updates.push(args); },
    reloadProviders: async () => {}, emitAccountsChanged: async () => {},
    setSaving() {}, setNotice() {}, setFormError() {}, setEditingApiKey() {}, t: (key: string) => key,
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
  account.api_base_url = undefined;
  await context.save();
  assert.equal(updates[2][3], 'openai_builtin', 'implicit official URL still links to the provider key');
});
