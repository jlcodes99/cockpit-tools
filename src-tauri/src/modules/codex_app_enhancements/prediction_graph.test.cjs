// Synthetic native module graph fixtures; no shipped desktop source or credentials.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const fastSource = fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8');
const preloadSource = fs.readFileSync(path.join(__dirname, 'startup_preload.js'), 'utf8');
const base = 'app://-/assets/';
const initialUrl = base + 'app-initial-fixture.js';
const entryUrl = base + 'index-fixture.js';
function setup({ missing = null, duplicate = false, enabled = true } = {}) {
  const files = new Map([
    ['app://-/index.html', '<script src="./assets/index-fixture.js"></script>'],
    [entryUrl, 'import "./app-initial-fixture.js";import "./app-main-fixture.js";'],
    [initialUrl, 'import "./composer-content-fixture.js";export const initial=1;'],
    [base + 'app-main-fixture.js', 'import "./initialize-app-host-services-fixture.js";import "./settings-lazy-routes-fixture.js";'],
    [base + 'settings-lazy-routes-fixture.js', 'const routes=["./general-settings-fixture.js"' + (duplicate ? ',"./general-settings-other.js"' : '') + '];'],
    [base + 'composer-content-fixture.js', 'export const composer=1;'],
    [base + 'initialize-app-host-services-fixture.js', 'export const host=1;'],
    [base + 'general-settings-fixture.js', 'export const settings=1;'],
    [base + 'general-settings-other.js', 'export const settings=2;'],
  ]);
  if (missing) files.delete(base + missing);
  const transformed = [], ready = [], reads = [];
  const root = {
    __cockpitEnhancements: { nativeFast: false, quotaCompat: false, nativePredictions: enabled,
      apiKeyAuthVerified: true, apiService: false, owner: 'test' },
    __cockpitNativePredictions: {
      transform(source, url) {
        const kind = url.includes('app-initial-') ? 'initial' : url.includes('composer-content-') ? 'composer'
          : url.includes('initialize-app-host-services-') ? 'host' : 'settings';
        transformed.push(kind);
        return {ok: true, patchedSource: source + '\n/*prediction-' + kind + '*/', diagnostics: {kind}};
      },
      setReady(value) { ready.push(value); },
    },
    __cockpitQuotaCompat: {},
    location: {protocol: 'app:', hostname: '-', pathname: '/index.html'},
    document: {documentElement: null, scripts: [{src: entryUrl}]},
    URL, AbortSignal,
    fetch: async url => ({ok: files.has(url), text: async () => files.get(url)}),
  };
  const read = url => {
    reads.push(url);
    if (!files.has(url)) throw Error('fixture-unavailable');
    return files.get(url);
  };
  root.XMLHttpRequest = class {
    open(method, url) { assert.equal(method, 'GET'); this.url = url; }
    send() { this.responseText = read(this.url); this.status = 200; }
  };
  root.window = root;
  vm.runInNewContext(fastSource, root);
  return {root, files, read, transformed, ready, reads};
}

test('prediction-only startup discovers lazy native routes and retains canonical URLs', () => {
  const {root, files, read, transformed, ready} = setup();
  const result = root.__cockpitNativeFast.preparePredictionGraphSync(files.get(initialUrl), initialUrl, read,
    [{url: entryUrl, source: files.get(entryUrl)}]);
  assert.equal(result.ok, true);
  assert.deepEqual(Array.from(result.modules, module => module.predictionKind), ['composer', 'host', 'settings']);
  assert.equal(new Set(result.modules.map(module => module.url)).size, 3);
  assert.deepEqual(transformed, ['initial', 'composer', 'host', 'settings']);
  assert.deepEqual(ready, [], 'preparation must not enable an uncommitted graph');
});

test('async handoff and document-start preload use the same complete prediction graph', async () => {
  const {root, ready} = setup();
  const prepared = await root.__cockpitNativeFast.prepareDocument();
  assert.equal(prepared.ok, true);
  assert.equal(prepared.modules.length, 3);
  assert.ok(prepared.patchedSource.includes('cockpit-native-predictions-graph-v1'));
  let installed;
  root.__cockpitNativeFast.installModule = (source, url, modules) => { installed = {source, url, modules}; return {ok: true}; };
  vm.runInNewContext(preloadSource, root);
  assert.equal(root.__cockpitStartupPreload.ok, true);
  assert.equal(installed.url, initialUrl);
  assert.equal(installed.modules.length, 3);
  assert.deepEqual(ready, [], 'graph commit, not source preparation, activates native service');
});

test('missing companion does not publish partial prediction edits or break the base graph', async () => {
  const {root} = setup({missing: 'general-settings-fixture.js'});
  const result = await root.__cockpitNativeFast.prepareDocument();
  assert.equal(result.ok, true);
  assert.equal(result.modules.length, 0);
  assert.ok(!result.patchedSource.includes('cockpit-native-predictions-graph-v1'));
  assert.equal(root.__cockpitEnhancements.nativePredictionPatchState, 'unsupported');
});

test('multiple matching companion contracts disable predictions as a unit', () => {
  const {root, files, read} = setup({duplicate: true});
  const result = root.__cockpitNativeFast.preparePredictionGraphSync(files.get(initialUrl), initialUrl, read,
    [{url: entryUrl, source: files.get(entryUrl)}]);
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'prediction-module-not-unique');
});

test('disabled setting performs no source discovery and no runtime activation', () => {
  const {root, files, read, reads, transformed} = setup({enabled: false});
  const result = root.__cockpitNativeFast.preparePredictionGraphSync(files.get(initialUrl), initialUrl, read);
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'disabled');
  assert.deepEqual(reads, []);
  assert.deepEqual(transformed, []);
});


test('unsupported quota does not block prediction-only mode when Fast is disabled', async () => {
  const {root} = setup();
  root.__cockpitEnhancements.quotaCompat = true;
  root.__cockpitQuotaCompat.transform = () => ({ok: false, diagnostics: {reason: 'quota-contract-changed'}});
  const result = await root.__cockpitNativeFast.prepareDocument();
  assert.equal(result.ok, true);
  assert.equal(result.mode, 'prediction-only');
  assert.equal(result.quotaReason, 'quota-contract-changed');
  assert.equal(result.modules.length, 3);
  assert.ok(result.patchedSource.includes('cockpit-native-predictions-graph-v1'));
  assert.ok(!result.patchedSource.includes('__cockpitQuotaCompat'));
});


test('one committed import map activates predictions and a lazy companion failure leaves native base active', async () => {
  const {root, ready} = setup();
  const result = await root.__cockpitNativeFast.prepareDocument();
  assert.equal(result.ok, true);
  const links = [], maps = [], revoked = [];
  let sequence = 0;
  root.URL = class extends URL {
    static createObjectURL() { return 'blob:synthetic-' + (++sequence); }
    static revokeObjectURL(value) { revoked.push(value); }
  };
  root.Blob = class {};
  root.HTMLScriptElement = {supports: kind => kind === 'importmap'};
  const doc = {
    documentElement: null,
    createElement(tag) {
      const element = {tag, parentNode: null, relList: {supports: value => value === 'modulepreload'},
        appendChild(child) { child.parentNode = this; if (child.tag === 'link') links.push(child); if (child.type === 'importmap') maps.push(child); }};
      return element;
    },
    appendChild(child) { child.parentNode = this; },
    removeChild(child) { child.parentNode = null; },
    addEventListener() {}, removeEventListener() {},
  };
  root.document = doc;
  const installed = root.__cockpitNativeFast.installModule(result.patchedSource, initialUrl,
    result.modules, result.mode, result.quotaReason);
  assert.equal(installed.ok, true);
  assert.equal(maps.length, 1);
  assert.equal(Object.keys(JSON.parse(maps[0].textContent).imports).length, 4);
  assert.deepEqual(ready, [true]);
  root.__cockpitNativeFastComplete();
  assert.equal(root.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(revoked.length, 1, 'lazy native route blobs survive initial completion');
  assert.equal(typeof links[1].onerror, 'function');
  root.__cockpitNativeFastDispose();
  assert.equal(new Set(revoked).size, 4, 'disposal releases every graph blob');
  // Before the graph completes, lazy companion failures disable predictions
  // alone. Existing Fast/quota completion and canonical module mapping survive.
  const other = setup();
  const pending = await other.root.__cockpitNativeFast.prepareDocument();
  const failedLinks = [];
  other.root.URL = root.URL; other.root.Blob = root.Blob; other.root.HTMLScriptElement = root.HTMLScriptElement;
  other.root.document = {...doc, createElement(tag) {
    return {tag, parentNode:null, relList:{supports:()=>true}, appendChild(child) {
      child.parentNode=this; if(child.tag==='link')failedLinks.push(child);
    }};
  }};
  assert.equal(other.root.__cockpitNativeFast.installModule(pending.patchedSource,initialUrl,pending.modules,pending.mode).ok,true);
  failedLinks[1].onerror();
  assert.equal(other.root.__cockpitEnhancements.nativePredictionPatchState,'unsupported');
  other.root.__cockpitNativeFastComplete();
  assert.equal(other.root.__cockpitEnhancements.nativePatchState,'active');
  assert.equal(other.ready.at(-1),false);
});


test('owner handover cancels old prediction work and reactivates only a verified existing graph', () => {
  const stateSource = fs.readFileSync(path.join(__dirname, 'fast.rs'), 'utf8')
    .match(/const STATE_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  const predictionSource = fs.readFileSync(path.join(__dirname, 'predictions.js'), 'utf8');
  const root = {URL, DOMException, AbortController, Symbol, setTimeout, clearTimeout,
    __cockpitEnhancements: {nativeFast:false,quotaCompat:false,apiService:false,nativePredictions:true,
      apiKeyAuthVerified:true,owner:'owner-a'}};
  root.window = root; root.globalThis = root;
  vm.createContext(root);
  vm.runInContext(predictionSource, root);
  const original = root.__cockpitNativePredictions;
  original.runtime.register({managerForHost:()=>({}),nativeEnabled:()=>true});
  original.setReady(true);
  Object.assign(root.__cockpitEnhancements, {nativePatchState:'active', nativePatchMode:'full',
    nativePatchRequestedFeatures:'00101',nativePatchFeatures:'00101',nativePredictionPatchState:'ready'});
  assert.equal(original.runtime.allow('local'),true);
  const next = {...root.__cockpitEnhancements,owner:'owner-b'};
  vm.runInContext(stateSource.replace('__COCKPIT_STATE_OPTIONS__',JSON.stringify(next)),root);
  assert.equal(original.runtime.allow('local'),false,'owner change revokes the old job generation first');
  vm.runInContext(predictionSource,root);
  assert.equal(root.__cockpitNativePredictions,original,'native module binding is reused, not duplicated');
  assert.equal(root.__cockpitEnhancements.nativePatchState,'active');
  assert.equal(original.runtime.allow('local'),true,'verified committed module graph reactivates without reload');
  const disabled = {...next,owner:'owner-c',nativePredictions:false};
  vm.runInContext(stateSource.replace('__COCKPIT_STATE_OPTIONS__',JSON.stringify(disabled)),root);
  vm.runInContext(predictionSource,root);
  assert.equal(original.runtime.allow('local'),false);
  assert.equal(root.__cockpitEnhancements.nativePredictionPatchState,'disabled');
  original.runtime.dispose();
});
