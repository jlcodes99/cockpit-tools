import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import type { CodexAccount } from '../../types/codex';
import { findCodexAccountsForModelProviderDeletion } from '../../utils/codexModelProviderAccountSync';
import { deferred } from '../../../tests/helpers/reactHookHarness';

const source = readFileSync(new URL('./CodexModelProviderManager.tsx', import.meta.url), 'utf8');
const sourceFile = ts.createSourceFile('manager.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const names = ['deleteProvidersAndAccounts', 'handleDeleteProvider', 'handleBatchDeleteProviders', 'handleDeleteBatchTestResults'];
const handlers: string[] = [];
function collect(node: ts.Node) {
  if (ts.isVariableDeclaration(node) && names.includes(node.name.getText(sourceFile))) {
    handlers.push(`const ${node.getText(sourceFile)};`);
  }
  ts.forEachChild(node, collect);
}
collect(sourceFile);
assert.equal(handlers.length, names.length);
const compiled = ts.transpileModule(`${handlers.join('\n')} Object.assign(globalThis, {${names.join(',')}});`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function account(id: string, overrides: Partial<CodexAccount> = {}): CodexAccount {
  return { id, email: `${id}@example.com`, auth_mode: 'apikey', api_provider_id: 'provider',
    openai_api_key: '', tokens: { id_token: '', access_token: '' }, created_at: 1, last_used: 1,
    ...overrides };
}

function harness() {
  const providers = [{ id: 'provider', name: 'Relay', baseUrl: 'https://relay.example/v1' }];
  const accounts = [
    account('healthy', { openai_api_key: 'key' }),
    account('unhealthy', { requires_reauth: true, authorization_status: 'revoked' }),
    account('url-match', { api_provider_id: 'legacy', api_base_url: 'https://relay.example/v1/' }),
    account('other', { api_provider_id: 'other', api_base_url: 'https://other.example/v1' }),
    account('oauth', { auth_mode: 'oauth' }),
  ];
  const calls: string[] = [];
  const prompts: { key: string; count?: number; accountCount?: number }[] = [];
  const notices: unknown[] = [];
  let confirmed = true;
  let deleteError: Error | null = null;
  let gate: ReturnType<typeof deferred<void>> | null = null;
  const context: Record<string, any> = {
    useCallback: (fn: unknown) => fn,
    accounts, providers, findCodexAccountsForModelProviderDeletion,
    providerDeletionRef: { current: false },
    selectedProviderIds: new Set(['provider']), batchTestResultSelectedProviderIds: new Set(['provider']),
    batchTestDeleting: false, batchTestSession: { running: false },
    setDeletingProviderIds() {}, setSelectedProviderIds() {}, setBatchTestSelectedProviderIds() {},
    setBatchTestResultSelectedProviderIds() {}, setBatchTestSession() {}, setBatchTestDeleting() {},
    setBatchTestError: (value: unknown) => { if (value) notices.push(value); },
    setNotice: (notice: unknown) => notices.push(notice),
    t: (key: string, options: Record<string, unknown> = {}) => { prompts.push({ key, ...options }); return key; },
    confirmDialog: async () => confirmed,
    parseServiceError: (error: Error) => error.message,
    async deleteAccounts(ids: string[]) {
      calls.push(`accounts:${ids.join(',')}`);
      if (gate) await gate.promise;
      if (deleteError) throw deleteError;
      for (let i = accounts.length - 1; i >= 0; i--) if (ids.includes(accounts[i].id)) accounts.splice(i, 1);
    },
    async deleteCodexModelProvider(id: string) { calls.push(`provider:${id}`); },
    async reloadProviders(merge: boolean) { assert.equal(merge, false); calls.push('reload'); },
  };
  vm.runInNewContext(compiled, context);
  return { context, providers, accounts, calls, prompts, notices,
    cancel() { confirmed = false; }, failDeletion(error: Error | null) { deleteError = error; },
    holdDeletion() { gate = deferred<void>(); return gate; } };
}

test('single deletion cascades to healthy and unhealthy linked accounts without account validation', async () => {
  const h = harness();
  await h.context.handleDeleteProvider(h.providers[0]);
  assert.deepEqual(h.calls, ['accounts:healthy,unhealthy,url-match', 'provider:provider', 'reload']);
  assert.deepEqual(h.accounts.map(account => account.id), ['other', 'oauth']);
  assert.equal(h.prompts[0].key, 'codex.modelProviders.confirmDeleteWithAccounts');
  assert.equal(h.prompts[0].count, 3);
  assert.equal(h.context.providerDeletionRef.current, false);
});

test('providers without accounts remain deletable and cancel does not mutate anything', async () => {
  const h = harness();
  h.accounts.splice(0);
  await h.context.handleDeleteProvider(h.providers[0]);
  assert.deepEqual(h.calls, ['provider:provider', 'reload']);
  assert.equal(h.prompts[0].key, 'codex.modelProviders.confirmDelete');
  const cancelled = harness();
  cancelled.cancel();
  await cancelled.context.handleDeleteProvider(cancelled.providers[0]);
  assert.deepEqual(cancelled.calls, []);
  assert.equal(cancelled.accounts.length, 5);
});

test('both batch deletion entries cascade through the same local deletion flow', async () => {
  for (const name of ['handleBatchDeleteProviders', 'handleDeleteBatchTestResults']) {
    const h = harness();
    await h.context[name]();
    assert.deepEqual(h.calls, ['accounts:healthy,unhealthy,url-match', 'provider:provider', 'reload']);
    assert.equal(h.prompts[0].accountCount, 3);
  }
});

test('duplicate deletion does not run twice while local storage is pending', async () => {
  const h = harness();
  const gate = h.holdDeletion();
  const deletion = h.context.handleDeleteProvider(h.providers[0]);
  await Promise.resolve();
  await h.context.handleDeleteProvider(h.providers[0]);
  assert.deepEqual(h.calls, ['accounts:healthy,unhealthy,url-match']);
  gate.resolve();
  await deletion;
  assert.deepEqual(h.calls, ['accounts:healthy,unhealthy,url-match', 'provider:provider', 'reload']);
});

test('real storage errors are reported and release the deletion lock for retry', async () => {
  const h = harness();
  h.failDeletion(new Error('disk write failed'));
  await h.context.handleDeleteProvider(h.providers[0]);
  assert.deepEqual(h.calls, ['accounts:healthy,unhealthy,url-match']);
  assert.equal(h.accounts.length, 5);
  assert.ok(h.notices.some(notice => (notice as { tone?: string })?.tone === 'error'));
  assert.equal(h.context.providerDeletionRef.current, false);
  h.failDeletion(null);
  await h.context.handleDeleteProvider(h.providers[0]);
  assert.ok(h.calls.includes('provider:provider'));
});
