const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');

// Execute the actual Rust-serialized outer guard, not a second implementation.
const rust = fs.readFileSync(path.join(__dirname, 'browser_preload.rs'), 'utf8');
const start = rust.indexOf('fn guarded_source(');
const end = rust.indexOf('\nasync fn discover', start);
const literal = /format!\(\s*("(?:\\.|[^"\\])*")\s*\)/.exec(rust.slice(start,end));
assert.ok(literal, 'guard format string must remain executable by this test');
const template = JSON.parse(literal[1]);
function run(location, document) {
  const window = {};
  const source = template.replace(/\{\{|\}\}|\{source\}/g, value => value === '{{' ? '{' : value === '}}' ? '}' : 'window.trustedRuntimeExecuted = true;');
  vm.runInNewContext(source, {location, document, window});
  return window;
}
const local = {protocol:'app:',hostname:'-',pathname:'/index.html'};

test('first-document guard executes trusted runtime only before local document starts', () => {
  assert.equal(run(local, {}).trustedRuntimeExecuted, true);
});
test('runImmediately on an already loaded document rejects before bootstrap or module preparation', () => {
  const window = run(local, {documentElement:{}});
  assert.equal(window.trustedRuntimeExecuted, undefined);
  assert.equal(window.__cockpitStartupPreload.ok, false);
  assert.equal(window.__cockpitStartupPreload.reason, 'document-already-started');
});
test('blank, external, wrong host and other local documents cannot execute the trusted runtime', () => {
  for (const location of [
    {protocol:'about:',hostname:'',pathname:'blank'},
    {...local, protocol:'https:'}, {...local,hostname:'untrusted'}, {...local,pathname:'/other.html'},
  ]) assert.deepEqual(run(location, {}), {});
});

const revokeStart = rust.indexOf('fn revoke_source(');
const revokeEnd = rust.indexOf('\nasync fn discover', revokeStart);
const revokeLiteral = /format!\(\s*("(?:\\.|[^"\\])*")\s*\)/.exec(rust.slice(revokeStart, revokeEnd));
assert.ok(revokeLiteral, 'revoke format string must remain executable by this test');
const revokeTemplate = JSON.parse(revokeLiteral[1]);
function revoke(window, location = local) {
  const source = revokeTemplate.replace(/\{\{|\}\}|\{owner\}/g, value => value === '{{' ? '{' : value === '}}' ? '}' : JSON.stringify('preload-9333'));
  return vm.runInNewContext(source, {location, window});
}
function pending(owner = 'preload-9333') {
  const state = {owner, nativeFast:true, quotaCompat:true, apiService:true, apiKeyAuthVerified:true,
    nativePatchState:'preloading'};
  const window = {__cockpitEnhancements:state, disposed:0,
    __cockpitNativeFastComplete() { state.nativePatchState = 'active'; },
    __cockpitNativeFastDispose() { window.disposed++; },
  };
  return window;
}
test('revoking browser-owned pending preload disables gates, disposes modules and removes the late completion callback', () => {
  const window = pending();
  assert.equal(revoke(window), true);
  assert.equal(window.disposed, 1);
  window.__cockpitNativeFastComplete?.();
  const state = window.__cockpitEnhancements;
  for (const key of ['nativeFast','quotaCompat','apiService','apiKeyAuthVerified','apiDictation']) assert.equal(state[key], false);
  assert.equal(state.nativePatchState, 'unsupported');
  assert.equal(state.nativeQuotaPatchState, 'disabled');
  assert.equal(state.nativePatchReason, 'browser-preload-revoked');
});
test('page-worker takeover owner is untouched by an older browser cleanup', () => {
  const window = pending('page-worker-unique');
  const before = {...window.__cockpitEnhancements};
  const callback = window.__cockpitNativeFastComplete;
  assert.equal(revoke(window), false);
  assert.equal(window.disposed, 0);
  assert.equal(window.__cockpitNativeFastComplete, callback);
  assert.deepEqual(window.__cockpitEnhancements, before);
});
test('a target navigated to a remote or unsupported document is untouched even with a matching owner', () => {
  for (const location of [{...local,protocol:'https:'},{...local,hostname:'other'},{...local,pathname:'/other.html'}]) {
    const window = pending();
    const before = {...window.__cockpitEnhancements};
    assert.equal(revoke(window,location), false);
    assert.deepEqual(window.__cockpitEnhancements,before);
    assert.equal(window.disposed,0);
  }
});
test('a throwing dispose hook cannot retain enabled gates or the pending completion callback', () => {
  const window = pending();
  window.__cockpitNativeFastDispose = () => { throw Error('already detached'); };
  assert.equal(revoke(window), true);
  assert.equal(window.__cockpitNativeFastComplete, undefined);
  assert.equal(window.__cockpitEnhancements.nativeFast, false);
  assert.equal(window.__cockpitEnhancements.quotaCompat, false);
  assert.equal(window.__cockpitEnhancements.apiService, false);
});
