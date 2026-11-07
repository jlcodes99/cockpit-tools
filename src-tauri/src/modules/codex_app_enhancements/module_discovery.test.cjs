const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const scripts = Object.fromEntries(['fast', 'quota', 'quota_current', 'startup_preload'].map(name =>
  [name, fs.readFileSync(path.join(__dirname, `${name}.js`), 'utf8')]));
const entry = 'app://-/assets/index-fixture.js';
const main = 'app://-/assets/app-main-fixture.js';
const initial = 'app://-/assets/app-initial-fixture.js';
function load(files = new Map()) {
  const reads = [], elements = [], blobs = new Map();
  let serial = 0;
  const read = url => { reads.push(url); if (!files.has(url)) throw Error('asset-unavailable'); return files.get(url); };
  const root = { module: { exports: {} }, Blob, AbortSignal,
    URL: class extends URL {
      static createObjectURL(blob) { const id = `blob:fixture-${++serial}`; blobs.set(id, blob); return id; }
      static revokeObjectURL(id) { blobs.delete(id); }
    },
    __cockpitEnhancements: { nativeFast: true, apiKeyAuthVerified: true, apiService: true, quotaCompat: false, owner: 'fixture' },
    location: { protocol: 'app:', hostname: '-', pathname: '/index.html' },
    setTimeout(fn, delay) { if (!delay) queueMicrotask(fn); return ++serial; }, clearTimeout() {},
    addEventListener() {}, removeEventListener() {}, HTMLScriptElement: { supports: () => true },
    document: { scripts: [{ src: entry }], documentElement: null,
      createElement(tag) { return { tag, relList: { supports: () => true }, appendChild(node) { elements.push(node); } }; },
      appendChild(node) { node.parentNode = this; this.documentElement = node; },
      removeChild(node) { node.parentNode = null; this.documentElement = null; },
      addEventListener() {}, removeEventListener() {},
    },
    fetch: async url => ({ ok: files.has(url), text: async () => read(url) }),
    XMLHttpRequest: class { open(method, url) { this.url = url; } send() { this.responseText = read(this.url); this.status = 200; } },
  };
  root.window = root;
  for (const name of ['fast', 'quota', 'quota_current']) vm.runInNewContext(scripts[name], root);
  return { root, api: root.__cockpitNativeFast, read, reads, elements, blobs };
}
for (const indirect of [false, true]) test(`module discovery supports ${indirect ? '26.1027 bootstrap' : 'direct legacy import'} in both paths`, async () => {
  const files = new Map([[entry, indirect ? 'await import(`./app-main-fixture.js`)' : 'import "./app-initial-fixture.js"'],
    [main, 'import {a} from "./app-initial-fixture.js"; import "./unrelated.js"']]);
  const app = load(files);
  for (const resolve of [app.api.resolveInitialModuleSync, app.api.resolveInitialModule]) {
    app.reads.length = 0;
    const result = await resolve([entry, entry], app.read);
    assert.equal(result.ok, true); assert.equal(result.url, initial);
    assert.deepEqual(app.reads, indirect ? [entry, main] : [entry]);
  }
});
test('missing, conflicting and excessive modules have distinct bounded diagnostics', async () => {
  for (const [source, reason, extra] of [
    ['// import "./app-initial-comment.js"\nconst unrelated="https://external/app-initial-x.js";', 'initial-module-not-found'],
    ['import "./app-initial-a.js"; import "./app-main-fixture.js";', 'ambiguous-module', 'import "./app-initial-b.js";'],
    [Array.from({length:5},(_,i)=>`import "./app-main-${i}.js"`).join(';'), 'ambiguous-bootstrap'],
  ]) {
    const app = load(new Map([[entry, source], [main, extra]]));
    const result = await app.api.resolveInitialModule([entry], app.read);
    assert.equal(result.reason, reason);
    assert.ok(app.reads.length <= 2);
  }
  const app = load();
  const result = await app.api.resolveInitialModule(['https://external/assets/index-x.js'], app.read);
  assert.equal(result.reason, 'unsupported-entry'); assert.equal(app.reads.length, 0);
});
test('missing bootstrap fails without selecting unrelated modules', async () => {
  const app = load(new Map([[entry, 'import "./app-main-fixture.js";']]));
  await assert.rejects(app.api.resolveInitialModule([entry], app.read), /asset-unavailable/);
});

// Opt-in local artifact verification: only transform/parse installed source, never
// execute the native app module or read credentials, conversations or live UI.
const artifact = process.env.COCKPIT_FAST_CLIENT_ENTRY;
test('installed client bootstrap, async reconnect and preload produce a valid Fast-only module', { skip: !artifact }, async () => {
  const dir = path.dirname(artifact), files = new Map();
  const entryUrl = `app://-/assets/${path.basename(artifact)}`;
  const entrySource = fs.readFileSync(artifact, 'utf8');
  files.set(entryUrl, entrySource);
  for (const name of fs.readdirSync(dir).filter(name => /^app-(main|initial|primary)-[\w-]+\.js$/.test(name)))
    files.set(`app://-/assets/${name}`, fs.readFileSync(path.join(dir, name), 'utf8'));
  files.set('app://-/index.html', `<script type="module" src="./assets/${path.basename(artifact)}"></script>`);
  const app = load(files); app.root.document.scripts = [{ src: entryUrl }];
  const prepared = await app.api.prepareDocument();
  assert.equal(prepared.ok, true, prepared.reason);
  assert.equal(prepared.mode, 'fast-only');
  assert.equal((prepared.patchedSource.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length, 4);
  assert.doesNotMatch(prepared.patchedSource, /globalThis\.__cockpitQuotaCompat/);
  const parsed = new vm.SourceTextModule(prepared.patchedSource);
  assert.ok(parsed.dependencySpecifiers.every(value => !value.startsWith('.')));
  const sync = load(files);
  vm.runInNewContext(scripts.startup_preload, sync.root);
  assert.equal(sync.root.__cockpitStartupPreload.ok, true, sync.root.__cockpitStartupPreload.reason);
  assert.equal(sync.root.__cockpitEnhancements.nativePatchState, 'preloading');
  const map = JSON.parse(sync.elements.find(e => e.type === 'importmap').textContent);
  assert.deepEqual(Object.keys(map.imports), [prepared.url]);
  const blob = sync.blobs.get(map.imports[prepared.url]);
  new vm.SourceTextModule(await blob.text());
  sync.root.__cockpitNativeFastComplete();
  assert.equal(sync.root.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(sync.root.__cockpitEnhancements.nativePatchMode, 'fast-only');
  assert.equal(sync.root.__cockpitEnhancements.nativeQuotaPatchState, 'unsupported');
});

test('background module preparation yields without clamped timers', async () => {
  const source = '/*' + 'x'.repeat(300000) + '*/\nimport "./dependency.js";';
  const app = load();
  app.root.setTimeout = () => { throw Error('background timer must not be used'); };
  let yields = 0;
  app.root.scheduler = { yield: async () => { yields++; } };
  assert.match(await app.api.rewriteModule(source, initial), /app:\/\/-\/assets\/dependency.js/);
  assert.ok(yields > 0);
  delete app.root.scheduler;
  app.root.MessageChannel = require('node:worker_threads').MessageChannel;
  assert.match(await app.api.rewriteModule(source, initial), /app:\/\/-\/assets\/dependency.js/);
});
