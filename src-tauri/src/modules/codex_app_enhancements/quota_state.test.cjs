const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const script = fs.readFileSync(path.join(__dirname, 'quota_state.js'), 'utf8');
const exhausted = () => ({user_id: 'user', account_id: 'account', plan_type: 'pro',
  rate_limit: {allowed: false, limit_reached: true, primary_window: {used_percent: 100, reset_at: 500}},
  code_review_rate_limit: {allowed: false, limit_reached: true},
  additional_rate_limits: [{limit_name: 'model', rate_limit: {allowed: false, limit_reached: true}}],
  credits: {has_credits: false, unlimited: false, balance: '0', overage_limit_reached: true},
  spend_control: {individual_limit: {limit: 10, limit_reached: true, current_month_usage: 10}},
  rate_limit_upsell: {banner_type: 'pro_rate_limit_reached'}, rate_limit_warning: {},
  image_generation: {allowed: false}, rate_limit_reached_type: {type: 'rate_limit_reached'}});
function harness() {
  class MessageEvent { constructor(data) { this.raw = data; } get data() { return this.raw; } }
  const responses = [];
  class Transport {
    static getInstance() {}
    async fetch(url, options) { responses.push({url, options}); if (this.error) throw this.error; return this.response; }
    async fetchJson(url, options) { const response = await this.fetch(url, options); return {body: await response.json()}; }
    get() {}
    cancelStream() {}
    sendRequest() {}
  }
  const require = Object.assign(() => {}, {c: {renumbered: {exports: {renamed: Transport}}}});
  const root = {URL, Headers, Response, MessageEvent, setTimeout, clearTimeout, queueMicrotask,
    __cockpitEnhancements: {apiService: true, quotaCompat: false, apiKeyAuthVerified: false}};
  root.window = root;
  const context = vm.createContext(root);
  const imports = [];
  let beforeImport = async () => {};
  vm.runInContext(script, context, {importModuleDynamically: async url => {
    imports.push(url); const exports = await beforeImport(url) ?? {__webpack_require__: require};
    const module = new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, {context});
    await module.link(() => {}); await module.evaluate(); return module;
  }});
  return {root, context, api: root.__cockpitQuotaState, require, Transport, responses, imports,
    beforeImport(callback) { beforeImport = callback; }};
}
function cache() {
  const queries = [], listeners = new Set();
  const store = {findAll: () => queries, subscribe(listener) {listeners.add(listener); return () => listeners.delete(listener);}};
  const client = {getQueryCache: () => store,
    setQueryData(key, data, options) {
      let query = queries.find(query => query.queryKey === key);
      if (!query) {query = {queryKey: key, state: {}}; queries.push(query);}
      query.state = {data, dataUpdatedAt: options?.updatedAt ?? 123};
      for (const listener of listeners) listener({type: 'updated', query});
    }};
  return {client, queries, listeners};
}
function startup(h, scripts = ['app://-/assets/1.abcdef.js']) {
  const c = cache();
  h.root.document = {readyState: 'complete', scripts: scripts.map(src => ({src})),
    getElementById: () => ({__reactFiber$test: {memoizedProps: {client: c.client}}})};
  return c;
}
function clock(h) {
  const timers = new Map(), waits = [];
  let id = 0;
  h.root.setTimeout = (callback, delay) => {timers.set(++id, {callback, delay}); return id;};
  h.root.clearTimeout = id => timers.delete(id);
  return {timers, waits, async finish(pending, beforeWait = () => {}) {
    let result;
    pending.then(value => {result = value;});
    for (let step = 0; step < 25 && !result; step++) {
      await new Promise(resolve => setImmediate(resolve));
      if (result) break;
      const timer = [...timers].find(([, value]) => value.delay === 250);
      assert.ok(timer, 'startup must either finish or yield a bounded discovery wait');
      timers.delete(timer[0]); waits.push(timer[1].delay);
      beforeWait(waits.length); timer[1].callback();
    }
    await new Promise(resolve => setImmediate(resolve));
    assert.ok(result, 'startup must settle within its attempt budget');
    assert.equal(timers.size, 0, 'the overall timeout is cleared after settlement');
    return result;
  }};
}
test('incoming usage has consistent available quota while preserving identity, subscription and images', () => {
  const {api, root} = harness(), raw = exhausted(), before = JSON.stringify(raw);
  const next = api.usage(raw);
  assert.equal(next.rate_limit.allowed, true);
  assert.equal(next.rate_limit.limit_reached, false);
  assert.equal(next.rate_limit.primary_window.used_percent, 0);
  assert.equal(next.rate_limit.primary_window.reset_at, 500);
  assert.equal(next.additional_rate_limits[0].rate_limit.allowed, true);
  assert.equal(next.code_review_rate_limit.allowed, true);
  assert.equal(next.credits.unlimited, true);
  assert.equal(next.credits.overage_limit_reached, false);
  assert.equal(next.spend_control.individual_limit.limit_reached, false);
  assert.equal(next.spend_control.individual_limit.current_month_usage, 0);
  assert.equal(next.rate_limit_upsell, null);
  for (const key of ['user_id', 'account_id', 'plan_type', 'image_generation']) assert.equal(next[key], raw[key]);
  assert.equal(JSON.stringify(raw), before);
  assert.equal(api.usage(raw), next);
  assert.equal(api.usage(next), next);
  root.__cockpitEnhancements.apiService = false;
  assert.equal(api.usage(raw), raw);
  for (const invalid of [null, {error: 'unauthorized'}, {rate_limit: {allowed: 'unknown'}}]) assert.equal(api.usage(invalid), invalid);
});
test('the native HTTP boundary covers reads and refreshes, preserving request and failure behavior', async () => {
  const h = harness(), client = new h.Transport();
  assert.equal(h.api.attachRuntime(h.require).ok, true);
  const options = {signal: new AbortController().signal, expectedIdentity: {accountId: 'account'}};
  for (const raw of [exhausted(), {...exhausted(), credits: {balance: '5', unlimited: false}}]) {
    client.response = new Response(JSON.stringify(raw), {headers: {'content-length': '999', 'x-version': 'v1'}});
    const response = await client.fetch('/wham/usage', options), data = await response.json();
    assert.equal(data.rate_limit.allowed, true);
    assert.equal(response.headers.has('content-length'), false);
    assert.equal(response.headers.get('x-version'), 'v1');
  }
  assert.equal(h.responses[0].options, options);
  for (const [url, options] of [['/other', {}], ['/wham/usage', {headers: {'OAI-Image-Generation-Limit-Reached': 'true'}}]]) {
    client.response = new Response(JSON.stringify(exhausted()));
    assert.equal(await client.fetch(url, options), client.response);
  }
  client.response = new Response('denied', {status: 401});
  assert.equal(await client.fetch('/wham/usage'), client.response);
  client.response = new Response('malformed');
  await assert.rejects(client.fetch('/wham/usage'), error => error.name === 'SyntaxError');
  const error = client.error = Error('network failure');
  await assert.rejects(client.fetch('/wham/usage'), value => value === error);
  h.api.dispose();
});
test('real-time quota snapshots use the same normalization without changing sequence, other clients or other topics', () => {
  const h = harness(); h.api.attachRuntime(h.require);
  const raw = [{type: 'message', topic_id: 'codex-work-usage-push-v1', offset: '42', payload: {
    type: 'usage.snapshot', payload: {version: 1, stream_id: 'stream', sequence: 2,
      usage_by_client: {electron: exhausted(), browser: exhausted()}}}},
    {type: 'message', topic_id: 'conversations', payload: {text: 'usage.snapshot'}}];
  const event = new h.root.MessageEvent(JSON.stringify(raw)), next = JSON.parse(event.data);
  assert.equal(next[0].payload.payload.usage_by_client.electron.rate_limit.allowed, true);
  assert.deepEqual(next[0].payload.payload.usage_by_client.browser, raw[0].payload.payload.usage_by_client.browser);
  assert.equal(next[0].offset, '42'); assert.equal(next[0].payload.payload.sequence, 2);
  assert.deepEqual(next[1], raw[1]);
  assert.equal(raw[0].payload.payload.usage_by_client.electron.rate_limit.allowed, false);
  assert.equal(event.data, event.data);
  for (const data of ['usage.snapshot malformed', 'ordinary content', [{type: 'message', topic_id: 'conversations', payload: raw[0].payload}]]) {
    assert.equal(new h.root.MessageEvent(data).data, data);
  }
  h.root.__cockpitEnhancements.apiService = false;
  assert.equal(event.data, event.raw);
  h.api.dispose();
});
test('existing cached quota and later updates are adapted once, with original timestamps and reversible cleanup', () => {
  const h = harness(), c = cache(), key = ['rate-limit-status', 'user', 'account'];
  const raw = exhausted(); c.client.setQueryData(key, raw, {updatedAt: 10});
  const image = ['rate-limit-status', 'image-generation']; c.client.setQueryData(image, raw);
  assert.equal(h.api.attachCache(c.client), true);
  assert.equal(c.queries[0].state.data.rate_limit.allowed, true);
  const removed = {queryKey: ['rate-limit-status', 'deleted'], state: {data: exhausted()}};
  for (const listener of c.listeners) listener({type: 'removed', query: removed});
  assert.equal(c.queries.length, 2, 'a removal notification cannot recreate a deleted query');
  assert.equal(c.queries[0].state.dataUpdatedAt, 10);
  assert.equal(c.queries[1].state.data, raw);
  assert.equal(h.api.attachCache(c.client), true); assert.equal(c.listeners.size, 1);
  const fresh = {...exhausted(), plan_type: 'business'};
  c.client.setQueryData(key, fresh, {updatedAt: 20});
  assert.equal(c.queries[0].state.data.plan_type, 'business');
  assert.equal(c.queries[0].state.data.rate_limit.allowed, true);
  h.api.dispose();
  assert.equal(c.listeners.size, 0);
  assert.equal(c.queries[0].state.data, fresh);
  assert.equal(c.queries[0].state.dataUpdatedAt, 20);
});
test('attachment is idempotent, revocable and scoped to the API Service instance', async () => {
  const h = harness(), original = h.Transport.prototype.fetch;
  h.root.__cockpitEnhancements.apiService = false;
  assert.equal(h.api.attachRuntime(h.require).reason, 'disabled');
  assert.equal(h.Transport.prototype.fetch, original);
  h.root.__cockpitEnhancements.apiService = true;
  h.api.attachRuntime(h.require); const wrapper = h.Transport.prototype.fetch;
  h.api.attachRuntime(h.require); assert.equal(h.Transport.prototype.fetch, wrapper);
  h.api.dispose(); assert.equal(h.Transport.prototype.fetch, original);
  assert.equal(h.api.attachRuntime(Object.assign(() => {}, {c: {}})).ok, false);
  h.api.attachRuntime(h.require); h.api.dispose();
});
test('the shipped parsed IPC transport also adapts quota without a fetch or fetchJson method', async () => {
  const h = harness();
  class ParsedTransport {
    static getInstance() {}
    get(url, headers) {return this.sendRequest('GET', url, {headers});}
    cancelStream() {}
    async sendRequest(method, url, options) {this.request = {method, url, options}; return this.response;}
  }
  assert.equal(h.api.attachRuntime({renamed: ParsedTransport}).ok, true);
  const client = new ParsedTransport();
  client.response = {body: exhausted(), status: 200, headers: {'x-native': 'yes'}};
  const result = await client.get('/wham/usage');
  assert.equal(result.body.rate_limit.allowed, true);
  assert.equal(client.response.body.rate_limit.allowed, false);
  assert.equal(result.status, 200); assert.equal(result.headers, client.response.headers);
  assert.equal(client.request.method, 'GET');
  client.response = {body: {error: 'denied'}, status: 403};
  assert.equal(await client.get('/wham/usage'), client.response);
  h.api.dispose();
  client.response = {body: exhausted(), status: 200};
  assert.equal((await client.get('/wham/usage')).body.rate_limit.allowed, false);
});
test('the appHost RPC proxy preceding the real HTTP class cannot steal transport discovery', () => {
  const h = harness(), original = h.Transport.prototype.sendRequest;
  const phantomPrototype = new Proxy({}, {get: () => function () {}});
  const appHost = new Proxy(function () {}, {get: (_, name) => name === 'prototype' ? phantomPrototype : function () {}});
  h.require.c = {GaA7: {exports: {appHost}}, GDNt: {exports: {_n: h.Transport}}};
  assert.equal(typeof appHost.getInstance, 'function');
  assert.equal(typeof appHost.prototype.sendRequest, 'function');
  assert.equal(Object.getOwnPropertyDescriptor(appHost.prototype, 'sendRequest'), undefined);
  assert.equal(h.api.attachRuntime(h.require).ok, true);
  assert.notEqual(h.Transport.prototype.sendRequest, original);
  h.api.dispose();
  assert.equal(h.Transport.prototype.sendRequest, original);
});

test('genuine readonly sendRequest and fetch boundaries preserve all original adapters', () => {
  for (const name of ['sendRequest', 'fetch']) {
    const h = harness(), descriptor = Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data');
    Object.defineProperty(h.Transport.prototype, name, {configurable: false, writable: false});
    const send = h.Transport.prototype.sendRequest, fetch = h.Transport.prototype.fetch;
    assert.equal(h.api.attachRuntime(h.require).reason, 'quota-http-boundary-readonly');
    assert.equal(Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data').get, descriptor.get);
    assert.equal(h.Transport.prototype.sendRequest, send);
    assert.equal(h.Transport.prototype.fetch, fetch);
  }
});

test('transport aliases deduplicate, while a second genuine transport cannot bypass a readonly boundary', () => {
  const h = harness(), descriptor = Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data');
  h.require.c.alias = {exports: {same: h.Transport}};
  assert.equal(h.api.attachRuntime(h.require).ok, true);
  h.api.dispose();
  Object.defineProperty(h.Transport.prototype, 'sendRequest', {configurable: false, writable: false});
  class OtherTransport {
    static getInstance() {}
    get() {} cancelStream() {} sendRequest() {} fetch() {}
  }
  const original = OtherTransport.prototype.sendRequest;
  h.require.c.other = {exports: {OtherTransport}};
  const result = h.api.attachRuntime(h.require);
  assert.equal(result.reason, 'quota-http-boundary-unavailable');
  assert.equal(result.detail, 'quota-http-boundary-ambiguous');
  assert.equal(OtherTransport.prototype.sendRequest, original);
  assert.equal(Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data').get, descriptor.get);
});

test('a transport that rejects installation rolls back fetch and the message boundary', () => {
  for (const failRestore of [false, true]) {
    const h = harness(), descriptor = Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data');
    function Transport() {}
    Transport.getInstance = () => {};
    const methods = {get() {}, cancelStream() {}, sendRequest() {}, fetch() {}};
    const original = methods.fetch;
    Transport.prototype = new Proxy(methods, {defineProperty(target, name, value) {
      if (name === 'sendRequest' || failRestore && name === 'fetch' && value.value === original)
        throw Error('private mutation failure');
      return Reflect.defineProperty(target, name, value);
    }});
    const result = h.api.attachRuntime({Transport});
    assert.equal(result.reason, 'quota-http-boundary-readonly');
    assert.equal(result.detail, failRestore ? 'quota-cleanup-failed' : undefined);
    if (!failRestore) assert.equal(methods.fetch, original);
    assert.equal(Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data').get, descriptor.get,
      'one rejected cleanup must not prevent remaining adapters from being restored');
    assert.doesNotMatch(JSON.stringify(result), /private/);
  }
});
test('startup and concurrent reattachment share one installation and adapt already loaded quota', async () => {
  const h = harness(), c = cache();
  c.client.setQueryData(['rate-limit-status', 'account'], exhausted());
  h.root.document = {readyState: 'complete', scripts: [{src: 'app://-/assets/987654.hash123.js'.replace('hash123', 'abcdef')}],
    getElementById: () => ({__reactFiber$test: {memoizedProps: {client: c.client}}})};
  const results = await Promise.all([h.api.install(), h.api.install()]);
  assert.equal(results.every(result => result.ok), true);
  assert.equal(h.imports.length, 1);
  assert.equal(h.root.__cockpitEnhancements.nativeQuotaPatchState, 'active');
  assert.equal(c.queries[0].state.data.rate_limit.allowed, true);
  h.api.dispose();
});
test('an interrupted or timed out module load cannot install later or claim success', async () => {
  for (const action of ['dispose', 'timeout']) {
    const h = harness(), original = h.Transport.prototype.fetch;
    h.root.document = {readyState: 'complete', scripts: [{src: 'app://-/assets/1.abcdef.js'}]};
    let release;
    h.beforeImport(() => new Promise(resolve => {release = resolve;}));
    const timers = new Map(); let id = 0;
    h.root.setTimeout = callback => {timers.set(++id, callback); return id;};
    h.root.clearTimeout = id => timers.delete(id);
    const pending = h.api.install();
    await new Promise(resolve => setImmediate(resolve));
    if (action === 'dispose') h.api.dispose(); else [...timers.values()][0]();
    release();
    const result = await pending;
    assert.equal(result.ok, false);
    assert.equal(result.reason, action === 'dispose' ? 'disabled' : 'quota-data-timeout');
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(h.Transport.prototype.fetch, original);
    assert.notEqual(h.root.__cockpitEnhancements.nativeQuotaPatchState, 'active');
  }
});

test('the overall timeout restores newly installed adapters while waiting for the native cache', async () => {
  const h = harness(), time = clock(h), originalSend = h.Transport.prototype.sendRequest;
  const originalFetch = h.Transport.prototype.fetch;
  const descriptor = Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data');
  startup(h);
  h.root.document.getElementById = () => null;
  const pending = h.api.install();
  await new Promise(resolve => setImmediate(resolve));
  assert.notEqual(h.Transport.prototype.sendRequest, originalSend);
  assert.notEqual(h.Transport.prototype.fetch, originalFetch);
  const deadline = [...time.timers].find(([, value]) => value.delay === 10000);
  time.timers.delete(deadline[0]); deadline[1].callback();
  const result = await pending;
  assert.equal(result.reason, 'quota-data-timeout');
  assert.equal(h.root.__cockpitEnhancements.nativeQuotaPatchState, 'unsupported');
  assert.equal(h.Transport.prototype.sendRequest, originalSend);
  assert.equal(h.Transport.prototype.fetch, originalFetch);
  assert.equal(Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data').get, descriptor.get);
  for (const [id, timer] of time.timers) {time.timers.delete(id); timer.callback();}
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(h.Transport.prototype.sendRequest, originalSend, 'late cache polling cannot reinstall after timeout');
  startup(h);
  assert.equal((await h.api.install()).ok, true, 'a later host retry can install a clean attempt');
  h.api.dispose();
});
test('cache discovery exceptions restore newly installed adapters without exposing the exception', async () => {
  const h = harness(), originalSend = h.Transport.prototype.sendRequest, originalFetch = h.Transport.prototype.fetch;
  const descriptor = Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data');
  startup(h);
  h.root.document.getElementById = () => {throw Error('private cache discovery failure');};
  const result = await h.api.install();
  assert.equal(result.reason, 'quota-runtime-unavailable');
  assert.doesNotMatch(JSON.stringify(result), /private/);
  assert.equal(h.Transport.prototype.sendRequest, originalSend);
  assert.equal(h.Transport.prototype.fetch, originalFetch);
  assert.equal(Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data').get, descriptor.get);
  assert.equal(h.root.__cockpitEnhancements.nativeQuotaPatchState, 'unsupported');
});

test('module discovery errors are observable failures and revocation refreshes the native cached view', async () => {
  const h = harness(), time = clock(h);
  startup(h);
  h.beforeImport(async () => {throw Error('private import URL and content');});
  const result = await time.finish(h.api.install());
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'quota-runtime-unavailable');
  assert.equal(result.detail, 'quota-runtime-import-failed');
  assert.equal(h.root.__cockpitEnhancements.nativeQuotaPatchDetail, 'quota-runtime-import-failed');
  assert.doesNotMatch(JSON.stringify(result), /private/);
  assert.equal(h.imports.length, 1, 'a failed URL is not repeatedly imported within one install');
  assert.equal(time.waits.length, 19);
  assert.equal(h.root.__cockpitEnhancements.nativeQuotaPatchState, 'unsupported');
  const c = cache(), refreshes = [];
  c.client.invalidateQueries = async options => {refreshes.push(options);};
  h.api.attachCache(c.client);
  h.root.__cockpitEnhancements.apiService = false;
  h.api.dispose();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(refreshes.length, 3);
  assert.equal(refreshes.every(options => options.refetchType === 'active'), true);
});

test('a failed entry import cannot mask a later valid entry or loaded named transport', async () => {
  for (const source of ['entry', 'resource']) {
    const h = harness();
    const valid = source === 'entry' ? 'app://-/assets/2.abcdef.js' : 'app://-/assets/app-initial-abcdef.js';
    startup(h, ['app://-/assets/1.abcdef.js', ...(source === 'entry' ? [valid] : [])]);
    if (source === 'resource') h.root.performance = {getEntriesByType: () => [{name: valid}]};
    h.beforeImport(async url => {
      if (url !== valid) throw Error('private failed import');
      if (source === 'resource') return {renamed: h.Transport};
    });
    const result = await h.api.install();
    assert.equal(result.ok, true);
    assert.deepEqual(h.imports, ['app://-/assets/1.abcdef.js', valid]);
    assert.equal(h.root.__cockpitEnhancements.nativeQuotaPatchDetail, null);
    h.api.dispose();
  }
});

test('startup rediscovers late entries and waits for a late transport in an existing loader', async () => {
  for (const late of ['entry', 'transport']) {
    const h = harness(), time = clock(h), originalCache = h.require.c;
    startup(h, late === 'entry' ? [] : undefined);
    if (late === 'transport') h.require.c = {};
    const result = await time.finish(h.api.install(), count => {
      assert.equal(count, 1);
      if (late === 'entry') h.root.document.scripts.push({src: 'app://-/assets/1.abcdef.js'});
      else h.require.c = originalCache;
    });
    assert.equal(result.ok, true);
    assert.equal(time.waits.length, 1);
    assert.equal(h.imports.length, 1);
    h.api.dispose();
  }
});

test('missing entries and transports stop after bounded discovery with retry-compatible reasons', async () => {
  for (const missing of ['entry', 'transport']) {
    const h = harness(), time = clock(h), original = h.Transport.prototype.sendRequest;
    startup(h, missing === 'entry' ? [] : undefined);
    if (missing === 'transport') h.require.c = {};
    const result = await time.finish(h.api.install());
    assert.equal(result.ok, false);
    assert.equal(result.reason, missing === 'entry' ? 'quota-runtime-unavailable' : 'quota-http-boundary-unavailable');
    assert.equal(result.detail, undefined);
    assert.equal(time.waits.length, 19);
    assert.equal(h.imports.length, missing === 'entry' ? 0 : 1);
    assert.equal(h.Transport.prototype.sendRequest, original);
  }
});
