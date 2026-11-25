// Attach to the original picker only after the renderer has mounted a composer.
(() => {
  if (window.__cockpitServiceTierLive) return;
  let plan;
  const notice = "__cockpitServiceTierCaptured";
  const allowed = () => {
    const state = window.__cockpitEnhancements;
    return state?.nativeFast === true && state.apiKeyAuthVerified === true;
  };

  function readRequirements() {
    return new Promise(resolve => {
      const id = "cockpit-service-tier:" + crypto.randomUUID();
      const finish = result => {
        clearTimeout(timer);
        window.removeEventListener("message", receive);
        resolve(result);
      };
      const receive = ({ data }) => {
        if (data?.type === "mcp-response" && data.hostId === "local" && data.message?.id === id) {
          finish(data.message.result ?? null);
        }
      };
      const timer = setTimeout(() => finish(null), 2200);
      window.addEventListener("message", receive);
      Promise.resolve().then(() => window.electronBridge.sendMessageFromView({
        type: "mcp-request", hostId: "local",
        request: { id, method: "configRequirements/read" },
        source: "cockpit_service_tier", priority: "interactive", timeoutMs: 2000, coalesce: false,
      })).catch(() => finish(null));
    });
  }

  function findTarget(fn, marker) {
    if (typeof fn !== "function") return null;
    const source = Function.prototype.toString.call(fn);
    if (!source.includes(marker)) return null;
    const matches = [...source.matchAll(/let\{serviceTierAccess:[$\w]+\}=\(0,([A-Za-z_$][\w$]*)\.U\)\(/g)];
    return matches.length === 1 ? { fn, binding: matches[0][1] } : null;
  }

  async function prepare() {
    if (!allowed() || location.protocol !== "app:" || location.hostname !== "-" ||
        location.pathname !== "/index.html") return { state: "unsupported" };
    if (plan?.captured.size === 2) return { state: "active" };
    if (plan) return { state: "ready", bindings: plan.targets.map(target => target.binding) };
    if (document.readyState !== "complete" ||
        !document.querySelector('[contenteditable="true"][role="textbox"]')) return { state: "pending" };

    const entries = [...document.scripts].map(script => script.src)
      .filter(url => /^app:\/\/-\/assets\/\d+\.[a-f0-9]+\.js$/.test(url));
    if (!entries.length || entries.length > 8) return { state: "unsupported" };
    // These are the native entry modules already evaluated by the page.
    const entry = (await Promise.allSettled(entries.map(url => import(url))))
      .map(result => result.value?.__webpack_require__).find(runtime => runtime?.c);
    const modules = entry?.c;
    const original = modules?.ipV4?.exports;
    const normalize = modules?.eWgO?.exports?.W6;
    const targets = [
      findTarget(modules?.gAqY?.exports?.iU, "serviceTierForRequest:"),
      findTarget(modules?.cyhU?.exports?.M, "availableOptions.length>1"),
    ];
    if (typeof original?.U !== "function" ||
        !Function.prototype.toString.call(original.U).includes("serviceTierAccess:") ||
        typeof normalize !== "function" ||
        typeof window.electronBridge?.sendMessageFromView !== "function" ||
        targets.some(target => !target)) return { state: "unsupported" };

    const requirements = await readRequirements();
    if (!requirements) return { state: "requirements-unavailable" };
    const access = normalize(requirements);
    if (typeof access?.fast !== "boolean" || typeof access.ultrafast !== "boolean" ||
        !(access.fast || access.ultrafast)) return { state: "unsupported" };

    const captured = new Set();
    const replacement = {
      ...original,
      U(...args) {
        const result = original.U(...args);
        if (captured.size !== targets.length || !allowed() || args[0]?.hostId !== "local" ||
            result?.isLoading !== false || result.serviceTierAccess != null) return result;
        return { ...result, serviceTierAccess: access };
      },
    };
    plan = { targets, original, replacement, captured, armed: false };
    return { state: "ready", bindings: targets.map(target => target.binding) };
  }

  function capture(index, original) {
    if (!plan?.armed || !allowed() || (original !== plan.original && original !== plan.replacement)) return original;
    plan.captured.add(index);
    if (plan.captured.size === plan.targets.length) {
      plan.armed = false;
      queueMicrotask(() => window[notice]?.("active"));
    }
    return plan.replacement;
  }

  window.__cockpitServiceTierLive = {
    prepare, capture,
    target: index => plan?.targets[index]?.fn,
    arm: () => {
      if (!plan || !allowed() || typeof window[notice] !== "function") return false;
      plan.armed = true;
      return true;
    },
  };
})();
