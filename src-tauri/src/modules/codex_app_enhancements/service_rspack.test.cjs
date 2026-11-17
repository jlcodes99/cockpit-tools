const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const menuIds = [159406, 29761, 414655, 434155, 553834, 634175, 884655];
const modelQueryIds = [161445, 177472, 291906, 32593, 328669, 40728, 415411, 416681, 455856, 503951, 870918];
const currentUltraQueries = [146068, 215569, 24705, 267924, 304386, 424504, 505901, 529959,
  551475, 613538, 613959, 662473, 860605, 906325, 953083, 961945];
const modelQuery = 'export const modelQuery=(C,F)=>({additionalAvailableModels:e,authMethod:a,enabledReasoningEfforts:b,hostId:h,includeUltraReasoningEffort:r=!0,limit:l=100,specialty:s,teamCatalog:t},{get:g,queryClient:q,scope:c})=>{let{data:d}=g(q.I2,h,{enabled:!1}),cfg=d,custom=(0,C.cc)(cfg),models=[],u=r&&g(F.JW,"1186680773");return{queryKey:[],select:()=>({includeUltraReasoningEffort:u,isCustomModelProvider:custom})}};';
const menu = 'export function menu(e,f,g){let h=e?.hostId??f,a=(0,g.tn)(h),m=a?.authMethod==="chatgpt"||a?.authMethod==="personalAccessToken"||a?.authMethod==="amazonBedrock",j=a?.authMethod??null;return {serviceTierAccess:m}}';
const queue = [
  'export function saved(e,{runtime:r}){return{async readServiceTier(){return (async f=>f())(async()=>{if("local"===e.getHostId()&&await r.isUsingCopilotInference())return!1;let a=await e.getAccount({priority:"critical"});return a.account?.type==="chatgpt"||a.account?.type==="amazonBedrock"})},async writeServiceTier(){}}}',
  'export function explicit(e,{settings:s,storage:u,filesystem:f,runtime:r}){return async(input)=>{let o=input.submissionOptions,h=o?.executionHostId??e.getHostId();let [tier]=await Promise.all([(async()=>{if(null==o.serviceTier)return null;let{account:a}=await e.getAccount({priority:"critical"});return a?.type==="chatgpt"||a?.type==="amazonBedrock"?o.serviceTier:null})(),s.ready]);return tier}}',
  'export function policy(e,D,R,nn,t,tE,c){return {authMethod:e.hostId===R.local?D.authMethod:null,includeUltraReasoningEffort:(0,nn.flag)(t,"1186680773"),isAuthLoading:D.loading,isCustomModelProvider:(0,tE.cc)(c),workspaceDependenciesEnabled:true}}',
].join('\n');
// The current host contains another use of the same feature ID, as well as
// unrelated fields between the auth and reasoning selectors.  The transform
// must only alter the model policy contract.
const currentHost = queue.replace('authMethod:e.hostId===R.local?D.authMethod:null,includeUltraReasoningEffort:',
  'authMethod:e.hostId===R.local?D.authMethod:null,enabledReasoningEfforts:new Set(["low","max","ultra"]),featureOverrides:{},hasConfiguredModelCatalog:true,hostId:e.hostId,includeUltraReasoningEffort:')
  + '\nexport function unrelatedGate(nn,t){return (0,nn.flag)(t,"1186680773")}';

function setup(flags = {}, { host = 182735, menus = menuIds, queries = modelQueryIds, hostSource = queue } = {}) {
  const modules = new Map();
  const sources = new Map([
    ...menus.map(id => [id, menu]),
    ...queries.map(id => [id, modelQuery]),
    [host, hostSource],
  ]);
  const html = '<script type="module" src="/assets/123.abcdef1234.js"></script>';
  const root = {
    URL: class extends URL { static createObjectURL(blob) {
      const id = String(modules.size);
      modules.set(id, blob);
      return id;
    } },
    Blob, AbortSignal, location: new URL('app://-/index.html'),
    __cockpitEnhancements: { nativeFast: true, apiKeyAuthVerified: true, apiService: true, ...flags },
    HTMLScriptElement: { supports: () => true }, addEventListener() {},
    document: {
      documentElement: null,
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
  return { root, sources, preload: () => load('startup_preload.js'), exports };
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
  assert.equal((await h.root.__cockpitServiceRspack.verifySpeedDocument()).ok, true);
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

test('verified bound OAuth restores Ultra without changing speed or user/catalog restrictions', async () => {
  const h = setup({ nativeFast: false, apiKeyAuthVerified: false, boundApiAuthVerified: true });
  h.preload();
  const state = h.root.__cockpitStartupPreload;
  assert.equal(state.ok, true);
  assert.equal(state.speed.reason, 'disabled');
  assert.equal(state.ultra.ok, true);
  assert.equal(h.root.__cockpitEnhancements.nativeUltraPatchState, 'active');
  assert.equal(Object.keys(h.root.imports).length, modelQueryIds.length + 1);
  for (const id of menuIds) assert.equal(h.root.imports['app://-/assets/async/' + id + '.abcdef1234.js'], undefined);
  const { policy, saved, explicit } = await h.exports(182735);
  const eligible = (hostId = 'local', authMethod = 'chatgpt', custom = true,
    config = {model_provider:'codex_local_access'}) => policy(
    {hostId}, {authMethod}, {local:'local'}, {flag:()=>false}, null, {cc:()=>custom}, config,
  ).includeUltraReasoningEffort;
  assert.equal(eligible(), true);
  assert.equal(eligible('local', null), true);
  for (const auth of ['apikey', 'personalAccessToken', 'amazonBedrock', 'copilot'])
    assert.equal(eligible('local', auth), false);
  assert.equal(eligible('remote'), false);
  assert.equal(eligible('local', 'chatgpt', false), false);
  assert.equal(eligible('local', 'chatgpt', true, {model_provider:'other'}), false);
  assert.equal(eligible('local', 'chatgpt', true, {}), false);
  const manager = { getHostId: () => 'local', getAccount: async () => ({account:{type:'apiKey'}}) };
  const runtime = { isUsingCopilotInference: async () => false };
  assert.equal(await saved(manager, {runtime}).readServiceTier(), false);
  assert.equal(await explicit(manager, {runtime, settings:{ready:true}})({submissionOptions:{serviceTier:'priority'}}), null);
  for (const id of modelQueryIds) {
    const { modelQuery: query } = await h.exports(id);
    const decide = (options, config={custom:true, model_provider:'codex_local_access'}) => query({cc:value=>value?.custom}, {JW:'flag'})(
      {hostId:'local', authMethod:'chatgpt', ...options},
      {get:key=>key==='flag' ? false : {data:config}, queryClient:{}},
    ).select().includeUltraReasoningEffort;
    assert.equal(decide({}), true);
    assert.equal(decide({}, {custom:true,model_provider:'other'}), false);
    assert.equal(decide({}, {custom:true}), false);
    for (const options of [{includeUltraReasoningEffort:false}, {teamCatalog:'loading'},
      {teamCatalog:{id:'team'}}, {hostId:'remote'}, {authMethod:'copilot'}]) assert.equal(decide(options), false);
  }
  h.root.__cockpitEnhancements.boundApiAuthVerified = false;
  assert.equal(eligible(), false, 'revoked host verification stops the extra gate immediately');
  h.root.__cockpitEnhancements.boundApiAuthVerified = true;
  h.root.__cockpitEnhancements.apiService = false;
  assert.equal(eligible(), false, 'a non-service OAuth profile stays native');
});

test('Ultra contracts are independent of speed and fail without partial installation', () => {
  const h = setup({nativeFast:false, apiKeyAuthVerified:false, boundApiAuthVerified:true});
  h.sources.set(menuIds[0], 'unsupported speed menu');
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
  const broken = setup({nativeFast:false, apiKeyAuthVerified:false, boundApiAuthVerified:true});
  broken.sources.set(modelQueryIds.at(-1), 'unsupported model query');
  broken.preload();
  assert.equal(broken.root.__cockpitStartupPreload.ultra.ok, false);
  assert.equal(broken.root.__cockpitEnhancements.nativeUltraPatchState, 'unsupported');
  assert.equal(broken.root.imports, undefined);
});

test('current Ultra-only layout verifies and installs all queries without touching speed', async () => {
  const h = setup({nativeFast:false, apiKeyAuthVerified:false, boundApiAuthVerified:true},
    {host:907538, menus:[], queries:currentUltraQueries, hostSource:currentHost});
  assert.equal((await h.root.__cockpitServiceRspack.verifySpeedDocument()).reason, 'disabled');
  assert.equal((await h.root.__cockpitServiceRspack.verifyRequestedDocument()).ok, true);
  assert.equal(h.root.imports, undefined, 'verification must not install modules');
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.reason, 'disabled');
  assert.equal(h.root.__cockpitStartupPreload.ultra.ok, true);
  assert.equal(Object.keys(h.root.imports).length, currentUltraQueries.length + 1);
  const {policy, saved, explicit, unrelatedGate} = await h.exports(907538);
  assert.equal(policy({hostId:'local'}, {authMethod:'chatgpt'}, {local:'local'},
    {flag:()=>false}, null, {cc:()=>true}, {model_provider:'codex_local_access'}).includeUltraReasoningEffort, true);
  assert.equal(policy({hostId:'local'}, {authMethod:'chatgpt'}, {local:'local'},
    {flag:()=>false}, null, {cc:()=>true}, {model_provider:'other'}).includeUltraReasoningEffort, false);
  assert.equal(unrelatedGate({flag:()=>false}, null), false);
  const manager = {getHostId: () => 'local', getAccount: async () => ({account:{type:'apiKey'}})};
  const runtime = {isUsingCopilotInference: async () => false};
  assert.equal(await saved(manager, {runtime}).readServiceTier(), false);
  assert.equal(await explicit(manager, {runtime, settings:{ready:true}})({submissionOptions:{serviceTier:'priority'}}), null);
  for (const id of currentUltraQueries) {
    const {modelQuery: query} = await h.exports(id);
    const select = (options, config={custom:true, model_provider:'codex_local_access'}) => query({cc:value=>value?.custom}, {JW:'flag'})(
      {hostId:'local', authMethod:'chatgpt', ...options},
      {get:key=>key==='flag' ? false : {data:config}, queryClient:{}},
    ).select().includeUltraReasoningEffort;
    assert.equal(select({}), true);
    assert.equal(select({}, {custom:true,model_provider:'other'}), false);
    assert.equal(select({}, {custom:true}), false);
    for (const options of [{includeUltraReasoningEffort:false}, {teamCatalog:'loading'},
      {hostId:'remote'}, {authMethod:'copilot'}]) assert.equal(select(options), false);
  }
});

test('requested verification checks every required contract and fails closed', async () => {
  const options = {host:907538, menus:[], queries:currentUltraQueries};
  const flags = {nativeFast:false, apiKeyAuthVerified:false, boundApiAuthVerified:true};
  for (const brokenId of [907538, currentUltraQueries[0], currentUltraQueries.at(-1)]) {
    const h = setup(flags, options);
    h.sources.set(brokenId, 'unsupported contract');
    const verify = await h.root.__cockpitServiceRspack.verifyRequestedDocument();
    assert.equal(verify.ok, false);
    assert.match(verify.reason, /^ultra-(reasoning|model-query)-contract$/);
    h.preload();
    assert.equal(h.root.__cockpitStartupPreload.ultra.ok, false);
    assert.equal(h.root.imports, undefined);
  }
  const disabled = setup({...flags, apiService:false}, options);
  assert.equal((await disabled.root.__cockpitServiceRspack.verifyRequestedDocument()).reason, 'disabled');
  const both = setup({boundApiAuthVerified:true});
  assert.equal((await both.root.__cockpitServiceRspack.verifyRequestedDocument()).ok, true);
  both.sources.set(menuIds[0], 'unsupported speed menu');
  assert.equal((await both.root.__cockpitServiceRspack.verifyRequestedDocument()).reason, 'speed-menu-contract');
  const unknownSpeed = setup({nativeFast:true, boundApiAuthVerified:true}, options);
  assert.equal((await unknownSpeed.root.__cockpitServiceRspack.verifyRequestedDocument()).reason, 'speed-layout-unsupported');
});
