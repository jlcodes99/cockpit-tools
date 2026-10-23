const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, 'startup_preload.js'), 'utf8');
const indexUrl = 'app://-/assets/index-fixture.js';
const initialUrl = 'app://-/assets/app-initial-fixture.js';
const primaryUrl = 'app://-/assets/app-primary-fixture.js';

function run({ documentElement = null, pathname = '/index.html' } = {}) {
  const files = new Map([
    ['app://-/index.html', `<script type="module" src="./assets/index-fixture.js"></script>`],
    [indexUrl, `import "./app-initial-fixture.js";`],
    [initialUrl, `const primary="./app-primary-fixture.js";`],
    [primaryUrl, `export const value = 1;`],
  ]);
  const calls = [];
  const root = {
    location: { protocol: 'app:', hostname: '-', pathname },
    document: { documentElement },
    __cockpitEnhancements: { quotaCompat: true, apiService: true, owner: 'preload-test' },
    XMLHttpRequest: class {
      open(method, url) {
        this.url = url;
        assert.equal(method, 'GET');
      }
      send() {
        const value = files.get(this.url);
        if (value == null) throw new Error('missing fixture');
        this.status = 200;
        this.responseText = value;
      }
    },
    __cockpitNativeFast: {
      localAsset(url, entry = false) {
        return entry
          ? /^app:\/\/-\/assets\/index-[A-Za-z0-9_-]+\.js$/.test(url)
          : /^app:\/\/-\/assets\/app-initial-[A-Za-z0-9_-]+\.js$/.test(url);
      },
      prepareLive(value, url) {
        return { ok: true, url, patchedSource: value, diagnostics: { reason: 'ready' } };
      },
      rewriteModuleSync(value, url) {
        calls.push(['rewrite', url]);
        return value.replace(/\.\/app-primary-[A-Za-z0-9_-]+\.js/g, primaryUrl);
      },
      installModule(initial, url, modules, mode) {
        calls.push(['install', url, modules, mode]);
        return { ok: true };
      },
    },
    __cockpitQuotaCompat: {
      transform(value, url) {
        calls.push(['initial-transform', url]);
        return { ok: true, patchedSource: value, diagnostics: { bindings: { hardBlocked: 'h', threadProvider: 'p' } } };
      },
      transformComposer(value, url, initial, bindings) {
        calls.push(['composer-transform', url, initial, bindings]);
        return { ok: true, patchedSource: value };
      },
    },
    URL,
  };
  root.window = root;
  vm.runInNewContext(source, root);
  return { root, calls };
}

test('preloads both modules synchronously before the document starts', () => {
  const { root, calls } = run();
  assert.deepEqual(JSON.parse(JSON.stringify(root.__cockpitStartupPreload)), {
    ok: true,
    reason: 'preloaded-before-document',
    initialUrl,
    primaryUrls: [primaryUrl],
  });
  const install = calls.find(([kind]) => kind === 'install');
  assert.ok(install);
  assert.equal(install[1], initialUrl);
  assert.equal(install[2].length, 1);
  assert.equal(install[2][0].url, primaryUrl);
  assert.equal(install[3], 'full');
  assert.match(install[2][0].source, /__cockpitNativeFastComplete\?\.\("quota"\)/);
});

test('never prepares an already started document', () => {
  const { root, calls } = run({ documentElement: {} });
  assert.deepEqual(JSON.parse(JSON.stringify(root.__cockpitStartupPreload)), {
    ok: false,
    reason: 'document-already-started',
  });
  assert.deepEqual(calls, []);
});

test('ignores detached and auxiliary application windows', () => {
  const { root, calls } = run({ pathname: '/detached-window.html' });
  assert.equal(root.__cockpitStartupPreload, undefined);
  assert.deepEqual(calls, []);
});
