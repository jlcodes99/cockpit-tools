(async (mode, settings, threadId) => {
  const bridge = window.electronBridge;
  if (location.protocol !== "app:" || location.hostname !== "-" ||
      location.pathname !== "/index.html" || document.visibilityState !== "visible" ||
      typeof bridge?.sendMessageFromView !== "function" ||
      !document.querySelector("[data-sidebar-destination], [data-codex-composer-root]")) {
    return { status: "pending" };
  }

  function request(method, params) {
    return new Promise((resolve) => {
      const id = "cockpit-resume:" + crypto.randomUUID();
      const finish = (reply) => {
        clearTimeout(timer);
        window.removeEventListener("message", receive);
        resolve(reply);
      };
      const receive = ({ data }) => {
        if (data?.type === "mcp-response" && data.hostId === "local" && data.message?.id === id) {
          finish(data.message);
        }
      };
      const timer = setTimeout(() => finish(null), 3500);
      window.addEventListener("message", receive);
      Promise.resolve().then(() => bridge.sendMessageFromView({
        type: "mcp-request", hostId: "local", request: { id, method, params },
        source: "cockpit_resume", priority: "interactive", timeoutMs: 3200, coalesce: false,
      })).catch(() => finish(null));
    });
  }

  const validId = (id) => typeof id === "string" &&
    /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(id);
  if (mode === "prepare" && !threadId) {
    if (settings.target === "previousDesktopSession") return { status: "missingThread" };
    if (settings.target === "specific") {
      threadId = settings.threadId;
    } else {
      const page = await request("thread/list", {
        archived: false, currentAccountOnly: false, limit: 1,
        sortKey: settings.target === "lastCreated" ? "created_at" : "updated_at",
        sourceKinds: ["cli", "vscode", "exec", "appServer", "unknown"],
      });
      if (!Array.isArray(page?.result?.data)) return { status: "pending" };
      if (!page.result.data.length) return { status: "missingThread" };
      threadId = page.result.data[0]?.id;
    }
  }
  if (!validId(threadId)) return { status: "missingThread" };

  const read = await request("thread/read", { threadId, includeTurns: false });
  if (!read) return { status: "pending" };
  const thread = read.result?.thread;
  if (thread?.id !== threadId) return { status: "unavailableThread" };

  if (mode === "prepare") {
    if (thread.status?.type === "notLoaded") {
      const resumed = await request("thread/resume", { threadId, excludeTurns: true });
      if (!resumed) return { status: "pending" };
      if (resumed.result?.thread?.id !== threadId) return { status: "unavailableThread" };
      if (resumed.result.thread.status?.type !== "idle") return { status: "busy" };
    } else if (thread.status?.type !== "idle") {
      return { status: "busy" };
    }
    return { status: "ready", threadId };
  }

  if (mode !== "send" || typeof settings.prompt !== "string" || !settings.prompt.trim()) {
    return { status: "invalid" };
  }
  if (thread.status?.type !== "idle") return { status: "busy" };
  window.dispatchEvent(new MessageEvent("message", {
    data: { type: "navigate-to-route", path: `/local/${threadId}` },
  }));
  // The app-server may accept a turn even when its response is lost.
  const sent = await request("turn/start", {
    threadId, input: [{ type: "text", text: settings.prompt, text_elements: [] }],
    turnTrigger: "cockpit_resume",
  });
  if (sent?.result?.turn?.id) return { status: "submitted" };
  return { status: sent?.error ? "rejected" : "uncertain" };
})
