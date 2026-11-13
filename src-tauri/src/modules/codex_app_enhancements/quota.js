(() => {
  const root = typeof window === 'object' ? window : globalThis;
  const identifier = '[A-Za-z_$][\\w$]*';
  const selector = new RegExp(
    `(?<name>${identifier})=${identifier}\\(${identifier},\\((?<host>${identifier}),\\{get:(?<get>${identifier})\\}\\)=>(?<expression>\\k<get>\\((?<family>${identifier}),\\k<host>\\)\\.hardBlocked)\\)`,
    'g',
  );

  function enabledFor(host, settings, threadProvider) {
    if (root.__cockpitEnhancements?.apiService === true) return false;
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
    // Only non-service mixed OAuth/API routes use composer compatibility.
    // API Service instances instead adapt the incoming quota data.
    return mode?.nativePatchState === 'active'
      && mode?.quotaCompat === true && host === 'local' && scope?.host === host && scope.compatible === true
      && usage?.rate_limit?.allowed === false
      && imageLimit == null
      && usage?.rate_limit_upsell?.model_slug == null && usage?.rate_limit_upsell?.blocked_model_slug == null
      && usageBannerTypes.has(usage?.rate_limit_upsell?.banner_type);
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
    return enabledFor(host, settings, threadProvider);
  }
  function wrapCurrentComposer(react, jsx, host, settings, provider, eligible, children) {
    return jsx(contextFor(react).Provider, {
      value: { host, compatible: eligible && enabledForCurrent(host, settings, provider), imageLimit: null }, children,
    });
  }
  function enabledForComposer(host, settings, provider, props, scope) {
    if (root.__cockpitEnhancements?.apiService === true) return false;
    const route = scope?.value;
    const eligible = props?.useAsbDirect !== true && props?.selectedProject?.type !== 'remote'
      && props?.homeRunLocationRemoteProject == null
      && (route?.kind === 'local' && (route.conversationId == null || typeof provider === 'string' && provider.length > 0)
        || route?.kind === 'new' && !route.cloudThreadPrototype);
    return eligible && enabledForCurrent(host, settings, provider);
  }
  function shouldHideCurrentBanner(scope, host, usage, imageLimit) {
    const mode = root.__cockpitEnhancements;
    // Other mixed OAuth/API routes still use their verified composer scope.
    // Keep unknown/global scope and image/safety notices native for them.
    return mode?.nativePatchState === 'active'
      && mode.quotaCompat === true
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
    };
  root.__cockpitQuotaCompat = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})();
