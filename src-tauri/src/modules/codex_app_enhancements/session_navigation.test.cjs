const assert = require("node:assert/strict");
const { randomUUID } = require("node:crypto");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const path = require("node:path");
const vm = require("node:vm");

const source = readFileSync(path.join(__dirname, "session_navigation.js"), "utf8");
const threadId = "0dd5a489-92b1-4928-940d-56fc4e8541e2";

function renderer({
  selected = true,
  focused = true,
  visible = "visible",
  pathname = "/index.html",
  composerIds = [threadId],
  ready = true,
  collapsed = false,
  readThread,
} = {}) {
  const events = [];
  const requests = [];
  const listeners = new Set();
  const document = {
    visibilityState: visible,
    hasFocus: () => focused,
    querySelector(selector) {
      if (selector === "[data-sidebar-destination]") return ready && !collapsed ? {} : null;
      if (selector === "[data-sidebar-destination], [data-codex-composer-root]") return ready ? {} : null;
      assert.match(selector, /thread-host-id="local"/);
      return selected ? {} : null;
    },
    querySelectorAll() {
      return composerIds.map((id) => ({
        getClientRects: () => [1],
        querySelector: () => ({ getAttribute: () => id }),
      }));
    },
  };
  const run = vm.runInNewContext(source, {
    document,
    location: { protocol: "app:", hostname: "-", pathname },
    window: {
      electronBridge: {
        sendMessageFromView({ hostId, request }) {
          requests.push(request);
          assert.equal(hostId, "local");
          const thread = readThread?.(request.params.threadId);
          for (const receive of listeners) {
            receive({ data: { type: "mcp-response", hostId: "remote", message: {
              id: request.id, result: { thread: { id: threadId } },
            } } });
            receive({ data: { type: "mcp-response", hostId, message: {
              id: request.id, result: { thread },
            } } });
          }
        },
      },
      addEventListener(_event, handler) { listeners.add(handler); },
      removeEventListener(_event, handler) { listeners.delete(handler); },
      dispatchEvent: (event) => events.push(event),
    },
    MessageEvent: class {
      constructor(type, { data }) { this.type = type; this.data = data; }
    },
    crypto: { randomUUID }, setTimeout, clearTimeout,
  });
  return { run, events, requests, listeners };
}

test("capture uses the visible local composer ID, independent of the sidebar row ID", async () => {
  const { run, events, requests } = renderer({ focused: false });
  const result = await run("capture");
  assert.equal(result.threadId, threadId);
  assert.equal(result.focused, false);
  assert.equal(events.length, 0);
  assert.equal(requests.length, 0);
});

test("hidden windows, remote selections, drafts and ambiguous composers do not give a thread", async () => {
  for (const options of [
    { visible: "hidden" },
    { pathname: "/detached-window.html" },
    { selected: false },
    { composerIds: ["client-new-thread:fixture"] },
    { composerIds: [threadId, threadId] },
  ]) {
    assert.equal(await renderer(options).run("capture"), null);
  }
});

test("open emits one local route and confirm emits no message", async () => {
  const { run, events } = renderer({ selected: false, composerIds: [] });
  assert.equal(await run("open", threadId), "dispatched");
  assert.equal(await run("confirm", threadId), null);
  assert.equal(events.length, 1);
  assert.equal(events[0].type, "message");
  assert.equal(events[0].data.type, "navigate-to-route");
  assert.equal(events[0].data.path, `/local/${threadId}`);
});

test("loading, invalid paths, and sessions already open require no route event", async () => {
  const current = renderer();
  assert.equal(await current.run("open", threadId), "opened");
  assert.equal(await current.run("open", "../settings"), null);
  assert.equal(current.events.length, 0);

  const loading = renderer({ ready: false, selected: false });
  assert.equal(await loading.run("open", threadId), null);
  assert.equal(loading.events.length, 0);
});

test("the collapsed sidebar uses a read-only local check before capturing or confirming", async () => {
  const { run, events, requests, listeners } = renderer({
    selected: false, collapsed: true, readThread: (id) => ({ id }),
  });
  assert.equal((await run("capture")).threadId, threadId);
  assert.equal(await run("confirm", threadId), "opened");
  assert.equal(events.length, 0);
  assert.equal(requests.length, 2);
  for (const request of requests) {
    assert.equal(request.method, "thread/read");
    assert.equal(request.params.threadId, threadId);
    assert.equal(request.params.includeTurns, false);
  }
  assert.equal(listeners.size, 0);
});

test("an unverified composer is not captured and a collapsed home can open a local route", async () => {
  const unknown = renderer({ selected: false, collapsed: true });
  assert.equal(await unknown.run("capture"), null);
  assert.equal(await unknown.run("confirm", threadId), null);
  const home = renderer({ selected: false, collapsed: true, composerIds: [] });
  assert.equal(await home.run("open", threadId), "dispatched");
  assert.equal(home.events[0].data.path, `/local/${threadId}`);
  assert.equal(home.requests.length, 0);
});

test("the composer ID must still match after the local thread read", async () => {
  const composerIds = [threadId];
  const { run } = renderer({
    selected: false, collapsed: true, composerIds,
    readThread(id) {
      composerIds[0] = randomUUID();
      return { id };
    },
  });
  assert.equal(await run("capture"), null);
});
