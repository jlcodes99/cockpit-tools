(() => {
  const root = typeof window === 'object' ? window : globalThis;
  const identifier = '[A-Za-z_$][\\w$]*';
  const selector = new RegExp(
    `(?<name>${identifier})=${identifier}\\(${identifier},\\((?<host>${identifier}),\\{get:(?<get>${identifier})\\}\\)=>(?<expression>\\k<get>\\((?<family>${identifier}),\\k<host>\\)\\.hardBlocked)\\)`,
    'g',
  );

  function enabledFor(host, settings, threadProvider) {
    if (host !== 'local' || root.__cockpitEnhancements?.quotaCompat !== true
      || settings?.isConfigDataPending !== false) return false;
    const config = settings.resolvedConfig;
    const providerId = threadProvider ?? config?.model_provider ?? 'openai';
    const provider = config?.model_providers?.[providerId];
    if (providerId === 'openai' || provider?.requires_openai_auth !== true
      || typeof provider.experimental_bearer_token !== 'string'
      || !provider.experimental_bearer_token.trim()) return false;
    try {
      const url = new URL(provider.base_url);
      return ['http:', 'https:'].includes(url.protocol) && !!url.hostname
        && !url.username && !url.password && !url.search && !url.hash;
    } catch { return false; }
  }

  // Share the existing composer's verified route with descendants, rather than
  // reading config again or leaking one conversation's route to another.
  // Reattaching the host must preserve Provider identity and the editor below
  // it, including drafts. Feature changes are read from root on each render.
  let bannerContext = root.__cockpitQuotaCompat?.getBannerContext?.();
  const contextFor = react => bannerContext ??= react.createContext(null);
  function wrapComposer(react, jsx, host, settings, provider, eligible, children, imageLimit = null) {
    return jsx(contextFor(react).Provider, {
      value: { host, compatible: eligible && enabledFor(host, settings, provider), imageLimit }, children,
    });
  }
  const useBannerScope = react => react.useContext(contextFor(react));
  const usageBannerTypes = new Set([
    'workspace_member_usage_limit_reached', 'workspace_owner_usage_limit_reached',
    'workspace_member_credits_depleted', 'workspace_owner_credits_depleted',
    'plus_rate_limit_reached', 'plus_gpt6_rate_limit_reached', 'prolite_rate_limit_reached',
    'pro_rate_limit_reached', 'free_trial_rate_limit_reached', 'go_trial_rate_limit_reached',
    'free_or_go_rate_limit_reached', 'business_rate_limit_reached', 'business_gpt6_rate_limit_reached',
    'cbp_rate_limit_reached', 'legacy_rate_limit_reached',
  ]);
  function shouldHideBanner(scope, host, usage, imageLimit) {
    const mode = root.__cockpitEnhancements;
    // The host supplies API Service identity from the persisted instance
    // binding, before React loads project settings. Keep this notice off for
    // that instance; ordinary accounts continue using their native policy.
    return (mode?.apiService === true || mode?.nativePatchState === 'active'
        && mode?.quotaCompat === true && host === 'local' && scope?.host === host && scope.compatible === true
        && usage?.rate_limit?.allowed === false)
      && imageLimit == null
      && usage?.rate_limit_upsell?.model_slug == null && usage?.rate_limit_upsell?.blocked_model_slug == null
      && usageBannerTypes.has(usage?.rate_limit_upsell?.banner_type);
  }

  const liveQuota = {
    client: null,
    originalSetQueryData: null,
    unsubscribe: null,
    observer: null,
    timer: null,
    retries: 0,
    rewriting: false,
    raw: new WeakMap(),
  };
  const liveQuotaEnabled = () => root.__cockpitEnhancements?.apiService === true
    && root.__cockpitEnhancements?.quotaCompat === true;
  const isMainUsage = value => !!value && typeof value === 'object'
    && !!value.rate_limit && typeof value.rate_limit.allowed === 'boolean';
  const suppressUsageUpsell = value => usageBannerTypes.has(value?.rate_limit_upsell?.banner_type)
    && value.rate_limit_upsell?.model_slug == null
    && value.rate_limit_upsell?.blocked_model_slug == null;
  function rewriteLiveUsage(value) {
    if (!liveQuotaEnabled() || !isMainUsage(value)) return value;
    let next = value;
    if (value.rate_limit.allowed !== true || value.rate_limit.limit_reached === true) {
      next = { ...next, rate_limit: { ...next.rate_limit, allowed: true, limit_reached: false } };
    }
    if (suppressUsageUpsell(value)) {
      next = { ...next, rate_limit_upsell: null, rate_limit_reached_type: null };
    }
    if (next !== value) liveQuota.raw.set(next, liveQuota.raw.get(value) || value);
    return next;
  }
  const trackedLiveUsage = value => rewriteLiveUsage(liveQuota.raw.get(value) || value);
  const mainUsageQueries = client => client?.getQueryCache?.()?.findAll?.({ queryKey: ['rate-limit-status'] })
    ?.filter?.(query => Array.isArray(query?.queryKey) && query.queryKey[1] !== 'image-generation') ?? [];

  function findLiveQueryClient() {
    if (liveQuota.client?.getQueryCache?.() && typeof liveQuota.client.setQueryData === 'function')
      return liveQuota.client;
    const looksLikeClient = value => !!value && typeof value.getQueryCache === 'function'
      && typeof value.setQueryData === 'function';
    const seen = new Set();
    const stack = [];
    for (const item of [root.document?.getElementById?.('root'), root.document?.body,
      root.document?.documentElement]) {
      if (!item) continue;
      const key = Object.keys(item).find(name => name.startsWith('__reactContainer$')
        || name.startsWith('__reactFiber$'));
      if (!key) continue;
      let fiber = item[key];
      if (fiber?.stateNode?.current) fiber = fiber.stateNode.current;
      stack.push(fiber);
    }
    let visited = 0;
    while (stack.length && visited < 10000) {
      const node = stack.pop();
      if (!node || typeof node !== 'object' || seen.has(node)) continue;
      seen.add(node);
      visited++;
      const candidates = [
        node.memoizedProps?.client,
        node.memoizedProps?.value,
        node.stateNode,
        node.memoizedState?.memoizedState,
      ];
      const found = candidates.find(looksLikeClient);
      if (found) return found;
      if (node.child) stack.push(node.child);
      if (node.sibling) stack.push(node.sibling);
    }
    return null;
  }

  function patchLiveQueryClient(client) {
    if (!client || client.__cockpitLiveQuotaClient) return client;
    liveQuota.client = client;
    liveQuota.originalSetQueryData = client.setQueryData;
    client.setQueryData = function cockpitLiveSetQueryData(queryKey, updater, ...rest) {
      if (!liveQuotaEnabled() || queryKey?.[0] !== 'rate-limit-status'
        || queryKey?.[1] === 'image-generation') {
        return liveQuota.originalSetQueryData.call(this, queryKey, updater, ...rest);
      }
      const next = typeof updater === 'function'
        ? previous => rewriteLiveUsage(updater(liveQuota.raw.get(previous) || previous))
        : rewriteLiveUsage(updater);
      liveQuota.rewriting = true;
      try {
        return liveQuota.originalSetQueryData.call(this, queryKey, next, ...rest);
      } finally {
        liveQuota.rewriting = false;
      }
    };
    Object.defineProperty(client, '__cockpitLiveQuotaClient', {
      value: true, configurable: true,
    });
    const cache = client.getQueryCache?.();
    if (cache?.subscribe) {
      liveQuota.unsubscribe = cache.subscribe(event => {
        if (liveQuota.rewriting || !liveQuotaEnabled()
          || event?.query?.queryKey?.[0] !== 'rate-limit-status'
          || event.query.queryKey[1] === 'image-generation') return;
        const current = event.query.state?.data;
        const next = trackedLiveUsage(current);
        if (next !== current) {
          liveQuota.rewriting = true;
          try {
            liveQuota.originalSetQueryData.call(client, event.query.queryKey, next);
          } finally {
            liveQuota.rewriting = false;
          }
        }
      });
    }
    return client;
  }

  function rewriteCachedLiveUsage(client) {
    let changed = 0;
    for (const query of mainUsageQueries(client)) {
      const current = query.state?.data;
      const next = trackedLiveUsage(current);
      if (next !== current) {
        liveQuota.rewriting = true;
        try {
          liveQuota.originalSetQueryData?.call(client, query.queryKey, next);
        } finally {
          liveQuota.rewriting = false;
        }
        changed++;
      }
    }
    return changed;
  }

  function refreshLiveComposers(plan) {
    let refreshed = 0;
    const tail = `||${plan.hardVariable}||${plan.guardVariable},`;
    for (const item of root.document?.querySelectorAll?.('[data-codex-composer-root]') ?? []) {
      let fiber = item[Object.keys(item).find(key => key.startsWith('__reactFiber$'))];
      for (let depth = 0; fiber && depth < 100; depth++, fiber = fiber.return) {
        if (typeof fiber.type !== 'function') continue;
        const source = fiber.type.toString();
        if (!source.includes('submitDisabled:') || !source.includes(tail)) continue;
        const hooks = [];
        for (let hook = fiber.memoizedState; hook; hook = hook.next) {
          if (hook.memoizedState instanceof Set && hook.memoizedState.size === 0
            && typeof hook.queue?.dispatch === 'function') hooks.push(hook);
        }
        if (hooks.length !== 1) continue;
        hooks[0].queue.dispatch(new Set());
        refreshed++;
        break;
      }
    }
    return refreshed;
  }

  function startLiveBannerObserver() {
    if (liveQuota.observer || !root.document?.documentElement) return;
    const hide = () => {
      if (!liveQuotaEnabled()) return;
      const text = root.document.body?.innerText || '';
      if (!/Codex 和工作使用额度已用完|You[’']re out of Codex and Work usage/i.test(text))
        return;
      for (const aside of root.document.querySelectorAll?.('aside[role="status"]') ?? []) {
        const content = aside.innerText || '';
        if (!/Codex 和工作使用额度已用完|You[’']re out of Codex and Work usage/i.test(content))
          continue;
        aside.style.setProperty('display', 'none', 'important');
        aside.setAttribute('data-cockpit-hidden-usage', 'true');
      }
    };
    liveQuota.observer = new root.MutationObserver(hide);
    liveQuota.observer.observe(root.document.documentElement, {
      childList: true, subtree: true, characterData: true,
    });
    hide();
  }

  function installLiveQuotaGuard(plan) {
    if (!liveQuotaEnabled()) return { ok: false, reason: 'disabled' };
    const active = root.__cockpitEnhancements;
    if (active.nativePatchState === 'active'
      && (active.nativePatchMode === 'full' && /^[01]{4}$/.test(active.nativePatchFeatures || '')
        || active.nativePatchMode === 'fast-only' && active.nativePatchFeatures?.startsWith('1'))) {
      root.clearTimeout?.(liveQuota.timer);
      liveQuota.timer = null;
      liveQuota.retries = 20;
      return { ok: true, preservedNativeModule: true,
        ...(active.nativePatchFeatures?.startsWith('1') ? { preservedNativeFast: true } : {}) };
    }
    startLiveBannerObserver();
    const apply = () => {
      const client = patchLiveQueryClient(findLiveQueryClient());
      const changed = client ? rewriteCachedLiveUsage(client) : 0;
      const refreshed = refreshLiveComposers(plan);
      return { client: !!client, changed, refreshed };
    };
    let applied = apply();
    if (!applied.client && liveQuota.retries < 20) {
      root.clearTimeout?.(liveQuota.timer);
      liveQuota.timer = root.setTimeout(() => {
        liveQuota.retries++;
        // Recheck activation before touching the page: Fast may have finished
        // loading while this older quota retry was waiting.
        installLiveQuotaGuard(plan);
      }, 250);
    }
    const config = root.__cockpitEnhancements;
    if (config) {
      config.nativePatchMode = 'live-quota';
      config.nativePatchFeatures = [false, true, false, true].map(Number).join('');
      config.nativePatchRequestedFeatures = [config.nativeFast, config.quotaCompat,
        config.apiKeyAuthVerified, config.apiService].map(Number).join('');
      config.nativePatchState = 'active';
      config.nativePatchReason = 'quota-breakpoint-armed';
    }
    return { ok: true, ...applied };
  }

  function disposeLiveQuotaGuard() {
    liveQuota.observer?.disconnect();
    liveQuota.observer = null;
    root.clearTimeout?.(liveQuota.timer);
    liveQuota.timer = null;
    for (const aside of root.document?.querySelectorAll?.('[data-cockpit-hidden-usage="true"]') ?? []) {
      aside.style.removeProperty('display');
      aside.removeAttribute('data-cockpit-hidden-usage');
    }
    liveQuota.unsubscribe?.();
    liveQuota.unsubscribe = null;
    const client = liveQuota.client;
    if (client) {
      for (const query of mainUsageQueries(client)) {
        const raw = liveQuota.raw.get(query.state?.data);
        if (raw) liveQuota.originalSetQueryData?.call(client, query.queryKey, raw);
      }
      if (liveQuota.originalSetQueryData) client.setQueryData = liveQuota.originalSetQueryData;
      delete client.__cockpitLiveQuotaClient;
    }
    liveQuota.client = null;
    liveQuota.originalSetQueryData = null;
  }

  async function prepareLiveQuotaGate() {
    if (!liveQuotaEnabled()) return { ok: false, reason: 'disabled' };
    const read = async value => {
      const response = await root.fetch(value, { cache: 'no-store' });
      if (!response.ok) throw new Error('asset-unavailable');
      return response.text();
    };
    try {
      const entries = [...(root.document?.scripts ?? [])]
        .map(script => script.src)
        .filter(value => /^app:\/\/-\/assets\/index-[A-Za-z0-9_-]+\.js$/.test(value));
      if (entries.length < 1 || entries.length > 4) return { ok: false, reason: 'unsupported-entry' };
      const initialUrls = new Set();
      for (const entry of entries) {
        for (const match of (await read(entry))
          .matchAll(/["'`]\.\/app-initial-[A-Za-z0-9_-]+\.js["'`]/g)) {
          initialUrls.add(new URL(match[0].slice(1, -1), entry).href);
        }
      }
      if (initialUrls.size !== 1) return { ok: false, reason: 'ambiguous-module' };
      const initialUrl = [...initialUrls][0];
      const initialSource = await read(initialUrl);
      const contract = transform(initialSource, initialUrl);
      if (!contract.ok) return { ok: false, reason: contract.diagnostics?.reason || 'quota-contract-failed' };
      const primaryUrls = new Set(
        [...initialSource.matchAll(/["'`]\.\/app-primary-[A-Za-z0-9_-]+\.js["'`]/g)]
          .map(match => new URL(match[0].slice(1, -1), initialUrl).href),
      );
      if (primaryUrls.size !== 1) return { ok: false, reason: 'primary-module-not-unique' };
      const primaryUrl = [...primaryUrls][0];
      const primary = transformComposer(
        await read(primaryUrl), primaryUrl, initialUrl, contract.diagnostics?.bindings,
      );
      if (primary.ok && contract.diagnostics?.bindings?.layout === 'adapter-v1') {
        const adapterUrl = new URL(contract.diagnostics.bindings.adapterPath, initialUrl).href;
        const adapter = transformAdapter(await read(adapterUrl), adapterUrl, initialUrl,
          primaryUrl, primary.diagnostics?.bindings);
        if (!adapter.ok) return { ok: false, reason: adapter.diagnostics?.reason || 'quota-adapter-contract-failed' };
        // The existing adapter has no thread-provider selector import. An
        // unscoped breakpoint would bypass official threads, and rewriting
        // usage cache would falsify real quota. Wait for a complete preload.
        return { ok: false, reason: 'quota-restart-required' };
      }
      if (!primary.ok || !primary.diagnostics?.location)
        return { ok: false, reason: primary.diagnostics?.reason || 'quota-location-unavailable' };
      return { ok: true, plan: primary.diagnostics.location };
    } catch {
      return { ok: false, reason: 'quota-location-failed' };
    }
  }

  const functionRange = (source, start) => root.__cockpitNativeFast?.functionRange(source, start);
  function transformSharedBanner(source, hideMethod = 'shouldHideBanner') {
    // Both legacy and unified announcement paths call this initial-module
    // renderer. Match its props and native fallback, never translated text.
    const matches = [...source.matchAll(new RegExp(
      `function ${identifier}\\(${identifier}\\)\\{let [\\s\\S]{0,90}?,\\{banner:(?<banner>${identifier}),fallbackContent:(?<fallback>${identifier}),lastImpressionKeyRef:${identifier},onDismiss:${identifier},onShown:${identifier},onOwnerRequestComplete:${identifier},presentationKey:${identifier},rateLimitStatus:(?<usage>${identifier}),`, 'g'))];
    if (matches.length !== 1) return null;
    const match = matches[0], range = functionRange(source, match.index);
    if (!range) return null;
    const component = source.slice(range.start, range.end);
    const react = component.match(new RegExp(`\\(0,(${identifier})\\.useRef\\)\\(null\\)`))?.[1];
    if (!react || !['banner_type===`luna_reserve`', 'banner_type===`image_generation_limit_reached`', 'fallbackContent:']
      .every(value => component.includes(value))) return null;
    const { banner, usage, fallback } = match.groups;
    // Every original hook still runs. Preserve image/model limits and the
    // native lower-priority content; quota snapshots are never modified.
    const modelGuard = hideMethod === 'shouldHideCurrentBanner' ? '' : `${banner}.model_slug==null&&${banner}.blocked_model_slug==null&&`;
    const inserted = `let __cockpitSharedScope=globalThis.__cockpitQuotaCompat.useBannerScope(${react});if(${modelGuard}globalThis.__cockpitQuotaCompat.${hideMethod}(__cockpitSharedScope,__cockpitSharedScope?.host,{...${usage},rate_limit_upsell:${banner}},__cockpitSharedScope?.imageLimit))return ${fallback}??null;`;
    return source.slice(0, range.returnAt) + inserted + source.slice(range.returnAt);
  }

  function transformBanner(source, composerName, host, settings) {
    const escape = value => value.replace(/[$]/g, '\\$&');
    const start = source.indexOf(`function ${composerName}(`), range = functionRange(source, start);
    if (!range) return null;
    const composer = source.slice(start, range.end);
    const props = composer.match(new RegExp(`^function ${escape(composerName)}\\((${identifier})\\)`))?.[1];
    const react = composer.match(new RegExp(`\\(0,(${identifier})\\.useRef\\)\\(`))?.[1];
    const jsx = composer.match(new RegExp(`\\(0,(${identifier})\\.jsxs?\\)\\(`))?.[1];
    const scopes = [...new Set([...composer.matchAll(new RegExp(`(${identifier})\\.value\\.kind===\`new\``, 'g'))].map(match => match[1]))];
    const returnAt = range.returnAt - start, closing = composer.length - 1;
    if (!props || !react || !jsx || scopes.length !== 1 || returnAt < 0
      || composer.slice(closing + 1).trim() || !composer.includes('useAsbDirect:')) return null;
    // The native return can contain a memo-cache comma expression. Keep its
    // complete evaluation inside the Provider's children argument.
    const originalReturn = composer.slice(returnAt + 7, closing);
    const imageLimits = [...composer.matchAll(new RegExp(`canShowUsageBanners:${identifier},workCreditConversationId:${identifier},imageGenerationLimit:(${identifier}),onDismissImageGenerationLimit:`, 'g'))];
    if (imageLimits.length > 1) return null;
    const imageLimit = imageLimits[0]?.[1] ?? 'null';
    const scope = scopes[0];
    const eligible = `!${props}.useAsbDirect&&${props}.selectedProject?.type!==\`remote\`&&${props}.homeRunLocationRemoteProject==null&&(${scope}.value.kind===\`local\`&&(${scope}.value.conversationId==null||typeof __cockpitQuotaProvider===\`string\`&&__cockpitQuotaProvider.length>0)||${scope}.value.kind===\`new\`&&!${scope}.value.cloudThreadPrototype)`;
    const wrapped = composer.slice(0, returnAt) + `return globalThis.__cockpitQuotaCompat.wrapComposer(${react},${jsx}.jsx,${host},${settings},__cockpitQuotaProvider,${eligible},(${originalReturn}),${imageLimit})` + composer.slice(closing);
    let patched = source.slice(0, start) + wrapped + source.slice(start + composer.length);

    // Match the native usage presenter, preserving image limits, enterprise
    // warnings, safety fallback content and every original hook invocation.
    const matches = [...patched.matchAll(new RegExp(
      `function (?<name>${identifier})\\(${identifier}\\)\\{let [\\s\\S]{0,120}?,\\{canShowUsageBanners:(?<enabled>${identifier}),hostId:${identifier},imageGenerationLimit:(?<image>${identifier}),onDismissImageGenerationLimit:${identifier},rateLimit:(?<usage>${identifier}),rateLimitWarningThreshold:${identifier},lowerPriorityContent:(?<fallback>${identifier}),`, 'g'))];
    if (matches.length !== 1) return null;
    const match = matches[0], bannerEnd = patched.indexOf('function ', match.index + 10);
    const banner = patched.slice(match.index, bannerEnd < 0 ? patched.length : bannerEnd);
    const bannerReact = banner.match(new RegExp(`\\(0,(${identifier})\\.useRef\\)\\(null\\)`))?.[1];
    // The host is the argument of the existing native Gg hook, not its result.
    const gate = new RegExp(`\\$?${identifier}\\(${identifier},(?<host>${identifier})\\);(?<gate>if\\(!${escape(match.groups.enabled)}\\)return ${escape(match.groups.fallback)};)`).exec(banner);
    if (!bannerReact || !gate || !banner.includes('`image_generation_limit_reached`')
      || !banner.includes('rateLimitStatus:') || !banner.includes('sendBlocked:')) return null;
    const { usage, image, fallback } = match.groups;
    const inserted = `let __cockpitBannerScope=globalThis.__cockpitQuotaCompat.useBannerScope(${bannerReact});if(globalThis.__cockpitQuotaCompat.shouldHideBanner(__cockpitBannerScope,${gate.groups.host},${usage},${image}))return ${fallback};`;
    patched = patched.slice(0, match.index + gate.index + gate[0].length - gate.groups.gate.length)
      + inserted + patched.slice(match.index + gate.index + gate[0].length - gate.groups.gate.length);
    return patched;
  }

  // Validate both initial-module exports consumed by the local composer. Keep
  // the quota atoms and server/cache snapshots intact, including reserve state.
  function transform(source, url) {
    const failed = reason => ({ ok: false, patchedSource: source, diagnostics: { reason } });
    if (typeof source !== 'string' || source.includes('__cockpitQuotaCompat.initialReady')) {
      return failed('unsupported-source');
    }
    try {
      const parsed = new URL(url);
      if (!((parsed.protocol === 'app:' && parsed.hostname === '-')
        || (parsed.protocol === 'file:' && !parsed.hostname))) return failed('unsupported-url');
    } catch { return failed('unsupported-url'); }
    const matches = [...source.matchAll(selector)];
    if (matches.length !== 1) return failed('quota-selector-not-unique');
    const match = matches[0];
    const { family, name } = match.groups;
    const context = source.slice(Math.max(0, match.index - 6000), match.index);
    const familyStart = context.lastIndexOf(`${family}=`);
    const familySource = familyStart < 0 ? '' : context.slice(familyStart);
    if (!familySource.includes('hardBlocked:')
      || !familySource.includes('rate_limit?.allowed')
      || !(familySource.includes('`chatgpt`')
        || (familySource.includes('authenticatedAccountId') && familySource.includes('user_id')))) {
      return failed('quota-contract-mismatch');
    }
    const providers = [...source.matchAll(new RegExp(
      `(?<name>${identifier})=${identifier}\\(\\(\\{summary:(?<summary>${identifier})\\}\\)=>\\k<summary>\\.modelProvider\\|\\|null\\)`, 'g'))];
    if (providers.length !== 1) return failed('quota-provider-not-unique');
    const exports = source.slice(source.lastIndexOf('export{'));
    const exported = name => {
      const escaped = name.replace(/[$]/g, '\\$&');
      return exports.match(new RegExp(`(?:\\{|,)${escaped} as (${identifier})(?=,|\\})`))?.[1];
    };
    const hardBlocked = exported(name), threadProvider = exported(providers[0].groups.name);
    if (!hardBlocked || !threadProvider) return failed('quota-exports-missing');
    const withBanner = transformSharedBanner(source);
    if (!withBanner) return root.__cockpitQuotaCurrent?.initial(source, url, { hardBlocked, threadProvider })
      ?? failed('quota-shared-banner-contract-mismatch');
    return {
      ok: true,
      patchedSource: `${withBanner}\n;globalThis.__cockpitQuotaCompat.initialReady=true;\n`,
      diagnostics: { reason: 'ready', bindings: { hardBlocked, threadProvider } },
    };
  }

  function transformComposer(source, url, initialUrl, bindings) {
    const failed = reason => ({ ok: false, patchedSource: source, diagnostics: { reason } });
    if (typeof source !== 'string' || source.includes('__cockpitQuotaCompat.enabledFor')) return failed('unsupported-source');
    try {
      const parsed = new URL(url), initial = new URL(initialUrl);
      if (parsed.protocol !== initial.protocol || parsed.host !== initial.host
        || !/\/assets\/app-primary-[A-Za-z0-9_-]+\.js$/.test(parsed.pathname)
        || parsed.search || parsed.hash || parsed.username || parsed.password
        || !((parsed.protocol === 'app:' && parsed.hostname === '-')
          || (parsed.protocol === 'file:' && !parsed.hostname))) return failed('unsupported-url');
    } catch { return failed('unsupported-url'); }
    if (bindings?.layout === 'adapter-v1') return root.__cockpitQuotaCurrent?.primary(source, url, initialUrl, bindings)
      ?? failed('quota-current-runtime-missing');
    const imports = [...source.matchAll(/import\{([^{}]+)\}from(["'`])([^"'`]+)\2/g)]
      .filter(match => { try { return new URL(match[3], url).href === initialUrl; } catch { return false; } });
    if (imports.length !== 1) return failed('quota-import-not-unique');
    const imported = name => typeof name === 'string'
      ? imports[0][1].match(new RegExp(`(?:^|,)${name.replace(/[$]/g, '\\$&')} as (${identifier})(?=,|$)`))?.[1] : null;
    const hardAtom = imported(bindings?.hardBlocked), providerAtom = imported(bindings?.threadProvider);
    if (!hardAtom || !providerAtom) return failed('quota-imports-missing');
    const reads = [...source.matchAll(new RegExp(
      `(?<hard>${identifier})=(?<read>${identifier})\\(${hardAtom.replace(/[$]/g, '\\$&')},(?<host>${identifier})\\)`, 'g'))];
    if (reads.length !== 1) return failed('quota-composer-not-unique');
    const match = reads[0], { hard, read, host } = match.groups;
    const start = source.lastIndexOf('function ', match.index);
    const range = functionRange(source, start);
    const composer = range ? source.slice(start, range.end) : '';
    if (start < 0 || !composer.includes('onSubmitLocal:') || !composer.includes('submitDisabled:')
      || !composer.includes('"data-codex-composer-root"')) return failed('quota-composer-contract-mismatch');
    const escape = value => value.replace(/[$]/g, '\\$&');
    const thread = composer.match(new RegExp(`${escape(host)}=${escape(read)}\\(${identifier},(?<id>${identifier})\\)`));
    const configs = [...composer.matchAll(new RegExp(`(?<config>${identifier})\\.isConfigDataPending\\?null:\\k<config>\\.resolvedConfig`, 'g'))];
    const guards = [...composer.matchAll(new RegExp(`(?<blocked>${identifier})=${identifier}\\(${escape(host)}===\`local\`&&(?<atom>${identifier})\\)`, 'g'))];
    if (!thread || configs.length !== 1 || guards.length !== 1) return failed('quota-send-contract-mismatch');
    const guard = guards[0].groups;
    const guardDefinition = new RegExp(`${escape(guard.atom)}=${identifier}\\(${identifier},\\(\\{get:${identifier}\\}\\)=>\\{`).exec(source);
    const guardSource = guardDefinition ? source.slice(guardDefinition.index, guardDefinition.index + 1500) : '';
    if (!['authMethod!==`chatgpt`', 'authenticatedAccountId', 'rate_limit?.allowed!==!1', 'remainingPercent:', 'windowMinutes:', 'resetsAt:']
      .every(value => guardSource.includes(value))) return failed('quota-send-contract-mismatch');
    const tail = new RegExp(`\\|\\|${escape(hard)}\\|\\|${escape(guard.blocked)}(?=,${identifier};)`, 'g');
    const tails = [...composer.matchAll(tail)];
    if (tails.length !== 1) return failed('quota-submit-not-unique');
    const submit = composer.slice(composer.lastIndexOf('let ', tails[0].index), tails[0].index)
      .match(new RegExp(`^let (${identifier})=`))?.[1];
    if (!submit || !composer.includes(`submitDisabled:${submit}`)) return failed('quota-submit-contract-mismatch');
    const settings = configs[0].groups.config;
    const replacement = `||((${hard}||${guard.blocked})&&!globalThis.__cockpitQuotaCompat.enabledFor(${host},${settings},__cockpitQuotaProvider))`;
    // The provider read is a hook: always run it, before the submit expression
    // can short-circuit on an unrelated disabled reason.
    const providerRead = `,__cockpitQuotaProvider=${read}(${providerAtom},${thread.groups.id})`;
    const patched = source.slice(0, start + tails[0].index)
      + replacement
      + source.slice(start + tails[0].index + tails[0][0].length);
    const withProvider = patched.slice(0, match.index + match[0].length)
      + providerRead + patched.slice(match.index + match[0].length);
    const composerName = composer.match(new RegExp(`^function (${identifier})\\(`))?.[1];
    const withBanner = composerName && transformBanner(withProvider, composerName, host, settings);
    if (!withBanner) return failed('quota-banner-contract-mismatch');
    let anchor = start + guards[0].index + guards[0][0].length;
    while (anchor < source.length && /[\s,;]/.test(source[anchor])) anchor++;
    const before = source.slice(0, anchor);
    return {
      ok: true,
      patchedSource: withBanner,
      diagnostics: {
        reason: 'ready',
        location: {
          url,
          lineNumber: before.split('\n').length - 1,
          columnNumber: anchor - before.lastIndexOf('\n') - 1,
          hardVariable: hard,
          guardVariable: guard.blocked,
          hostVariable: host,
        },
      },
    };
  }

  function enabledForCurrent(host, settings, threadProvider) {
    if (enabledFor(host, settings, threadProvider)) return true;
    const mode = root.__cockpitEnhancements;
    if (host !== 'local' || mode?.apiService !== true || mode.apiKeyAuthVerified !== true
      || settings?.isConfigDataPending !== false) return false;
    const config = settings.resolvedConfig, providerId = threadProvider ?? config?.model_provider ?? 'openai';
    const provider = config?.model_providers?.[providerId];
    if (providerId === 'openai' || provider?.requires_openai_auth !== false
      || typeof provider.experimental_bearer_token !== 'string' || !provider.experimental_bearer_token.trim()) return false;
    try {
      const url = new URL(provider.base_url);
      return ['http:', 'https:'].includes(url.protocol) && !!url.hostname
        && !url.username && !url.password && !url.search && !url.hash;
    } catch { return false; }
  }
  function wrapCurrentComposer(react, jsx, host, settings, provider, eligible, children) {
    return jsx(contextFor(react).Provider, {
      value: { host, compatible: eligible && enabledForCurrent(host, settings, provider), imageLimit: null }, children,
    });
  }
  function enabledForComposer(host, settings, provider, props, scope) {
    const route = scope?.value;
    const eligible = props?.useAsbDirect !== true && props?.selectedProject?.type !== 'remote'
      && props?.homeRunLocationRemoteProject == null
      && (route?.kind === 'local' && (route.conversationId == null || typeof provider === 'string' && provider.length > 0)
        || route?.kind === 'new' && !route.cloudThreadPrototype);
    return eligible && enabledForCurrent(host, settings, provider);
  }
  function shouldHideCurrentBanner(scope, host, usage, imageLimit) {
    const mode = root.__cockpitEnhancements;
    // Both mixed OAuth/API and verified API-only routes have their own quota.
    // An instance flag alone cannot authorize hiding an official thread's
    // notice. Keep unknown/global scope and image/safety notices native.
    return mode?.nativePatchState === 'active'
      && (mode.quotaCompat === true || mode.apiService === true && mode.apiKeyAuthVerified === true)
      && scope?.compatible === true && host === 'local' && scope.host === host
      && imageLimit == null && usageBannerTypes.has(usage?.rate_limit_upsell?.banner_type)
      && (usage?.rate_limit?.allowed === false
        || typeof usage.rate_limit_upsell.model_slug === 'string' && usage.rate_limit_upsell.model_slug.length > 0
        || typeof usage.rate_limit_upsell.blocked_model_slug === 'string' && usage.rate_limit_upsell.blocked_model_slug.length > 0);
  }
  function transformAdapter(source, url, initialUrl, primaryUrl, bindings) {
    return root.__cockpitQuotaCurrent?.adapter(source, url, initialUrl, primaryUrl, bindings)
      ?? { ok: false, patchedSource: source, diagnostics: { reason: 'quota-current-runtime-missing' } };
  }
  const api = { enabledFor, enabledForCurrent, enabledForComposer, wrapComposer, wrapCurrentComposer, useBannerScope, shouldHideBanner,
    shouldHideCurrentBanner, transformSharedBanner, transformAdapter,
    getBannerContext: () => bannerContext, transform, transformComposer,
    prepareLiveQuotaGate, installLiveQuotaGuard, disposeLiveQuotaGuard };
  root.__cockpitQuotaCompat = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})();
