const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const { fixture } = require('./fast_fixture_current.cjs');

const script = fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8');
const catalog = JSON.parse(fs.readFileSync(path.join(__dirname,
  '../../../../resources/codex/codex_client_models.json'), 'utf8'));
const efforts = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'];

// Independently authored model-query contracts. The current result binding
// deliberately shadows host, as desktop 26.1027 does. Never execute client code.
function queryFixture(current = true) {
  const options = 'selectModels({additionalAvailableModels:new Set(additional),apiKeyDaybreakSupported:daybreak,authMethod:auth,availableModels:defaults.availableModels,defaultModel:defaults.defaultModel,enabledReasoningEfforts:efforts,hasConfiguredModelCatalog:catalog,includeUltraReasoningEffort:ultra,isCustomModelProvider:custom,models:models,useHiddenModels:defaults.useHiddenModels})';
  const select = current
    ? `({data:models,isTeamCatalog:host=!1})=>({...${options},isTeamCatalog:host})`
    : `({data:models})=>${options}`;
  return 'var makeQuery=register(({additionalAvailableModels:additional,authMethod:auth,enabledReasoningEfforts:efforts,hasConfiguredModelCatalog:catalog,hostId:host,includeUltraReasoningEffort:requested,isCustomModelProvider:custom,limit:limit,modelCatalogPath:catalogPath,modelProvider:provider'
    + (current ? ',teamCatalog:teamCatalog' : '')
    + '},{get:read,queryClient:client,scope:scope})=>{let daybreak=false,defaults=read(defaultsKey),ultra=requested&&read(gate,`1186680773`);return{queryKey:[],queryFn:()=>{let priority=`interactive`;return manager(scope,host).sendRequest(`model/list`,{includeHidden:!0,cursor:null,limit:limit},{priority:priority})},select:'
    + select + '}});';
}

function load(current = true, transform = true, settings = {}, source = queryFixture(current)) {
  const sandbox = { module: { exports: {} }, URL, gate: 'gate', defaultsKey: 'defaults',
    __cockpitEnhancements: { nativeFast: true, ...settings }, register: fn => fn,
    // Native filtering semantics: both inclusion policy and explicit user
    // efforts apply, without inventing any model capabilities or defaults.
    selectModels: options => ({ models: options.models.map(model => ({ ...model,
      supportedReasoningEfforts: model.supportedReasoningEfforts.filter(level =>
        (options.includeUltraReasoningEffort || level.reasoningEffort !== 'ultra')
          && options.enabledReasoningEfforts.has(level.reasoningEffort)),
    })) }) };
  sandbox.window = sandbox;
  vm.runInNewContext(script, sandbox);
  const api = sandbox.module.exports;
  const result = api.transform(fixture + '\n' + source);
  assert.equal(result.ok, true);
  vm.runInNewContext(transform ? result.patchedSource : fixture + '\n' + source, sandbox);
  return { api, sandbox, diagnostics: result.diagnostics,
    select(data, overrides = {}, nativeGate = false, isTeamCatalog = false) {
      const args = { additionalAvailableModels: [], authMethod: 'apikey',
        enabledReasoningEfforts: new Set(efforts), hasConfiguredModelCatalog: true,
        hostId: 'local', includeUltraReasoningEffort: true, isCustomModelProvider: true,
        limit: 100, modelCatalogPath: 'managed-catalog.json', modelProvider: 'cockpit',
        teamCatalog: null, ...overrides };
      const query = sandbox.makeQuery(args, { get: key => key === 'gate' ? nativeGate
        : { availableModels: [], defaultModel: null, useHiddenModels: false },
      queryClient: {}, scope: {} });
      return query.select({ data, isTeamCatalog });
    } };
}

function models() {
  return ['gpt-6.1-sol', 'gpt-6-astra', 'gpt-6-sol', 'gpt-6-luna'].map(slug => {
    const model = catalog.models.find(model => model.slug === slug);
    return { model: slug, defaultReasoningEffort: model.default_reasoning_level,
      supportedReasoningEfforts: model.supported_reasoning_levels.map(level => ({
        reasoningEffort: level.effort, description: level.description,
      })), serviceTiers: [{ id: 'priority' }] };
  });
}
const levels = model => Array.from(model.supportedReasoningEfforts, level => level.reasoningEffort);

for (const current of [false, true]) {
  test(`${current ? 'current' : 'legacy'} query: reproduce hidden Ultra, restore catalog capabilities with native gate off`, () => {
    const input = models(), before = JSON.stringify(input);
    const native = load(current, false).select(input);
    assert.equal(levels(native.models[0]).includes('ultra'), false);
    const env = load(current), output = env.select(input);
    assert.equal(env.diagnostics.modelCatalog, 1);
    assert.equal(env.diagnostics.reasoning, 1);
    output.models.forEach((model, i) => {
      assert.deepEqual(levels(model), levels(input[i]));
      assert.equal(model.defaultReasoningEffort, input[i].defaultReasoningEffort);
    });
    assert.equal(JSON.stringify(input), before, 'native query cache remains untouched');
    assert.equal(levels(output.models[3]).includes('ultra'), false, 'Luna does not gain Ultra');
    assert.deepEqual(levels(env.select(input, { modelProvider: 'cockpit-remote',
      modelCatalogPath: 'remote-catalog.json' }).models[0]), levels(input[0]), 'LAN uses the same selector');
  });
}

test('honor caller exclusions, user efforts, restricted catalogs and native defaults', () => {
  const env = load(), input = models();
  for (const overrides of [{ includeUltraReasoningEffort: false },
    { enabledReasoningEfforts: new Set(['low', 'max']) }]) {
    assert.equal(levels(env.select(input, overrides).models[0]).includes('ultra'), false);
  }
  input[0].supportedReasoningEfforts = [{ reasoningEffort: 'high', description: 'custom' }];
  assert.deepEqual(levels(env.select(input).models[0]), ['high']);
  assert.equal(env.select(input).models[0].supportedReasoningEfforts[0], input[0].supportedReasoningEfforts[0]);
});

test('preserve OAuth, Copilot, remote execution, first-party and team catalog policies', () => {
  const env = load(), input = models();
  for (const overrides of [
    ...['chatgpt', 'personalAccessToken', 'amazonBedrock', 'copilot', 'unknown', null].map(authMethod => ({ authMethod })),
    { hostId: 'remote' }, { isCustomModelProvider: false }, { teamCatalog: 'loading' },
    { teamCatalog: { accountId: 'team', userId: 'user' } },
  ]) {
    assert.equal(levels(env.select(input, overrides).models[0]).includes('ultra'), false);
    assert.equal(levels(env.select(input, overrides, true).models[0]).includes('ultra'), true);
  }
  env.sandbox.__cockpitEnhancements.nativeFast = false;
  assert.equal(levels(env.select(input).models[0]).includes('ultra'), false);
});

test('missing API auth uses profile verification and never applies to remote execution', () => {
  const env = load(true, true, { apiKeyAuthVerified: true }), input = models();
  assert.equal(levels(env.select(input, { authMethod: null }).models[0]).includes('ultra'), true);
  assert.equal(levels(env.select(input, { authMethod: null, hostId: 'remote' }).models[0]).includes('ultra'), false);
});

test('current selector captures host before result destructuring and preserves team result metadata', () => {
  const tier = { id: 'ultrafast' };
  const env = load(true, true, { serviceTierFallbacks: { 'gpt-6.1-sol': tier } }), input = models();
  const result = env.select(input, { hasConfiguredModelCatalog: false });
  assert.equal(result.models[0].serviceTiers[1].id, 'ultrafast');
  assert.equal(result.isTeamCatalog, false);
  const team = env.select(input, { hasConfiguredModelCatalog: false, teamCatalog: 'team' }, false, true);
  assert.equal(team.isTeamCatalog, true);
  assert.equal(team.models[0].serviceTiers, input[0].serviceTiers);
  assert.equal(levels(team.models[0]).includes('ultra'), false);
});

test('unknown query/gate contracts stay native; duplicate selectors reject the whole transform', () => {
  for (const source of [queryFixture().replace('1186680773', 'unrecognized'),
    queryFixture().replace('includeUltraReasoningEffort:requested', 'otherFlag:requested')]) {
    const env = load(true, true, {}, source);
    assert.equal(env.diagnostics.reasoning, 0);
    assert.equal(levels(env.select(models()).models[0]).includes('ultra'), false);
  }
  const { api } = load();
  const source = fixture + queryFixture() + queryFixture(false);
  const result = api.transform(source);
  assert.equal(result.ok, false);
  assert.equal(result.patchedSource, source);
  assert.equal(result.diagnostics.reason, 'ambiguous-model-catalog');
});
