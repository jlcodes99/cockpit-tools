import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { isDeepSeekAccount, resolveDeepSeekBindAccountId } from '../src/utils/codexDeepSeekAccess';
import { isCodexProviderAccessAccount, resolveCodexProviderAccessMode } from '../src/utils/codexProviderAccess';
import { buildCodexProviderGatewayBindId, CODEX_PROVIDER_GATEWAY_BIND_PREFIX } from '../src/types/instance';
import { deferred, settlePromises } from './helpers/reactHookHarness';

// Run the production callbacks, mocking IPC and process starts. No real account files are used.
function callback(file: string, name: string, state: Record<string, any>) {
  const source = ts.createSourceFile(file, readFileSync(new URL(`../${file}`, import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let initializer: ts.Expression | undefined;
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name) initializer = node.initializer;
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(initializer && ts.isCallExpression(initializer));
  vm.runInNewContext(ts.transpileModule(`globalThis.run = ${initializer.arguments[0].getText(source)};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, state);
  return state.run as (...args: any[]) => Promise<boolean>;
}

function account(provider = 'custom', mode = 'gateway') {
  return {
    id: 'fixture-account', email: 'fixture@example.invalid', auth_mode: 'apikey',
    api_provider_id: provider, api_provider_mode: 'custom', api_wire_api: 'responses',
    api_base_url: provider === 'deepseek' ? 'https://api.deepseek.com' : 'https://provider.example.invalid/v1',
    api_instance_access_mode: mode, api_sync_model_catalog_to_codex: true,
  };
}

const entries = ['account', 'instance', 'provider'] as const;
function harness(entry: typeof entries[number], original: ReturnType<typeof account>, save?: (...args: any[]) => Promise<any>) {
  const writes: any[][] = [];
  const bindings: string[] = [];
  const events: string[] = [];
  const bind = async (input: any) => { events.push('bind'); bindings.push(input.bindAccountId); };
  const state: Record<string, any> = {
    isDeepSeekAccount, resolveDeepSeekBindAccountId, isCodexProviderAccessAccount,
    resolveCodexProviderAccessMode, buildCodexProviderGatewayBindId, CODEX_PROVIDER_GATEWAY_BIND_PREFIX,
    updateAccountInstanceAccess: async (...args: any[]) => {
      writes.push(args); events.push('save');
      return save ? save(...args) : { ...original, api_instance_access_mode: args[1] };
    },
  };
  let run: (...args: any[]) => Promise<boolean>;
  if (entry === 'account') {
    Object.assign(state, {
      activeLaunchPreviewAccount: original, launchPreviewInstanceId: 'fixture-instance',
      DEFAULT_CODEX_INSTANCE_ID: 'default', persistCodexLaunchPreviewLastInstanceId() {},
      launchPreviewCliIntent: false, setLaunchPreviewAccount() {},
      codexInstanceStore: { updateInstance: bind, startInstance: async () => { events.push('start'); } },
    });
    run = callback('src/pages/useCodexAccountsAccessController.tsx', 'handleExecuteLaunchPreview', state);
  } else if (entry === 'instance') {
    Object.assign(state, {
      launchPreview: { instance: { id: 'fixture-instance', bindAccountId: buildCodexProviderGatewayBindId(original.id) }, account: original },
      pendingLaunchResolve: { current: () => { events.push('continue-start'); } }, setLaunchPreview() {},
      instanceStore: { updateInstance: bind },
    });
    run = callback('src/pages/CodexInstancesPage.tsx', 'executeLaunchPreview', state);
  } else {
    Object.assign(state, {
      providerLaunchPreview: { account: original, provider: { id: 'fixture-provider', name: 'Fixture', baseUrl: original.api_base_url }, instanceId: 'fixture-instance', instanceName: 'Fixture' },
      setNotice() {}, resolveCodexApiProviderPresetId: () => 'custom',
      resolveGatewayModeByWireApi: () => 'direct', resolveProviderWireApi: () => 'responses',
      updateCodexInstance: bind, startCodexInstance: async () => { events.push('start'); },
      reloadCurrentAccount: async () => {}, reloadLocalAccessState: async () => {},
      reloadCodexInstances: async () => {}, setLastEnabledProviderId() {}, setProviderLaunchPreview() {},
      t: (key: string) => key, parseServiceError: String,
    });
    run = callback('src/components/codex/CodexModelProviderManager.tsx', 'executeProviderLaunchPreview', state);
  }
  return { run, writes, bindings, events, state };
}

for (const entry of entries) {
  test(`${entry}: custom Responses access is saved before binding and starting`, async () => {
    const original = account();
    const h = harness(entry, original);
    const imageGenerationAccountModels = {};
    assert.equal(await h.run(true, { deepSeekAccessMode: 'direct', imageGenerationAccountIds: [], imageGenerationAccountModels }), true);
    assert.deepEqual(h.writes, [[original.id, 'direct', null, [], imageGenerationAccountModels]]);
    assert.deepEqual(h.bindings, [original.id]);
    assert.deepEqual(h.events, ['save', 'bind', entry === 'instance' ? 'continue-start' : 'start']);
  });

  test(`${entry}: pending or rejected persistence cannot bind or start`, async () => {
    const pending = deferred<any>();
    const h = harness(entry, account(), () => pending.promise);
    const result = h.run(true, { deepSeekAccessMode: 'direct' });
    await settlePromises();
    assert.deepEqual(h.events, ['save']);
    pending.reject(new Error('synthetic write failure'));
    if (entry === 'provider') assert.equal(await result, false);
    else await assert.rejects(result, /synthetic write failure/);
    assert.deepEqual(h.events, ['save']);
  });
}

test('instance binding retains the gateway when the saved account still requires it', async () => {
  const original = account();
  const h = harness('instance', original, async () => original);
  await h.run(false, { deepSeekAccessMode: 'direct' });
  assert.deepEqual(h.events, ['save', 'continue-start'], 'binding must use the saved mode, not the requested mode');
});

test('direct access preserves follow-local-account instances without explicit gateway bindings', async () => {
  const h = harness('instance', account());
  Object.assign(h.state.launchPreview.instance, { bindAccountId: null, followLocalAccount: true });
  await h.run(true, { deepSeekAccessMode: 'direct' });
  assert.deepEqual(h.events, ['save', 'continue-start']);
});

test('DeepSeek retains its explicit gateway binding and image-forwarding options', async () => {
  const original = account('deepseek', 'direct');
  const h = harness('instance', original);
  h.state.launchPreview.instance.bindAccountId = original.id;
  const imageGenerationAccountModels = { 'image-fixture': 'image-model' };
  await h.run(true, { deepSeekAccessMode: 'gateway', imageGenerationAccountIds: ['image-fixture'], imageGenerationAccountModels });
  assert.deepEqual(h.writes, [[original.id, 'gateway', null, ['image-fixture'], imageGenerationAccountModels]]);
  assert.deepEqual(h.bindings, [buildCodexProviderGatewayBindId(original.id)]);
});
