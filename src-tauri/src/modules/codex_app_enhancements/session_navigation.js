(async (mode, threadId) => {
  const bridge = window.electronBridge;
  if (location.protocol !== "app:" || location.hostname !== "-" ||
      location.pathname !== "/index.html" || document.visibilityState !== "visible" ||
      typeof bridge?.sendMessageFromView !== "function") {
    return null;
  }

  const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;
  const sidebarSelector = "[data-sidebar-destination]";
  const selectedLocalThread = '[data-app-action-sidebar-thread-selected="true"][data-app-action-sidebar-thread-kind="local"][data-app-action-sidebar-thread-host-id="local"]';
  function visibleConversation() {
    const composers = [...document.querySelectorAll(
      '[data-codex-composer-root][data-composer-placement="thread"]'
    )].filter((root) => root.getClientRects().length > 0);
    const portal = composers.length === 1
      ? composers[0].querySelector("[data-above-composer-conversation-id]")
      : null;
    const conversation = portal?.getAttribute("data-above-composer-conversation-id");
    return typeof conversation === "string" && uuid.test(conversation) ? conversation.toLowerCase() : null;
  }

  function readLocalThread(id) {
    return new Promise((resolve) => {
      const requestId = "cockpit-navigation:" + crypto.randomUUID();
      const finish = (thread) => {
        clearTimeout(timer);
        window.removeEventListener("message", receive);
        resolve(thread);
      };
      const receive = ({ data }) => {
        if (data?.type === "mcp-response" && data.hostId === "local" && data.message?.id === requestId) {
          finish(data.message.result?.thread);
        }
      };
      const timer = setTimeout(() => finish(null), 1800);
      window.addEventListener("message", receive);
      Promise.resolve().then(() => bridge.sendMessageFromView({
        type: "mcp-request", hostId: "local",
        request: { id: requestId, method: "thread/read", params: { threadId: id, includeTurns: false } },
        source: "cockpit_navigation", priority: "interactive", timeoutMs: 1500, coalesce: false,
      })).catch(() => finish(null));
    });
  }

  const conversation = visibleConversation();
  let activeThread = null;
  if (conversation && (mode === "capture" || conversation === threadId)) {
    if (document.querySelector(selectedLocalThread)) {
      activeThread = conversation;
    } else if (!document.querySelector(sidebarSelector)) {
      // Collapsing the sidebar unmounts the selection. Confirm the visible
      // composer with the local app-server before treating it as a local thread.
      const thread = await readLocalThread(conversation);
      if (thread?.id === conversation && document.visibilityState === "visible" &&
          visibleConversation() === conversation &&
          (!document.querySelector(sidebarSelector) || document.querySelector(selectedLocalThread))) {
        activeThread = conversation;
      }
    }
  }

  if (mode === "capture") {
    return activeThread ? { threadId: activeThread, focused: document.hasFocus() } : null;
  }
  if (!uuid.test(threadId)) return null;
  if (activeThread === threadId) return "opened";
  if (mode !== "open" || !document.querySelector(`${sidebarSelector}, [data-codex-composer-root]`)) return null;

  // Deliver the same route event the desktop host sends. This only selects a view.
  window.dispatchEvent(new MessageEvent("message", {
    data: { type: "navigate-to-route", path: `/local/${threadId}` },
  }));
  return "dispatched";
})
