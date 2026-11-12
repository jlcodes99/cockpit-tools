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

function composer(mac) {
  return [
    'export function itX7(n){',
    'const p=n("tl5a"),r=n("eLmE"),u=n("a6kU");',
    'return function(e,{reader:x,config:c,scope:s,thread:t,load:l}){',
    'const {atMentionRankingContext:am,a1:v1,useAsbDirect:asb,a2:v2,homeRunLocationRemoteProject:remote,a3:v3,selectedProject:project,other:unused}=e,d="local"===s.value.kind?e.k:e.ComposerContent,h=e.hostId;',
    'let a=true,b=(0,x._U)(r.F9,h),q=(0,x._U)(e.M,null==t?null:{hostId:h,threadId:t});',
    'let settings=(0,c.Fc)(h);',
    'const cfg=settings.isConfigDataPending?null:settings.resolvedConfig,usage=(0,x.Ul)("local"===h&&u.U);',
    mac
      ? 'let reserve=b&&"reserve-limit",limit=usage&&"usage-limit";return Boolean(e.disabled||l?.isLoading||reserve||limit)'
      : 'let blocked=e.disabled||l?.isLoading===!0||b||usage;return Boolean(blocked)',
    '}}',
  ].join('\n');
}

function setup(flags = {}, { host = 182735, menus = menuIds, queries = modelQueryIds } = {}) {
  const modules = new Map();
  const sources = new Map([
    ...menus.map(id => [id, menu]),
    ...queries.map(id => [id, modelQuery]),
    [host, queue], [543778, composer(true)], [634303, composer(false)],
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

test('macOS and Windows only hand local API Service quota decisions to the gateway', async () => {
  for (const mixed of [false, true]) {
    const h = setup({nativeFast: !mixed, apiKeyAuthVerified: !mixed, quotaCompat: mixed});
    h.preload();
    assert.equal(h.root.__cockpitStartupPreload.quota.ok, true, h.root.__cockpitStartupPreload.quota.reason);
    for (const id of [543778, 634303]) {
      const modules = { tl5a: {jR: 'threadProvider'}, eLmE: {F9: 'reserve'}, a6kU: {U: 'usage'} };
      const {itX7: factory} = await h.exports(id);
      const blocked = factory(name => modules[name]);
      const provider = { requires_openai_auth: mixed, experimental_bearer_token: 'agt_codex_test', base_url: 'http://127.0.0.1:8765/v1' };
      const settings = { isConfigDataPending: false, resolvedConfig: {
        model_provider: 'codex_local_access', model_providers: {codex_local_access: provider},
      } };
      function submit({props, route, thread, threadProvider, reserve = true, usage = true, load} = {}) {
        return blocked({hostId: 'local', ...props}, {
          reader: { _U: key => key === 'threadProvider' ? threadProvider : key === 'reserve' && reserve, Ul: key => key === 'usage' && usage },
          config: { Fc: () => settings }, scope: {value: route ?? {kind:'new'}}, thread, load,
        });
      }
      assert.equal(submit(), false);
      assert.equal(submit({reserve: false}), false);
      assert.equal(submit({usage: false}), false);
      assert.equal(submit({props: {disabled: true}}), true);
      assert.equal(submit({load: {isLoading: true}}), true);
      assert.equal(submit({props: {hostId: 'remote'}}), true);
      assert.equal(submit({props: {useAsbDirect: true}}), true);
      assert.equal(submit({props: {selectedProject: {type: 'remote'}}}), true);
      assert.equal(submit({route: {kind: 'new', cloudThreadPrototype: {}}}), true);
      assert.equal(submit({route: {kind: 'cloud'}}), true);
      const existing = {thread: 't1', route: {kind: 'local', conversationId: 't1'}};
      assert.equal(submit(existing), true);
      assert.equal(submit({...existing, threadProvider: 'openai'}), true);
      assert.equal(submit({...existing, threadProvider: 'codex_local_access'}), false);
      provider.base_url = 'https://remote.example/v1';
      assert.equal(submit(), true);
      provider.base_url = 'http://localhost:8765/v1';
      settings.isConfigDataPending = true;
      assert.equal(submit(), true);
    }
  }
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

test('live quota resolves the Mac and Windows guards in both lazy chunks without changing the document', async () => {
  const h = setup();
  const entry = 'app://-/assets/123.abcdef1234.js';
  h.root.document.scripts = [{src: entry}];
  h.root.fetch = async url => {
    const xhr = new h.root.XMLHttpRequest();
    xhr.open('GET', url);
    xhr.send();
    return {ok: true, text: async () => xhr.responseText};
  };
  h.root.AbortSignal = AbortSignal;
  const result = await h.root.__cockpitServiceRspack.prepareLiveQuotaGate();
  assert.equal(result.ok, true, result.reason);
  assert.deepEqual(Array.from(result.rspackPlans, plan => plan.hardVariable), ['b','b']);
  assert.deepEqual(Array.from(result.rspackPlans, plan => plan.guardVariable), ['usage','usage']);
  assert.deepEqual(Array.from(result.rspackPlans, plan => plan.scope), ['s','s']);
  assert.equal(h.root.document.documentElement, null);
});

test('a changed speed contract preserves quota and publishes no partial speed graph', () => {
  const h = setup();
  h.sources.delete(menuIds.at(-1));
  h.preload();
  assert.equal(h.root.__cockpitStartupPreload.speed.ok, false);
  assert.equal(h.root.__cockpitStartupPreload.quota.ok, true, h.root.__cockpitStartupPreload.quota.reason);
  assert.equal(Object.keys(h.root.imports).length, 2);
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
    assert.equal(Object.keys(h.root.imports).length, apiKeyAuthVerified ? 22 : 3);
    assert.ok(Object.hasOwn(h.root.imports, url));
  }
});
