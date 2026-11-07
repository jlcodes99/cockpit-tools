const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

const script = fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8');
const { parts, fixture } = require('./fast_fixture_current.cjs');

function load(settings = {}) {
  const sandbox = { module: { exports: {} }, URL, Blob, setTimeout, clearTimeout,
    __cockpitEnhancements: { nativeFast: true, quotaCompat: false, ...settings } };
  sandbox.window = sandbox;
  vm.runInNewContext(script, sandbox);
  return { sandbox, api: sandbox.module.exports };
}

function scenario(auth = 'apiKey', host = 'local', settings = {}) {
  const { sandbox, api } = load(settings);
  const state = {
    requirements: { serviceTiers: [null, 'fast', 'ultrafast'], marker: 'existing native requirements' },
    pending: false, identityLoading: false, copilot: false,
    accountReads: 0, requirementReads: 0, copilotReads: 0, cached: [],
  };
  Object.assign(sandbox, {
    defaultHost: host,
    readIdentity: () => ({ authMethod: auth, isLoading: state.identityLoading }),
    readRequirements: () => ({ data: state.requirements, isPending: state.pending }),
    selectAccess: (value) => value,
    readAuth: async () => auth,
    fetchRequirements: async () => { state.requirementReads++; return state.requirements; },
    cacheKey: 'native-requirements',
    resolveTier: async (_client, _logger, model, eligible) => await eligible()
      && state.requirements?.serviceTiers.includes(model.selected)
      && (model.selected == null || model.serviceTiers.some((tier) => tier.id === model.selected))
        ? model.selected : null,
  });
  const transformed = api.transform(fixture);
  assert.equal(transformed.ok, true);
  vm.runInNewContext(transformed.patchedSource, sandbox);
  const manager = { getHostId: () => host, getAccount: async () => {
    state.accountReads++;
    return { account: auth == null ? null : { type: auth } };
  } };
  const dependencies = {
    runtime: { isUsingCopilotInference: async () => { state.copilotReads++; return state.copilot; } },
    storage: { readGlobalState: async () => { throw Error('26.1021 must use the runtime Copilot selection contract'); } },
    settings: { readPermissionRefreshEnabled: async () => true },
  };
  const scope = { query: { setData: (...args) => state.cached.push(args) } };
  const model = { model: 'gpt-6-astra', serviceTiers: [{ id: 'fast' }, { id: 'ultrafast' }] };
  const queue = sandbox.queued(manager, dependencies, () => {}, () => {});
  const submit = sandbox.explicit(manager, dependencies, async () => 'native-default');
  return { api, sandbox, state, manager, dependencies, scope, model,
    queued: (tier) => queue.readServiceTier({ ...model, selected: tier }),
    explicit: (tier, executionHostId = host) => submit(null, {
      context: {}, submissionOptions: { serviceTier: tier, executionHostId },
    }),
  };
}

test('26.1021 matches each complete gate exactly once and rejects missing or repeated gates', () => {
  const { api } = load();
  const result = api.transform(fixture);
  assert.equal(result.ok, true);
  assert.deepEqual({ ...result.diagnostics.gates }, { ui: 1, request: 1, queued: 1, explicit: 1 });
  for (const key of Object.keys(parts)) {
    for (const source of [Object.entries(parts).filter(([name]) => name !== key).map(([, value]) => value).join('\n'), fixture + '\n' + parts[key]]) {
      const rejected = api.transform(source);
      assert.equal(rejected.ok, false, key);
      assert.equal(rejected.patchedSource, source, key);
    }
  }
});

test('26.1021 does not accept arbitrary auth expressions, unknown runtime contracts or shadowed bindings', () => {
  const { api } = load();
  for (const source of [
    fixture.replace('identity?.authMethod===`amazonBedrock`', 'identity?.authMethod===`amazonBedrock`||identity?.authMethod===`futureAuth`'),
    fixture.replace('auth!==`amazonBedrock`', 'auth!==`amazonBedrock`&&externalPolicy(auth)'),
    fixture.replace('await runtime.isUsingCopilotInference()', 'await runtime.unknownCopilotPolicy()'),
    fixture.replace('result.account?.type===`amazonBedrock`', 'result.account?.type===`amazonBedrock`||externalPolicy(result)'),
    fixture.replace('account?.type===`amazonBedrock`?options.serviceTier', 'account?.type===`futureAuth`?options.serviceTier'),
    fixture.replace('let{account:account}=await manager.getAccount', 'let{account:manager}=await manager.getAccount')
      .replace('return account?.type===`chatgpt`||account?.type===`amazonBedrock`?', 'return manager?.type===`chatgpt`||manager?.type===`amazonBedrock`?'),
  ]) {
    const rejected = api.transform(source);
    assert.equal(rejected.ok, false);
    assert.equal(rejected.patchedSource, source);
  }
});

for (const auth of ['apiKey', 'apikey']) {
  test(`26.1021 ${auth} retains the three native speed choices and sends exact queued and explicit values`, async () => {
    const env = scenario(auth);
    const requirements = env.state.requirements;
    const before = JSON.stringify(env.model);
    assert.equal(env.sandbox.ui({}).serviceTierAccess, requirements);
    assert.equal(await env.sandbox.request(env.scope, 'local'), requirements);
    assert.equal(env.state.cached[0][2], requirements);
    for (const tier of [null, 'fast', 'ultrafast']) {
      assert.equal(await env.queued(tier), tier);
      assert.equal(await env.explicit(tier), tier);
    }
    assert.equal(await env.queued('unsupported-tier'), null);
    assert.equal(env.state.accountReads, 6, 'queue reads once per attempt; explicit standard requires no account read');
    assert.equal(JSON.stringify(env.model), before, 'do not rewrite model serviceTiers');
    assert.deepEqual(requirements.serviceTiers, [null, 'fast', 'ultrafast']);
  });
}

test('26.1021 keeps missing and pending native requirements unavailable', async () => {
  const env = scenario();
  env.state.requirements = null;
  assert.equal(env.sandbox.ui({}).serviceTierAccess, null);
  assert.equal(await env.sandbox.request(env.scope, 'local'), null);
  assert.equal(await env.queued('fast'), null);
  env.state.requirements = { serviceTiers: [null], marker: 'no model fast support' };
  assert.equal(await env.queued('fast'), null);
  env.state.pending = true;
  assert.equal(env.sandbox.ui({}).serviceTierAccess, null);
  assert.equal(env.sandbox.ui({}).isLoading, true);
  env.state.pending = false;
  env.state.identityLoading = true;
  assert.equal(env.sandbox.ui({}).serviceTierAccess, null);
});

test('26.1021 missing auth is admitted only for a host-verified local API profile', async () => {
  for (const verified of [false, true]) {
    const env = scenario(null, 'local', { apiKeyAuthVerified: verified });
    assert.equal(env.sandbox.ui({}).serviceTierAccess, verified ? env.state.requirements : null);
    assert.equal(await env.sandbox.request(env.scope, 'local'), verified ? env.state.requirements : null);
    assert.equal(await env.queued('ultrafast'), verified ? 'ultrafast' : null);
    assert.equal(await env.explicit('ultrafast'), verified ? 'ultrafast' : null);
  }
});

test('26.1021 remote API identity and remote explicit execution stay blocked', async () => {
  for (const auth of ['apiKey', null]) {
    const env = scenario(auth, 'remote-host', { apiKeyAuthVerified: true });
    assert.equal(env.sandbox.ui({}).serviceTierAccess, null);
    assert.equal(await env.sandbox.request(env.scope, 'remote-host'), null);
    assert.equal(env.state.requirementReads, 0);
    assert.equal(await env.queued('fast'), null);
    assert.equal(await env.explicit('fast'), null);
  }
  const env = scenario();
  assert.equal(await env.explicit('fast', 'remote-host'), null);
});

test('26.1021 runtime Copilot selection blocks API queue and explicit submission', async () => {
  const env = scenario();
  env.state.copilot = true;
  assert.equal(await env.queued('ultrafast'), null);
  assert.equal(env.state.accountReads, 0, 'native Copilot early return stays before account/read');
  assert.equal(await env.explicit('ultrafast'), null);
  assert.equal(env.state.accountReads, 1);
  assert.equal(env.state.copilotReads, 2);
});

test('26.1021 preserves OAuth, personal-token and Bedrock native policy without admitting other auth', async () => {
  for (const auth of ['chatgpt', 'personalAccessToken', 'amazonBedrock', 'copilot', 'chatgptAuthTokens', 'unknown']) {
    const env = scenario(auth, 'local', { apiKeyAuthVerified: true });
    const hasUi = ['chatgpt', 'personalAccessToken', 'amazonBedrock'].includes(auth);
    const hasQueue = ['chatgpt', 'amazonBedrock'].includes(auth);
    assert.equal(env.api.allow('local', auth), false, auth);
    assert.equal(env.sandbox.ui({}).serviceTierAccess, hasUi ? env.state.requirements : null, auth);
    assert.equal(await env.sandbox.request(env.scope, 'local'), hasUi ? env.state.requirements : null, auth);
    assert.equal(await env.queued('fast'), hasQueue ? 'fast' : null, auth);
    assert.equal(await env.explicit('fast'), hasQueue ? 'fast' : null, auth);
    env.sandbox.__cockpitEnhancements.nativeFast = false;
    assert.equal(env.sandbox.ui({}).serviceTierAccess, hasUi ? env.state.requirements : null, auth);
    assert.equal(await env.explicit('ultrafast'), hasQueue ? 'ultrafast' : null, auth);
  }
});

test('26.1021 leaves native remote OAuth and Bedrock service tiers available', async () => {
  for (const auth of ['chatgpt', 'amazonBedrock']) {
    const env = scenario(auth, 'remote-host');
    assert.equal(env.api.allow('remote-host', auth), false);
    assert.equal(env.sandbox.ui({}).serviceTierAccess, env.state.requirements);
    assert.equal(await env.sandbox.request(env.scope, 'remote-host'), env.state.requirements);
    assert.equal(await env.queued('ultrafast'), 'ultrafast');
    assert.equal(await env.explicit('ultrafast'), 'ultrafast');
    assert.equal(env.state.copilotReads, 0, 'retain the native local-only Copilot guard');
  }
});

test('26.1021 runtime disabling and API-auth revocation close all extension gates', async () => {
  for (const revokeVerified of [false, true]) {
    const env = scenario(revokeVerified ? null : 'apiKey', 'local', { apiKeyAuthVerified: true });
    if (revokeVerified) env.sandbox.__cockpitEnhancements.apiKeyAuthVerified = false;
    else env.sandbox.__cockpitEnhancements.nativeFast = false;
    assert.equal(env.sandbox.ui({}).serviceTierAccess, null);
    assert.equal(await env.sandbox.request(env.scope, 'local'), null);
    assert.equal(await env.queued('fast'), null);
    assert.equal(await env.explicit('fast'), null);
  }
  const { api } = load({ nativeFast: false });
  const disabled = api.transform(fixture);
  assert.equal(disabled.ok, false);
  assert.equal(disabled.patchedSource, fixture);
});
