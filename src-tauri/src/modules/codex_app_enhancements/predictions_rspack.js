// Codex 26.1029 ships Rspack ESM chunks. Keep the native editor and its
// lifecycle intact; replace only the same four prediction contracts as Vite.
(() => {
  "use strict";
  const root = typeof window === "object" ? window : globalThis;
  const P = "window.__cockpitNativePredictions";
  const chunks = { initial: 559161, composer: 563813, host: 24191, settings: 572868 };
  const currentChunks = { initial: 16389, composer: 39017, host: 182735, settings: 553834 };
  const N = "[A-Za-z_$][\\w$]*";
  const esc = value => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

  function transform(source, kind, id = chunks[kind]) {
    const original = source;
    const replace = (pattern, callback, expected = 1) => {
      let count = 0;
      source = source.replace(pattern, (...args) => { count++; return callback(...args); });
      if (count !== expected) throw Error(`${kind}-contract`);
    };
    try {
      if (typeof source !== "string" || source.length > 24 * 1024 * 1024
        || !hasChunkId(source, id)) throw Error("unsupported-chunk");
      if (kind === "initial") {
        replace(new RegExp(`(${N}\\(${N}\\.${N},"4290031608"\\))(&&\\(0,${N}\\.${N}\\)\\(${N},\\(0,${N}\\.${N}\\)\\(\\)\\.enabled\\)&&null!=${N}\\.appServices\\.cumberbatch)`, "g"),
          (_, gate, rest) => {
            const allow = `${P}?.runtime.allow()===true`;
            if (id !== currentChunks.initial) return `(${gate}||${allow})${rest}`;
            const at = rest.indexOf("&&null!=");
            return `(${gate}||${allow})${rest.slice(0, at)}&&(${rest.slice(at + 2)}||${allow})`;
          });
      } else if (kind === "composer") {
        // This dependency and component uniquely own the catch-up query.
        const match = new RegExp(`hostId:(${N}),pluginHostId:${N},roots:${N}\\}=${N},(${N})=\\(0,${N}\\.${N}\\)\\(${N}\\.${N}\\),${N}=\\(0,${N}\\.${N}\\)\\(${N}\\.${N}\\),${N}=\\(0,${N}\\.${N}\\)\\(${N}\\.${N}\\),(${N})=(${N})\\.appServices\\.cumberbatch`).exec(source);
        if (!match || !source.includes('generationTrigger:"catch_up"')) throw Error("composer-contract");
        const [, host, scope, service, services] = match;
        replace(new RegExp(esc(`${service}=${services}.appServices.cumberbatch`), "g"),
          () => `${service}=${P}.runtime.serviceFor(${scope},${host},${services}.appServices.cumberbatch)`);
      } else if (kind === "host") {
        const name = id === currentChunks.host ? `(?: ${N})?` : ` ${N}`;
        const match = new RegExp(`async function${name}\\((${N}),${N},(${N}),${N}\\)\\{let (${N})=(${N})\\.appServices\\?\\.cumberbatch;`).exec(source);
        if (!match || !source.includes("Cumberbatch background prediction did not finish")) throw Error("host-contract");
        const [, scope, target, service, services] = match;
        replace(new RegExp(esc(`let ${service}=${services}.appServices?.cumberbatch;`), "g"),
          () => `let ${service}=${P}.runtime.serviceFor(${scope},${target}.hostId,${services}.appServices?.cumberbatch);`);
        replace(new RegExp(`\\(0,${N}\\.${N}\\)\\(${esc(scope)},"4290031608"\\)`, "g"),
          gate => `(${gate}||${P}?.runtime.allow(${target}.hostId)===true)`, 2);
        // Dgwo exports the scope-bound RPC accessor; the native preference reader
        // and factory remain authoritative, including cancellation on disable.
        if (id === currentChunks.host) {
          replace(new RegExp(`Dgwo\\((${N}),(${N}),(${N})\\)\\{(?=[^}]{1,430}\\.d\\(\\2,\\{\\},\\{V:function\\()`, "g"),
            (signature, _module, exports, require) => signature
              + `${P}.runtime.register({managerForHost:(...args)=>${exports}.V(...args),nativeEnabled:scope=>${require}("RxZZ").CN(scope.get.bind(scope),${require}("eEqo").W().enabled)===true});`);
        } else {
          replace(/n\.d\(t,\{V:\(\)=>p\},\{p:d\}\)\}/g, () =>
            `${P}.runtime.register({managerForHost:p,nativeEnabled:scope=>n("RxZZ").CN(scope.get.bind(scope),n("eEqo").W().enabled)===true});n.d(t,{V:()=>p},{p:d})}`);
        }
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

  const hasChunkId = (source, id) => source.includes(`export const __rspack_esm_ids=[${id}];`)
    || source.includes(`export const __rspack_esm_id=${id};`);

  function chunkReader(html, read) {
    const entries = [...html.matchAll(/<script\b[^>]*\bsrc=(["'])(.*?)\1/g)]
      .map(match => new URL(match[2], "app://-/index.html").href)
      .filter(url => /^app:\/\/-\/assets\/\d+\.[a-f0-9]+\.js$/.test(url));
    if (!entries.length) return null;
    const hashes = new Map();
    for (const source of entries.map(read)) {
      for (const [, manifest] of source.matchAll(/\.u=\w+=>"assets\/async\/"\+\w+\+"\."\+\(\{([^}]+)\}/g)) {
        for (const [, id, hash] of manifest.matchAll(/(?:^|,)(\d+):"([a-f0-9]+)"/g)) {
          hashes.set(id, hashes.has(id) && hashes.get(id) !== hash ? null : hash);
        }
      }
    }
    const urlFor = id => {
      const hash = hashes.get(String(id));
      if (!hash) throw Error("chunk-manifest-contract");
      return "app://-/assets/async/" + id + "." + hash + ".js";
    };
    const readChunk = id => {
      const url = urlFor(id);
      const source = read(url);
      if (source.length > 24 * 1024 * 1024 || !hasChunkId(source, id))
        throw Error("unsupported-chunk");
      return { url, source };
    };
    readChunk.urlFor = urlFor;
    readChunk.has = id => Boolean(hashes.get(String(id)));
    return readChunk;
  }

  function prepare(html, read, existingReader) {
    try {
      const readChunk = existingReader ?? chunkReader(html, read);
      if (!readChunk) return { ok: false, reason: "not-rspack" };
      const layout = readChunk.has(currentChunks.initial) ? currentChunks : chunks;
      const modules = Object.entries(layout).map(([kind, id]) => {
        const { url, source } = readChunk(id);
        const result = transform(source, kind, id);
        if (!result.ok) throw Error(result.reason);
        return { url, source: result.source };
      });
      return { ok: true, modules };
    } catch (error) { return { ok: false, reason: error.message }; }
  }

  function installModules(modules, onPageHide) {
    const doc = root.document;
    if (!modules.length || !doc || doc.documentElement || !root.HTMLScriptElement?.supports?.("importmap")) return false;
    const imports = {}, blobs = [], preloads = [];
    let container;
    try {
      for (const { url, source } of modules) {
        if (Object.hasOwn(imports, url)) throw Error("duplicate-module");
        const preload = doc.createElement("link");
        if (preload.relList?.supports?.("modulepreload") !== true) throw Error("unsupported-modulepreload");
        // Rspack imports resolve relative to their original chunk, not blob:.
        const absolute = source.replace(/((?:from|import)\s*\(?\s*)(["'])(\.{1,2}\/[^"']+)\2/g,
          (_, prefix, quote, path) => prefix + quote + new URL(path, url).href + quote);
        const blob = root.URL.createObjectURL(new root.Blob([absolute], { type: "text/javascript" }));
        blobs.push(blob); imports[url] = blob;
        preload.rel = "modulepreload"; preload.href = blob; preloads.push(preload);
      }
      const map = doc.createElement("script");
      map.type = "importmap"; map.textContent = JSON.stringify({ imports });
      container = doc.createElement("html");
      doc.appendChild(container); container.appendChild(map);
      // The document's CSP excludes blob scripts. Populate the module map
      // before its meta tag is parsed, including chunks imported later.
      preloads.forEach(preload => container.appendChild(preload));
      // Lazy chunks must retain their URLs for the lifetime of this document.
      root.addEventListener("pagehide", () => {
        blobs.forEach(blob => root.URL.revokeObjectURL(blob));
        onPageHide?.();
      }, { once: true });
      return true;
    } catch {
      blobs.forEach(blob => root.URL.revokeObjectURL(blob));
      return false;
    } finally {
      if (container?.parentNode === doc) doc.removeChild(container);
    }
  }

  const api = { transform, prepare, chunkReader, installModules };
  root.__cockpitPredictionRspack = api;
  if (typeof module === "object" && module.exports) module.exports = api;
})();
