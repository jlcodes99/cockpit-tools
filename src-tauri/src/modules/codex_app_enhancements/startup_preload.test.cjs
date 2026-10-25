const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, 'startup_preload.js'), 'utf8');
const indexUrl = 'app://-/assets/index-fixture.js';
const initialUrl = 'app://-/assets/app-initial-fixture.js';
const primaryUrl = 'app://-/assets/app-primary-fixture.js';

function run({ documentElement = null, pathname = '/index.html', initialMode = 'full',
  quotaFailure = null, primaryFailure = null, fastOnlyFailure = null, missingPrimary = false } = {}) {
  const files = new Map([
    ['app://-/index.html', `<script type="module" src="./assets/index-fixture.js"></script>`],
    [indexUrl, `import "./app-initial-fixture.js";`],
    [initialUrl, `const primary="./app-primary-fixture.js";`],
    [primaryUrl, `export const value = 1;`],
  ]);
  if (missingPrimary) files.delete(primaryUrl);
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
        return { ok: true, url, mode: initialMode,
          patchedSource: value + (initialMode === 'full' ? '\n/*quota-initial*/' : '\n/*fast-only*/'),
          ...(initialMode === 'fast-only' ? { quotaReason: 'quota-shared-banner-contract-mismatch' } : {}),
          diagnostics: { reason: 'ready' } };
      },
      prepareFastOnly(value, url, quotaReason) {
        calls.push(['fast-only', url, value, quotaReason]);
        if (fastOnlyFailure) return { ok: false, diagnostics: { reason: fastOnlyFailure } };
        return { ok: true, url, patchedSource: value + '\n/*fast-only*/', mode: 'fast-only', quotaReason };
      },
      rewriteModuleSync(value, url) {
        calls.push(['rewrite', url]);
        return value.replace(/\.\/app-primary-[A-Za-z0-9_-]+\.js/g, primaryUrl);
      },
      installModule(initial, url, modules, mode, quotaReason) {
        calls.push(['install', url, modules, mode, quotaReason, initial]);
        return { ok: true };
      },
    },
    __cockpitQuotaCompat: {
      transform(value, url) {
        calls.push(['initial-transform', url]);
        if (quotaFailure) return { ok: false, diagnostics: { reason: quotaFailure } };
        return { ok: true, patchedSource: value, diagnostics: { bindings: { hardBlocked: 'h', threadProvider: 'p' } } };
      },
      transformComposer(value, url, initial, bindings) {
        calls.push(['composer-transform', url, initial, bindings]);
        if (primaryFailure) return { ok: false, diagnostics: { reason: primaryFailure } };
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

test('an already prepared Fast-only module never reads or transforms the unsupported quota primary', () => {
  const { root, calls } = run({ initialMode: 'fast-only', missingPrimary: true });
  const install = calls.find(([kind]) => kind === 'install');
  assert.equal(root.__cockpitStartupPreload.ok, true);
  assert.equal(root.__cockpitStartupPreload.mode, 'fast-only');
  assert.equal(root.__cockpitStartupPreload.quotaReason, 'quota-shared-banner-contract-mismatch');
  assert.equal(install[2].length, 0);
  assert.equal(install[3], 'fast-only');
  assert.equal(install[4], 'quota-shared-banner-contract-mismatch');
  assert.equal(calls.some(([kind]) => kind === 'initial-transform' || kind === 'composer-transform'), false);
});

test('quota contract or primary failure restarts from the untouched initial source', () => {
  for (const options of [
    { quotaFailure: 'quota-shared-banner-contract-mismatch' },
    { primaryFailure: 'quota-imports-missing' },
    { missingPrimary: true },
  ]) {
    const { root, calls } = run(options);
    const fallback = calls.find(([kind]) => kind === 'fast-only');
    const install = calls.find(([kind]) => kind === 'install');
    assert.equal(root.__cockpitStartupPreload.ok, true);
    assert.equal(install[3], 'fast-only');
    assert.equal(install[2].length, 0);
    assert.equal(install[4], fallback[3]);
    assert.equal(fallback[2], 'const primary="./app-primary-fixture.js";');
    assert.match(install[5], /\/\*fast-only\*\//);
    assert.doesNotMatch(install[5], /quota-initial|NativeFastComplete\?\.\("quota"\)/);
    assert.equal(root.__cockpitEnhancements.quotaCompat, true, 'requested configuration stays intact');
  }
});

test('quota failure never bypasses failed native Fast gates', () => {
  const { root, calls } = run({ primaryFailure: 'quota-imports-missing', fastOnlyFailure: 'unsupported-gates' });
  assert.equal(root.__cockpitStartupPreload.ok, false);
  assert.equal(root.__cockpitStartupPreload.reason, 'unsupported-gates');
  assert.equal(calls.some(([kind]) => kind === 'install'), false);
});

const hostSource = fs.readFileSync(path.join(__dirname, 'fast.rs'), 'utf8');
const stateLiteral = hostSource.match(/const STATE_SCRIPT: &str = r#"([\s\S]*?)"#;/);
assert.ok(stateLiteral);
function upgradeState(previous, overrides = {}) {
  const next = { nativeFast: true, quotaCompat: false, apiKeyAuthVerified: true, apiService: true,
    owner: 'new-owner', ...overrides };
  const sandbox = { __cockpitEnhancements: previous };
  sandbox.window = sandbox;
  vm.runInNewContext(stateLiteral[1].replace('__COCKPIT_STATE_OPTIONS__', JSON.stringify(next)), sandbox);
  return sandbox.__cockpitEnhancements;
}

test('host reattachment retains verified Fast-only activation and its separate quota failure', () => {
  const previous = { nativePatchState: 'active', nativePatchMode: 'fast-only',
    nativePatchRequestedFeatures: '1011', nativePatchFeatures: '1010',
    nativePatchReason: 'fast-only-quota-unsupported', nativeQuotaPatchState: 'unsupported',
    nativeQuotaPatchReason: 'quota-imports-missing' };
  const next = upgradeState(previous);
  assert.equal(next.nativePatchState, 'active');
  assert.equal(next.nativePatchFeatures, '1010');
  assert.equal(next.nativePatchReason, 'fast-only-quota-unsupported');
  assert.equal(next.nativeQuotaPatchState, 'unsupported');
  assert.equal(next.nativeQuotaPatchReason, 'quota-imports-missing');
});

test('host rejects stale or overstated Fast-only feature masks', () => {
  const previous = { nativePatchState: 'active', nativePatchMode: 'fast-only',
    nativePatchRequestedFeatures: '1011', nativePatchFeatures: '1010',
    nativeQuotaPatchState: 'unsupported', nativeQuotaPatchReason: 'quota-imports-missing' };
  for (const next of [
    upgradeState({ ...previous, nativePatchFeatures: '1011' }),
    upgradeState({ ...previous, nativePatchRequestedFeatures: '1111' }),
    upgradeState(previous, { nativeFast: false }),
  ]) {
    assert.equal(next.nativePatchState, 'pending');
    assert.equal(next.nativeQuotaPatchState, undefined);
  }
});
