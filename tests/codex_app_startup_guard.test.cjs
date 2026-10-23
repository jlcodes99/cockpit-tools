const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const { test } = require('node:test');
const source = fs.readFileSync(path.resolve('src-tauri/src/modules/codex_app_enhancements/startup_guard.js'), 'utf8');
function start(options = {}) {
  const listeners = new Map(); let reloads = 0, time = 0;
  const editor = { isConnected: true, value: options.draft || '', getClientRects: () => [1],
    closest: query => query.includes('data-composer-placement="home"') && options.home ? {} : null,
    querySelector: () => options.embedded ? {} : null };
  Object.defineProperty(editor, 'textContent', { get: () => editor.value });
  if (options.home) editor.pmViewDesc = { node: {
    get textContent() { return editor.value; },
    toJSON: () => options.document || ({ type: 'doc', content: [{ type: 'paragraph',
      content: [{ type: 'text', text: editor.value }] }] }),
  } };
  const item = { isConnected: true, getClientRects: () => [1], closest: () => null };
  const sandbox = { Date: { now: () => time }, location: { protocol: 'app:', hostname: '-', reload() { reloads++; } },
    performance: options.noHistory ? {} : { eventCounts: new Map(options.history || []) },
    __cockpitEnhancements: { quotaCompat: options.quotaCompat ?? true, ...(options.active ? { nativePatchState: 'active' } : {}) },
    document: { readyState: options.loading ? 'loading' : 'complete',
      addEventListener: (key, fn) => listeners.set(key, fn), removeEventListener: key => listeners.delete(key),
      querySelectorAll: query => query.startsWith('.ProseMirror') ? options.noEditor ? [] : [editor]
        : query.startsWith('[role="menu"]') ? options.menu ? [item] : [] : options.protected ? [item] : [] },
  };
  sandbox.window = sandbox; sandbox.top = sandbox;
  vm.runInNewContext(source, sandbox);
  return { guard: sandbox.__cockpitStartupGuard, sandbox, listeners, editor,
    event: (type, extra = {}) => listeners.get(type)({ type, isTrusted: true, ...extra }),
    advance: value => { time = value; }, reloads: () => reloads };
}
test('clean startup reload is claimed and executed once', () => {
  const env = start();
  assert.equal(env.guard.reload().ok, true); assert.equal(env.reloads(), 1);
  assert.equal(env.guard.reload().reason, 'already-claimed'); assert.equal(env.reloads(), 1);
});
test('drafts, attachments, history and active task markers prevent startup reload', () => {
  for (const options of [{ draft: 'unsent' }, { embedded: true }, { protected: true }, { noEditor: true }, { active: true }]) {
    const env = start(options); assert.equal(env.guard.reload().ok, false); assert.equal(env.reloads(), 0);
  }
});
test('ordinary interaction before attaching only delays activation until idle', () => {
  for (const key of ['keydown', 'pointerdown', 'beforeinput', 'paste', 'drop']) {
    const env = start({ history: [[key, 1]] });
    assert.equal(env.guard.reload().reason, 'interaction-settling');
    env.advance(1000);
    assert.equal(env.guard.reload().ok, true, key); assert.equal(env.reloads(), 1);
  }
});
test('clicks and key presses during preparation settle without permanently disabling controls', () => {
  const env = start();
  env.advance(8000); env.event('pointerdown');
  assert.equal(env.guard.reload().reason, 'ui-busy');
  env.advance(8100); env.event('pointerup');
  assert.equal(env.guard.reload().reason, 'interaction-settling');
  env.advance(9100);
  assert.equal(env.guard.reload().ok, true); assert.equal(env.reloads(), 1);
});
test('new native event counts reset the quiet interval even when no listener observed the event', () => {
  const env = start();
  env.advance(5000); env.sandbox.performance.eventCounts.set('keydown', 1);
  assert.equal(env.guard.inspect(), 'interaction-settling');
  env.advance(5999); assert.equal(env.guard.inspect(), 'interaction-settling');
  env.advance(6000); assert.equal(env.guard.reload().ok, true);
});
test('held pointer, IME composition and open native menus stay protected', () => {
  for (const [begin, end] of [['pointerdown', 'pointercancel'], ['compositionstart', 'compositionend']]) {
    const env = start(); env.event(begin); env.advance(2000);
    assert.equal(env.guard.reload().reason, 'ui-busy'); assert.equal(env.reloads(), 0);
    env.event(end); env.advance(3000); assert.equal(env.guard.reload().ok, true);
  }
  const options = { menu: true }, env = start(options);
  assert.equal(env.guard.reload().reason, 'ui-busy'); assert.equal(env.reloads(), 0);
  options.menu = false; assert.equal(env.guard.reload().ok, true);
});
test('typing during preparation still protects the draft after input settles', () => {
  const env = start(); env.advance(8000); env.event('beforeinput'); env.editor.value = 'unsent work';
  assert.equal(env.guard.reload().reason, 'interaction-settling');
  env.advance(10000);
  assert.equal(env.guard.reload().reason, 'draft-present'); assert.equal(env.reloads(), 0);
});
test('synthetic render events do not masquerade as user activity', () => {
  const env = start(); env.event('beforeinput', { isTrusted: false });
  assert.equal(env.guard.reload().ok, true);
});
test('missing optional event counters do not block activation; loading and expiry still protect', () => {
  assert.equal(start({ noHistory: true }).guard.reload().ok, true);
  assert.equal(start({ loading: true }).guard.reload().reason, 'document-loading');
  const env = start(); env.advance(45001);
  assert.equal(env.guard.reload().reason, 'startup-expired'); assert.equal(env.reloads(), 0);
});
test('reinjection retains the current idle window and does not reset its claim', () => {
  const env = start(); env.event('keydown');
  vm.runInNewContext(source, env.sandbox);
  assert.equal(env.sandbox.__cockpitStartupGuard, env.guard);
  assert.equal(env.guard.reload().reason, 'interaction-settling');
  env.advance(1000); assert.equal(env.guard.reload().ok, true);
  vm.runInNewContext(source, env.sandbox);
  assert.equal(env.guard.reload().reason, 'already-claimed');
  env.guard.dispose(); assert.equal(env.listeners.size, 0);
});
test('draft check and reload happen in one synchronous call', () => {
  const env = start(); env.editor.value = 'just typed';
  assert.equal(env.guard.reload().reason, 'draft-present'); assert.equal(env.reloads(), 0);
  env.editor.value = '';
  assert.equal(env.guard.reload().ok, true); assert.equal(env.reloads(), 1);
});

test('exact durable home draft proof permits one reload without changing any text', () => {
  const env = start({ home: true, draft: '你好\nunsent' });
  const snapshot = env.guard.snapshotDraft();
  assert.equal(snapshot.placement, 'home');
  assert.equal(env.guard.inspect(), 'draft-present');
  const reordered = JSON.parse(JSON.stringify(snapshot, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
    ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item));
  assert.equal(env.guard.approveSavedDraft(reordered).ok, true);
  assert.equal(env.guard.reload().ok, true);
  assert.equal(env.editor.value, '你好\nunsent');
  assert.equal(env.reloads(), 1);
  assert.equal(env.guard.reload().reason, 'already-claimed');
});

test('proof never admits thread composers, rich drafts, attachments, tasks or unknown editors', () => {
  for (const options of [
    { draft: 'unsent' }, { home: true, draft: 'unsent', quotaCompat: false },
    { home: true, draft: 'unsent', protected: true },
    { home: true, draft: 'unsent', document: { type: 'doc', content: [{ type: 'image' }] } },
    { home: true, draft: 'unsent', document: { type: 'doc', content: [{ type: 'text', text: 'unsent', marks: [{ type: 'bold' }] }] } },
  ]) {
    const env = start(options);
    assert.equal(env.guard.snapshotDraft(), null);
    assert.equal(env.guard.approveSavedDraft({ placement: 'home', document: {} }).ok, false);
    assert.equal(env.guard.reload().ok, false);
    assert.equal(env.reloads(), 0);
  }
});

test('typing, composition, document changes and expired proof cancel the approval', () => {
  for (const mutation of [
    env => { env.editor.value = 'new unsaved input'; },
    env => env.event('beforeinput'),
    env => env.event('compositionstart'),
    env => env.advance(2001),
    env => { env.sandbox.__cockpitEnhancements.quotaCompat = false; },
  ]) {
    const env = start({ home: true, draft: 'saved' });
    assert.equal(env.guard.approveSavedDraft(env.guard.snapshotDraft()).ok, true);
    mutation(env);
    assert.equal(env.guard.reload().ok, false);
    assert.equal(env.reloads(), 0);
  }
  const env = start({ home: true, draft: 'saved' });
  const stale = env.guard.snapshotDraft();
  env.editor.value = 'newer';
  assert.equal(env.guard.approveSavedDraft(stale).ok, false);
});

test('new busy state after disk proof still prevents the final navigation', () => {
  const options = { home: true, draft: 'saved' }, env = start(options);
  assert.equal(env.guard.approveSavedDraft(env.guard.snapshotDraft()).ok, true);
  options.protected = true;
  assert.equal(env.guard.reload().reason, 'conversation-busy');
  assert.equal(env.reloads(), 0);
});
