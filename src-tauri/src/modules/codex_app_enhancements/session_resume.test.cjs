const assert = require("node:assert/strict");
const { randomUUID } = require("node:crypto");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { test } = require("node:test");
const vm = require("node:vm");

const source = readFileSync(join(__dirname, "session_resume.js"), "utf8");
const threadId = "0dd5a489-92b1-4928-940d-56fc4e8541e2";
const settings = { target: "specific", threadId, prompt: "Continue fixture" };

function renderer(replies, visible = "visible") {
  const listeners = new Set();
  const methods = [];
  const routes = [];
  const window = {
    electronBridge: {
      sendMessageFromView({ hostId, request }) {
        assert.equal(hostId, "local");
        methods.push(request);
        const reply = replies.shift();
        assert.equal(request.method, reply.method);
        for (const receive of listeners) {
          receive({ data: { type: "mcp-response", hostId: "remote", message: {
            id: request.id, error: { message: "not the local response" },
          } } });
          receive({ data: { type: "mcp-response", hostId, message: {
            id: request.id, result: reply.result, error: reply.error,
          } } });
        }
        return Promise.resolve();
      },
    },
    addEventListener(_event, handler) { listeners.add(handler); },
    removeEventListener(_event, handler) { listeners.delete(handler); },
    dispatchEvent(event) { routes.push(event.data); },
  };
  const run = vm.runInNewContext(source, {
    window,
    location: { protocol: "app:", hostname: "-", pathname: "/index.html" },
    document: { querySelector: () => ({}), visibilityState: visible },
    MessageEvent: class { constructor(_event, { data }) { this.data = data; } },
    crypto: { randomUUID }, setTimeout, clearTimeout,
  });
  return { run, methods, routes, listeners };
}

const read = (type, id = threadId) => ({
  method: "thread/read", result: { thread: { id, status: { type } } },
});

test("selects recent activity and resumes the identified thread without sending", async () => {
  const client = renderer([
    { method: "thread/list", result: { data: [{ id: threadId }] } },
    read("notLoaded"),
    { ...read("idle"), method: "thread/resume" },
  ]);
  const result = await client.run("prepare", { target: "recentActivity" });
  assert.equal(result.status, "ready");
  assert.equal(result.threadId, threadId);
  assert.equal(client.methods[0].params.sortKey, "updated_at");
  assert.equal(client.methods[0].params.archived, false);
  assert.equal(client.methods[2].params.excludeTurns, true);
  assert.equal(client.routes.length, 0);
  assert.equal(client.listeners.size, 0);
});

test("a local idle thread gets exactly one route and a single prompt", async () => {
  const client = renderer([read("idle"), { method: "turn/start", result: { turn: { id: "confirmed" } } }]);
  assert.equal((await client.run("send", settings, threadId)).status, "submitted");
  assert.equal(client.routes[0].path, `/local/${threadId}`);
  assert.equal(client.methods[1].params.input[0].text, settings.prompt);
  assert.equal(client.listeners.size, 0);
});

test("the send phase rechecks busy and mismatched thread replies", async () => {
  for (const [response, status] of [
    [read("active"), "busy"],
    [read("idle", randomUUID()), "unavailableThread"],
  ]) {
    const client = renderer([response]);
    assert.equal((await client.run("send", settings, threadId)).status, status);
    assert.equal(client.routes.length, 0);
    assert.equal(client.methods.length, 1);
  }
});

test("an unconfirmed turn result is left uncertain, with one wire attempt", async () => {
  const client = renderer([read("idle"), { method: "turn/start" }]);
  assert.equal((await client.run("send", settings, threadId)).status, "uncertain");
  assert.equal(client.methods.filter((request) => request.method === "turn/start").length, 1);
});

test("a hidden window makes no requests and a missing session is not substituted", async () => {
  const hidden = renderer([], "hidden");
  assert.equal((await hidden.run("prepare", settings)).status, "pending");
  const missing = renderer([{ method: "thread/read", error: { code: -32602 } }]);
  assert.equal((await missing.run("prepare", settings)).status, "unavailableThread");
  assert.equal(missing.methods.length, 1);
});

test("previous desktop selection never falls back to a different session", async () => {
  const mode = { target: "previousDesktopSession" };
  const missing = renderer([]);
  assert.equal((await missing.run("prepare", mode, null)).status, "missingThread");
  assert.equal(missing.methods.length, 0);
  const retained = renderer([read("idle")]);
  const prepared = await retained.run("prepare", mode, threadId);
  assert.equal(prepared.threadId, threadId);
  assert.equal(prepared.status, "ready");
  assert.equal(retained.methods[0].params.threadId, threadId);
});
