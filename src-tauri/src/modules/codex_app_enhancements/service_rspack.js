// Rspack duplicates factories in independent lazy routes. Keep the native
// selectors and submission pipeline, changing only their gates.
(() => {
  "use strict";
  const root = typeof window === "object" ? window : globalThis;
  const N = "[A-Za-z_$][\\w$]*";
  const fast = "globalThis.__cockpitNativeFast";
  const pattern = source => new RegExp(source, "g");
  const escape = value => value.replace(/\$/g, "\\$");
  const speedLayouts = [
    {
      host: 739301,
      menus: [184348, 222624, 244404, 29761, 309546, 363454, 390941],
      modelQueries: [201528, 256736, 304386, 377014, 379153, 415411, 504134, 669972,
        687108, 750210, 782647, 838074, 872702, 963873],
    },
    {
      host: 182735,
      menus: [159406, 29761, 414655, 434155, 553834, 634175, 884655],
      modelQueries: [161445, 177472, 291906, 32593, 328669, 40728, 415411, 416681,
        455856, 503951, 870918],
    },
  ];

  function one(source, re, reason) {
    const matches = [...source.matchAll(re)];
    if (matches.length !== 1) throw Error(reason);
    return matches[0];
  }

  function edit(source, match, value) {
    return source.slice(0, match.index) + value + source.slice(match.index + match[0].length);
  }

  function speedMenu(source) {
    const match = one(source, pattern(String.raw`\b(?<host>${N})=${N}\?\.hostId\?\?${N},(?<account>${N})=\(0,${N}\.tn\)\(\k<host>\),${N}=(?<native>\k<account>\?\.authMethod==="chatgpt"\|\|\k<account>\?\.authMethod==="personalAccessToken"\|\|\k<account>\?\.authMethod==="amazonBedrock")(?=,${N}=\k<account>\?\.authMethod\?\?null;[\s\S]{0,650}serviceTierAccess:)`), "speed-menu-contract");
    const { host, account, native } = match.groups;
    return edit(source, match, match[0].replace(native, () => `(${native}||${fast}.allow(${host},${account}?.authMethod))`));
  }

  function speedQueue(source) {
    const queued = one(source, pattern(String.raw`async\(\)=>\{if\("local"===(?<manager>${N})\.getHostId\(\)&&await ${N}\.isUsingCopilotInference\(\)\)return!1;let (?<account>${N})=await \k<manager>\.getAccount\(\{priority:"critical"\}\);return (?<native>\k<account>\.account\?\.type==="chatgpt"\|\|\k<account>\.account\?\.type==="amazonBedrock")(?=\}\)\},async writeServiceTier\()`), "speed-saved-request-contract");
    const { manager, account, native } = queued.groups;
    source = edit(source, queued, queued[0].replace(native, () => `(${native}||${fast}.allow(${manager}.getHostId(),${account}.account?.type))`));

    const explicit = one(source, pattern(String.raw`let (?<options>${N})=${N}\.submissionOptions,[\s\S]{0,240}?(?<host>${N})=\k<options>\?\.executionHostId\?\?(?<manager>${N})\.getHostId\(\);[\s\S]{0,1700}?if\(null==\k<options>\.serviceTier\)return null;let\{account:(?<account>${N})\}=await \k<manager>\.getAccount\(\{priority:"critical"\}\);return (?<native>\k<account>\?\.type==="chatgpt"\|\|\k<account>\?\.type==="amazonBedrock")(?=\?\k<options>\.serviceTier:null\}\)\(\),)`), "speed-explicit-request-contract");
    const runtime = source.slice(explicit.index - 180, explicit.index)
      .match(new RegExp(String.raw`\{settings:${N},storage:${N},filesystem:${N},runtime:(${N})\}`))?.[1];
    if (!runtime) throw Error("speed-explicit-runtime-contract");
    const vars = explicit.groups;
    const allow = `${vars.host}==="local"&&!${vars.manager}.isCloudHost&&${fast}.allow(${vars.manager}.getHostId(),${vars.account}?.type)&&!await ${runtime}.isUsingCopilotInference()`;
    return edit(source, explicit, explicit[0].replace(vars.native, () => `(${vars.native}||${allow})`));
  }

  async function verifySpeedDocument() {
    if (root.__cockpitEnhancements?.nativeFast !== true) return { ok: false, reason: "disabled" };
    const entries = [...(root.document?.scripts ?? [])].map(script => script.src)
      .filter(url => /^app:\/\/-\/assets\/\d+\.[a-f0-9]+\.js$/.test(url));
    if (!entries.length || entries.length > 8) return { ok: false, reason: "unsupported-entry" };
    const read = async url => {
      const response = await root.fetch(url, { signal: AbortSignal.timeout(6000) });
      if (!response.ok) throw Error("asset-unavailable");
      const source = await response.text();
      if (source.length > 24 * 1024 * 1024) throw Error("source-limit");
      return source;
    };
    try {
      const roots = new Map(await Promise.all(entries.map(async url => [url, await read(url)])));
      const html = entries.map(url => '<script src="' + url + '"></script>').join("");
      const reader = root.__cockpitPredictionRspack.chunkReader(html, url => roots.get(url));
      const layout = speedLayouts.find(candidate => reader.has(candidate.host));
      if (!layout) return { ok: false, reason: "speed-layout-unsupported" };
      await Promise.all([[layout.host, speedQueue], ...layout.menus.map(id => [id, speedMenu])]
        .map(async ([id, transform]) => {
          const source = await read(reader.urlFor(id));
          if (!source.includes(`export const __rspack_esm_ids=[${id}];`)
            && !source.includes(`export const __rspack_esm_id=${id};`)) throw Error("unsupported-chunk");
          transform(source);
        }));
      return { ok: true };
    } catch (error) {
      return { ok: false, reason: error.message || "speed-contract-failed" };
    }
  }

  function ultraReasoning(source) {
    const match = one(source, pattern(String.raw`authMethod:(?<host>${N}\.hostId)===${N}\.${N}\?(?<auth>${N}\.authMethod):null,[^;]{0,900}includeUltraReasoningEffort:(?<gate>\(0,${N}\.${N}\)\(${N},"1186680773"\)),isAuthLoading:[\s\S]{0,550}?isCustomModelProvider:(?<custom>\(0,${N}\.cc\)\(${N}\)),workspaceDependenciesEnabled:`), "ultra-reasoning-contract");
    const { host, auth, gate, custom } = match.groups;
    return edit(source, match, match[0].replace(gate,
      () => `${fast}.includeUltra(${gate},${host},${auth},${custom},null)`));
  }

  function ultraModelQuery(source) {
    const match = one(source, pattern(String.raw`\{additionalAvailableModels:${N},authMethod:(?<auth>${N}),enabledReasoningEfforts:${N},hostId:(?<host>${N}),includeUltraReasoningEffort:(?<requested>${N})=!0,limit:${N}=100,specialty:${N},teamCatalog:(?<team>${N})\},\{get:(?<get>${N}),queryClient:${N},scope:${N}\}\)=>\{[^;]{1,1200}?(?<custom>${N})=\(0,${N}\.cc\)\(${N}\),[^;]{1,1200}?(?<selected>${N})=(?<native>\k<requested>&&\k<get>\(${N}\.JW,"1186680773"\));(?=return\{queryKey:)`), "ultra-model-query-contract");
    const { auth, host, requested, team, custom, selected, native } = match.groups;
    if (!source.slice(match.index, match.index + 3500).includes(`includeUltraReasoningEffort:${selected},isCustomModelProvider:${custom}`))
      throw Error("ultra-model-selector-contract");
    return edit(source, match, match[0].replace(native,
      () => `${requested}&&${fast}.includeUltra(${native},${host},${auth},${custom},${team})`));
  }

  function preload(html, read) {
    const state = root.__cockpitEnhancements;
    const rspack = root.__cockpitPredictionRspack;
    const reader = rspack.chunkReader(html, read);
    if (!reader) return null;
    const chunks = new Map();
    const readChunk = id => {
      if (!chunks.has(id)) chunks.set(id, reader(id));
      return chunks.get(id);
    };
    readChunk.has = reader.has;
    const wantSpeed = state?.nativeFast === true;
    const wantPredictions = state?.nativePredictions === true
      && (state.apiKeyAuthVerified === true || state.apiService === true);
    const modules = new Map();
    const prepare = (enabled, build) => {
      if (!enabled) return { ok: false, reason: "disabled" };
      try {
        const batch = build();
        for (const module of batch) modules.set(module.url, module);
        return { ok: true };
      } catch (error) {
        return { ok: false, reason: error.message };
      }
    };
    const patch = (id, transform) => {
      const { url, source } = readChunk(id);
      return { url, source: transform(modules.get(url)?.source ?? source) };
    };
    const speedLayout = speedLayouts.find(layout => readChunk.has(layout.host));
    const speed = prepare(wantSpeed, () => {
      if (!speedLayout) throw Error("speed-layout-unsupported");
      return [patch(speedLayout.host, speedQueue), ...speedLayout.menus.map(id => patch(id, speedMenu))];
    });
    const ultra = prepare(wantSpeed && speed.ok, () => [
      patch(speedLayout.host, ultraReasoning),
      ...speedLayout.modelQueries.map(id => patch(id, ultraModelQuery)),
    ]);
    const predictions = prepare(wantPredictions, () => {
      const readWithPreparedModules = id => {
        const chunk = readChunk(id);
        return modules.get(chunk.url) ?? chunk;
      };
      readWithPreparedModules.has = readChunk.has;
      const result = rspack.prepare(html, read, readWithPreparedModules);
      if (!result.ok) throw Error(result.reason);
      return result.modules;
    });
    const ok = rspack.installModules([...modules.values()], () => root.__cockpitNativePredictions?.runtime.dispose());
    const reason = ok ? "rspack-contracts-installed"
      : modules.size ? "rspack-install-failed"
      : [speed, predictions].find(result => result.reason !== "disabled")?.reason ?? "rspack-contracts-disabled";
    const mask = [wantSpeed, state?.quotaCompat, state?.apiKeyAuthVerified, state?.apiService]
      .map(value => value === true ? "1" : "0").join("") + (state?.nativePredictions ? "1" : "");
    if (state) {
      root.__cockpitNativePredictions?.setReady(ok && predictions.ok);
      Object.assign(state, {
        nativePatchState: ok ? "active" : "unsupported",
        nativePatchMode: "rspack",
        nativePatchReason: reason,
        nativePatchFeatures: mask,
        nativePatchRequestedFeatures: mask,
        nativePredictionPatchState: wantPredictions ? ok && predictions.ok ? "ready" : "unsupported" : "disabled",
        nativePredictionPatchReason: predictions.reason ?? (ok ? undefined : "rspack-install-failed"),
      });
    }
    return { ok, reason, mode: "rspack", speed, ultra, predictions };
  }

  root.__cockpitServiceRspack = { preload, verifySpeedDocument };
})();
