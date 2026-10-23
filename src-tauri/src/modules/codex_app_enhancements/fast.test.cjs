const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

const script = fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8');
const moduleUrl = 'app://-/assets/app-initial-fixture.js';
const fixture = `
function ui(argument){let memo=0,fallback=defaultHost,host=argument?.hostId??fallback,identity=readIdentity(host),allowed=identity?.authMethod===\`chatgpt\`||identity?.authMethod===\`personalAccessToken\`,method=identity?.authMethod??null,key={authMethod:method,hostId:host};let{data:requirements,isPending:pending}=readRequirements(0,key),loading=!!identity?.isLoading||allowed&&pending,result=allowed&&!loading&&requirements!=null?selectAccess(requirements):null;return{serviceTierAccess:result,isLoading:loading}}
async function request(scope,host){let auth=await readAuth(scope,host);if(auth!==\`chatgpt\`&&auth!==\`personalAccessToken\`)return null;let requirements=await fetchRequirements(scope,host,{priority:\`critical\`});return scope.query.setData(cacheKey,{authMethod:auth,hostId:host},requirements),selectAccess(requirements)}
function queued(manager,{runtime:runtime,storage:storage},before,changed){let pending=null;return{async readServiceTier(model){for(;pending!=null;)await pending.catch(()=>void 0);return before(),resolveTier(manager.requestClient,manager.logger,model,async()=>manager.getHostId()===\`local\`&&await storage.readGlobalState(\`use-copilot-auth-if-available\`)===!0&&await runtime.isCopilotApiAvailable()?!1:(await manager.getAccount({priority:\`critical\`})).account?.type===\`chatgpt\`)},async writeServiceTier(){}}}
function explicit(manager,{settings:settings,storage:storage,filesystem:filesystem,runtime:runtime},readDefault,ignored,alsoIgnored){return async(id,message)=>{let options=message.submissionOptions,host=options?.executionHostId??manager.getHostId();let[tier,permission]=await Promise.all([(async()=>{if(options==null)return readDefault(null);if(options.serviceTier==null)return null;let{account:account}=await manager.getAccount({priority:\`critical\`});return account?.type===\`chatgpt\`?options.serviceTier:null})(),options?.usePermissionSelection??settings.readPermissionRefreshEnabled()]);return tier}}
`;

function load(options = {}) {
  const sandbox = { module: { exports: {} }, URL, Blob, setTimeout, clearTimeout,
    __cockpitEnhancements: { nativeFast: true, quotaCompat: false, ...options } };
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  vm.runInNewContext(script, sandbox);
  return { api: sandbox.module.exports, sandbox };
}

function evaluateFixture(auth = 'apiKey', host = 'local', options = {}) {
  const { api, sandbox } = load(options);
  const requirements = { serviceTiers: ['default', 'fast'], token: 'existing-requirements' };
  Object.assign(sandbox, {
    defaultHost: host,
    readIdentity: () => ({ authMethod: auth, isLoading: false }),
    readRequirements: () => ({ data: requirements, isPending: false }),
    selectAccess: value => value,
    readAuth: async () => auth,
    fetchRequirements: async () => requirements,
    cacheKey: 'native-requirements',
    resolveTier: async (_client, _logger, model, allowed) => await allowed() && model.serviceTiers.includes(model.selected) ? model.selected : null,
  });
  const transformed = api.transform(fixture);
  assert.equal(transformed.ok, true);
  vm.runInNewContext(transformed.patchedSource, sandbox);
  const manager = { getHostId: () => host, getAccount: async () => ({ account: { type: auth } }) };
  const dependencies = {
    runtime: { isCopilotApiAvailable: async () => false },
    storage: { readGlobalState: async () => false },
    settings: { readPermissionRefreshEnabled: async () => true },
  };
  return { api, sandbox, requirements, manager, dependencies };
}

test('four structural gates must all match exactly once', () => {
  const { api } = load();
  const result = api.transform(fixture);
  assert.equal(result.ok, true);
  assert.deepEqual({ ...result.diagnostics.gates }, { ui: 1, request: 1, queued: 1, explicit: 1 });
  for (const source of [fixture.replace('serviceTierAccess:', 'otherAccess:'), fixture + fixture, 'unknown version']) {
    const unsupported = api.transform(source);
    assert.equal(unsupported.ok, false);
    assert.equal(unsupported.patchedSource, source);
  }
});

test('disabled compatibility leaves the source unchanged', () => {
  const { api } = load({ nativeFast: false });
  assert.equal(api.transform(fixture).patchedSource, fixture);
  assert.equal(api.transform(fixture).ok, false);
});

test('local API key retains requirements and chosen supported service tier', async () => {
  const { sandbox, manager, dependencies, requirements } = evaluateFixture();
  assert.equal(sandbox.ui({}).serviceTierAccess, requirements);
  const scope = { query: { setData() {} } };
  assert.equal(await sandbox.request(scope, 'local'), requirements);
  const queue = sandbox.queued(manager, dependencies, () => {}, () => {});
  assert.equal(await queue.readServiceTier({ serviceTiers: ['default', 'fast'], selected: 'fast' }), 'fast');
  assert.equal(await queue.readServiceTier({ serviceTiers: ['default'], selected: 'fast' }), null);
  assert.equal(await sandbox.explicit(manager, dependencies, async () => 'default')(null, { submissionOptions: { serviceTier: 'fast' } }), 'fast');
});

test('host-verified API profile admits a missing renderer auth method without replacing requirements', async () => {
  const { sandbox, api, requirements } = evaluateFixture(null, 'local', { apiKeyAuthVerified: true });
  assert.equal(api.allow('local', null), true);
  assert.equal(api.allow('local', undefined), true);
  assert.equal(sandbox.ui({}).serviceTierAccess, requirements);
  assert.equal(sandbox.readIdentity().authMethod, null, 'do not rewrite the native identity');
  assert.equal(await sandbox.request({ query: { setData() {} } }, 'local'), requirements);

  sandbox.readRequirements = () => ({ data: null, isPending: false });
  assert.equal(sandbox.ui({}).serviceTierAccess, null, 'missing requirements must still fail closed');
  sandbox.readRequirements = () => ({ data: requirements, isPending: true });
  assert.equal(sandbox.ui({}).serviceTierAccess, null, 'pending requirements must not expose speed options');
  assert.equal(sandbox.ui({}).isLoading, true);
});

test('verified API fallback never admits remote or conflicting auth methods and closes on revocation', () => {
  const { sandbox, api } = evaluateFixture(null, 'local', { apiKeyAuthVerified: true });
  for (const auth of ['chatgpt', 'personalAccessToken', 'copilot', 'amazonBedrock', 'unknown', '']) {
    assert.equal(api.allow('local', auth), false, auth);
  }
  assert.equal(api.allow('remote-machine', null), false);
  sandbox.__cockpitEnhancements.apiKeyAuthVerified = false;
  assert.equal(api.allow('local', null), false);
  assert.equal(sandbox.ui({}).serviceTierAccess, null);
  sandbox.__cockpitEnhancements.apiKeyAuthVerified = true;
  sandbox.__cockpitEnhancements.nativeFast = false;
  assert.equal(api.allow('local', null), false);
});

test('remote API hosts remain blocked by all four gates', async () => {
  const { sandbox, manager, dependencies } = evaluateFixture('apiKey', 'remote-machine');
  assert.equal(sandbox.ui({}).serviceTierAccess, null);
  assert.equal(await sandbox.request({ query: { setData() { throw Error('must not query'); } } }, 'remote-machine'), null);
  assert.equal(await sandbox.queued(manager, dependencies, () => {}).readServiceTier({ serviceTiers: ['fast'], selected: 'fast' }), null);
  assert.equal(await sandbox.explicit(manager, dependencies, async () => null)(null, { submissionOptions: { serviceTier: 'fast' } }), null);
});

test('OAuth and personal-token policy is not broadened by API compatibility', async () => {
  for (const auth of ['chatgpt', 'personalAccessToken']) {
    const { sandbox, api, manager, dependencies, requirements } = evaluateFixture(auth);
    assert.equal(api.allow('local', auth), false);
    assert.equal(sandbox.ui({}).serviceTierAccess, requirements);
    const expected = auth === 'chatgpt' ? 'fast' : null;
    assert.equal(await sandbox.explicit(manager, dependencies, async () => null)(null, { submissionOptions: { serviceTier: 'fast' } }), expected);
  }
});

test('runtime disabling immediately closes already transformed API gates', async () => {
  const { sandbox, manager, dependencies } = evaluateFixture();
  sandbox.__cockpitEnhancements.nativeFast = false;
  assert.equal(sandbox.ui({}).serviceTierAccess, null);
  assert.equal(await sandbox.queued(manager, dependencies, () => {}).readServiceTier({ serviceTiers: ['fast'], selected: 'fast' }), null);
  assert.equal(await sandbox.explicit(manager, dependencies, async () => null)(null, { submissionOptions: { serviceTier: 'fast' } }), null);
});

test('explicit remote execution and selected Copilot cannot reuse local API identity', async () => {
  const { sandbox, manager, dependencies } = evaluateFixture();
  const submit = sandbox.explicit(manager, dependencies, async () => null);
  assert.equal(await submit(null, { submissionOptions: { executionHostId: 'remote', serviceTier: 'fast' } }), null);
  dependencies.storage.readGlobalState = async () => true;
  dependencies.runtime.isCopilotApiAvailable = async () => true;
  assert.equal(await submit(null, { submissionOptions: { serviceTier: 'fast' } }), null);
  assert.equal(await sandbox.queued(manager, dependencies, () => {}).readServiceTier({ serviceTiers: ['fast'], selected: 'fast' }), null);
});

test('module rewriting preserves ordinary literals and rewrites actual module edges', async () => {
  const { api } = load();
  const source = 'import a from "./a.js";export{a}from"../b.js";const plain="./keep.txt", pattern=/import[(]"x"[)]/;const object={import:()=>import(`./c.js`)};const href=import.meta.url;const nested=`text ${import("./d.js")}`;';
  const rewritten = await api.rewriteModule(source, moduleUrl);
  assert.equal(api.rewriteModuleSync(source, moduleUrl), rewritten);
  assert.match(rewritten, /import a from "app:\/\/-\/assets\/a.js"/);
  assert.match(rewritten, /from"app:\/\/-\/b.js"/);
  assert.match(rewritten, /import\("app:\/\/-\/assets\/c.js"\)/);
  assert.match(rewritten, /import\("app:\/\/-\/assets\/d.js"\)/);
  assert.match(rewritten, /const href="app:\/\/-\/assets\/app-initial-fixture.js"/);
  assert.ok(rewritten.includes('plain="./keep.txt"'));
  assert.ok(rewritten.includes('pattern=/import[(]"x"[)]/'));
  for (const unsupported of ['import(computed)', 'import("./"+suffix)', 'const p=import.meta.resolve("./x.js")']) {
    await assert.rejects(() => api.rewriteModule(unsupported, moduleUrl));
    assert.throws(() => api.rewriteModuleSync(unsupported, moduleUrl));
  }
});

test('remote module URLs and unsupported source stay untouched', async () => {
  const { api } = load();
  for (const url of ['https://example.com/assets/app-initial-a.js', 'app://remote/assets/app-initial-a.js', 'app://-/assets/app-initial-a.js?token=redacted']) {
    const result = await api.prepareModule(fixture, url);
    assert.equal(result.ok, false);
    assert.equal(result.patchedSource, fixture);
  }
  assert.equal((await api.prepareModule('unknown', moduleUrl)).patchedSource, 'unknown');
});

test('combined quota and tier adaptation is atomic', async () => {
  const { api, sandbox } = load({ quotaCompat: true });
  sandbox.__cockpitQuotaCompat = { transform: source => ({ ok: false, patchedSource: source, diagnostics: { reason: 'unknown-quota' } }) };
  const result = await api.prepareModule(fixture, moduleUrl);
  assert.equal(result.ok, false);
  assert.equal(result.patchedSource, fixture);
});

test('reconnecting reuses known transformed gates without widening them', () => {
  const { api } = load();
  const first = api.prepareLive(fixture, moduleUrl);
  assert.equal(first.ok, true);
  const second = api.prepareLive(first.patchedSource, moduleUrl);
  assert.equal(second.ok, true);
  assert.equal(second.patchedSource, first.patchedSource);
});

function handoffBrowser(options = {}) {
  const result = load({ owner: 'fixture-owner', ...options });
  const { sandbox } = result, listeners = new Map(), timers = new Map(), elements = [];
  let serial = 0;
  sandbox.name = 'existing-window';
  sandbox.location = { protocol: 'app:', hostname: '-', reload() { sandbox.reloaded = true; } };
  sandbox.AbortSignal = AbortSignal;
  sandbox.addEventListener = (key, callback) => listeners.set(key, callback);
  sandbox.removeEventListener = key => listeners.delete(key);
  sandbox.setTimeout = fn => { timers.set(++serial, fn); return serial; };
  sandbox.clearTimeout = id => timers.delete(id);
  sandbox.fetch = async url => ({ ok: true, text: async () => url.includes('index-') ? 'import "./app-initial-fixture.js"' : fixture });
  sandbox.URL = class extends URL {
    static createObjectURL() { return 'blob:fixture'; }
    static revokeObjectURL() {}
  };
  sandbox.Blob = Blob;
  sandbox.HTMLScriptElement = { supports: () => true };
  sandbox.document = {
    scripts: [{ src: 'app://-/assets/index-fixture.js' }], documentElement: null,
    createElement(tag) { const node = { tag, relList: { supports: () => true }, appendChild(child) { elements.push(child); } }; return node; },
    appendChild(node) { node.parentNode = this; this.documentElement = node; },
    removeChild(node) { node.parentNode = null; this.documentElement = null; },
    addEventListener() {}, removeEventListener() {},
  };
  sandbox.__cockpitStartupGuard = { inspect: () => null, reload() { sandbox.location.reload(); return { ok: true }; } };
  return { ...result, listeners, timers, elements };
}

test('small handoff returns metadata, restores window name and requires actual module execution', async () => {
  const first = handoffBrowser();
  const metadata = await first.api.prepareHandoff('nonce-1');
  assert.equal(metadata.ok, true); assert.equal(metadata.url, moduleUrl);
  assert.equal(metadata.patchedSource, undefined); assert.equal(metadata.source, undefined);
  assert.ok(metadata.sourceLength > fixture.length);
  assert.equal(first.sandbox.name, 'existing-window', 'preparing a running document must not change its name');
  assert.equal(first.api.reloadPreparedDocument('nonce-1').ok, true);
  assert.equal(first.sandbox.reloaded, true);
  const next = handoffBrowser(); next.sandbox.name = first.sandbox.name;
  assert.equal(next.api.consumeHandoff('nonce-1').ok, true);
  assert.equal(next.sandbox.name, 'existing-window');
  assert.equal(next.sandbox.__cockpitEnhancements.nativePatchState, 'preloading');
  assert.equal(next.sandbox.__cockpitEnhancements.nativePatchFeatures, undefined);
  next.sandbox.__cockpitNativeFastComplete();
  assert.equal(next.sandbox.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(next.sandbox.__cockpitEnhancements.nativePatchFeatures, '1000');
  assert.equal(next.elements.filter(node => node.type === 'importmap').length, 1);
  first.api.cancelHandoff('nonce-1'); next.api.cancelHandoff('nonce-1');
});

test('later user navigation stages the cached module and cleanup removes that listener', async () => {
  const env = handoffBrowser(); await env.api.prepareHandoff('nonce-2');
  env.listeners.get('beforeunload')();
  assert.ok(env.sandbox.name.startsWith('cockpit-native-handoff-v1:'));
  assert.equal(env.api.cancelHandoff('different-nonce'), false);
  assert.ok(env.listeners.has('beforeunload'));
  env.api.cancelHandoff('nonce-2');
  assert.equal(env.sandbox.name, 'existing-window'); assert.equal(env.listeners.size, 0);
});

test('draft and failed final guard do not leave a staged name or trigger navigation', async () => {
  const env = handoffBrowser(); await env.api.prepareHandoff('nonce-3');
  env.sandbox.__cockpitStartupGuard.inspect = () => 'draft-present';
  assert.equal(env.api.reloadPreparedDocument('nonce-3').reason, 'draft-present');
  assert.equal(env.sandbox.name, 'existing-window'); assert.equal(env.sandbox.reloaded, undefined);
  env.sandbox.__cockpitStartupGuard.inspect = () => null;
  env.sandbox.__cockpitStartupGuard.reload = () => ({ ok: false, reason: 'interaction-settling' });
  assert.equal(env.api.reloadPreparedDocument('nonce-3').ok, false);
  assert.equal(env.sandbox.name, 'existing-window'); env.api.cancelHandoff('nonce-3');
});

test('wrong identity, stale handoff and invalid source are rejected after restoring name', async () => {
  const first = handoffBrowser(); await first.api.prepareHandoff('nonce-4');
  first.listeners.get('beforeunload')();
  const prefix = 'cockpit-native-handoff-v1:';
  for (const mutate of [value => { value.owner = 'different-owner'; }, value => { value.features = '111'; },
    value => { value.createdAt = 0; }, value => { value.source = 'untrusted'; },
    value => { value.url = 'https://example.com/asset.js'; }]) {
    const value = JSON.parse(first.sandbox.name.slice(prefix.length)); mutate(value);
    const next = handoffBrowser(); next.sandbox.name = prefix + JSON.stringify(value);
    assert.equal(next.api.consumeHandoff('nonce-4').ok, false);
    assert.equal(next.sandbox.name, 'existing-window'); assert.equal(next.elements.length, 0);
  }
  first.api.cancelHandoff('nonce-4');
});

test('retired dictation bridges and capability-only handoffs cannot be reinstalled', async () => {
  const first = handoffBrowser();
  await first.api.prepareHandoff('retired-dictation');
  first.listeners.get('beforeunload')();
  const prefix = 'cockpit-native-handoff-v1:';
  for (const mutate of [
    value => { value.source += ';globalThis.__cockpitApiDictation.transcribe(blob, options)'; },
    value => { value.source += ';globalThis.__cockpitNativeDictation.capability(true, false)'; },
    value => { value.mode = 'capability-only'; value.features = '0001:capability-only'; },
  ]) {
    const value = JSON.parse(first.sandbox.name.slice(prefix.length));
    mutate(value);
    const next = handoffBrowser();
    next.sandbox.name = prefix + JSON.stringify(value);
    assert.equal(next.api.consumeHandoff('retired-dictation').reason, 'handoff-invalid');
    assert.equal(next.sandbox.name, 'existing-window');
    assert.equal(next.elements.length, 0);
    assert.equal(next.sandbox.reloaded, undefined);
    assert.equal(next.sandbox.__cockpitEnhancements.nativePatchState, 'unsupported');
  }
  first.api.cancelHandoff('retired-dictation');
});

test('canceled navigation restores name and cancellation invalidates in-flight preparation', async () => {
  const env = handoffBrowser(); await env.api.prepareHandoff('nonce-5');
  env.listeners.get('beforeunload')(); [...env.timers.values()][0]();
  assert.equal(env.sandbox.name, 'existing-window'); env.api.cancelHandoff('nonce-5');
  let release; env.sandbox.fetch = () => new Promise(resolve => { release = resolve; });
  const pending = env.api.prepareHandoff('nonce-6');
  env.api.cancelHandoff('nonce-6');
  env.sandbox.fetch = async () => ({ ok: true, text: async () => fixture });
  release({ ok: true, text: async () => 'import "./app-initial-fixture.js"' });
  assert.equal((await pending).reason, 'handoff-canceled');
  assert.equal(env.listeners.size, 0);
});

test('cleanup restores a late inherited handoff only for the matching owner and nonce', async () => {
  const first = handoffBrowser(); await first.api.prepareHandoff('late-name');
  first.listeners.get('beforeunload')();
  const encoded = first.sandbox.name;
  const next = handoffBrowser(); next.sandbox.name = encoded;
  next.api.cancelHandoff('another-nonce');
  assert.equal(next.sandbox.name, encoded);
  next.sandbox.__cockpitEnhancements.owner = 'another-owner';
  next.api.cancelHandoff('late-name');
  assert.equal(next.sandbox.name, encoded);
  next.sandbox.__cockpitEnhancements.owner = first.sandbox.__cockpitEnhancements.owner;
  next.api.cancelHandoff('late-name');
  assert.equal(next.sandbox.name, 'existing-window');
  next.sandbox.name = encoded;
  await next.api.prepareHandoff('new-nonce');
  assert.equal(next.sandbox.name, 'existing-window');
  next.api.cancelHandoff('new-nonce');
  first.api.cancelHandoff('late-name');
});

test('current local app artifact has all four gates and retains serviceTiers', { skip: !process.env.COCKPIT_NATIVE_FAST_FIXTURE }, async () => {
  const { api, sandbox } = load({ quotaCompat: true, apiService: true });
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'quota.js'), 'utf8'), sandbox);
  const source = fs.readFileSync(process.env.COCKPIT_NATIVE_FAST_FIXTURE, 'utf8');
  const transformed = api.transform(source);
  assert.equal(transformed.ok, true);
  assert.equal(transformed.patchedSource.split('serviceTiers').length, source.split('serviceTiers').length);
  const prepared = await api.prepareModule(source, moduleUrl);
  assert.equal(prepared.ok, true);
  assert.equal((prepared.patchedSource.match(/globalThis\.__cockpitQuotaCompat\.initialReady=true/g) || []).length, 1);
  assert.equal(prepared.patchedSource.includes('globalThis.__cockpitNativeDictation.'), false);
  assert.equal(prepared.patchedSource.includes('globalThis.__cockpitApiDictation.'), false);
  assert.equal(prepared.patchedSource.includes('cockpitDictationScope'), false);
  const live = api.prepareLive(source, moduleUrl);
  assert.equal(live.ok, true);
  for (const name of ['YLo', 'XLo', 'l6u']) {
    const marker = `function ${name}(`;
    const start = source.indexOf(marker), next = source.indexOf('function ', start + marker.length);
    assert.ok(start >= 0 && next > start, `installed native ${name} function is present`);
    assert.ok(live.patchedSource.includes(source.slice(start, next)), `native ${name} capability/transcription function remains unchanged`);
  }
  assert.equal(api.prepareLive(prepared.patchedSource, moduleUrl).ok, true);
  assert.ok(prepared.patchedSource.includes('//# sourceURL=' + moduleUrl));
  const parsed = new vm.SourceTextModule(prepared.patchedSource);
  assert.ok(parsed.dependencySpecifiers.every(specifier => !specifier.startsWith('.')));
});
