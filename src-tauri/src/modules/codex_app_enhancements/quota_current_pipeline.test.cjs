const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const { parse } = require('@babel/parser');
const dir = process.env.COCKPIT_NATIVE_QUOTA_CURRENT_DIR;
const prefix = 'cockpit-native-handoff-v1:';
const scripts = Object.fromEntries(['fast', 'quota', 'quota_current', 'startup_preload'].map(name => [name, fs.readFileSync(path.join(__dirname, `${name}.js`), 'utf8')]));
let fixture;
function files() {
  if (fixture) return fixture;
  const initialName = process.env.COCKPIT_NATIVE_QUOTA_CURRENT_INITIAL || 'app-initial-feb30b196d11.js';
  const initial = fs.readFileSync(path.join(dir, initialName), 'utf8');
  const dependent = prefix => {
    const names = [...new Set([...initial.matchAll(new RegExp(`["'\u0060]\\./(${prefix}-[A-Za-z0-9_-]+\\.js)["'\u0060]`, 'g'))].map(match => match[1]))];
    assert.equal(names.length, 1); return names[0];
  };
  const primaryName = dependent('app-primary'), adapterName = dependent('codex-composer-adapter');
  const initialUrl = `app://-/assets/${initialName}`, primaryUrl = `app://-/assets/${primaryName}`, adapterUrl = `app://-/assets/${adapterName}`;
  const indexUrl = 'app://-/assets/index-current-fixture.js';
  fixture = { initial, initialUrl, primaryUrl, adapterUrl, indexUrl, map: new Map([
    [initialUrl, initial], [primaryUrl, fs.readFileSync(path.join(dir, primaryName), 'utf8')],
    [adapterUrl, fs.readFileSync(path.join(dir, adapterName), 'utf8')],
    [indexUrl, `import './${initialName}';`], ['app://-/index.html', `<script type="module" src="./assets/index-current-fixture.js"></script>`],
  ]) };
  return fixture;
}
function browser({ config = {}, adapter = undefined, owner = 'current-owner' } = {}) {
  const f = files(), map = new Map(f.map), elements = [], blobs = new Map(), revoked = [], listeners = new Map(), timers = new Map();
  if (adapter === null) map.delete(f.adapterUrl); else if (typeof adapter === 'string') map.set(f.adapterUrl, adapter);
  let serial = 0;
  const root = { URL: class extends URL {
    static createObjectURL(blob) { const key = `blob:current-${++serial}`; blobs.set(key, blob); return key; }
    static revokeObjectURL(key) { revoked.push(key); }
  }, Blob, AbortSignal, module: { exports: {} }, name: 'existing-name',
    location: { protocol: 'app:', hostname: '-', pathname: '/index.html' },
    __cockpitEnhancements: { nativeFast: true, quotaCompat: false, apiKeyAuthVerified: true, apiService: true, owner, ...config },
    setTimeout(fn, delay) { const id = ++serial; if (delay === 0) queueMicrotask(fn); else timers.set(id, fn); return id; }, clearTimeout(id) { timers.delete(id); },
    addEventListener(key, fn) { listeners.set(key, fn); }, removeEventListener(key) { listeners.delete(key); },
    HTMLScriptElement: { supports: () => true },
    document: { scripts: [{ src: f.indexUrl }], documentElement: null,
      createElement(tag) { return { tag, relList: { supports: () => true }, appendChild(node) { elements.push(node); } }; },
      appendChild(node) { node.parentNode = this; this.documentElement = node; },
      removeChild(node) { node.parentNode = null; this.documentElement = null; },
      addEventListener() {}, removeEventListener() {},
    },
    fetch: async url => ({ ok: map.has(url), text: async () => map.get(url) }),
    XMLHttpRequest: class { open(method, url, async) { assert.equal(method, 'GET'); assert.equal(async, false); this.url = url; }
      send() { this.status = map.has(this.url) ? 200 : 404; this.responseText = map.get(this.url) || ''; } },
  };
  root.window = root; root.globalThis = root;
  for (const name of ['fast', 'quota', 'quota_current']) vm.runInNewContext(scripts[name], root);
  return { root, api: root.__cockpitNativeFast, files: f, map, elements, blobs, revoked, listeners };
}
const active = app => app.root.__cockpitEnhancements.nativePatchState;
function assertNoQuota(prepared) {
  assert.equal(prepared.mode, 'fast-only'); assert.equal(prepared.modules.length, 0);
  assert.doesNotMatch(prepared.patchedSource, /__cockpitQuotaCompat|quota-primary/);
  assert.equal((prepared.patchedSource.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length, 4);
}
function complete(app) {
  app.root.__cockpitNativeFastComplete(); app.root.__cockpitNativeFastComplete('quota-primary');
  assert.equal(active(app), 'preloading', 'initial and primary are insufficient without the adapter');
  app.root.__cockpitNativeFastComplete('quota-primary'); app.root.__cockpitNativeFastComplete('unknown');
  assert.equal(active(app), 'preloading'); assert.equal(app.revoked.length, 0);
  app.root.__cockpitNativeFastComplete('quota');
  assert.equal(active(app), 'active'); assert.equal(app.revoked.length, 3);
  assert.equal(app.root.__cockpitEnhancements.nativeQuotaPatchState, 'active');
}
async function staged() {
  const app = browser(); const result = await app.api.prepareHandoff('current-three');
  assert.equal(result.ok, true, JSON.stringify({ reason: result.reason, mode: result.mode, diagnostics: result.diagnostics })); assert.equal(result.mode, 'full');
  app.listeners.get('beforeunload')();
  return { app, payload: JSON.parse(app.root.name.slice(prefix.length)) };
}
test('real current prepareDocument rewrites and parses the complete three-module graph with native audio unchanged', { skip: !dir }, async () => {
  const app = browser(), result = await app.api.prepareDocument();
  assert.equal(result.ok, true, JSON.stringify({ reason: result.reason, mode: result.mode, diagnostics: result.diagnostics })); assert.equal(result.mode, 'full');
  assert.deepEqual([...result.modules].map(module => module.url), [app.files.primaryUrl, app.files.adapterUrl]);
  for (const module of [{ url: app.files.initialUrl, source: result.patchedSource }, ...result.modules]) {
    const ast = parse(module.source, { sourceType: 'module' });
    for (const node of ast.program.body.filter(node => node.type === 'ImportDeclaration'))
      assert.equal(node.source.value.startsWith('.'), false, 'static import resolves to the same app graph');
    if (vm.SourceTextModule) {
      const parsed = new vm.SourceTextModule(module.source);
      assert.ok(parsed.dependencySpecifiers.every(specifier => !specifier.startsWith('.')));
    }
  }
  const functions = parse(app.files.initial, { sourceType: 'module' }).program.body
    .filter(node => node.type === 'FunctionDeclaration').map(node => app.files.initial.slice(node.start, node.end));
  for (const [contract, markers] of [
    ['native capability', ['dictationEnabled', 'whisper_killswitch']],
    ['native multipart', ['fetch(`/transcribe`', 'multipart/form-data']],
    ['native request', ['safePost(`/transcribe`', 'onRequestSuccess']],
  ]) {
    const found = functions.filter(text => markers.every(marker => text.includes(marker)));
    assert.equal(found.length, 1, contract);
    const expected = await app.api.rewriteModule(found[0], app.files.initialUrl);
    assert.ok(result.patchedSource.includes(expected), `${contract}: exact native function survives full preparation`);
  }
  assert.doesNotMatch(result.patchedSource, /__cockpitNativeDictation|__cockpitApiDictation|cockpitDictationScope/);
});
test('real current handoff installs one graph and waits for initial, primary and adapter completion', { skip: !dir }, async () => {
  const { app, payload } = await staged(), next = browser(); next.root.name = app.root.name;
  assert.equal(payload.modules.length, 2);
  assert.equal(next.api.consumeHandoff('current-three').ok, true); assert.equal(next.root.name, 'existing-name');
  const maps = next.elements.filter(element => element.type === 'importmap'); assert.equal(maps.length, 1);
  assert.deepEqual(Object.keys(JSON.parse(maps[0].textContent).imports), [app.files.initialUrl, app.files.primaryUrl, app.files.adapterUrl]);
  assert.equal(next.blobs.size, 3); complete(next);
  assert.equal(next.root.__cockpitEnhancements.nativePatchFeatures, '1011');
  app.api.cancelHandoff('current-three'); next.api.cancelHandoff('current-three');
});
test('real current synchronous startup uses the same three-module completion protocol', { skip: !dir }, () => {
  const app = browser(); vm.runInNewContext(scripts.startup_preload, app.root);
  assert.equal(app.root.__cockpitStartupPreload.ok, true, JSON.stringify(app.root.__cockpitStartupPreload));
  assert.equal(app.blobs.size, 3); complete(app);
});
test('missing or changed current adapter never leaves partial quota edits in async preparation or startup', { skip: !dir }, async () => {
  for (const adapter of [null, 'export const unsupported=true;']) {
    const app = browser({ adapter }); const result = await app.api.prepareDocument();
    assert.equal(result.ok, true, JSON.stringify({ reason: result.reason, mode: result.mode, diagnostics: result.diagnostics })); assertNoQuota(result);
    const sync = browser({ adapter }); vm.runInNewContext(scripts.startup_preload, sync.root);
    assert.equal(sync.root.__cockpitStartupPreload.ok, true); assert.equal(sync.root.__cockpitStartupPreload.mode, 'fast-only');
    assert.equal(sync.blobs.size, 1);
    const source = await [...sync.blobs.values()][0].text(); assert.doesNotMatch(source, /__cockpitQuotaCompat|quota-primary/);
    sync.root.__cockpitNativeFastComplete(); assert.equal(active(sync), 'active');
    assert.equal(sync.root.__cockpitEnhancements.nativePatchFeatures, '1010');
    const disabled = browser({ adapter, config: { nativeFast: false } });
    assert.equal((await disabled.api.prepareHandoff('disabled')).ok, false);
    assert.equal(disabled.listeners.size, 0); assert.equal(disabled.blobs.size, 0);
    vm.runInNewContext(scripts.startup_preload, disabled.root); assert.equal(disabled.root.__cockpitStartupPreload.ok, false);
    assert.equal(disabled.blobs.size, 0);
  }
});
test('current handoff refuses wrong nonce, owner, paths, completion kinds and partial graphs', { skip: !dir }, async () => {
  const { app, payload } = await staged();
  const wrongNonce = browser(); wrongNonce.root.name = app.root.name;
  assert.equal(wrongNonce.api.consumeHandoff('wrong-nonce').ok, false); assert.equal(wrongNonce.blobs.size, 0);
  for (const mutate of [
    value => { value.owner = 'other-profile'; }, value => { value.modules.pop(); }, value => { value.modules.shift(); },
    value => { value.source = value.source.replace('globalThis.__cockpitQuotaCompat.initialLayout="adapter-v1";', ''); },
    value => { value.source += ';globalThis.__cockpitQuotaCompat.initialLayout="adapter-v1";'; },
    value => { value.modules.reverse(); }, value => { value.modules[1].url = 'https://example.test/assets/codex-composer-adapter-a.js'; },
    value => { value.modules[1].url = 'app://-/other/assets/codex-composer-adapter-a.js'; },
    value => { value.modules[1].source = value.modules[1].source.replace('window.__cockpitNativeFastComplete?.("quota")', ''); },
    value => { value.modules[0].source = value.modules[0].source.replace('"quota-primary"', '"quota"'); },
    value => { value.requestedFeatures = '1111'; }, value => { value.features = '1010'; },
    value => { value.modules.pop(); value.modules[0].source = value.modules[0].source.replace('"quota-primary"', '"quota"'); },
  ]) {
    const changed = { ...payload, modules: payload.modules.map(module => ({ ...module })) }; mutate(changed);
    const next = browser(); next.root.name = prefix + JSON.stringify(changed);
    const result = next.api.consumeHandoff('current-three');
    assert.equal(result.ok, false, `partial/tampered graph accepted: ${mutate}`);
    assert.equal(next.blobs.size, 0); assert.equal(next.elements.length, 0);
  }
  app.api.cancelHandoff('current-three');
});
test('late completions after adapter preload failure cannot activate the current graph', { skip: !dir }, async () => {
  const { app } = await staged(), next = browser(); next.root.name = app.root.name;
  assert.equal(next.api.consumeHandoff('current-three').ok, true);
  next.elements.filter(element => element.tag === 'link')[2].onerror();
  assert.equal(active(next), 'unsupported'); assert.equal(next.revoked.length, 3);
  for (const kind of ['initial', 'quota-primary', 'quota']) next.root.__cockpitNativeFastComplete(kind);
  assert.equal(active(next), 'unsupported'); app.api.cancelHandoff('current-three');
});

test('current quota-only and API-only startup preserve their exact requested feature masks', { skip: !dir }, () => {
  for (const [config, mask] of [
    [{ nativeFast: false, quotaCompat: true, apiKeyAuthVerified: false }, '0101'],
    [{ nativeFast: false, quotaCompat: false, apiKeyAuthVerified: true }, '0011'],
  ]) {
    const app = browser({ config }); vm.runInNewContext(scripts.startup_preload, app.root);
    assert.equal(app.root.__cockpitStartupPreload.ok, true);
    assert.equal(app.blobs.size, 3); complete(app);
    assert.equal(app.root.__cockpitEnhancements.nativePatchFeatures, mask);
    assert.equal(app.root.__cockpitEnhancements.nativePatchRequestedFeatures, mask);
    assert.equal(app.root.__cockpitEnhancements.nativePatchMode, 'full');
  }
});
