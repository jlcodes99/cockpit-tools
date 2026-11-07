const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const { fixture } = require('./fast_fixture_current.cjs');
const script = fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8');
const initialUrl = 'app://-/assets/app-initial-fixture.js';
const primaryUrl = 'app://-/assets/app-primary-fixture.js';
const prefix = 'cockpit-native-handoff-v1:';

function harness({ failure = 'initial', config = {}, source = fixture } = {}) {
  const listeners = new Map(), elements = [], blobs = new Map(), timers = new Map();
  let serial = 0, primaryReads = 0;
  const initial = `${source}\nconst primary='./app-primary-fixture.js';`;
  const root = { module: { exports: {} }, Blob, AbortSignal, name: 'existing-name',
    location: { protocol: 'app:', hostname: '-' },
    __cockpitEnhancements: { nativeFast: true, quotaCompat: false, apiService: true,
      apiKeyAuthVerified: true, owner: 'independent-fast', ...config },
    setTimeout(fn) { timers.set(++serial, fn); return serial; }, clearTimeout(id) { timers.delete(id); },
    addEventListener: (key, fn) => listeners.set(key, fn), removeEventListener: key => listeners.delete(key),
    HTMLScriptElement: { supports: () => true },
    URL: class extends URL {
      static createObjectURL(blob) { const key = `blob:${++serial}`; blobs.set(key, blob); return key; }
      static revokeObjectURL(key) { blobs.delete(key); }
    },
    document: { scripts: [{ src: 'app://-/assets/index-fixture.js' }], documentElement: null,
      createElement(tag) { return { tag, relList: { supports: () => true }, appendChild(node) { elements.push(node); } }; },
      appendChild(node) { node.parentNode = this; this.documentElement = node; },
      removeChild(node) { node.parentNode = null; this.documentElement = null; },
      addEventListener() {}, removeEventListener() {},
    },
    fetch: async url => {
      if (url === primaryUrl) {
        primaryReads++;
        if (failure === 'fetch') throw Error('asset-unavailable');
      }
      return { ok: true, text: async () => url === initialUrl ? initial
        : url === primaryUrl ? 'export const originalPrimary=true;' : 'import "./app-initial-fixture.js";' };
    },
    __cockpitQuotaCompat: {
      transform: value => failure === 'initial'
        ? { ok: false, patchedSource: value, diagnostics: { reason: 'quota-shared-banner-contract-mismatch' } }
        : { ok: true, patchedSource: value + ';globalThis.__cockpitQuotaCompat.initialReady=true;', diagnostics: { bindings: {} } },
      transformComposer: value => failure === 'primary'
        ? { ok: false, patchedSource: value, diagnostics: { reason: 'quota-imports-missing' } }
        : { ok: true, patchedSource: value },
    },
  };
  root.window = root; root.globalThis = root;
  vm.runInNewContext(script, root);
  return { root, api: root.module.exports, listeners, elements, blobs, primaryReads: () => primaryReads };
}

for (const failure of ['initial', 'primary', 'fetch']) {
  test(`complete Fast survives ${failure} quota failure without retaining partial quota edits`, async () => {
    const app = harness({ failure });
    const prepared = await app.api.prepareDocument();
    assert.equal(prepared.ok, true);
    assert.equal(prepared.mode, 'fast-only');
    assert.ok(prepared.quotaReason);
    assert.equal(prepared.modules.length, 0);
    assert.equal(prepared.patchedSource.includes('__cockpitQuotaCompat'), false);
    assert.equal((prepared.patchedSource.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length, 4);
    if (failure === 'initial') assert.equal(app.primaryReads(), 0);
    const metadata = await app.api.prepareHandoff('independent');
    assert.equal(metadata.ok, true);
    assert.equal(metadata.mode, 'fast-only');
    assert.equal(metadata.source, undefined);
    app.listeners.get('beforeunload')();
    const next = harness(); next.root.name = app.root.name;
    assert.equal(next.api.consumeHandoff('independent').ok, true);
    assert.equal(next.root.name, 'existing-name');
    assert.equal(next.blobs.size, 1, 'only the fully validated Fast module is staged');
    assert.equal(next.root.__cockpitEnhancements.nativePatchState, 'preloading');
    next.root.__cockpitNativeFastComplete('quota');
    assert.equal(next.root.__cockpitEnhancements.nativePatchState, 'preloading');
    next.root.__cockpitNativeFastComplete();
    const state = next.root.__cockpitEnhancements;
    assert.equal(state.nativePatchState, 'active');
    assert.equal(state.nativePatchMode, 'fast-only');
    assert.equal(state.nativePatchFeatures, '1010');
    assert.equal(state.nativePatchRequestedFeatures, '1011');
    assert.equal(state.nativeQuotaPatchState, 'unsupported');
    assert.equal(state.nativeQuotaPatchReason, prepared.quotaReason);
    assert.equal(state.nativePatchReason, 'fast-only-quota-unsupported');
    app.api.cancelHandoff('independent'); next.api.cancelHandoff('independent');
  });
}

test('known complete quota and Fast contracts still require both modules to evaluate', async () => {
  const app = harness({ failure: null, config: { quotaCompat: true } });
  const prepared = await app.api.prepareDocument();
  assert.equal(prepared.mode, 'full'); assert.equal(prepared.modules.length, 1);
  assert.equal(app.api.installModule(prepared.patchedSource, prepared.url, prepared.modules, prepared.mode).ok, true);
  app.root.__cockpitNativeFastComplete();
  assert.equal(app.root.__cockpitEnhancements.nativePatchState, 'preloading');
  app.root.__cockpitNativeFastComplete('quota');
  assert.equal(app.root.__cockpitEnhancements.nativePatchFeatures, '1111');
  assert.equal(app.root.__cockpitEnhancements.nativeQuotaPatchState, 'active');
});

test('incomplete Fast or disabled Fast never receives a quota-failure fallback', async () => {
  for (const options of [ { config: { nativeFast: false } }, { source: fixture.replace('async readServiceTier(', 'async unsupportedTier(') },
    { source: fixture + fixture } ]) {
    const app = harness(options);
    assert.equal((await app.api.prepareHandoff('invalid')).ok, false);
    assert.equal(app.listeners.size, 0);
    assert.equal(app.root.name, 'existing-name');
    assert.equal(app.blobs.size, 0);
  }
});

test('fast-only handoff rejects partial quota code, inaccurate features and cross-instance reuse', async () => {
  const app = harness(); await app.api.prepareHandoff('tamper'); app.listeners.get('beforeunload')();
  const payload = JSON.parse(app.root.name.slice(prefix.length));
  for (const mutate of [
    value => { value.source += ';globalThis.__cockpitQuotaCompat.initialReady=true;'; },
    value => { value.features = value.requestedFeatures; },
    value => { value.modules = [{ url: primaryUrl, source: 'quota module' }]; },
    value => { value.quotaReason = null; },
    value => { value.quotaReason = 'unbounded sensitive error details'; },
    value => { value.owner = 'other-profile'; },
    value => { value.mode = 'full'; },
    value => { value.source = value.source.replace('window.__cockpitNativeFast?.allow(', 'unknown.allow('); },
  ]) {
    const changed = structuredClone(payload); mutate(changed);
    const next = harness(); next.root.name = prefix + JSON.stringify(changed);
    assert.equal(next.api.consumeHandoff('tamper').ok, false);
    assert.equal(next.root.name, 'existing-name'); assert.equal(next.blobs.size, 0);
  }
  const changedConfig = harness({ config: { quotaCompat: true } }); changedConfig.root.name = app.root.name;
  assert.equal(changedConfig.api.consumeHandoff('tamper').ok, false);
  app.api.cancelHandoff('tamper');
});

test('host reconnection preserves actual Fast-only features and revokes them after configuration changes', async () => {
  const source = fs.readFileSync(path.join(__dirname, 'fast.rs'), 'utf8');
  const stateScript = source.match(/const STATE_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  const app = harness();
  const prepared = await app.api.prepareDocument();
  assert.equal(app.api.installModule(prepared.patchedSource, prepared.url, [], prepared.mode, prepared.quotaReason).ok, true);
  app.root.__cockpitNativeFastComplete();
  const config = { nativeFast: true, quotaCompat: false, apiKeyAuthVerified: true, apiService: true, owner: 'reconnected' };
  vm.runInNewContext(stateScript.replace('__COCKPIT_STATE_OPTIONS__', JSON.stringify(config)), app.root);
  assert.equal(app.root.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(app.root.__cockpitEnhancements.nativePatchFeatures, '1010');
  assert.equal(app.root.__cockpitEnhancements.nativePatchRequestedFeatures, '1011');
  assert.equal(app.root.__cockpitEnhancements.nativeQuotaPatchReason, prepared.quotaReason);
  config.nativeFast = false;
  vm.runInNewContext(stateScript.replace('__COCKPIT_STATE_OPTIONS__', JSON.stringify(config)), app.root);
  assert.equal(app.root.__cockpitEnhancements.nativePatchState, 'pending');
  assert.equal(app.root.__cockpitEnhancements.nativeQuotaPatchState, undefined);
  assert.equal(app.api.allow('local', 'apiKey'), false);
});

test('page handover waits only for the matching browser installation and never replaces its pending owner', async () => {
  const source = fs.readFileSync(path.join(__dirname, 'fast.rs'), 'utf8');
  const probeSource = source.match(/const RUNTIME_PROBE_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  const app = harness({ config: { owner: 'preload-9222' } });
  const prepared = await app.api.prepareDocument();
  app.api.installModule(prepared.patchedSource, prepared.url, [], prepared.mode, prepared.quotaReason);
  const expected = { owner: 'preload-9222', nativeFast: true, quotaCompat: false,
    apiKeyAuthVerified: true, apiService: true };
  const probe = options => vm.runInNewContext(probeSource
    .replace('__COCKPIT_BROWSER_SNAPSHOT__', JSON.stringify({ ...expected, ...options }))
    .replace('__COCKPIT_WORKER_OWNER__', '"worker-new"'), app.root);
  assert.equal(probe().preloading, true);
  assert.equal(probe().installed, false);
  assert.equal(app.root.__cockpitEnhancements.owner, 'preload-9222');
  assert.equal(probe({ owner: 'preload-9223' }).preloading, false);
  assert.equal(probe({ nativeFast: false }).preloading, false);
  app.root.__cockpitNativeFastComplete();
  assert.equal(app.root.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(probe().preloading, false);
  const stateScript = source.match(/const STATE_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  vm.runInNewContext(stateScript.replace('__COCKPIT_STATE_OPTIONS__', JSON.stringify({ ...expected, owner: 'worker-new' })), app.root);
  assert.equal(probe().installed, true);
  assert.equal(app.root.__cockpitEnhancements.nativePatchState, 'active');
});

test('browser and page scripts share a single document installation in either order', async () => {
  const rust = fs.readFileSync(path.join(__dirname, 'fast.rs'), 'utf8');
  const template = rust.match(/const DOCUMENT_INSTALL_GUARD: &str = r#"([\s\S]*?)"#;/)[1];
  for (const owner of ['preload-9222', 'worker-first']) {
    const app = harness({ config: { owner } });
    const prepared = await app.api.prepareDocument();
    app.api.installModule(prepared.patchedSource, prepared.url, [], prepared.mode, prepared.quotaReason);
    const count = app.elements.length;
    app.root.name = prefix + JSON.stringify({ nonce: 'same-document', owner: 'worker-later', previousName: 'original-name' });
    vm.runInNewContext(template
      .replace('__COCKPIT_SKIPPED__', 'window.__cockpitNativeFast.discardHandoff("same-document", "worker-later");')
      .replace('__COCKPIT_SOURCE__', 'window.__cockpitEnhancements.owner="clobbered"; window.didBootstrap=true;'), app.root);
    assert.equal(app.root.didBootstrap, undefined);
    assert.equal(app.root.__cockpitEnhancements.owner, owner);
    assert.equal(app.root.name, 'original-name');
    assert.equal(app.api.installModule(prepared.patchedSource, prepared.url, [], prepared.mode, prepared.quotaReason).reason, 'installation-already-started');
    assert.equal(app.elements.length, count);
    app.root.__cockpitNativeFastComplete();
    assert.equal(app.root.__cockpitEnhancements.nativePatchState, 'active');
  }
});
