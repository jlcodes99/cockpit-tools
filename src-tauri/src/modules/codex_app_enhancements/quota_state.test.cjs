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
    imports.push(url); await beforeImport();
    const module = new vm.SyntheticModule(['__webpack_require__'], function () {
      this.setExport('__webpack_require__', require);
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
test('a readonly transport reports failure without leaving a partially installed message adapter', () => {
  const h = harness(), descriptor = Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data');
  Object.defineProperty(h.Transport.prototype, 'sendRequest', {value() {}, configurable: false, writable: false});
  assert.equal(h.api.attachRuntime(h.require).reason, 'quota-http-boundary-readonly');
  assert.equal(Object.getOwnPropertyDescriptor(h.root.MessageEvent.prototype, 'data').get, descriptor.get);
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
test('module discovery errors are observable failures and revocation refreshes the native cached view', async () => {
  const h = harness();
  h.root.document = {readyState: 'complete', scripts: [{src: 'app://-/assets/1.abcdef.js'}]};
  h.beforeImport(async () => {throw Error('load failed');});
  const result = await h.api.install();
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'quota-runtime-unavailable');
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
