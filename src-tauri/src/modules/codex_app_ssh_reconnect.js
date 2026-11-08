// One-shot native reconnect. The IPC ACK means queued, so wait for a new
// connection and a correlated account/read response before reporting success.
(async () => {
  "use strict";
  const request = __COCKPIT_SSH_RECONNECT__;
  if (location.protocol !== "app:" || location.host !== "-"
      || typeof request.hostId !== "string" || !request.hostId || request.hostId === "local"
      || typeof window.electronBridge?.sendMessageFromView !== "function")
    return { ok: false, reason: "bridge_unavailable" };
  const key = Symbol.for("cockpit.ssh.reconnect");
  const operations = window[key] ??= new Map();
  const previous = operations.get(request.hostId);
  if (previous?.id === request.requestId) return previous.promise;
  previous?.cancel?.();
  const entry = { id: request.requestId };
  const accountFailure = value => {
    const account = value?.account;
    const normalized = text => typeof text === "string" ? text.trim().toLowerCase() : "";
    if (request.authKind && account?.type !== request.authKind
        && !(request.authKind === "apiKey" && !account && value?.requiresOpenaiAuth === false)) return "account_mismatch";
    if (request.email && normalized(account?.email) !== normalized(request.email)) return "account_mismatch";
    if (request.accountId && account?.accountId && account.accountId !== request.accountId) return "account_mismatch";
    return !value || !account && value.requiresOpenaiAuth !== false ? "account_unavailable" : null;
  };
  entry.promise = new Promise(resolve => {
    let finished = false, connecting = false, reading = false;
    const readId = "cockpit-ssh-account:" + request.requestId;
    const complete = result => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      window.removeEventListener("message", receive);
      resolve(result);
    };
    const send = message => {
      const failed = error => complete({ ok: false, reason: !connecting
        && /connection for host.*not found|no connection for host/i.test(String(error)) ? "not_ready" : "bridge_failed" });
      try { Promise.resolve(window.electronBridge.sendMessageFromView(message)).catch(failed); }
      catch (error) { failed(error); }
    };
    const connectionChanged = data => {
      if (data.state === "connecting" || data.state === "restarting") connecting = true;
      if (connecting && data.state === "error") complete({ ok: false, reason: "connection_failed" });
      if (connecting && data.state === "connected" && !reading) {
        reading = true;
        send({ type: "mcp-request", hostId: request.hostId,
          request: { id: readId, method: "account/read", params: { refreshToken: false } },
          priority: "critical", source: "cockpit_ssh_reconnect", coalesce: false });
      }
    };
    const receive = event => {
      const data = event?.data;
      if (finished || data?.hostId !== request.hostId || event.source && event.source !== window) return;
      if (data.type === "codex-app-server-connection-changed" && data.isSnapshot !== true) connectionChanged(data);
      if (reading && data.type === "mcp-response" && data.message?.id === readId) {
        if (data.message.error) return complete({ ok: false, reason: "account_read_failed" });
        const reason = accountFailure(data.message.result);
        complete(reason ? { ok: false, reason } : { ok: true });
      }
    };
    const timer = setTimeout(() => complete({ ok: false, reason: "timeout" }),
      Math.max(100, Math.min(30000, request.timeoutMs || 30000)));
    entry.cancel = () => complete({ ok: false, reason: "superseded" });
    window.addEventListener("message", receive);
    // Set before dispatch: native state events may precede the bridge ACK.
    operations.set(request.hostId, entry);
    send({ type: "codex-app-server-restart", hostId: request.hostId, intent: "reconnect" });
  });
  return entry.promise;
})()
