const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const menuIds = [159406, 29761, 414655, 434155, 553834, 634175, 884655];
const modelQueryIds = [161445, 177472, 291906, 32593, 328669, 40728, 415411, 416681, 455856, 503951, 870918];
const modelQuery = 'export const modelQuery=(C,F)=>({additionalAvailableModels:e,authMethod:a,enabledReasoningEfforts:b,hostId:h,includeUltraReasoningEffort:r=!0,limit:l=100,specialty:s,teamCatalog:t},{get:g,queryClient:q,scope:c})=>{let{data:d}=g(q.I2,h,{enabled:!1}),cfg=d,custom=(0,C.cc)(cfg),models=[],u=r&&g(F.JW,"1186680773");return{queryKey:[],select:()=>({includeUltraReasoningEffort:u,isCustomModelProvider:custom})}};';
const menu = 'export function menu(e,f,g){let h=e?.hostId??f,a=(0,g.tn)(h),m=a?.authMethod==="chatgpt"||a?.authMethod==="personalAccessToken"||a?.authMethod==="amazonBedrock",j=a?.authMethod??null;return {serviceTierAccess:m}}';
const queue = [
  'export function saved(e,{runtime:r}){return{async readServiceTier(){return (async f=>f())(async()=>{if("local"===e.getHostId()&&await r.isUsingCopilotInference())return!1;let a=await e.getAccount({priority:"critical"});return a.account?.type==="chatgpt"||a.account?.type==="amazonBedrock"})},async writeServiceTier(){}}}',
  'export function explicit(e,{settings:s,storage:u,filesystem:f,runtime:r}){return async(input)=>{let o=input.submissionOptions,h=o?.executionHostId??e.getHostId();let [tier]=await Promise.all([(async()=>{if(null==o.serviceTier)return null;let{account:a}=await e.getAccount({priority:"critical"});return a?.type==="chatgpt"||a?.type==="amazonBedrock"?o.serviceTier:null})(),s.ready]);return tier}}',
  'export function policy(e,D,R,nn,t,tE,c){return {authMethod:e.hostId===R.local?D.authMethod:null,includeUltraReasoningEffort:(0,nn.flag)(t,"1186680773"),isAuthLoading:D.loading,isCustomModelProvider:(0,tE.cc)(c),workspaceDependenciesEnabled:true}}',
].join('\n');

function setup(flags = {}, { host = 182735, menus = menuIds, queries = modelQueryIds, policy } = {}) {
  const modules = new Map();
  const requestedUrls = [];
  const routeIds = [...new Set([...menus, ...queries])];
  const sources = new Map(routeIds.map(id => [
    id, [menus.includes(id) ? menu : '', queries.includes(id) ? modelQuery : ''].join('\n'),
  ]));
  sources.set(host, queue);
  const assetName = id => id + '.abcdef1234.js';
  const html = '<script type="module" src="/assets/123.abcdef1234.js"></script>';
  const policies = policy ? [{ content: policy }] : [];
  const root = {
    DOMParser: class { parseFromString() { return { querySelectorAll: () => policies }; } },
    URL: class extends URL { static createObjectURL(blob) {
      const id = String(modules.size);
      modules.set(id, blob);
      return id;
    } },
    Blob, AbortSignal, location: new URL('app://-/index.html'),
    __cockpitEnhancements: {
      nativeFast: true, apiKeyAuthVerified: true, apiService: true,
      rspackSpeedAssets: {
        entryScripts: ['123.abcdef1234.js'],
        hosts: [assetName(host)], menus: menus.map(assetName), modelQueries: queries.map(assetName),
      },
      ...flags,
    },
    HTMLScriptElement: { supports: () => true }, addEventListener() {},
    document: {
      documentElement: null,
      querySelectorAll: () => policies,
      scripts: [{ src: 'app://-/assets/123.abcdef1234.js' }],
      createElement() {
        return { relList: { supports: () => true }, appendChild: node => {
          if (node.type === 'importmap') root.imports = JSON.parse(node.textContent).imports;
        } };
      },
      appendChild(node) { node.parentNode = this; this.documentElement = node; },
      removeChild() { this.documentElement = null; },
    },
    XMLHttpRequest: class {
      open(_, url) { this.url = url; }
      send() {
        requestedUrls.push(this.url);
        if (this.url.endsWith('/index.html')) this.responseText = html;
        else if (this.url.endsWith('/123.abcdef1234.js'))
          this.responseText = 'r.u=e=>"assets/async/"+e+"."+({' + [...sources.keys()].map(id => id + ':"abcdef1234"').join(',') + '})[e]+".js"';
        else {
          const id = Number(this.url.split('/').at(-1).split('.')[0]);
          this.responseText = 'export const __rspack_esm_ids=[' + id + '];' + sources.get(id);
        }
      }
    },
  };
  root.fetch = async url => {
    const request = new root.XMLHttpRequest();
    request.open('GET', url);
    request.send();
    return { ok: true, text: async () => request.responseText };
  };
  root.window = root;
  const context = vm.createContext(root);
  const load = file => vm.runInContext(fs.readFileSync(path.join(__dirname, file), 'utf8'), context);
  for (const file of ['fast.js', 'quota.js', 'predictions_rspack.js', 'service_rspack.js']) load(file);
  async function exports(id) {
    const blob = modules.get(root.imports['app://-/assets/async/' + id + '.abcdef1234.js']);
    const module = new vm.SourceTextModule(await blob.text(), { context });
    await module.link(() => { throw Error('unexpected dependency'); });
    await module.evaluate();
    return module.namespace;
  }
  return { root, sources, requestedUrls, preload: () => load('startup_preload.js'), exports };
}

test('Rspack enables the native local API speed menu and both queued request paths', async () => {
  const h = setup();
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, true, h.root.__cockpitStartupPreload.speed.reason);
  for (const id of menuIds) {
    const {menu: show} = await h.exports(id);
    const lookup = { tn: () => ({ authMethod: 'apiKey' }) };
    assert.equal(show({}, 'local', lookup).serviceTierAccess, true);
    assert.equal(show({hostId: 'remote'}, 'local', lookup).serviceTierAccess, false);
    assert.equal(show({}, 'local', { tn: () => ({ authMethod: 'chatgpt' }) }).serviceTierAccess, true);
  }
  const {saved, explicit} = await h.exports(182735);
  const manager = { getHostId: () => 'local', getAccount: async () => ({account: {type:'apiKey'}}) };
  const run = (copilot = false) => ({
    runtime: { isUsingCopilotInference: async () => copilot }, settings: {ready: true},
  });
  assert.equal(await saved(manager, run()).readServiceTier(), true);
  assert.equal(await saved(manager, run(true)).readServiceTier(), false);
  for (const tier of ['default', 'priority', 'ultrafast']) {
    const input = { submissionOptions: { serviceTier: tier } };
    assert.equal(await explicit(manager, run())(input), tier);
    assert.equal(await explicit(manager, run(true))(input), null);
    assert.equal(await explicit({...manager, isCloudHost: true}, run())(input), null);
    assert.equal(await explicit(manager, run())({submissionOptions: {...input.submissionOptions, executionHostId: 'remote'}}), null);
  }
});

test('the updated desktop manifest selects the new speed routes and forwards the selected tier', async () => {
  const menus = [184348, 222624, 244404, 29761, 309546, 363454, 390941];
  const queries = [201528, 256736, 304386, 377014, 379153, 415411, 504134, 669972,
    687108, 750210, 782647, 838074, 872702, 963873];
  const host = 739301;
  const h = setup({}, { host, menus, queries });
  assert.equal((await h.root.__cockpitServiceRspack.verifyRequestedDocument()).ok, true);
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, true);
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
  const { menu: show } = await h.exports(390941);
  assert.equal(show({}, 'local', { tn: () => ({ authMethod: 'apiKey' }) }).serviceTierAccess, true);
  const { explicit } = await h.exports(host);
  const manager = { getHostId: () => 'local', getAccount: async () => ({ account: { type: 'apiKey' } }) };
  const runtime = { isUsingCopilotInference: async () => false };
  assert.equal(await explicit(manager, { runtime, settings: { ready: true } })({
    submissionOptions: { serviceTier: 'priority' },
  }), 'priority');
});

test('Codex 26.1111.71025 restores the speed menu and forwards GPT-6.2 Sol Ultrafast', async () => {
  const host = 907538;
  const menus = [131020, 155556, 166418, 184348, 24668, 29761, 360659, 884042, 955783];
  const queries = [184659, 24668, 262625, 263369, 304386, 415411, 440403, 579702,
    692212, 780248, 84646, 961945, 968289];
  const h = setup({}, { host, menus, queries });
  assert.equal((await h.root.__cockpitServiceRspack.verifyRequestedDocument()).ok, true);
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, true);
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
  const { menu: show, modelQuery: query } = await h.exports(24668);
  assert.equal(show({}, 'local', { tn: () => ({ authMethod: 'apikey' }) }).serviceTierAccess, true);
  assert.equal(query({ cc: data => data?.custom }, { JW: 'flag' })(
    { hostId: 'local', authMethod: 'apikey' },
    { get: key => key === 'flag' ? false : { data: { custom: true } }, queryClient: {} },
  ).select().includeUltraReasoningEffort, true);
  const { saved, explicit } = await h.exports(host);
  const manager = { getHostId: () => 'local', getAccount: async () => ({ account: { type: 'apiKey' } }) };
  const environment = { runtime: { isUsingCopilotInference: async () => false }, settings: { ready: true } };
  assert.equal(await saved(manager, environment).readServiceTier(), true);
  assert.equal(await explicit(manager, environment)({
    submissionOptions: { model: 'gpt-6.2-sol', serviceTier: 'ultrafast' },
  }), 'ultrafast');
});

test('Codex 26.1120 reads the thread provider and applies each native speed selection', async () => {
  const host = 49792;
  const h = setup({}, { host });
  h.sources.set(host, h.sources.get(host)
    .replace('async readServiceTier(){return (async f=>f())(',
      'async readServiceTier(thread){return (async(f,provider)=>await f()&&provider)(')
    .replace('})},async writeServiceTier()', '},e.getThreadModelProvider(thread))},async writeServiceTier()'));

  const verified = await h.root.__cockpitServiceRspack.verifyRequestedDocument();
  assert.equal(verified.ok, true, verified.reason);
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, true);

  const { menu: show } = await h.exports(menuIds[0]);
  assert.equal(show({}, 'local', { tn: () => ({ authMethod: 'apiKey' }) }).serviceTierAccess, true);

  const { saved, explicit } = await h.exports(host);
  const manager = {
    getHostId: () => 'local',
    getAccount: async () => ({ account: { type: 'apiKey' } }),
    getThreadModelProvider: thread => thread === 'active-thread' ? 'active-provider' : null,
  };
  const environment = { runtime: { isUsingCopilotInference: async () => false }, settings: { ready: true } };
  assert.equal(await saved(manager, environment).readServiceTier('active-thread'), 'active-provider');

  const submit = explicit(manager, environment);
  for (const serviceTier of ['priority', 'ultrafast', null]) {
    assert.equal(await submit({ submissionOptions: { serviceTier } }), serviceTier);
  }
});

test('previously unseen host and lazy chunks retain the speed selector and model query', async () => {
  const host = 400000;
  const menus = [400001, 400002];
  const h = setup({}, { host, menus, queries: [400002, 400003] });
  for (const id of [host, ...menus, 400003]) {
    h.sources.set(id, h.sources.get(id).replaceAll('.tn', '.renamedLookup')
      .replaceAll('.cc', '.renamedProviderCheck').replaceAll('.JW', '.renamedFlags'));
  }
  h.sources.set(400004, 'export const unrelated = { serviceTierAccess: false };');
  h.root.__cockpitEnhancements.rspackSpeedAssets.menus.push('400004.abcdef1234.js');
  assert.equal((await h.root.__cockpitServiceRspack.verifyRequestedDocument()).ok, true);
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, true);
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
  assert.ok(!Object.keys(h.root.imports).some(url => url.includes('400004.')));
  for (const id of menus) {
    const { menu: show } = await h.exports(id);
    assert.equal(show({}, 'local', { renamedLookup: () => ({ authMethod: 'apiKey' }) }).serviceTierAccess, true);
  }
});

test('candidates for another installed build do not patch the running document', async () => {
  const h = setup();
  h.root.__cockpitEnhancements.rspackSpeedAssets.entryScripts = ['123.ff00ff00ff.js'];
  const verified = await h.root.__cockpitServiceRspack.verifyRequestedDocument();
  assert.equal(verified.reason, 'speed-assets-build-mismatch');
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, false);
  assert.equal(h.root.imports, undefined);
});

test('a restrictive script policy skips patch assets and leaves the native load in place', async () => {
  for (const policy of [
    "default-src 'none'; script-src 'self'; img-src 'self' blob:",
    "script-src 'self' blob:; script-src-elem 'self'",
  ]) {
    const h = setup({ nativePredictions: true }, { policy });
    const verified = await h.root.__cockpitServiceRspack.verifyRequestedDocument();
    assert.equal(verified.ok, false);
    assert.equal(verified.reason, 'blob-scripts-blocked-by-csp');
    h.preload();
    assert.equal(h.root.__cockpitStartupPreload.reason, 'blob-scripts-blocked-by-csp');
    assert.equal(h.root.__cockpitStartupPreload.speed.ok, false);
    assert.equal(h.root.__cockpitStartupPreload.predictions.ok, false);
    assert.equal(h.root.imports, undefined);
    assert.deepEqual(h.requestedUrls, ['app://-/index.html']);
  }
});

test('the earlier Rspack build with the same host still enables native speed', async () => {
  const h = setup({}, {
    host: 907538,
    menus: [156209, 184348, 24705, 29761, 356505, 360659, 524154, 884042],
    queries: [146068, 215569, 24705, 267924, 304386, 424504, 505901, 529959,
      551475, 613538, 613959, 662473, 860605, 906325, 953083, 961945],
  });
  assert.equal((await h.root.__cockpitServiceRspack.verifyRequestedDocument()).ok, true);
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, true);
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
});


test('Ultra reasoning respects native eligibility and only adds the verified local custom route', async () => {
  const h = setup();
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
  const {policy} = await h.exports(182735);
  const eligible = (hostId, authMethod, custom, native) => policy(
    {hostId}, {authMethod}, {local:'local'}, {flag:()=>native}, null, {cc:()=>custom}, null,
  ).includeUltraReasoningEffort;
  assert.equal(eligible('local', 'apikey', true, false), true);
  assert.equal(eligible('local', 'apikey', false, false), false);
  assert.equal(eligible('local', 'chatgpt', true, false), false);
  assert.equal(eligible('remote', 'apikey', true, false), false);
  assert.equal(eligible('remote', 'chatgpt', false, true), true);
  for (const id of modelQueryIds) {
    const {modelQuery: query} = await h.exports(id);
    const decide = options => query({cc:value=>value?.custom}, {JW:'flag'})(
      {hostId:'local', authMethod:'apiKey', ...options},
      {get:key=>key==='flag' ? false : {data:{custom:true}}, queryClient:{}},
    ).select().includeUltraReasoningEffort;
    assert.equal(decide({}), true);
    assert.equal(decide({includeUltraReasoningEffort: false}), false);
    assert.equal(decide({teamCatalog: 'loading'}), false);
    assert.equal(decide({hostId:'remote'}), false);
  }
});


test('predictions share the Rspack import map in both API Service auth modes', () => {
  for (const apiKeyAuthVerified of [true, false]) {
    const h = setup({nativePredictions: true, nativeFast: apiKeyAuthVerified, apiKeyAuthVerified});
    const readiness = [];
    h.root.__cockpitNativePredictions = {
      setReady: value => readiness.push(value),
      runtime: { dispose() {} },
    };
    const url = 'app://-/assets/async/42.abcdef1234.js';
    h.root.__cockpitPredictionRspack.prepare = () => ({
      ok: true,
      modules: [{ url, source: 'export const __rspack_esm_ids=[42];' }],
    });
    h.preload();
    assert.deepEqual(readiness, [true]);
    assert.equal(Object.keys(h.root.imports).length, apiKeyAuthVerified ? 20 : 1);
    assert.ok(Object.hasOwn(h.root.imports, url));
  }
});

test('bound OAuth predictions pass preflight independently of Fast', async () => {
  const h = setup({nativePredictions: true, nativeFast: false, apiKeyAuthVerified: false});
  const rspack = h.root.__cockpitPredictionRspack;
  const transform = rspack.transform;
  const checked = [];
  rspack.selectLayout = () => ({ initial: 42, composer: 43, host: 44, settings: 45 });
  rspack.transform = (source, kind, id) => {
    checked.push([kind, id]);
    return { ok: true, source };
  };
  for (const id of [42, 43, 44, 45]) h.sources.set(id, '');
  const verified = await h.root.__cockpitServiceRspack.verifyRequestedDocument();
  assert.equal(verified.ok, true, verified.reason);
  assert.deepEqual(checked, [['initial', 42], ['composer', 43], ['host', 44], ['settings', 45]]);

  rspack.transform = transform;
  const unsupported = await h.root.__cockpitServiceRspack.verifyRequestedDocument();
  assert.equal(unsupported.ok, false);
  assert.equal(unsupported.reason, 'initial-contract');
});
