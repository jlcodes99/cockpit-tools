const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

const moduleUrl = 'app://-/assets/app-initial-fixture.js';
const fixture = `
var kKt=Y(U,({get:e})=>{if(!navigator?.mediaDevices?.getUserMedia||typeof MediaRecorder>\`u\`)return{isLoading:!1,isError:!1,isCapable:!1};let t=e(ig,\`4100906017\`),n=e(rg,\`4100906017\`),{authLoading:r,authMethod:i}=e(Ag),a=r||t;return{isLoading:a,isError:!1,isCapable:!a&&n&&i===\`chatgpt\`}});
var MKt=Y(U,({get:e})=>e(ig,\`770071981\`)?{isLoading:!0,isError:!1,isCapable:!1}:e(rg,\`770071981\`)?SKt(e,[\`chatgpt.workspace.feature.dictation.access\`]):{isLoading:!1,isError:!1,isCapable:!0});
var definitions={dictation:{accessPolicy:\`workspace-dictation\`,configFeatures:[{key:\`in_app_dictation\`,host:\`default\`,clients:[\`electron\`,\`extension\`]}],supportedClients:[\`browser\`,\`chrome-extension\`,\`electron\`,\`extension\`],children:{}}};
var zKt={dictation:kKt,'workspace-dictation':MKt},ty=wc(U,(e,{get:t})=>{let n={accessPolicies:['workspace-dictation','dictation'],configFeatures:definitions.dictation.configFeatures};
let r=[...n.accessPolicies.map(e=>{let n=zKt[e];return n==null?{isLoading:!1,isError:!0,isCapable:!1}:t(n)}),...policySettings(),...n.configFeatures.map(n=>{if(n.clients?.includes(\`electron\`)===!1)return{isLoading:!1,isError:!1,isCapable:!0};n.host;let r=n.host==="default"?Ls:e.hostId;if(r==null)throw Error(\`Explicit-host capability requires hostId: \${e.name}\`);let i=t(Jh,r),a=i.isError,o=i.isLoading||!a&&i.data==null,s=i.data?.find(e=>e.name===n.key);return{isLoading:o,isError:a,isCapable:!o&&!a&&s?.enabled!==!1}})];
return r.length===0?{isLoading:!1,isError:!1,isCapable:!0}:{isLoading:r.some(e=>e.isLoading),isError:r.some(e=>e.isError),isCapable:r.every(e=>e.isCapable)}},{isEqual:JKt});
function JKt(e,t){return e.isLoading===t.isLoading&&e.isError===t.isError&&e.isCapable===t.isCapable}
function l6u(e){let t=(0,u6u.c)(2),n=e!==void 0&&e?\`dictation.chatgpt\`:\`dictation.composer\`,r;t[0]===n?r=t[1]:(r={name:n},t[0]=n,t[1]=r);let i=Eu(ty,r);return i.isLoading?null:i.isCapable}
function Cbd(e){let {sourceComposerScope:n,chatgpt:r,conversationId:i,executionTargetHostId:a,isComposerInputVisible:o,isDictationEnabled:s,productSurface:c,onDictationTranscriptInsert:p}=e;
let I=Eu(xUt,r!=null)&&p!=null,L;
let R=abd(L),z=l6u(r!=null),B=s&&z===!0,V=o&&B;
let {result}=W3u({chatgpt:r,enabled:B,productSurface:c,onTranscriptInsert:p});
return {...result,capability:z,enabled:B,presentation:I};}
function W3u(e){let n=Eu(xUt,e.chatgpt!=null),r;
r={...e,cleanupEnabled:!1,dictationAudio:Oi?.dictationAudio,streamingEnabled:n};return {result:G3u(r)};}
function G3u(e){let t=[],E=undefined;
let {chatgpt:n,onTranscriptStart:u,onTranscriptUpdate:d,onTranscriptCancel:f,onTranscriptInsert:p}=e;
let je=()=>({chatgpt:n,onTranscriptStart:u,onTranscriptUpdate:d,onTranscriptCancel:f,onTranscriptInsert:p,onTranscriptRetry:undefined});
let Me=fu(je),Ne;t[25]===E;
let Ee={current:null};return {streaming:e.streamingEnabled,record(){Ee.current=Me();return Ee.current},async stop(n,i){
let c=Ee.current!=null&&(Oq()||Ee.current.chatgpt!=null)?Ee.current:Me();
if(c.chatgpt!=null)return XLo(n,{signal:i.abortController.signal,language:i.language});
return YLo(n,{signal:i.abortController.signal,language:i.language})}};}
function f6u(e,t,n){if(t===\`transcription\`){return {message:'native transcription failure',canRetry:true}}return {message:'native start failure',canRetry:false}}
async function YLo(e,t={}){nativeCalls.push({blob:e,options:t,kind:'local'});return {text:'native local'};}
async function XLo(e,t){nativeCalls.push({blob:e,options:t,kind:'chatgpt'});return {text:'native chatgpt'};}
`;

function load({ apiService = true, nativeFast = true, quotaCompat = true } = {}) {
  const calls = [], nativeCalls = [];
  const policy = { authMethod: 'chatgpt', authLoading: false, rollout: true, rolloutLoading: false,
    workspaceRollout: true, workspaceRolloutLoading: false,
    workspace: { isLoading: false, isError: false, isCapable: true },
    configFeatures: { isLoading: false, isError: false, data: [{ name: 'in_app_dictation', enabled: true }] },
    settings: { isLoading: false, isError: false, isCapable: true } };
  const ig = {}, rg = {}, Ag = {}, Jh = {};
  const sandbox = {
    module: { exports: {} }, URL, DOMException, Reflect, Object,
    __cockpitEnhancements: { apiService, nativeFast, quotaCompat },
    __cockpitApiDictation: {
      ready: true, session: 'session-1',
      async transcribe(blob, options) { calls.push({ blob, options }); throw new Error('removed API transcription bridge must never execute'); },
    },
    nativeCalls, U: {}, ig, rg, Ag, Jh, Ls: 'local', policy,
    navigator: { mediaDevices: { getUserMedia() {} } }, MediaRecorder: class {},
    policySettings: () => [policy.settings],
    Y: (_scope, read) => read, wc: (_scope, read) => ({ read }),
    u6u: { c: () => [] }, xUt: {}, abd: () => null,
    Oi: {}, fu: value => value, Oq: () => false,
    SKt: () => policy.workspace,
  };
  const get = (atom, key) => {
    if (atom === ig) return key === '770071981' ? policy.workspaceRolloutLoading : policy.rolloutLoading;
    if (atom === rg) return key === '770071981' ? policy.workspaceRollout : policy.rollout;
    if (atom === Ag) return policy;
    if (atom === Jh) return policy.configFeatures;
    if (atom === 'workspace') return policy.workspace;
    if (typeof atom === 'function') return atom({ get });
    throw new Error('unexpected native policy');
  };
  sandbox.Eu = (selector, argument) => selector === sandbox.ty ? selector.read(argument, { get }) : true;
  sandbox.readPolicy = get;
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  for (const name of ['fast.js', 'quota.js']) {
    vm.runInContext(fs.readFileSync(path.join(__dirname, name), 'utf8'), sandbox);
  }
  return { sandbox, api: sandbox.__cockpitNativeFast, calls, nativeCalls, policy };
}


const fastFixtureSource = fs.readFileSync(path.join(__dirname, 'fast.test.cjs'), 'utf8');
const fastLiteral = fastFixtureSource.match(/const fixture = ([\s\S]*?);\r?\n\r?\nfunction load/);
assert.ok(fastLiteral, 'exercise the complete Fast structural contracts');
const fastFixture = vm.runInNewContext(fastLiteral[1]);
const quotaFixture = require('../../../../tests/helpers/codexNativeQuotaFixture.cjs')();
const original = fastFixture + fixture + quotaFixture.initial;

function evaluate(options) {
  const result = load(options);
  const transformed = result.api.prepareLive(original, moduleUrl);
  if (options?.apiService !== false || options?.nativeFast !== false || options?.quotaCompat !== false) {
    assert.equal(transformed.ok, true, JSON.stringify(transformed.diagnostics));
  }
  // The native initial module exports quota atoms. This VM executes the same
  // function bodies without its ESM export table; Chromium integration covers ESM.
  vm.runInContext(transformed.patchedSource.replace(/export\{[^}]+\};/g, ''), result.sandbox);
  return { ...result, transformed };
}

function composer(sandbox, host = 'local', chatgpt = null, enabled = true) {
  return sandbox.Cbd({ executionTargetHostId: host, chatgpt,
    isDictationEnabled: enabled, isComposerInputVisible: true,
    onDictationTranscriptInsert() {} });
}

for (const apiService of [true, false]) {
  test(`${apiService ? 'API Service' : 'ordinary OAuth'} retains native dictation entry and local transcription`, async () => {
    const { sandbox, calls, nativeCalls } = evaluate({ apiService });
    const control = composer(sandbox);
    assert.equal(control.capability, true);
    assert.equal(control.enabled, true);
    assert.equal(control.presentation, true);
    assert.equal(control.streaming, true);
    control.record();
    const blob = new Blob(['recorded audio']), signal = new AbortController().signal;
    assert.equal((await control.stop(blob, { abortController: { signal }, language: 'zh' })).text, 'native local');
    assert.equal(nativeCalls.length, 1);
    assert.equal(nativeCalls[0].kind, 'local');
    assert.equal(nativeCalls[0].blob, blob);
    assert.equal(nativeCalls[0].options.signal, signal);
    assert.equal(nativeCalls[0].options.language, 'zh');
    assert.equal(calls.length, 0);
    assert.equal(sandbox.__cockpitNativeDictation, undefined);
  });

  test(`${apiService ? 'API Service' : 'ordinary OAuth'} preserves native remote and ChatGPT transcription`, async () => {
    const { sandbox, calls, nativeCalls } = evaluate({ apiService });
    for (const control of [composer(sandbox, 'remote-host'), composer(sandbox, 'local', {})]) {
      assert.equal(control.capability, true);
      assert.equal(control.enabled, true);
      control.record();
      await control.stop(new Blob(['audio']), { abortController: new AbortController() });
    }
    assert.deepEqual(nativeCalls.map(call => call.kind), ['local', 'chatgpt']);
    assert.equal(calls.length, 0);
  });

  test(`${apiService ? 'API Service' : 'ordinary OAuth'} pure API credentials retain native capability denial`, () => {
    const { sandbox, policy, calls, nativeCalls } = evaluate({ apiService });
    policy.authMethod = 'apiKey';
    assert.equal(composer(sandbox).capability, false);
    assert.equal(composer(sandbox).enabled, false);
    assert.equal(calls.length, 0);
    assert.equal(nativeCalls.length, 0);
  });
}

test('Fast and quota switches cannot hide or force the native dictation entry', () => {
  for (const apiService of [true, false]) for (const nativeFast of [true, false]) for (const quotaCompat of [true, false]) {
    const { sandbox, transformed } = evaluate({ apiService, nativeFast, quotaCompat });
    assert.equal(composer(sandbox).capability, true);
    assert.equal(composer(sandbox).enabled, true);
    const nativeStart = transformed.patchedSource.indexOf('var kKt=');
    assert.equal(transformed.patchedSource.slice(nativeStart, nativeStart + fixture.trimStart().length), fixture.trimStart());
    assert.equal(transformed.patchedSource.includes('__cockpitApiDictation'), false);
    assert.equal(transformed.patchedSource.includes('__cockpitNativeDictation'), false);
    if (nativeFast) assert.equal((transformed.patchedSource.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length, 4);
    if (quotaCompat || apiService) assert.equal((transformed.patchedSource.match(/globalThis\.__cockpitQuotaCompat\.initialReady=true/g) || []).length, 1);
  }
});

test('native rollout, loading, workspace, hardware, configuration and user gates remain authoritative', () => {
  const mutations = [
    ({ policy }) => { policy.rollout = false; },
    ({ policy }) => { policy.rolloutLoading = true; },
    ({ policy }) => { policy.authLoading = true; },
    ({ policy }) => { policy.workspace.isCapable = false; },
    ({ policy }) => { policy.workspace.isLoading = true; },
    ({ policy }) => { policy.configFeatures.data = [{ name: 'in_app_dictation', enabled: false }]; },
    ({ policy }) => { policy.settings.isCapable = false; },
    ({ sandbox }) => { sandbox.navigator.mediaDevices = undefined; },
    ({ sandbox }) => { sandbox.MediaRecorder = undefined; },
  ];
  for (const apiService of [true, false]) for (const mutate of mutations) {
    const result = evaluate({ apiService });
    mutate(result);
    assert.notEqual(composer(result.sandbox).capability, true);
    assert.equal(composer(result.sandbox).enabled, false);
    assert.equal(result.calls.length, 0);
  }
  const { sandbox } = evaluate();
  assert.equal(composer(sandbox, 'local', null, false).enabled, false);
});

test('unsupported native module contracts fail without changing dictation or activating partial patches', () => {
  const { api } = load();
  const changed = original.replace('serviceTierAccess:', 'unknownAccess:');
  const result = api.prepareLive(changed, moduleUrl);
  assert.equal(result.ok, false);
  assert.equal(result.patchedSource, changed);
});


const hostSource = fs.readFileSync(path.join(__dirname, 'fast.rs'), 'utf8');
const stateLiteral = hostSource.match(/const STATE_SCRIPT: &str = r#"([\s\S]*?)"#;/);
assert.ok(stateLiteral, 'exercise the actual renderer upgrade state script');
const stateScript = stateLiteral[1].replace('__COCKPIT_STATE_OPTIONS__', JSON.stringify({
  nativeFast: true, quotaCompat: true, apiKeyAuthVerified: true, apiService: true, owner: 'rollback-owner',
}));

test('upgrading an existing renderer revokes the bridge and restores native capability without losing drafts', () => {
  for (const throwingDispose of [false, true]) {
    let disposals = 0;
    const oldState = { apiDictation: true, nativePatchState: 'active-capability',
      nativePatchMode: 'capability-only', nativePatchFeatures: '0001:capability-only' };
    const oldHook = { capability: () => false, scopedHandlers: { active: true } };
    const nativeHandlers = oldHook.scopedHandlers;
    const sandbox = {
      __cockpitEnhancements: oldState,
      __cockpitNativeDictation: oldHook,
      __cockpitApiDictation: { dispose() { disposals++; if (throwingDispose) throw Error('retired bridge cleanup failure'); } },
      document: { draft: 'unsent user work' },
      location: { reload() { throw Error('upgrade must not reload a running conversation'); } },
    };
    sandbox.window = sandbox;
    vm.runInNewContext(stateScript, sandbox);
    assert.equal(disposals, 1);
    assert.equal(oldState.apiDictation, false, 'already evaluated handlers can retain the previous state object');
    assert.equal(sandbox.__cockpitEnhancements.apiDictation, false);
    assert.equal(sandbox.__cockpitEnhancements.nativePatchState, 'pending');
    assert.equal(sandbox.__cockpitNativeDictation, oldHook, 'already evaluated modules retain their hook object');
    assert.equal(oldHook.scopedHandlers, nativeHandlers);
    for (const value of [true, false, null]) {
      assert.equal(oldHook.capability(value), value);
      assert.equal(oldHook.capability(value, false), value);
      assert.equal(oldHook.capability(value, true), value);
    }
    assert.equal(sandbox.document.draft, 'unsent user work');
  }
});

test('fresh renderer rollback state does not create a dictation bridge or force capability', () => {
  const sandbox = {};
  sandbox.window = sandbox;
  vm.runInNewContext(stateScript, sandbox);
  assert.equal(sandbox.__cockpitEnhancements.apiDictation, false);
  assert.equal(sandbox.__cockpitNativeDictation, undefined);
  assert.equal(sandbox.__cockpitApiDictation, undefined);
});
