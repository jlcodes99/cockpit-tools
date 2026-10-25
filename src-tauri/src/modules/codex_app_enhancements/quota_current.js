// Current desktop layout: initial exports, primary usage presenters and a
// separate Codex composer adapter. Validate every boundary before publishing.
(() => {
  const root = typeof window === 'object' ? window : globalThis;
  const id = '[A-Za-z_$][\\w$]*';
  const escape = value => value.replace(/[$]/g, '\\$&');
  const range = (source, start) => root.__cockpitNativeFast?.functionRange(source, start);
  const one = matches => matches.length === 1 ? matches[0] : null;
  const failed = (source, reason) => ({ ok: false, patchedSource: source, diagnostics: { reason } });
  const exported = (source, name) => source.slice(source.lastIndexOf('export{'))
    .match(new RegExp(`(?:\\{|,)${escape(name)} as (${id})(?=,|\\})`))?.[1];
  const importFrom = (source, url, target) => one([...source.matchAll(/import\{([^{}]+)\}from(["'`])([^"'`]+)\2/g)]
    .filter(match => { try { return new URL(match[3], url).href === target; } catch { return false; } }));
  const imported = (entry, name) => typeof name === 'string' && entry
    ? entry[1].match(new RegExp(`(?:^|,)${escape(name)} as (${id})(?=,|$)`))?.[1] : null;
  function initial(source, url, bindings) {
    const adapters = [...new Set([...source.matchAll(/["'`](\.\/codex-composer-adapter-[A-Za-z0-9_-]+\.js)["'`]/g)].map(match => match[1]))];
    const composers = [...new Set([...source.matchAll(/["'`](\.\/composer-content-[A-Za-z0-9_-]+\.js)["'`]/g)].map(match => match[1]))];
    if (adapters.length !== 1 || composers.length !== 1) return failed(source, 'quota-current-layout-mismatch');
    return { ok: true, patchedSource: `${source}\n;globalThis.__cockpitQuotaCompat.initialLayout="adapter-v1";\n;globalThis.__cockpitQuotaCompat.initialReady=true;\n`,
      diagnostics: { reason: 'ready', bindings: { ...bindings, layout: 'adapter-v1', adapterPath: adapters[0] } } };
  }
  function primary(source, url, initialUrl, bindings) {
    if (!importFrom(source, url, initialUrl)) return failed(source, 'quota-current-primary-import-mismatch');
    const guard = one([...source.matchAll(new RegExp(
      `(?<atom>${id})=${id}\\(${id},\\(\\{get:(?<get>${id})\\}\\)=>\\{let (?<account>${id})=\\k<get>\\(${id}\\),(?<usage>${id})=\\k<get>\\(${id}\\)\\.data;if\\(\\k<account>\\.authMethod!==\`chatgpt\``, 'g'))]);
    if (!guard) return failed(source, 'quota-current-guard-not-unique');
    const definition = source.slice(guard.index, guard.index + 1100);
    if (!['authLoading', 'accountLoading', 'authenticatedAccountId', 'user_id', 'plan_type',
      'rate_limit?.allowed!==!1', 'remainingPercent:', 'windowMinutes:', 'resetsAt:'].every(term => definition.includes(term)))
      return failed(source, 'quota-current-guard-contract-mismatch');
    const genericBlocked = exported(source, guard.groups.atom);
    if (!genericBlocked) return failed(source, 'quota-current-guard-export-missing');
    const shared = root.__cockpitQuotaCompat.transformSharedBanner(source, 'shouldHideCurrentBanner');
    if (!shared) return failed(source, 'quota-current-shared-banner-mismatch');
    const presenters = [...shared.matchAll(new RegExp(
      `function ${id}\\(${id}\\)\\{let [\\s\\S]{0,120}?,\\{canShowUsageBanners:(?<enabled>${id}),hostId:${id},imageGenerationLimit:(?<image>${id}),onDismissImageGenerationLimit:${id},rateLimit:(?<usage>${id}),rateLimitWarningThreshold:${id},lowerPriorityContent:(?<fallback>${id}),`, 'g'))];
    const presenter = one(presenters), section = presenter && range(shared, presenter.index);
    if (!section) return failed(source, 'quota-current-presenter-not-unique');
    const body = shared.slice(section.start, section.end);
    const react = body.match(new RegExp(`\\(0,(${id})\\.useRef\\)\\(null\\)`))?.[1];
    const gate = new RegExp(`${id}\\(${id},(?<host>${id})\\);(?<gate>if\\(!${escape(presenter.groups.enabled)}\\)return ${escape(presenter.groups.fallback)};)`).exec(body);
    if (!react || !gate || !['`image_generation_limit_reached`', 'rateLimitStatus:', 'sendBlocked:', 'selectedModel:'].every(term => body.includes(term)))
      return failed(source, 'quota-current-presenter-contract-mismatch');
    const { usage, image, fallback } = presenter.groups;
    const point = section.start + gate.index + gate[0].length - gate.groups.gate.length;
    const insert = `let __cockpitBannerScope=globalThis.__cockpitQuotaCompat.useBannerScope(${react});if(globalThis.__cockpitQuotaCompat.shouldHideCurrentBanner(__cockpitBannerScope,${gate.groups.host},${usage},${image}))return ${fallback};`;
    return { ok: true, patchedSource: shared.slice(0, point) + insert + shared.slice(point),
      diagnostics: { reason: 'ready', bindings: { ...bindings, genericBlocked } } };
  }
  function adapter(source, url, initialUrl, primaryUrl, bindings) {
    if (typeof source !== 'string' || source.includes('__cockpitQuotaProvider')
      || ![bindings?.hardBlocked, bindings?.genericBlocked, bindings?.threadProvider].every(value => typeof value === 'string' && new RegExp(`^${id}$`).test(value))) return failed(source, 'unsupported-source');
    try {
      const parsed = new URL(url), initial = new URL(initialUrl);
      if (parsed.href !== new URL(bindings?.adapterPath, initialUrl).href
        || parsed.protocol !== initial.protocol || parsed.host !== initial.host
        || !/\/assets\/codex-composer-adapter-[A-Za-z0-9_-]+\.js$/.test(parsed.pathname)
        || parsed.search || parsed.hash || parsed.username || parsed.password
        || !((parsed.protocol === 'app:' && parsed.hostname === '-') || (parsed.protocol === 'file:' && !parsed.hostname)))
        return failed(source, 'unsupported-url');
    } catch { return failed(source, 'unsupported-url'); }
    const initialImport = importFrom(source, url, initialUrl), primaryImport = importFrom(source, url, primaryUrl);
    const hardAtom = imported(initialImport, bindings?.hardBlocked), genericAtom = imported(primaryImport, bindings?.genericBlocked);
    if (!hardAtom || !genericAtom || !bindings?.threadProvider) return failed(source, 'quota-current-adapter-imports-missing');
    const hardRead = one([...source.matchAll(new RegExp(`(?<hard>${id})=(?<read>${id})\\(${escape(hardAtom)},(?<host>${id})\\)`, 'g'))]);
    if (!hardRead) return failed(source, 'quota-current-adapter-not-unique');
    const start = source.lastIndexOf('function ', hardRead.index), section = range(source, start);
    if (!section || hardRead.index >= section.end) return failed(source, 'quota-current-adapter-range-mismatch');
    const body = source.slice(start, section.end), { hard, read, host } = hardRead.groups;
    const props = body.match(new RegExp(`^function ${id}\\((${id})\\)`))?.[1];
    const react = body.match(new RegExp(`\\(0,(${id})\\.useRef\\)\\(`))?.[1];
    const jsx = body.match(new RegExp(`\\(0,(${id})\\.jsxs?\\)\\(`))?.[1];
    const scope = one([...new Set([...body.matchAll(new RegExp(`(${id})\\.value\\.kind===\`new\``, 'g'))].map(match => match[1]))]);
    const thread = body.match(new RegExp(`${escape(host)}=${escape(read)}\\(${id},(?<thread>${id})\\)`))?.groups.thread;
    const config = one([...body.matchAll(new RegExp(`(?<settings>${id})\\.isConfigDataPending\\?null:\\k<settings>\\.resolvedConfig`, 'g'))]);
    const genericRead = one([...body.matchAll(new RegExp(`(?<blocked>${id})=${id}\\(${escape(host)}===\`local\`&&${escape(genericAtom)}\\)`, 'g'))]);
    if (!props || !react || !jsx || !scope || !thread || !config || !genericRead
      || !['useAsbDirect:', 'submitDisabled:', 'onSubmitLocal:', 'selectedProject:'].every(term => body.includes(term)))
      return failed(source, 'quota-current-adapter-contract-mismatch');
    const generic = genericRead.groups.blocked, settings = config.groups.settings;
    const tail = one([...body.matchAll(new RegExp(`\\|\\|${escape(hard)}\\|\\|${escape(generic)}(?=,${id};)`, 'g'))]);
    const submitStart = tail && body.lastIndexOf('let ', tail.index);
    const submit = tail && body.slice(submitStart, tail.index).match(new RegExp(`^let (${id})=`))?.[1];
    if (!tail || !submit || !body.includes(`submitDisabled:${submit}`)) return failed(source, 'quota-current-submit-mismatch');
    const eligible = `globalThis.__cockpitQuotaCompat.enabledForComposer(${host},${settings},__cockpitQuotaProvider,${props},${scope})`;
    const returnOffset = section.returnAt - start;
    const replacement = `||((${hard}||${generic})&&!${eligible})`;
    const providerRead = `,__cockpitQuotaProvider=${read}(__cockpitQuotaProviderAtom,${thread})`;
    const originalReturn = body.slice(returnOffset + 7, -1);
    // Apply original-source edits from right to left. Hooks remain unconditional,
    // memo cache expressions and every unrelated submit predicate are retained.
    const edits = [
      [section.returnAt, section.end - 1, `return globalThis.__cockpitQuotaCompat.wrapCurrentComposer(${react},${jsx}.jsx,${host},${settings},__cockpitQuotaProvider,${eligible},(${originalReturn}))`],
      [start + tail.index, start + tail.index + tail[0].length, replacement],
      [hardRead.index + hardRead[0].length, hardRead.index + hardRead[0].length, providerRead],
      [initialImport.index + 7, initialImport.index + 7, `${bindings.threadProvider} as __cockpitQuotaProviderAtom,`],
    ].sort((a, b) => b[0] - a[0]);
    let patched = source;
    for (const [from, to, value] of edits) patched = patched.slice(0, from) + value + patched.slice(to);
    const anchor = start + submitStart, before = source.slice(0, anchor);
    return { ok: true, patchedSource: patched, diagnostics: { reason: 'ready', location: {
      layout: 'adapter-v1', url, lineNumber: before.split('\n').length - 1,
      columnNumber: anchor - before.lastIndexOf('\n') - 1,
      hardVariable: hard, guardVariable: generic, hostVariable: host,
      settingsVariable: settings, scopeVariable: scope, threadVariable: thread, propsVariable: props,
    } } };
  }
  root.__cockpitQuotaCurrent = { initial, primary, adapter };
})();
