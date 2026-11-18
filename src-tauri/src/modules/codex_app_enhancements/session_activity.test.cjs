const assert = require("node:assert/strict");
const { randomUUID } = require("node:crypto");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");
const path = require("node:path");

const script = readFileSync(path.join(__dirname, "session_activity.js"), "utf8");

async function probe({ pages, states = {}, reject = false }) {
  const listeners = new Set();
  const requests = [];
  const window = {
    addEventListener(type, listener) {
      assert.equal(type, "message");
      listeners.add(listener);
    },
    removeEventListener(type, listener) {
      assert.equal(type, "message");
      assert.equal(listeners.delete(listener), true);
    },
    electronBridge: {
      async sendMessageFromView({ type, hostId, request }) {
        assert.equal(type, "mcp-request");
        assert.equal(hostId, "local");
        requests.push(request);
        if (reject) throw new Error("no bridge");
        let result;
        if (request.method === "thread/loaded/list") {
          result = pages.get(request.params.cursor);
        } else {
          assert.equal(request.method, "thread/read");
          assert.equal(request.params.includeTurns, false);
          const id = request.params.threadId;
          result = { thread: { id, status: { type: states[id] } } };
        }
        for (const listener of listeners) {
          listener({ data: { type: "mcp-response", hostId: "remote", message: { id: request.id, error: {} } } });
          listener({ data: { type: "mcp-response", hostId, message: { id: "wrong-id", error: {} } } });
          listener({ source: null, origin: "", data: { type: "mcp-response", hostId, message: { id: request.id, result } } });
        }
      },
    },
  };
  const state = await vm.runInNewContext(script, {
    window, location: { protocol: "app:", hostname: "-" },
    crypto: { randomUUID }, setTimeout, clearTimeout,
  });
  assert.equal(listeners.size, 0);
  return { state, requests };
}

const twoPages = new Map([
  [null, { data: ["first"], nextCursor: "second-page" }],
  ["second-page", { data: ["second"], nextCursor: null }],
]);

test("an active turn on a later page keeps the desktop busy", async () => {
  const { state, requests } = await probe({
    pages: twoPages,
    states: { first: "notLoaded", second: "active" },
  });
  assert.equal(state, "busy");
  assert.equal(requests[2].params.cursor, "second-page");
});

test("a complete idle response or empty loaded list allows progress", async () => {
  const { state } = await probe({
    pages: twoPages,
    states: { first: "idle", second: "idle" },
  });
  assert.equal(state, "idle");
  const empty = new Map([[null, { data: [], nextCursor: null }]]);
  assert.equal((await probe({ pages: empty })).state, "idle");
});

test("an unconfirmed turn, incomplete list, or broken bridge gives no idle signal", async () => {
  assert.equal((await probe({
    pages: twoPages,
    states: { first: "notLoaded", second: "idle" },
  })).state, "unknown");
  assert.equal((await probe({ pages: twoPages, reject: true })).state, "unknown");
  assert.equal((await probe({
    pages: new Map([[null, { data: [], nextCursor: "missing" }]]),
  })).state, "unknown");
  assert.equal((await probe({
    pages: new Map([[null, { data: [], nextCursor: "same" }], ["same", { data: [], nextCursor: "same" }]]),
  })).state, "unknown");
});
