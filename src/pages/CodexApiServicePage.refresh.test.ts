import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

// Exercise production draft declarations, refresh effects and save handlers using one hook scheduler.
function controller() {
  const file = ts.createSourceFile('page.tsx', readFileSync(new URL('./CodexApiServicePage.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const owner = file.statements.find((node): node is ts.FunctionDeclaration => ts.isFunctionDeclaration(node) && node.name?.text === 'useCodexApiServicePageController');
  assert.ok(owner?.body);
  const values = new Set([
    'state', 'collection', 'busy', 'error', 'notice', 'routingSaving', 'portInput', 'proxyInput', 'apiKeyDrafts',
    'modelAliasesText', 'excludedModelsText', 'accountModelRuleError', 'accountModelRulesOpen', 'accountModelRuleDrafts',
    'accountModelRuleSelected', 'accountModelRuleBulkText', 'pricingModalOpen', 'pricingDrafts', 'pricingError',
    'pricingRepriceProgress', 'timeoutsModalOpen', 'timeoutDrafts', 'timeoutsError', 'selectedTimeoutPresetId',
    'timeoutPresetNameDraft', 'sessionIdentityMappingEnabledDraft', 'sessionAffinityDraft', 'sessionAffinityTtlDraft',
    'responsesWebsocketsEnabledDraft', 'maxRetryCredentialsDraft', 'maxRetryIntervalDraft', 'disableCoolingDraft',
    'immediateSseResponseDraft', 'maxConcurrentImageRequestsDraft', 'maxAccountConcurrencyDraft', 'accountConcurrencyWaitDraft',
  ]);
  const actions = new Set([
    'reloadState', 'runAction', 'runModalAction', 'handleSavePort', 'handleSaveProxy', 'handleSaveApiKeyLabel', 'handleSaveModelRules',
    'resetAccountModelRuleDraftsFromCollection', 'handleOpenAccountModelRules', 'handleCloseAccountModelRules',
    'handleSaveAccountModelRules', 'handleOpenPricingModal', 'updatePricingDraft', 'handleSaveModelPricings',
    'handleSaveRoutingOptions', 'updateTimeoutDraft', 'parseTimeoutDraftPayload', 'handleSaveTimeouts',
  ]);
  const names: string[] = [];
  function bindings(node: ts.BindingName): string[] {
    return ts.isIdentifier(node) ? [node.text] : node.elements.flatMap((element) => ts.isBindingElement(element) ? bindings(element.name) : []);
  }
  const statements = owner.body.statements.filter((node) => {
    if (ts.isVariableStatement(node)) {
      const found = node.declarationList.declarations.flatMap((decl) => bindings(decl.name));
      if (found.some((name) => values.has(name) || actions.has(name) || name === 'openTimeoutSettings')) {
        names.push(...found); return true;
      }
    }
    // Including reset effects makes a reintroduced background-reset effect fail these tests.
    return ts.isExpressionStatement(node) && ts.isCallExpression(node.expression) && node.expression.expression.getText(file) === 'useEffect'
      && /set(ApiKeyDrafts|ModelAliasesText|ExcludedModelsText|AccountModelRuleDrafts|TimeoutDrafts|PricingDrafts)\(/.test(node.expression.arguments[0].getText(file));
  });
  const helpers = file.statements.filter((node) => ts.isFunctionDeclaration(node) && node.pos < owner.pos).map((node) => node.getText(file)).join('\n');
  const h = loadHookModule(new URL('../hooks/useRefreshSafeDraft.ts', import.meta.url), {});
  const navigation = loadHookModule(new URL('../hooks/useCodexTimeoutSettingsNavigation.ts', import.meta.url), { react: h.hooks });
  let server: any = { collection: {
    port: 8317, upstreamProxyUrl: 'http://saved.invalid', apiKeys: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }],
    modelAliases: [], excludedModels: [], accountModelRules: [{ accountId: 'account', excludedModels: ['old'] }],
    sessionAffinityTtlMs: 3600000, maxRetryCredentials: 1, timeouts: {}, activeTimeoutPresetId: 'long_wait', timeoutPresets: [],
  } };
  let failure = false;
  const calls: Array<{ method: string; args: any[] }> = [];
  const service = new Proxy({}, { get(_target, method: string) { return async (...args: any[]) => {
    if (method === 'getCodexLocalAccessState') return structuredClone(server);
    calls.push({ method, args }); if (failure) throw new Error('save failed');
    if (method.endsWith('Port')) server.collection.port = args[0];
    if (method.endsWith('UpstreamProxyConfig')) server.collection.upstreamProxyUrl = args[0];
    if (method.endsWith('ApiKey')) server.collection.apiKeys.find((key: any) => key.id === args[0]).label = args[1].label;
    if (method.endsWith('RoutingOptions')) Object.assign(server.collection, args[0]);
    if (method.endsWith('AccountModelRules')) server.collection.accountModelRules = args[0];
    if (method.endsWith('Timeouts')) server.collection.timeouts = args[0];
    if (method.endsWith('ModelPricings')) server.collection.modelPricings = args[0];
    return structuredClone(server);
  }; } });
  const context: any = {
    ...h.hooks, ...h.exports, useCodexTimeoutSettingsNavigation: navigation.exports.useCodexTimeoutSettingsNavigation,
    codexLocalAccessService: service, stateRequestSeqRef: { current: 0 }, mountedRef: { current: true },
    memberAccounts: [{ id: 'account' }], pricingRepriceActive: false,
    modelPricingRows: [{ modelId: 'model', inputUsdPerMillion: 1, outputUsdPerMillion: 2, hasPreset: false, custom: false }],
    updateModelPricingDraft: (draft: object, field: string, value: string) => ({ ...draft, [field]: value }),
    t: (key: string) => key, presentWindowsOperationError: () => false,
  };
  const source = `${helpers}\n(() => { ${statements.map((node) => node.getText(file)).join('\n')}\nreturn { ${names.join(',')} }; })`;
  const render = vm.runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  h.render(render);
  return { h, context, calls, read: () => h.flush(), async refresh() { await h.flush().reloadState(); return h.flush(); },
    change(patch: object) { Object.assign(server.collection, patch); }, fail(value: boolean) { failure = value; },
  };
}

test('status refresh preserves port/proxy edits and independently hydrates untouched fields; saves use the draft', async () => {
  const p = controller(); await p.refresh();
  p.read().setPortInput('9000'); await p.refresh(); assert.equal(p.read().portInput, '9000');
  p.change({ upstreamProxyUrl: 'http://updated.invalid' }); await p.refresh();
  assert.equal(p.read().proxyInput, 'http://updated.invalid');
  p.read().setProxyInput('http://edited.invalid'); p.change({ port: 8318 }); await p.refresh();
  assert.equal(p.read().portInput, '9000'); assert.equal(p.read().proxyInput, 'http://edited.invalid');
  p.fail(true); await p.read().handleSavePort(); await p.refresh();
  assert.equal(p.read().portInput, '9000'); assert.match(p.read().error, /save failed/);
  p.fail(false); await p.read().handleSavePort(); await p.read().handleSaveProxy();
  assert.equal(p.calls[p.calls.length - 2].args[0], 9000);
  assert.equal(p.calls[p.calls.length - 1].args[0], 'http://edited.invalid'); p.h.unmount();
});

test('API Key label edits survive refresh, preserve other key updates and save the edited value', async () => {
  const p = controller(); await p.refresh();
  p.read().setApiKeyDrafts((drafts: object) => ({ ...drafts, a: 'Edited A' }));
  p.change({ apiKeys: [{ id: 'a', label: 'A' }, { id: 'b', label: 'Updated B' }, { id: 'c', label: 'New C' }] });
  await p.refresh(); assert.equal(p.read().apiKeyDrafts.a, 'Edited A');
  assert.equal(p.read().apiKeyDrafts.b, 'Updated B'); assert.equal(p.read().apiKeyDrafts.c, 'New C');
  p.fail(true); await p.read().handleSaveApiKeyLabel('a', 'A'); await p.refresh(); assert.equal(p.read().apiKeyDrafts.a, 'Edited A');
  p.fail(false); await p.read().handleSaveApiKeyLabel('a', 'A');
  assert.equal(p.calls[p.calls.length - 1].args[1].label, 'Edited A'); p.h.unmount();
});

test('unrelated and changed configuration refreshes preserve model rules and routing edits independently', async () => {
  const p = controller(); await p.refresh();
  p.read().setModelAliasesText('source => alias'); p.read().setExcludedModelsText('blocked');
  p.read().setSessionAffinityTtlDraft('120'); p.read().setSessionAffinityDraft(false);
  p.change({ modelAliases: [], excludedModels: [], sessionAffinityTtlMs: 1800000, maxRetryCredentials: 3 });
  await p.refresh(); assert.equal(p.read().modelAliasesText, 'source => alias'); assert.equal(p.read().excludedModelsText, 'blocked');
  assert.equal(p.read().sessionAffinityTtlDraft, '120'); assert.equal(p.read().sessionAffinityDraft, false);
  assert.equal(p.read().maxRetryCredentialsDraft, '3');
  await p.read().handleSaveRoutingOptions(); assert.equal(p.calls[p.calls.length - 1].args[0].sessionAffinityTtlMs, 120000);
  await p.read().handleSaveModelRules(); assert.equal(p.calls[p.calls.length - 1].args[0][0].alias, 'alias');
  assert.equal(p.calls[p.calls.length - 1].args[1][0], 'blocked'); p.h.unmount();
});

test('account model rules retain draft, bulk input and selection on refresh and failure, then reset on reopen', async () => {
  const p = controller(); await p.refresh(); p.read().handleOpenAccountModelRules();
  p.read().setAccountModelRuleDrafts({ account: 'edited' }); p.read().setAccountModelRuleSelected(new Set(['account']));
  p.read().setAccountModelRuleBulkText('bulk'); await p.refresh();
  assert.equal(p.read().accountModelRuleDrafts.account, 'edited'); assert.equal(p.read().accountModelRuleBulkText, 'bulk');
  assert.equal(p.read().accountModelRuleSelected.has('account'), true);
  p.fail(true); await p.read().handleSaveAccountModelRules(); await p.refresh();
  assert.equal(p.read().accountModelRulesOpen, true); assert.equal(p.read().accountModelRuleDrafts.account, 'edited');
  assert.equal(p.read().accountModelRuleError, 'save failed'); assert.equal(p.read().error, '');
  p.fail(false); await p.read().handleSaveAccountModelRules(); assert.equal(p.read().accountModelRulesOpen, false);
  assert.equal(p.calls[p.calls.length - 1].args[0][0].excludedModels[0], 'edited');
  p.change({ accountModelRules: [{ accountId: 'account', excludedModels: ['latest'] }] }); await p.refresh();
  p.read().handleOpenAccountModelRules(); assert.equal(p.read().accountModelRuleDrafts.account, 'latest');
  assert.equal(p.read().accountModelRuleSelected.size, 0); p.h.unmount();
});

test('price edits and validation errors survive refreshed model objects; save and reopen use correct values', async () => {
  const p = controller(); await p.refresh(); p.read().handleOpenPricingModal();
  p.read().updatePricingDraft('model', 'inputUsdPerMillion', 'invalid'); await p.read().handleSaveModelPricings();
  const error = p.read().pricingError; assert.ok(error);
  p.context.modelPricingRows = structuredClone(p.context.modelPricingRows); await p.refresh();
  assert.equal(p.read().pricingModalOpen, true); assert.equal(p.read().pricingDrafts[0].inputUsdPerMillion, 'invalid');
  assert.equal(p.read().pricingError, error);
  p.read().updatePricingDraft('model', 'inputUsdPerMillion', '9');
  p.fail(true); await p.read().handleSaveModelPricings(); await p.refresh();
  assert.equal(p.read().pricingModalOpen, true); assert.equal(p.read().pricingDrafts[0].inputUsdPerMillion, '9');
  assert.equal(p.read().pricingError, 'save failed'); assert.equal(p.read().error, '');
  p.fail(false); await p.read().handleSaveModelPricings(); assert.equal(p.calls[p.calls.length - 1].args[0][0].inputUsdPerMillion, 9);
  assert.equal(p.read().pricingDrafts[0].custom, true);
  p.read().setPricingModalOpen(false); p.h.flush(); p.context.modelPricingRows[0].inputUsdPerMillion = 7;
  p.read().handleOpenPricingModal(); assert.equal(p.read().pricingDrafts[0].inputUsdPerMillion, '7');
  assert.equal(p.read().pricingError, ''); p.h.unmount();
});

test('timeout drafts and selected presets survive refresh and failed save; reopen hydrates latest timeouts', async () => {
  const p = controller(); await p.refresh(); p.read().openTimeoutSettings();
  p.read().updateTimeoutDraft('sidecarStreamOpenTimeoutMs', 'invalid'); await p.read().handleSaveTimeouts();
  const error = p.read().timeoutsError; assert.ok(error);
  p.read().setSelectedTimeoutPresetId('custom'); await p.refresh();
  assert.equal(p.read().timeoutDrafts.sidecarStreamOpenTimeoutMs, 'invalid');
  assert.equal(p.read().timeoutsError, error); assert.equal(p.read().selectedTimeoutPresetId, 'custom');
  p.read().updateTimeoutDraft('sidecarStreamOpenTimeoutMs', '25'); p.fail(true);
  await p.read().handleSaveTimeouts(); await p.refresh();
  assert.equal(p.read().timeoutsModalOpen, true); assert.equal(p.read().timeoutDrafts.sidecarStreamOpenTimeoutMs, '25');
  assert.equal(p.read().timeoutsError, 'save failed'); assert.equal(p.read().error, '');
  p.fail(false); await p.read().handleSaveTimeouts(); assert.equal(p.read().timeoutsModalOpen, false);
  assert.equal(p.calls[p.calls.length - 1].args[0].sidecarStreamOpenTimeoutMs, 25000);
  p.change({ timeouts: { sidecarStreamOpenTimeoutMs: 45000 }, activeTimeoutPresetId: 'short_wait' }); await p.refresh();
  p.read().openTimeoutSettings(); assert.equal(p.read().timeoutDrafts.sidecarStreamOpenTimeoutMs, '45');
  assert.equal(p.read().selectedTimeoutPresetId, 'short_wait'); assert.equal(p.read().timeoutsError, ''); p.h.unmount();
});
