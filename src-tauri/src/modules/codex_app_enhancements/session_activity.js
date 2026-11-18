(async () => {
  const bridge = window.electronBridge;
  if (location.protocol !== "app:" || location.hostname !== "-" ||
      typeof bridge?.sendMessageFromView !== "function") {
    return "unknown";
  }

  const pending = new Map();
  const deadline = Date.now() + 5000;
  const receive = ({ data }) => {
    if (data?.type !== "mcp-response" || data.hostId !== "local") return;
    const reply = data.message;
    pending.get(reply?.id)?.(reply);
  };
  const request = (method, params) => new Promise((resolve, reject) => {
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      reject(new Error("timeout"));
      return;
    }
    const id = "cockpit-idle:" + crypto.randomUUID();
    const complete = (reply) => {
      if (!pending.delete(id)) return;
      clearTimeout(timer);
      if (!reply || reply.error != null) reject(new Error("request failed"));
      else resolve(reply.result);
    };
    const timer = setTimeout(() => complete(null), remaining);
    pending.set(id, complete);
    Promise.resolve().then(() => bridge.sendMessageFromView({
      type: "mcp-request",
      hostId: "local",
      request: { id, method, params },
      source: "cockpit_idle_probe",
      priority: "background",
      timeoutMs: remaining,
      coalesce: false,
    })).catch(() => complete(null));
  });

  window.addEventListener("message", receive);
  try {
    let cursor = null;
    let unconfirmed = false;
    const visited = new Set();
    do {
      const page = await request("thread/loaded/list", { cursor, limit: 100 });
      if (!Array.isArray(page?.data) || !page.data.every((id) => typeof id === "string" && id)) {
        return "unknown";
      }
      for (const id of page.data) {
        const result = await request("thread/read", { threadId: id, includeTurns: false });
        const thread = result?.thread;
        if (thread?.id !== id) return "unknown";
        if (thread.status?.type === "active") return "busy";
        if (thread.status?.type !== "idle") unconfirmed = true;
      }
      cursor = page.nextCursor ?? null;
      if (cursor !== null) {
        if (typeof cursor !== "string" || !cursor || visited.has(cursor)) return "unknown";
        visited.add(cursor);
      }
    } while (cursor !== null);
    return unconfirmed ? "unknown" : "idle";
  } catch {
    return "unknown";
  } finally {
    window.removeEventListener("message", receive);
  }
})()
