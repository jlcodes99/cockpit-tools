// Codex 26.1029 ships Rspack ESM chunks. Keep the native editor and its
// lifecycle intact; replace only the same four prediction contracts as Vite.
(() => {
  "use strict";
  const root = typeof window === "object" ? window : globalThis;
  const P = "window.__cockpitNativePredictions";
  const chunks = { initial: 559161, composer: 563813, host: 24191, settings: 572868 };
  const N = "[A-Za-z_$][\\w$]*";
  const esc = value => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

  function transform(source, kind) {
    const original = source;
    const replace = (pattern, callback, expected = 1) => {
      let count = 0;
      source = source.replace(pattern, (...args) => { count++; return callback(...args); });
      if (count !== expected) throw Error(`${kind}-contract`);
    };
    try {
      if (typeof source !== "string" || source.length > 24 * 1024 * 1024
        || !source.includes(`export const __rspack_esm_id=${chunks[kind]};`)) throw Error("unsupported-chunk");
      if (kind === "initial") {
        replace(new RegExp(`(${N}\\(${N}\\.${N},"4290031608"\\))(&&\\(0,${N}\\.${N}\\)\\(${N},\\(0,${N}\\.${N}\\)\\(\\)\\.enabled\\)&&null!=${N}\\.appServices\\.cumberbatch)`, "g"),
          (_, gate, rest) => `(${gate}||${P}?.runtime.allow()===true)${rest}`);
      } else if (kind === "composer") {
        // This dependency and component uniquely own the catch-up query.
        const match = new RegExp(`hostId:(${N}),pluginHostId:${N},roots:${N}\\}=${N},(${N})=\\(0,${N}\\.${N}\\)\\(${N}\\.${N}\\),${N}=\\(0,${N}\\.${N}\\)\\(${N}\\.${N}\\),${N}=\\(0,${N}\\.${N}\\)\\(${N}\\.${N}\\),(${N})=(${N})\\.appServices\\.cumberbatch`).exec(source);
        if (!match || !source.includes('generationTrigger:"catch_up"')) throw Error("composer-contract");
        const [, host, scope, service, services] = match;
        replace(new RegExp(esc(`${service}=${services}.appServices.cumberbatch`), "g"),
          () => `${service}=${P}.runtime.serviceFor(${scope},${host},${services}.appServices.cumberbatch)`);
      } else if (kind === "host") {
        const match = new RegExp(`async function ${N}\\((${N}),${N},(${N}),${N}\\)\\{let (${N})=(${N})\\.appServices\\?\\.cumberbatch;`).exec(source);
        if (!match || !source.includes("Cumberbatch background prediction did not finish")) throw Error("host-contract");
        const [, scope, target, service, services] = match;
        replace(new RegExp(esc(`let ${service}=${services}.appServices?.cumberbatch;`), "g"),
          () => `let ${service}=${P}.runtime.serviceFor(${scope},${target}.hostId,${services}.appServices?.cumberbatch);`);
        replace(new RegExp(`\\(0,${N}\\.${N}\\)\\(${esc(scope)},"4290031608"\\)`, "g"),
          gate => `(${gate}||${P}?.runtime.allow(${target}.hostId)===true)`, 2);
        // Dgwo exports the scope-bound RPC accessor; the native preference reader
        // and factory remain authoritative, including cancellation on disable.
        replace(/n\.d\(t,\{V:\(\)=>p\},\{p:d\}\)\}/g, () =>
          `${P}.runtime.register({managerForHost:p,nativeEnabled:scope=>n("RxZZ").CN(scope.get.bind(scope),n("eEqo").W().enabled)===true});n.d(t,{V:()=>p},{p:d})}`);
      } else if (kind === "settings") {
        const match = new RegExp(`(${N})=\\(0,${N}\\.${N}\\)\\(${N}\\.u\\);[\\s\\S]{0,600}?(${N})=\\(0,(${N}\\.${N})\\)\\("4290031608"\\),(${N})=(${N})\\.appServices\\.cumberbatch`).exec(source);
        if (!match || !source.includes('"cumberbatch-availability"')) throw Error("settings-contract");
        const [, scope, gate, check, service, services] = match;
        replace(new RegExp(esc(`${gate}=(0,${check})("4290031608"),${service}=${services}.appServices.cumberbatch`), "g"),
          () => `${gate}=((0,${check})("4290031608")||${P}?.runtime.allow()===true),${service}=${P}.runtime.serviceFor(${scope},"local",${services}.appServices.cumberbatch)`);
      } else throw Error("unsupported-kind");
      return { ok: true, source };
    } catch (error) { return { ok: false, source: original, reason: error.message }; }
  }

  function prepare(html, read) {
    const base = "app://-/index.html";
    const entries = [...html.matchAll(/<script\b[^>]*\bsrc=(["'])(.*?)\1/g)]
      .map(match => new URL(match[2], base).href)
      .filter(url => /^app:\/\/-\/assets\/\d+\.[a-f0-9]+\.js$/.test(url));
    if (!entries.length) return { ok: false, reason: "not-rspack" };
    try {
      const sources = entries.map(read).flatMap(source =>
        [...source.matchAll(/\.u=\w+=>"assets\/async\/"\+\w+\+"\."\+\(\{([^}]+)\}/g)].map(match => `{${match[1]}`));
      const modules = Object.entries(chunks).map(([kind, id]) => {
        const hashes = new Set(sources.flatMap(source => [...source.matchAll(new RegExp(`(?:^|[,\\{])${id}:"([a-f0-9]+)"`, "g"))].map(match => match[1])));
        if (hashes.size !== 1) throw Error("chunk-manifest-contract");
        const url = `app://-/assets/async/${id}.${[...hashes][0]}.js`;
        const result = transform(read(url), kind);
        if (!result.ok) throw Error(result.reason);
        // Rspack imports resolve relative to their original chunk, not blob:.
        const source = result.source.replace(/((?:from|import)\s*\(?\s*)(["'])(\.{1,2}\/[^"']+)\2/g,
          (_, prefix, quote, path) => `${prefix}${quote}${new URL(path, url).href}${quote}`);
        return { url, source };
      });
      return { ok: true, modules };
    } catch (error) { return { ok: false, reason: error.message }; }
  }

  function install(prepared) {
    const doc = root.document;
    if (!prepared.ok || doc?.documentElement || !root.HTMLScriptElement?.supports?.("importmap")) return false;
    const imports = {}, blobs = [];
    try {
      for (const module of prepared.modules) {
        const blob = root.URL.createObjectURL(new root.Blob([module.source], { type: "text/javascript" }));
        blobs.push(blob); imports[module.url] = blob;
      }
      const map = doc.createElement("script");
      map.type = "importmap"; map.textContent = JSON.stringify({ imports });
      const container = doc.createElement("html");
      doc.appendChild(container); container.appendChild(map);
      root.__cockpitNativePredictions.setReady(true);
      root.__cockpitEnhancements.nativePatchState = "unsupported";
      root.__cockpitEnhancements.nativePatchReason = "rspack-fast-unsupported";
      root.__cockpitEnhancements.nativePredictionPatchState = "ready";
      root.__cockpitEnhancements.nativePredictionPatchReason = "rspack-contracts-installed";
      // Lazy chunks must retain their URLs for the lifetime of this document.
      root.addEventListener("pagehide", () => {
        root.__cockpitNativePredictions.runtime.dispose();
        blobs.forEach(blob => root.URL.revokeObjectURL(blob));
      }, { once: true });
      return true;
    } catch {
      blobs.forEach(blob => root.URL.revokeObjectURL(blob));
      root.__cockpitNativePredictions.setReady(false);
      return false;
    }
  }
  const api = { transform, prepare, install };
  root.__cockpitPredictionRspack = api;
  if (typeof module === "object" && module.exports) module.exports = api;
})();
