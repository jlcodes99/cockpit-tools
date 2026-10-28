const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const script = fs.readFileSync(path.join(__dirname, "predictions.js"), "utf8");
const target = { hostId: "local", threadId: "source", latestTurnId: "source-turn", generationTrigger: "catch_up" };

function environment(options = {}) {
  const timers = new Set(), messages = new Set(), bridgeCalls = [];
  let fake, nextId = 0;
  const context = { module: { exports: {} }, URL, DOMException, AbortController, Symbol,
    crypto: { randomUUID: () => `sample-${++nextId}` },
    setTimeout(callback, delay) { const value = setTimeout(callback, options.timeout ?? delay); timers.add(value); return value; },
    clearTimeout(value) { timers.delete(value); clearTimeout(value); },
    addEventListener(type, listener) { assert.equal(type, "message"); messages.add(listener); },
    removeEventListener(type, listener) { assert.equal(type, "message"); messages.delete(listener); },
    electronBridge: { sendMessageFromView(payload) { bridgeCalls.push(payload); return fake.bridge(payload, env); } },
    __cockpitEnhancements: { nativePredictions: true, apiKeyAuthVerified: true, ...options.flags },
  };
  context.window = context;
  vm.runInNewContext(script, context);
  const api = context.module.exports;
  const env = { api, context, timers, messages, bridgeCalls,
    attach(value) { fake = value; },
    get samples() { return bridgeCalls.filter(call => call.type === "mcp-request"); },
    emit(data) { for (const listener of messages) listener({ data }); },
    respond(id, result = sampleResult()) { env.emit({ type: "mcp-response", hostId: "local", message: { id, result } }); },
    stop() { api.runtime.dispose(); for (const timer of timers) clearTimeout(timer); },
  };
  return env;
}

function sampleResult(overrides = {}) {
  return { responseId: "response-sample", sourceTurnId: "source-turn", text: '{"suggestion":"Inspect the next diagnostic result."}', ...overrides };
}

function nativeManager(options = {}) {
  const calls = [], listeners = new Set(), eventListeners = new Map(), broken = new Set();
  let latest = "source-turn", auth = options.auth ?? null, disposed = 0;
  const metadata = { latestTurnId: "source-turn", streamRole: { role: "owner" }, ephemeral: false, sideConversation: false, ...options.metadata };
  const emit = event => { for (const listener of listeners) listener(event); };
  const manager = {
    getAuthMethod: async () => auth,
    getFollowUpContext: async () => ({ mode: options.plan ? "plan" : "default", ephemeral: false, sideConversation: false, ...options.context }),
    getLatestTurnId: async () => latest,
    getCachedConversationMetadata: async () => ({ ...metadata, latestTurnId: latest }),
    subscribe(request) {
      const selected = request.type === "notification" ? listeners : (eventListeners.get(request.event) ?? new Set());
      if (request.type === "event") eventListeners.set(request.event, selected);
      selected.add(request.listener);
      let onBroken;
      return { onRpcBroken(listener) { onBroken = listener; broken.add(listener); },
        [Symbol.dispose]() { broken.delete(onBroken); if (selected.delete(request.listener)) disposed++; } };
    },
    async sendRequest(method, params, requestOptions) {
      calls.push({ method, params, options: requestOptions });
      if (options.request) {
        const result = options.request(method, params, emit);
        if (result !== undefined) return result;
      }
      if (method === "thread/resume") return { thread: { id: "source", status: { type: "idle" } }, model: "api-model", modelProvider: "local-api", ...options.resumed };
      return {};
    },
  };
  return { manager, calls, emit,
    bridge(payload, env) {
      if (options.bridge) {
        const result = options.bridge(payload, env);
        if (result !== undefined) return result;
      }
      if (payload.type === "mcp-request" && !options.pending)
        env.respond(payload.request.id, sampleResult(Object.hasOwn(options, "text") ? { text: options.text } : {}));
      return Promise.resolve({});
    },
    emitEvent(name, event) { for (const listener of eventListeners.get(name) ?? []) listener(event); },
    breakRpc() { for (const listener of broken) listener(); },
    listeners, setLatest(value) { latest = value; }, setAuth(value) { auth = value; },
    get listenerCount() { return listeners.size + [...eventListeners.values()].reduce((sum, selected) => sum + selected.size, 0); },
    get disposed() { return disposed; },
  };
}

function activate(env, fake) {
  env.attach(fake);
  env.api.runtime.register({ managerForHost: () => fake.manager, nativeEnabled: scope => scope.nativeOption !== false });
  env.api.setReady(true);
  return env.api.runtime.serviceFor({}, "local", null);
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function until(predicate) {
  for (let attempts = 0; attempts < 100; attempts++) {
    if (predicate()) return;
    await new Promise(resolve => setImmediate(resolve));
  }
  assert.fail("Expected asynchronous state was not reached");
}

const fixtures = {
  initial: "function managerFor(scope,host){let registry=scope.get(registryAtom);if(registry)return registry.forHost(host);throw Error(`AppServerManager RPC is unavailable for hostId: ${host}`)}function initialize(){return slot=atom(base,({get:read})=>read(gates,`4290031608`)&&setting(read,descriptor().enabled)&&services.cumberbatch!=null)}",
  composer: "function Prediction(properties){let memo=cache(15),{controller:controller,canShowPrediction:visible,hostId:host,pluginHostId:plugin,roots:roots}=properties,scope=useScope(scopeAtom),target=read(targetAtom),service=services.cumberbatch,unused;let request={...target,generationTrigger:`catch_up`};return service.requestUserMessagePrediction(request)}",
  host: "async function background(scope,manager,target,current){let service=services?.cumberbatch;if(service!=null&&!remote(target.hostId)&&current()&&gate(scope,`4290031608`))try{if(!current()||!gate(scope,`4290031608`))return;return service.requestUserMessagePrediction(target)}catch(error){log(`Cumberbatch background prediction did not finish`,error)}}",
  settings: "function Settings(){let memo=cache(16),scope=useScope(scopeAtom),result;let gate=check(`4290031608`),service=services.cumberbatch,next=read(nextAtom),unused;const key=`cumberbatch-availability`;return gate&&service.isEnabledByRemoteConfig()}",
};
const urls = { initial: "app://-/assets/app-initial-random.js", composer: "app://-/assets/composer-content-random.js", host: "app://-/assets/initialize-app-host-services-random.js", settings: "app://-/assets/general-settings-random.js" };

test("all four synthetic native contracts transform together without fixed symbols", () => {
  const env = environment();
  try {
    for (const [kind, source] of Object.entries(fixtures)) {
      const result = env.api.transform(source, urls[kind]);
      assert.equal(result.ok, true, JSON.stringify(result.diagnostics));
      assert.equal(result.diagnostics.kind, kind);
      assert.ok(result.patchedSource.includes(`__cockpitNativePredictionContract:${kind}:v1`));
      assert.doesNotThrow(() => new vm.Script(result.patchedSource));
      assert.ok(!/addEventListener\(["']keydown|createElement|fetch\(/.test(result.patchedSource));
    }
    assert.match(env.api.transform(fixtures.initial, urls.initial).patchedSource, /managerForHost:managerFor/);
  } finally { env.stop(); }
});

test("ambiguous, missing and reinstalled contracts fail without a partial source", () => {
  const env = environment();
  try {
    for (const [kind, source] of Object.entries(fixtures)) {
      const duplicate = `${source}\n${source}`;
      const result = env.api.transform(duplicate, urls[kind]);
      assert.equal(result.ok, false);
      assert.equal(result.patchedSource, duplicate);
      assert.equal(env.api.transform("function unrelated(){}", urls[kind]).ok, false);
      const once = env.api.transform(source, urls[kind]);
      assert.equal(env.api.transform(once.patchedSource, urls[kind]).ok, false);
      assert.equal(env.api.transform(source, urls[kind].replace("app://-", "https://untrusted.example")).ok, false);
    }
  } finally { env.stop(); }
});

test("readiness and verified API flags are both required", async () => {
  const env = environment(), fake = nativeManager();
  try {
    env.api.runtime.register({ managerForHost: () => fake.manager, nativeEnabled: () => true });
    assert.equal(env.api.runtime.allow(), false);
    env.api.setReady(true);
    assert.equal(env.api.runtime.allow("local"), true);
    assert.equal(env.api.runtime.allow("ssh:example"), false);
    env.context.__cockpitEnhancements.apiKeyAuthVerified = false;
    env.api.runtime.update();
    assert.equal(env.api.runtime.allow(), false);
    assert.deepEqual(fake.calls, []);
  } finally { env.stop(); }
});

test("a synchronous auth failure returns null and removes its initialized job", async () => {
  const env = environment(), fake = nativeManager();
  fake.manager.getAuthMethod = () => { throw new Error("Auth unavailable"); };
  try {
    const pending = activate(env, fake).requestUserMessagePrediction(target);
    assert.equal(await pending, null);
    assert.equal(env.api.runtime.status().inFlight, 0);
    assert.equal(env.api.runtime.status().reason, "request-failed");
    assert.deepEqual(fake.calls, []);
    assert.equal(fake.listeners.size, 0);
  } finally { env.stop(); }
});

test("native sample uses the loaded source and preserves its response shape", async () => {
  const env = environment(), fake = nativeManager();
  try {
    const result = await activate(env, fake).requestUserMessagePrediction(target);
    assert.equal(result.text, "Inspect the next diagnostic result.");
    assert.equal(result.responseId, "response-sample");
    assert.equal(result.sourceTurnId, target.latestTurnId);
    assert.equal(result.predictionThreadId, undefined);
    assert.equal(result.predictionTurnId, undefined);
    assert.equal(result.promptVersion, "cockpit-native-predictions-v2");
    assert.deepEqual(fake.calls.map(call => call.method), ["thread/resume"]);
    assert.equal(fake.calls[0].params.threadId, target.threadId);
    assert.equal(fake.calls[0].params.excludeTurns, true);
    const sample = env.samples[0], params = sample.request.params;
    assert.match(sample.request.id, /^thread\/sample:sample-1$/);
    assert.equal(sample.request.method, "thread/sample");
    assert.equal(sample.hostId, "local");
    assert.equal(sample.priority, "background");
    assert.equal(sample.source, "composer_predictions");
    assert.equal(sample.timeoutMs, 45000);
    assert.equal(sample.coalesce, false);
    assert.equal(params.threadId, target.threadId);
    assert.equal(params.expectedTurnId, target.latestTurnId);
    assert.equal(params.turnTrigger, "composer_predictions");
    assert.equal(params.effort, "low");
    assert.equal(params.disableMcp, true);
    assert.match(params.prompt, /Make zero tool calls/);
    assert.match(params.prompt, /exactly one field/);
    assert.equal(params.responsesapiClientMetadata.prediction_source_thread_id, target.threadId);
    assert.equal(params.responsesapiClientMetadata.prediction_source_turn_id, target.latestTurnId);
    assert.equal(params.responsesapiClientMetadata.prediction_trigger, target.generationTrigger);
    assert.equal(params.model, undefined);
    assert.equal(params.modelProvider, undefined);
    assert.equal(params.context, undefined);
    assert.equal(fake.disposed, 2);
    assert.equal(fake.listenerCount, 0);
    assert.equal(env.messages.size, 0);
    assert.equal(env.timers.size, 0);
    assert.equal(env.api.runtime.status().inFlight, 0);
    for (const removed of ["thread/fork", "turn/start", "turn/interrupt", "thread/unsubscribe"])
      assert.equal(script.includes(`"${removed}"`), false, removed);
    assert.equal(script.includes("fetch("), false);
  } finally { env.stop(); }
});

test("matching requests share one native job and completed source turns stay cached", async () => {
  const env = environment(), fake = nativeManager();
  try {
    const service = activate(env, fake);
    const first = service.requestUserMessagePrediction(target), second = service.requestUserMessagePrediction(target);
    assert.equal(first, second);
    await first;
    await service.requestUserMessagePrediction(target);
    assert.equal(env.samples.length, 1);
  } finally { env.stop(); }
});

test("a sample response can complete before its enqueue acknowledgement", async () => {
  const ack = deferred();
  const env = environment(), fake = nativeManager({ bridge(payload, context) {
    if (payload.type === "mcp-request") {
      context.respond(payload.request.id);
      return ack.promise;
    }
  } });
  try {
    const result = await activate(env, fake).requestUserMessagePrediction(target);
    assert.equal(result.text, "Inspect the next diagnostic result.");
    assert.equal(fake.listenerCount, 0);
    assert.equal(env.messages.size, 0);
    assert.equal(env.api.runtime.status().inFlight, 0);
    ack.resolve({});
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(env.bridgeCalls.length, 1);
    assert.deepEqual(fake.calls.map(call => call.method), ["thread/resume"]);
  } finally { ack.resolve({}); env.stop(); }
});

test("cancel retires locally before ACK then orders abandon before critical sample cancel", async () => {
  const ack = deferred(), abandon = deferred(), cancelAck = deferred();
  const env = environment(), fake = nativeManager({ pending: true,
    bridge(payload) {
      if (payload.type === "mcp-request") return ack.promise;
      if (payload.type === "mcp-request-abandon") return abandon.promise;
    },
    request(method) { if (method === "thread/sample/cancel") return cancelAck.promise; },
  });
  try {
    const pending = activate(env, fake).requestUserMessagePrediction(target);
    await until(() => env.samples.length === 1);
    const requestId = env.samples[0].request.id;
    pending[Symbol.dispose]();
    assert.equal(fake.listenerCount, 0);
    assert.equal(env.messages.size, 0);
    assert.equal(env.timers.size, 0);
    assert.equal(await pending, null);
    assert.equal(env.api.runtime.status().inFlight, 0);
    assert.equal(env.bridgeCalls.length, 1);
    assert.equal(fake.calls.some(call => call.method === "thread/sample/cancel"), false);
    ack.resolve({});
    await until(() => env.bridgeCalls.length === 2);
    assert.equal(env.bridgeCalls[1].type, "mcp-request-abandon");
    assert.equal(env.bridgeCalls[1].hostId, "local");
    assert.equal(env.bridgeCalls[1].requestId, requestId);
    assert.equal(fake.calls.some(call => call.method === "thread/sample/cancel"), false);
    abandon.resolve({});
    await until(() => fake.calls.some(call => call.method === "thread/sample/cancel"));
    const cancel = fake.calls.at(-1);
    assert.equal(cancel.params.requestId, requestId);
    assert.equal(cancel.options.priority, "critical");
    assert.equal(cancel.options.source, "composer_predictions");
    assert.equal(cancel.options.coalesce, false);
    assert.equal(await pending, null);
    assert.equal(fake.disposed, 2);
    cancelAck.resolve({});
  } finally { ack.resolve({}); abandon.resolve({}); cancelAck.resolve({}); env.stop(); }
});

test("failed abandon and cancel acknowledgements never change the retired result", async () => {
  for (const mode of ["throw", "reject"]) {
    const cancelAck = deferred();
    const env = environment(), fake = nativeManager({ pending: true,
      bridge(payload) {
        if (payload.type !== "mcp-request-abandon") return;
        if (mode === "throw") throw new Error("Abandon disconnected");
        return Promise.reject(new Error("Abandon disconnected"));
      },
      request(method) { if (method === "thread/sample/cancel") return cancelAck.promise; },
    });
    try {
      const pending = activate(env, fake).requestUserMessagePrediction(target);
      await until(() => env.samples.length === 1);
      pending[Symbol.dispose]();
      assert.equal(await pending, null);
      await until(() => fake.calls.some(call => call.method === "thread/sample/cancel"));
      cancelAck.reject(new Error("Cancel disconnected"));
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(await pending, null);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
      assert.equal(env.api.runtime.status().inFlight, 0);
    } finally { cancelAck.resolve({}); env.stop(); }
  }
});

test("an old disposed job and late response cannot disturb a reacquired source job", async () => {
  const ack = deferred();
  const env = environment(), fake = nativeManager({ pending: true,
    bridge(payload) { if (payload.type === "mcp-request" && payload.request.id.endsWith("sample-1")) return ack.promise; },
  });
  try {
    const service = activate(env, fake);
    const first = service.requestUserMessagePrediction(target);
    await until(() => env.samples.length === 1);
    const oldId = env.samples[0].request.id;
    env.api.runtime.dispose();
    env.api.setReady(true);
    const second = service.requestUserMessagePrediction(target);
    assert.equal(await first, null);
    await until(() => env.samples.length === 2);
    assert.equal(env.api.runtime.status().inFlight, 1);
    assert.equal(service.requestUserMessagePrediction(target), second);
    env.respond(oldId);
    ack.resolve({});
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(env.api.runtime.status().inFlight, 1);
    assert.equal(env.api.runtime.status().completed, 0);
    assert.equal(service.requestUserMessagePrediction(target), second);
    second[Symbol.dispose]();
    assert.equal(await second, null);
    assert.equal(env.api.runtime.status().inFlight, 0);
  } finally { ack.resolve({}); env.stop(); }
});

test("repeating the same bootstrap preserves active native bindings and jobs", async () => {
  const env = environment(), fake = nativeManager({ pending: true });
  try {
    const promise = activate(env, fake).requestUserMessagePrediction(target);
    await until(() => env.samples.length === 1);
    vm.runInNewContext(script, env.context);
    assert.equal(env.context.__cockpitNativePredictions, env.api);
    assert.equal(env.api.runtime.allow(), true);
    assert.equal(env.api.runtime.status().inFlight, 1);
    env.respond(env.samples[0].request.id, sampleResult({ text: '{"suggestion":"Continue the native test."}' }));
    assert.equal((await promise).text, "Continue the native test.");
  } finally { env.stop(); }
});

test("reconnect revocation requires the graph owner's explicit readiness grant", () => {
  const env = environment(), fake = nativeManager();
  try {
    activate(env, fake);
    env.api.runtime.dispose();
    vm.runInNewContext(script, env.context);
    assert.equal(env.api.runtime.allow(), false, "rebootstrap alone cannot reactivate a revoked graph");
    assert.equal(env.api.setReady(true), true, "a verified owner can reuse the same native module binding");
  } finally { env.stop(); }
});

test("a verified active graph can adopt native bindings after an owner reconnect", () => {
  const env = environment(), fake = nativeManager();
  try {
    activate(env, fake);
    env.api.runtime.dispose();
    env.context.__cockpitEnhancements.nativePatchState = "active";
    env.context.__cockpitEnhancements.nativePredictionPatchState = "ready";
    vm.runInNewContext(script, env.context);
    assert.equal(env.api.runtime.allow(), true);
    env.api.runtime.dispose();
    env.context.__cockpitEnhancements.nativePredictionPatchState = "unsupported";
    vm.runInNewContext(script, env.context);
    assert.equal(env.api.runtime.allow(), false);
  } finally { env.stop(); }
});

test("official and remote requests delegate to the existing native service", async () => {
  const env = environment(), fake = nativeManager({ auth: "chatgpt" });
  const calls = [];
  const original = { isEnabledByRemoteConfig: async () => false, requestUserMessagePrediction(request) { calls.push(request); return Promise.resolve({ text: "Official result" }); } };
  try {
    activate(env, fake);
    const service = env.api.runtime.serviceFor({}, "local", original);
    assert.equal(await service.isEnabledByRemoteConfig(), false);
    assert.equal((await service.requestUserMessagePrediction(target)).text, "Official result");
    const remoteTarget = { ...target, hostId: "ssh:host" };
    const remote = env.api.runtime.serviceFor({}, remoteTarget.hostId, original);
    assert.equal((await remote.requestUserMessagePrediction(remoteTarget)).text, "Official result");
    assert.equal(calls.length, 2);
    assert.deepEqual(fake.calls, []);
    assert.equal(env.samples.length, 0);
  } finally { env.stop(); }
});

test("ineligible source contexts and incompatible resume results never sample", async () => {
  const cases = [
    { plan: true }, { stale: true }, { context: { ephemeral: true } }, { context: { sideConversation: true } },
    { context: { isClosedSubagent: true } }, { context: { latestCollaborationMode: { mode: "plan" } } },
    { metadata: { streamRole: { role: "subscriber" } } }, { metadata: { status: "running" } },
    { metadata: { activeTurnId: "active" } }, { metadata: { ephemeral: true } }, { metadata: { sideConversation: true } },
    { resumed: { collaborationMode: { mode: "plan" } } }, { resumed: { model: null } }, { resumed: { modelProvider: null } },
    { resumed: { thread: { id: "other", status: { type: "idle" } } } },
    { resumed: { thread: { id: "source", status: { type: "active" } } } },
  ];
  for (const options of cases) {
    const env = environment(), fake = nativeManager(options);
    try {
      if (options.stale) fake.setLatest("new-turn");
      assert.equal(await activate(env, fake).requestUserMessagePrediction(target), null);
      assert.equal(env.samples.length, 0, JSON.stringify(options));
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});

test("source turn and deletion notifications cancel only the active sample ID", async () => {
  for (const method of ["turn/started", "thread/deleted"]) {
    const env = environment(), fake = nativeManager({ pending: true });
    try {
      const promise = activate(env, fake).requestUserMessagePrediction(target);
      await until(() => env.samples.length === 1);
      fake.emit({ method, params: { threadId: "unrelated" } });
      assert.equal(env.api.runtime.status().inFlight, 1);
      fake.emit({ method, params: { threadId: "source", turn: { id: "new-turn" } } });
      assert.equal(await promise, null);
      await until(() => fake.calls.some(call => call.method === "thread/sample/cancel"));
      assert.equal(fake.calls.at(-1).params.requestId, env.samples[0].request.id);
      assert.deepEqual(fake.calls.map(call => call.method), ["thread/resume", "thread/sample/cancel"]);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});

test("account, auth status and broken RPC invalidate an in-flight sample", async () => {
  for (const mode of ["account", "auth-status", "rpc-broken"]) {
    const env = environment(), fake = nativeManager({ pending: true });
    try {
      const promise = activate(env, fake).requestUserMessagePrediction(target);
      await until(() => env.samples.length === 1);
      if (mode === "account") {
        fake.setAuth("chatgpt");
        fake.emit({ method: "account/updated", params: { authMode: "chatgpt" } });
      } else if (mode === "auth-status") fake.emitEvent(mode, {});
      else fake.breakRpc();
      assert.equal(await promise, null);
      await until(() => fake.calls.some(call => call.method === "thread/sample/cancel"));
      assert.equal(fake.calls.at(-1).params.requestId, env.samples[0].request.id);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});

test("native Show predictions prevents a sample and cancels an active one", async () => {
  const env = environment(), fake = nativeManager({ pending: true });
  let watchCallback, unwatchCount = 0;
  const scope = { nativeOption: false, watch(callback) { watchCallback = callback; return () => { unwatchCount++; }; } };
  try {
    activate(env, fake);
    const service = env.api.runtime.serviceFor(scope, "local", null);
    assert.equal(await service.requestUserMessagePrediction(target), null);
    assert.equal(env.samples.length, 0);
    scope.nativeOption = true;
    const pending = service.requestUserMessagePrediction(target);
    await until(() => env.samples.length === 1);
    scope.nativeOption = false;
    watchCallback();
    assert.equal(await pending, null);
    await until(() => fake.calls.some(call => call.method === "thread/sample/cancel"));
    assert.equal(unwatchCount, 1);
    assert.equal(fake.listenerCount, 0);
    assert.equal(env.messages.size, 0);
  } finally { env.stop(); }
});

test("a synchronous watch cancellation releases the returned unwatch immediately", async () => {
  const env = environment(), fake = nativeManager();
  let unwatchCount = 0;
  const scope = { nativeOption: true, watch(callback) {
    scope.nativeOption = false;
    callback();
    return () => { unwatchCount++; };
  } };
  try {
    activate(env, fake);
    const result = await env.api.runtime.serviceFor(scope, "local", null).requestUserMessagePrediction(target);
    assert.equal(result, null);
    assert.equal(unwatchCount, 1);
    assert.equal(fake.disposed, 0);
    assert.equal(fake.listenerCount, 0);
    assert.equal(env.messages.size, 0);
    assert.equal(env.timers.size, 0);
    assert.equal(env.api.runtime.status().inFlight, 0);
    assert.deepEqual(fake.calls, []);
    assert.equal(env.samples.length, 0);
  } finally { env.stop(); }
});

test("timeout, feature revocation and manager replacement cancel without retry", async () => {
  for (const mode of ["timeout", "disabled", "manager-replaced"]) {
    const env = environment(mode === "timeout" ? { timeout: 20 } : {}), fake = nativeManager({ pending: true });
    try {
      const promise = activate(env, fake).requestUserMessagePrediction(target);
      await until(() => env.samples.length === 1);
      if (mode === "disabled") { env.context.__cockpitEnhancements.nativePredictions = false; env.api.runtime.update(); }
      if (mode === "manager-replaced") env.api.runtime.register({ managerForHost: () => nativeManager().manager, nativeEnabled: () => true });
      assert.equal(await promise, null);
      await until(() => fake.calls.some(call => call.method === "thread/sample/cancel"));
      assert.equal(env.samples.length, 1);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});

test("only a matching local mcp-response completes a sample", async () => {
  const env = environment(), fake = nativeManager({ pending: true });
  let delivered = false;
  try {
    const pending = activate(env, fake).requestUserMessagePrediction(target);
    pending.then(() => { delivered = true; });
    await until(() => env.samples.length === 1);
    const id = env.samples[0].request.id;
    for (const data of [null, {}, { type: "other", hostId: "local", message: { id, result: sampleResult() } },
      { type: "mcp-response", hostId: "remote", message: { id, result: sampleResult() } },
      { type: "mcp-response", hostId: "local", message: { id: "old-id", result: sampleResult() } },
      { type: "mcp-response", hostId: "local", message: { result: sampleResult() } }]) env.emit(data);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(delivered, false);
    assert.equal(env.api.runtime.status().inFlight, 1);
    assert.equal(env.api.runtime.status().completed, 0);
    // Preload dispatches a synthetic MessageEvent without source or origin.
    env.respond(id);
    assert.equal((await pending).responseId, "response-sample");
    assert.equal(env.messages.size, 0);
  } finally { env.stop(); }
});

test("missing IDs, wrong source turns and unsupported sample errors fail once without fallback", async () => {
  for (const mode of ["response-id", "source-turn", "unsupported"]) {
    const env = environment(), fake = nativeManager({ bridge(payload, context) {
      if (payload.type !== "mcp-request") return;
      if (mode === "unsupported") context.emit({ type: "mcp-response", hostId: "local",
        message: { id: payload.request.id, error: { code: -32601, message: "Method not found" } } });
      else context.respond(payload.request.id, sampleResult(mode === "response-id" ? { responseId: null } : { sourceTurnId: "stale-turn" }));
      return Promise.resolve({});
    } });
    try {
      const service = activate(env, fake);
      assert.equal(await service.requestUserMessagePrediction(target), null);
      assert.equal(env.api.runtime.status().reason, "request-failed");
      assert.equal(await service.requestUserMessagePrediction(target), null);
      assert.equal(env.samples.length, 1);
      assert.deepEqual(fake.calls.map(call => call.method), ["thread/resume"]);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});

test("a missing or failed bridge returns null through the existing diagnostic", async () => {
  for (const mode of ["missing", "throw", "reject"]) {
    const env = environment(), fake = nativeManager({ bridge(payload) {
      if (payload.type !== "mcp-request") return;
      if (mode === "throw") throw new Error("Bridge disconnected");
      return Promise.reject(new Error("Bridge disconnected"));
    } });
    try {
      const service = activate(env, fake);
      if (mode === "missing") delete env.context.electronBridge;
      assert.equal(await service.requestUserMessagePrediction(target), null);
      assert.equal(env.api.runtime.status().reason, "request-failed");
      assert.equal(await service.requestUserMessagePrediction(target), null);
      assert.equal(env.samples.length, mode === "missing" ? 0 : 1);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
      assert.equal(env.api.runtime.status().inFlight, 0);
      await new Promise(resolve => setImmediate(resolve));
    } finally { env.stop(); }
  }
});

test("final auth, source turn and provider checks can discard a completed sample", async () => {
  for (const mode of ["auth", "source", "provider"]) {
    const context = {};
    const env = environment(), fake = nativeManager({ pending: true, context });
    try {
      const pending = activate(env, fake).requestUserMessagePrediction(target);
      await until(() => env.samples.length === 1);
      if (mode === "auth") fake.setAuth("chatgpt");
      if (mode === "source") fake.setLatest("new-turn");
      if (mode === "provider") context.modelProvider = "other-provider";
      env.respond(env.samples[0].request.id);
      assert.equal(await pending, null);
      assert.equal(fake.calls.some(call => call.method === "thread/sample/cancel"), false);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});

test("null sample text and malformed or long suggestions never publish", async () => {
  const inputs = [null, "not JSON", '{"suggestion":"x","extra":true}', '{"suggestion":false}',
    JSON.stringify({ suggestion: "x".repeat(201) }), '{"suggestion":null}', '{"suggestion":"line\\nbreak"}', '{"suggestion":" "}'];
  for (const text of inputs) {
    const env = environment(), fake = nativeManager({ text });
    try {
      const service = activate(env, fake);
      assert.equal(await service.requestUserMessagePrediction(target), null);
      assert.equal(env.api.runtime.status().reason, "empty");
      assert.equal(env.api.runtime.status().completed, 1);
      assert.equal(await service.requestUserMessagePrediction(target), null);
      assert.equal(env.samples.length, 1);
      assert.equal(fake.listenerCount, 0);
      assert.equal(env.messages.size, 0);
    } finally { env.stop(); }
  }
});
