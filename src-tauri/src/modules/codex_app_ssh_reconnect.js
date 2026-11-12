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
  const readErrorDetails = error => ({
    code: typeof error.code === "number" && Number.isFinite(error.code) ? error.code : null,
    message: String(error.message || "Native account read failed")
      .replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, "<email>")
      .replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, "<token>")
      .replace(/\b(access_token|refresh_token|id_token|authorization|OPENAI_API_KEY)\b["'\s:=]+(?:Bearer\s+)?[^\s"',}]+/gi, "$1=<redacted>")
      .replace(/\bsk-[A-Za-z0-9_-]+\b/g, "<key>").slice(0, 512),
  });
  entry.promise = new Promise(resolve => {
    let finished = false, connecting = false, connected = false, reading = false;
    let readId = null, readSequence = 0, retryTimer = null, lastReadError = null;
    const complete = result => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      clearTimeout(retryTimer);
      window.removeEventListener("message", receive);
      if (!result.ok && operations.get(request.hostId) === entry) operations.delete(request.hostId);
      resolve(result);
    };
    const send = message => {
      const failed = error => complete({ ok: false, reason: !connecting
        && /connection for host.*not found|no connection for host/i.test(String(error)) ? "not_ready" : "bridge_failed" });
      try { Promise.resolve(window.electronBridge.sendMessageFromView(message)).catch(failed); }
      catch (error) { failed(error); }
    };
    const readAccount = () => {
      retryTimer = null;
      if (finished || !connected || reading) return;
      reading = true;
      readId = "cockpit-ssh-account:" + request.requestId + ":" + ++readSequence;
      send({ type: "mcp-request", hostId: request.hostId,
        request: { id: readId, method: "account/read", params: { refreshToken: false } },
        priority: "critical", source: "cockpit_ssh_reconnect", coalesce: false });
    };
    const connectionChanged = data => {
      if (data.state !== "connected") {
        connected = false;
        reading = false;
        readId = null;
        clearTimeout(retryTimer);
        retryTimer = null;
      }
      if (data.state === "connecting" || data.state === "restarting") connecting = true;
      if (connecting && data.state === "error") return complete({ ok: false, reason: "connection_failed" });
      if (connecting && data.state === "connected") {
        connected = true;
        if (retryTimer === null) readAccount();
      }
    };
    const receive = event => {
      const data = event?.data;
      if (finished || data?.hostId !== request.hostId || event.source && event.source !== window) return;
      if (data.type === "codex-app-server-connection-changed" && data.isSnapshot !== true) connectionChanged(data);
      if (reading && data.type === "mcp-response" && data.message?.id === readId) {
        if (data.message.error) {
          // The native bridge wraps arbitrary transport exceptions as read errors.
          // Verification is bounded by the deadline, not by exception wording.
          const error = data.message.error;
          lastReadError = readErrorDetails(error);
          if ([-32600, -32601, -32602].includes(error.code))
            return complete({ ok: false, reason: "account_read_failed", readError: lastReadError });
          reading = false;
          readId = null;
          retryTimer = setTimeout(readAccount, 250);
          return;
        }
        const reason = accountFailure(data.message.result);
        complete(reason ? { ok: false, reason } : { ok: true });
      }
    };
    const timer = setTimeout(() => complete(lastReadError
      ? { ok: false, reason: "account_read_failed", readError: lastReadError }
      : { ok: false, reason: "timeout" }),
      Math.max(100, Math.min(30000, request.timeoutMs || 30000)));
    entry.cancel = () => complete({ ok: false, reason: "superseded" });
    window.addEventListener("message", receive);
    // Set before dispatch: native state events may precede the bridge ACK.
    operations.set(request.hostId, entry);
    // Reconnecting the transport alone reuses a daemon with cached old auth.
    // The native restart path also recreates that host's app-server process.
    send({ type: "codex-app-server-restart", hostId: request.hostId, intent: "restart", killCodexProcess: true });
  });
  return entry.promise;
})()
