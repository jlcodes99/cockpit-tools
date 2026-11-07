import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const source = ts.createSourceFile('useCodexAccountsAccessController.tsx', readFileSync(
  new URL('./useCodexAccountsAccessController.tsx', import.meta.url), 'utf8',
), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function harness(overrides: Record<string, any> = {}) {
  const calls: { cli: any[][]; updates: any[][]; preview: unknown[]; intent: boolean[]; errors: unknown[] } = {
    cli: [], updates: [], preview: [], intent: [], errors: [],
  };
  const context = vm.createContext({
    getCodexSwitchOrLaunchBlockedReason: () => null,
    isCodexWebSessionAccount: () => false,
    isDeepSeekAccount: (account: any) => account.api_provider_id === 'deepseek',
    updateAccountInstanceAccess: async (...args: any[]) => {
      calls.updates.push(args);
      return { id: args[0], api_provider_id: 'deepseek', api_instance_access_mode: args[1] };
    },
    resolveDeepSeekBindAccountId: (account: any) => `${account.id}:${account.api_instance_access_mode}`,
    buildCodexAccountPresentation: (account: any) => ({ displayName: account.id }),
    openCodexCliLaunchModal: (...args: any[]) => { calls.cli.push(args); },
    restoreLaunchPreviewInstanceId: () => {},
    setLaunchPreviewCliIntent: (value: boolean) => { calls.intent.push(value); },
    setLaunchPreviewAccount: (value: unknown) => { calls.preview.push(value); },
    setMessage: (value: unknown) => { calls.errors.push(value); },
    cliLaunchModal: null, cliLaunchingAccountId: null, localAccessCollection: {},
    CODEX_API_SERVICE_BIND_ID: '__api_service__', t: (key: string) => key,
    ...overrides,
  });
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && node.initializer &&
      ['handleLaunchCodexCli', 'handleLaunchLocalAccessCli'].includes(node.name.getText(source))) {
      vm.runInContext(ts.transpileModule(`globalThis.${node.name.getText(source)} = ${node.initializer.getText(source)}`, {
        compilerOptions: { target: ts.ScriptTarget.ES2022 },
      }).outputText, context);
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  return { context, calls };
}

test('CLI inside an existing account preview opens CLI immediately rather than reopening the preview', async () => {
  const h = harness();
  await h.context.handleLaunchCodexCli({ id: 'api-account' }, {});
  assert.deepEqual(h.calls.cli, [['account', 'api-account', 'api-account', 'api-account']]);
  assert.deepEqual(h.calls.preview, []);
  assert.deepEqual(h.calls.intent, [false]);
});

test('CLI from an account list still opens the preview before preparing CLI', async () => {
  const h = harness();
  const account = { id: 'oauth-account' };
  await h.context.handleLaunchCodexCli(account);
  assert.deepEqual(h.calls.cli, []);
  assert.deepEqual(h.calls.preview, [account]);
  assert.deepEqual(h.calls.intent, [true]);
});

test('DeepSeek CLI uses the confirmed access mode and image configuration', async () => {
  const h = harness();
  await h.context.handleLaunchCodexCli({ id: 'deepseek-account', api_provider_id: 'deepseek' }, {
    deepSeekAccessMode: 'gateway', imageGenerationAccountIds: ['image-account'],
    imageGenerationAccountModels: { 'image-account': 'gpt-image-2.5' },
  });
  assert.deepEqual(h.calls.updates, [['deepseek-account', 'gateway', null,
    ['image-account'], { 'image-account': 'gpt-image-2.5' }]]);
  assert.equal(h.calls.cli[0][3], 'deepseek-account:gateway');
  assert.deepEqual(h.calls.preview, []);
});

test('preview CLI errors propagate to the current modal instead of the hidden page', async () => {
  const blocked = harness({ getCodexSwitchOrLaunchBlockedReason: () => 'account unavailable' });
  await assert.rejects(blocked.context.handleLaunchCodexCli({ id: 'blocked' }, {}), /account unavailable/);
  assert.deepEqual(blocked.calls.errors, []);
  assert.deepEqual(blocked.calls.cli, []);
  const missing = harness({ localAccessCollection: null });
  assert.throws(() => missing.context.handleLaunchLocalAccessCli({}), /codex.localAccess.testUnavailable/);
  assert.deepEqual(missing.calls.errors, []);
});

test('API service preview opens its CLI dialog without reopening the launch preview', () => {
  const h = harness();
  h.context.handleLaunchLocalAccessCli({});
  assert.deepEqual(h.calls.cli, [['apiService', '__api_service__', 'codex.localAccess.title']]);
  assert.deepEqual(h.calls.preview, []);
});

test('failed DeepSeek settings save prevents opening CLI and can be retried', async () => {
  let attempts = 0;
  const h = harness({ updateAccountInstanceAccess: async () => {
    if (++attempts === 1) throw new Error('disk full');
    return { id: 'deepseek-account', api_provider_id: 'deepseek', api_instance_access_mode: 'gateway' };
  } });
  const account = { id: 'deepseek-account', api_provider_id: 'deepseek' };
  const options = { deepSeekAccessMode: 'gateway' };
  await assert.rejects(h.context.handleLaunchCodexCli(account, options), /disk full/);
  assert.deepEqual(h.calls.cli, []);
  assert.deepEqual(h.calls.errors, []);
  await h.context.handleLaunchCodexCli(account, options);
  assert.equal(h.calls.cli.length, 1);
});
