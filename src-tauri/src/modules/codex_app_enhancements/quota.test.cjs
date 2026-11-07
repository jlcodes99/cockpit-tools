const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

const initialUrl = 'app://-/assets/app-initial-fixture.js';
const primaryUrl = 'app://-/assets/app-primary-fixture.js';
const script = fs.readFileSync(path.join(__dirname, 'quota.js'), 'utf8');
const fastScript = fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8');
const sharedBanner = `function SharedUsage(props){let memo=0,{banner:banner,fallbackContent:fallback,lastImpressionKeyRef:ref,onDismiss:dismiss,onShown:shown,onOwnerRequestComplete:owner,presentationKey:key,rateLimitStatus:usage,renderContent:render}=props;
let nativeRef=(0,React.useRef)(null);let special=banner.banner_type===\`luna_reserve\`||banner.banner_type===\`image_generation_limit_reached\`;
return (0,JSX.jsx)("quota-banner",{banner,special,fallbackContent:fallback})}`;
// The reserve selector and the generic OAuth guard are independent in the
// desktop artifact. Exhaustion can block send with reserve hardBlocked=false.
const initial = `
var family,hardSelector,threadProvider;
function initialize(){family=familyAtom(store,(host,{get:read})=>{let usage=read(status).data;
let identity=read(account);if(usage.user_id!==identity.userId||usage.account_id!==identity.authenticatedAccountId)return{hardBlocked:false};
return{hardBlocked:usage.rate_limit?.allowed===!1}}),hardSelector=familyAtom(store,(host,{get:read})=>read(family,host).hardBlocked);
threadProvider=atom(({summary:item})=>item.modelProvider||null)}
${sharedBanner}
export{hardSelector as reserveExport,threadProvider as providerExport};
`;
const primary = `
import{reserveExport as reserveAtom,providerExport as providerAtom}from"./app-initial-fixture.js";
var quotaGuard=atom(store,({get:read})=>{let account=read(identity),usage=read(status).data;
if(account.authMethod!==\`chatgpt\`||account.authenticatedAccountId==null||usage.rate_limit?.allowed!==!1)return!1;
let windows=usageWindows(usage).filter(({remainingPercent:p})=>p===0);
return windows.every(({windowMinutes:w,resetsAt:r})=>w>0&&r>0)});
function UsageBanner(props){let memo=0,{canShowUsageBanners:enabled,hostId:hostProp,imageGenerationLimit:image,onDismissImageGenerationLimit:dismiss,rateLimit:usage,rateLimitWarningThreshold:threshold,lowerPriorityContent:fallback,sendBlocked:blocked}=props;
let host=hostProp===void 0?\`local\`:hostProp,ref=(0,React.useRef)(null),nativeNotice=useFamily(nativeNoticeAtom,host);if(!enabled)return fallback;
let imageType=usage?.rate_limit_upsell?.banner_type===\`image_generation_limit_reached\`;
return(0,JSX.jsx)("quota-banner",{rateLimitStatus:usage,sendBlocked:blocked,imageType,fallback})}
function Composer(props){let{onSubmitLocal:send,submitDisabled:other,useAsbDirect:direct}=props;
let scope=readScope(),isNew=scope.value.kind===\`new\`,ref=(0,React.useRef)(null);
let threadId=props.threadId,host=useFamily(hostAtom,threadId),reserve=useFamily(reserveAtom,host);
let settings=props.settings,saved=settings.isConfigDataPending?null:settings.resolvedConfig;
let generic=useValue(host===\`local\`&&quotaGuard);
let disabled=other||reserve||generic,next;
function nested(){return "nested return stays untouched"}
return (0,JSX.jsx)("composer",{submitDisabled:disabled,onSubmitLocal:send,saved,"data-codex-composer-root":""})}
`;

function load() {
  const context = { URL, module: { exports: {} }, __cockpitEnhancements: { quotaCompat: true } };
  context.window = context; context.globalThis = context;
  vm.runInNewContext(fastScript, context);
  vm.runInNewContext(script, context);
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'quota_current.js'), 'utf8'), context);
  return { api: context.module.exports, context };
}

function mixedSettings() {
  return { isConfigDataPending: false, resolvedConfig: { model_provider: 'relay', model_providers: {
    relay: { requires_openai_auth: true, experimental_bearer_token: 'independent-api-key', base_url: 'http://localhost:58554/v1' },
  } } };
}

function composerHarness(source = primary) {
  const { api, context } = load();
  const contract = api.transform(initial, initialUrl);
  assert.equal(contract.ok, true);
  const transformed = api.transformComposer(source, primaryUrl, initialUrl, contract.diagnostics.bindings);
  assert.equal(transformed.ok, true, JSON.stringify(transformed.diagnostics));
  const values = { host: 'local', reserve: false, generic: true, provider: null };
  const reads = [];
  Object.assign(context, { store: {}, identity: {}, status: {}, atom: () => ({}), hostAtom: 'host', reserveAtom: 'reserve', providerAtom: 'provider',
    nativeNoticeAtom: 'nativeNotice', readScope: () => ({ value: { kind: 'local' } }),
    React: { useRef: () => ({}), createContext: () => ({ Provider: 'provider' }), useContext: () => null },
    JSX: { jsx: (_type, props) => props.children ?? props },
    useFamily(atom) { reads.push(atom); return values[atom]; }, useValue: () => values.generic });
  vm.runInNewContext(transformed.patchedSource.replace(/^import[^;]+;/m, ''), context);
  return { api, context, values, reads, transformed,
    render: overrides => context.Composer({ settings: mixedSettings(), submitDisabled: false, ...overrides }) };
}

test('the original composer blocks an independent API on generic OAuth exhaustion', () => {
  const h = composerHarness();
  const original = primary.replace(/^import[^;]+;/m, '');
  vm.runInNewContext(original, h.context);
  assert.equal(h.render().submitDisabled, true);
  vm.runInNewContext(h.transformed.patchedSource.replace(/^import[^;]+;/m, ''), h.context);
  assert.equal(h.render().submitDisabled, false);
});

test('local independent API handles either quota guard without changing any server data', () => {
  const h = composerHarness();
  const settings = mixedSettings(), snapshot = JSON.stringify(settings);
  for (const reserve of [false, true]) for (const generic of [false, true]) {
    Object.assign(h.values, { reserve, generic });
    assert.equal(h.render({ settings }).submitDisabled, false);
    assert.equal(h.render({ settings, submitDisabled: true }).submitDisabled, true, 'unrelated disabling remains effective');
  }
  assert.equal(JSON.stringify(settings), snapshot);
  const contract = h.api.transform(initial, initialUrl);
  assert.ok(contract.patchedSource.startsWith(initial.slice(0, initial.indexOf(sharedBanner))), 'original quota selectors must remain untouched');
});

test('official, remote, pending, missing and disabled configurations retain quota blocking', () => {
  const h = composerHarness();
  for (const settings of [null, { isConfigDataPending: false }, { ...mixedSettings(), isConfigDataPending: true },
    { isConfigDataPending: false, resolvedConfig: { model_provider: 'openai' } }]) {
    // The desktop uses a settings object even while its config is loading.
    assert.equal(h.api.enabledFor('local', settings, null), false);
    if (settings) assert.equal(h.render({ settings }).submitDisabled, true);
  }
  h.values.host = 'remote-machine';
  assert.equal(h.render().submitDisabled, true);
  h.values.host = 'local'; h.context.__cockpitEnhancements.quotaCompat = false;
  assert.equal(h.render().submitDisabled, true);
});

test('the actual thread provider takes precedence over the profile default', () => {
  const h = composerHarness();
  h.values.provider = 'openai';
  assert.equal(h.render().submitDisabled, true);
  h.values.provider = 'unknown';
  assert.equal(h.render().submitDisabled, true);
  h.values.provider = 'relay';
  assert.equal(h.render().submitDisabled, false);
  const settings = mixedSettings();
  settings.resolvedConfig.model_provider = 'openai';
  assert.equal(h.render({ settings }).submitDisabled, false, 'an existing API thread retains its own provider');
});

test('provider hook runs on every render even when an unrelated disabled guard is true', () => {
  const h = composerHarness();
  h.render({ submitDisabled: true }); h.render({ submitDisabled: false });
  assert.deepEqual(h.reads, ['host', 'reserve', 'provider', 'host', 'reserve', 'provider']);
});

test('nested declarations and lexical braces cannot move the composer Provider inside a callback', () => {
  const source = primary.replace('function nested(){return "nested return stays untouched"}',
    'function nested(){let text="}",regex=/[{}]/,template=`literal } ${JSON.stringify({open:"{"})}`;return text+template+regex.source}');
  const h = composerHarness(source);
  assert.equal(h.render().submitDisabled, false);
  const ast = require('@babel/parser').parse(h.transformed.patchedSource, { sourceType: 'module' });
  const composer = ast.program.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === 'Composer');
  const returned = composer.body.body.findLast(node => node.type === 'ReturnStatement');
  assert.equal(returned.argument.callee.property.name, 'wrapComposer');
  assert.ok(h.transformed.patchedSource.includes('return text+template+regex.source}'));
});

function bannerHarness() {
  const h = composerHarness(), React = require('react'), { renderToStaticMarkup } = require('react-dom/server');
  const usage = { rate_limit: { allowed: false }, rate_limit_upsell: { banner_type: 'pro_rate_limit_reached' } };
  const bannerProps = { canShowUsageBanners: true, hostId: 'local', rateLimit: usage,
    imageGenerationLimit: null, lowerPriorityContent: null };
  let scope = { kind: 'local' };
  Object.assign(h.context, { React, readScope: () => ({ value: scope }), JSX: { jsx(type, props) {
    if (type === 'composer') return React.createElement('main', null, React.createElement(h.context.UsageBanner, { ...bannerProps }));
    if (type === 'quota-banner') return React.createElement('aside', { 'data-oauth-banner': '' }, 'OAuth quota exhausted');
    return React.createElement(type, props);
  } } });
  h.context.__cockpitEnhancements.nativePatchState = 'active';
  return { ...h, React, usage, bannerProps, setScope: value => { scope = value; },
    renderBanner: overrides => renderToStaticMarkup(React.createElement(h.context.Composer, { settings: mixedSettings(), ...overrides })),
    renderOutside: () => renderToStaticMarkup(React.createElement(h.context.UsageBanner, bannerProps)),
    renderSiblings: () => renderToStaticMarkup(React.createElement(React.Fragment, null,
      React.createElement(h.context.Composer, { settings: mixedSettings() }),
      React.createElement(h.context.Composer, { settings: { isConfigDataPending: false, resolvedConfig: { model_provider: 'openai' } } }))),
  };
}

test('only an active independent API composer suppresses the exhausted OAuth banner and its layout slot', () => {
  const h = bannerHarness(), snapshot = JSON.stringify(h.usage);
  assert.equal(h.renderBanner(), '<main></main>');
  assert.match(h.renderOutside(), /data-oauth-banner/, 'no composer route must mean no suppression');
  for (const state of ['pending', 'preloading', 'unsupported']) {
    h.context.__cockpitEnhancements.nativePatchState = state;
    assert.match(h.renderBanner(), /data-oauth-banner/);
  }
  h.context.__cockpitEnhancements.nativePatchState = 'active';
  assert.equal(h.renderBanner(), '<main></main>');
  assert.equal(JSON.stringify(h.usage), snapshot, 'retain actual quota and banner data');
});

test('OAuth, remote, pending config, direct official and cloud routes restore the native banner', () => {
  const h = bannerHarness();
  h.values.provider = 'openai'; assert.match(h.renderBanner(), /data-oauth-banner/);
  h.values.provider = 'relay'; assert.equal(h.renderBanner(), '<main></main>');
  h.values.host = 'remote'; assert.match(h.renderBanner(), /data-oauth-banner/);
  h.values.host = 'local';
  assert.match(h.renderBanner({ settings: { ...mixedSettings(), isConfigDataPending: true } }), /data-oauth-banner/);
  assert.match(h.renderBanner({ useAsbDirect: true }), /data-oauth-banner/);
  assert.match(h.renderBanner({ selectedProject: { type: 'remote' } }), /data-oauth-banner/);
  assert.match(h.renderBanner({ homeRunLocationRemoteProject: {} }), /data-oauth-banner/);
  h.setScope({ kind: 'local', conversationId: 'existing-thread' });
  h.values.provider = null; assert.match(h.renderBanner(), /data-oauth-banner/, 'existing thread provider is not yet confirmed');
  h.values.provider = 'relay'; assert.equal(h.renderBanner(), '<main></main>');
  for (const scope of [{ kind: 'chatgpt' }, { kind: 'cloud' }, { kind: 'new', cloudThreadPrototype: {} }]) {
    h.setScope(scope); assert.match(h.renderBanner(), /data-oauth-banner/);
  }
  h.setScope({ kind: 'new' }); assert.equal(h.renderBanner(), '<main></main>');
  h.context.__cockpitEnhancements.quotaCompat = false;
  assert.match(h.renderBanner(), /data-oauth-banner/);
});

test('API Service instances keep the subscription notice off from first render regardless of project config or thread provider', () => {
  const h = bannerHarness();
  h.context.__cockpitEnhancements.apiService = true;
  for (const state of ['pending', 'preloading', 'active', 'unsupported']) {
    h.context.__cockpitEnhancements.nativePatchState = state;
    for (const config of [{ isConfigDataPending: true }, { isConfigDataPending: false, resolvedConfig: { model_provider: 'openai' } }, mixedSettings()]) {
      for (const provider of [null, 'openai', 'relay', 'unknown']) {
        h.values.provider = provider;
        assert.equal(h.renderBanner({ settings: config }), '<main></main>');
      }
    }
  }
  h.context.__cockpitEnhancements.quotaCompat = false;
  assert.equal(h.renderBanner(), '<main></main>', 'instance binding does not depend on the mixed-mode setting');
  assert.equal(h.renderOutside(), '', 'an API Service window does not wait for a composer route scope');
  h.context.__cockpitEnhancements.apiService = false;
  assert.match(h.renderBanner(), /data-oauth-banner/, 'returning to an ordinary account restores the notice immediately');
});

test('API Service visibility never rewrites quota data, removes safety content or suppresses model/image notices', () => {
  const h = bannerHarness();
  h.context.__cockpitEnhancements.apiService = true;
  const snapshot = JSON.stringify(h.usage);
  assert.equal(h.renderBanner(), '<main></main>');
  assert.equal(JSON.stringify(h.usage), snapshot);
  for (const type of ['image_generation_limit_reached', 'selected_model_limit_reached', 'luna_reserve', 'unknown']) {
    h.usage.rate_limit_upsell.banner_type = type;
    assert.match(h.renderBanner(), /data-oauth-banner/);
  }
  h.usage.rate_limit_upsell.banner_type = 'pro_rate_limit_reached';
  h.bannerProps.lowerPriorityContent = h.React.createElement('p', null, 'Safety notice');
  assert.equal(h.renderBanner(), '<main><p>Safety notice</p></main>');
  h.values.host = 'remote';
  assert.equal(h.renderBanner(), '<main><p>Safety notice</p></main>', 'the policy belongs to the API Service window, not its current project host');
  h.context.__cockpitEnhancements.apiService = false;
  assert.match(h.renderBanner(), /data-oauth-banner/);
});

test('image limits, model limits, warnings, unknown banners and safety fallback remain visible', () => {
  const h = bannerHarness();
  for (const type of ['image_generation_limit_reached', 'selected_model_limit_reached', 'luna_reserve', 'new_unknown_limit']) {
    h.usage.rate_limit_upsell.banner_type = type;
    assert.match(h.renderBanner(), /data-oauth-banner/);
  }
  h.usage.rate_limit_upsell.banner_type = 'pro_rate_limit_reached';
  for (const key of ['model_slug', 'blocked_model_slug']) {
    h.usage.rate_limit_upsell[key] = 'limited-model';
    assert.match(h.renderBanner(), /data-oauth-banner/);
    delete h.usage.rate_limit_upsell[key];
  }
  h.usage.rate_limit.allowed = true; assert.match(h.renderBanner(), /data-oauth-banner/);
  h.usage.rate_limit.allowed = false;
  h.bannerProps.imageGenerationLimit = { resetsAt: 123 };
  assert.match(h.renderBanner(), /data-oauth-banner/);
  h.bannerProps.imageGenerationLimit = null;
  h.bannerProps.lowerPriorityContent = h.React.createElement('p', null, 'Safety warning');
  assert.equal(h.renderBanner(), '<main><p>Safety warning</p></main>');
});

test('independent sibling composers cannot hide an official conversation banner', () => {
  const h = bannerHarness(), html = h.renderSiblings();
  assert.equal((html.match(/data-oauth-banner/g) || []).length, 1);
  assert.match(h.renderOutside(), /data-oauth-banner/);
});

test('host reattachment preserves Provider identity and does not replace the editor subtree', () => {
  const h = bannerHarness();
  assert.equal(h.renderBanner(), '<main></main>');
  const before = h.context.__cockpitQuotaCompat.getBannerContext();
  assert.ok(before);
  vm.runInNewContext(script, h.context);
  assert.equal(h.context.__cockpitQuotaCompat.getBannerContext(), before);
  assert.equal(h.renderBanner(), '<main></main>');
  h.values.provider = 'openai';
  assert.match(h.renderBanner(), /data-oauth-banner/);
});

test('missing independent credentials and malformed endpoints fail closed', () => {
  const { api } = load();
  for (const patch of [{ experimental_bearer_token: '' }, { experimental_bearer_token: null },
    { requires_openai_auth: false }, { base_url: 'file:///tmp/api' }, { base_url: 'https://user:pass@example.test/v1' },
    { base_url: 'http://localhost/v1?token=secret' }, { base_url: 'http://localhost/v1#fragment' }]) {
    const settings = mixedSettings(); Object.assign(settings.resolvedConfig.model_providers.relay, patch);
    assert.equal(api.enabledFor('local', settings), false);
  }
});

test('unknown or ambiguous module contracts are never partially transformed', () => {
  const { api } = load(), bindings = api.transform(initial, initialUrl).diagnostics.bindings;
  for (const source of [primary + primary, primary.replace('submitDisabled:disabled', 'disabled:disabled'),
    primary.replace('rate_limit?.allowed', 'another_limit?.allowed'), primary.replace('reserve||generic', 'generic||reserve'),
    primary.replace('providerExport as providerAtom', 'otherExport as providerAtom')]) {
    const result = api.transformComposer(source, primaryUrl, initialUrl, bindings);
    assert.equal(result.ok, false); assert.equal(result.patchedSource, source);
  }
  for (const source of [primary.replace('canShowUsageBanners:enabled', 'anotherBannerFlag:enabled'),
    primary.replace('if(!enabled)return fallback;', 'if(!enabled)return null;'),
    primary.replace('rateLimitStatus:usage', 'otherStatus:usage')]) {
    const result = api.transformComposer(source, primaryUrl, initialUrl, bindings);
    assert.equal(result.ok, false); assert.equal(result.patchedSource, source, 'unsupported banner cancels the entire patch');
  }
  for (const source of [initial + initial, initial.replace('modelProvider||null', 'unknownProvider||null'),
    initial.replace('hardSelector as reserveExport', 'anotherSelector as reserveExport'),
    initial.replace('onOwnerRequestComplete:owner', 'anotherOwnerCallback:owner')]) {
    assert.equal(api.transform(source, initialUrl).ok, false);
  }
  for (const url of ['https://example.test/assets/app-primary-fixture.js', primaryUrl + '?changed=1']) {
    assert.equal(api.transformComposer(primary, url, initialUrl, bindings).ok, false);
  }
});

test('live quota plan anchors after both submit gates without changing other blocks', () => {
  const h = composerHarness();
  const location = h.transformed.diagnostics.location;
  assert.equal(location.url, primaryUrl);
  assert.equal(location.hardVariable, 'reserve');
  assert.equal(location.guardVariable, 'generic');
  assert.equal(location.hostVariable, 'host');
  const line = primary.split('\n')[location.lineNumber];
  assert.equal(line.slice(location.columnNumber, location.columnNumber + 12), 'let disabled');
});

test('live quota condition updates evaluated V8 locals without pausing or reloading', async () => {
  const h = composerHarness();
  h.context.__cockpitEnhancements.apiService = true;
  const { Session } = require('node:inspector');
  const session = new Session();
  session.connect();
  const post = (method, params = {}) => new Promise((resolve, reject) => {
    session.post(method, params, (error, result) => error ? reject(error) : resolve(result));
  });
  let pauses = 0;
  session.on('Debugger.paused', () => { pauses++; session.post('Debugger.resume'); });
  try {
    await post('Debugger.enable');
    // Preserve offsets while removing the synthetic module import for the VM.
    vm.runInNewContext(primary.replace(/^import[^;]+;/m, match => ' '.repeat(match.length)),
      h.context, { filename: primaryUrl });
    assert.equal(h.render().submitDisabled, true);
    const plan = h.transformed.diagnostics.location;
    const condition = `(window.__cockpitEnhancements?.apiService===true&&window.__cockpitEnhancements?.quotaCompat===true&&(${plan.hardVariable}=false,${plan.guardVariable}=false),false)`;
    const breakpoint = await post('Debugger.setBreakpointByUrl', {
      url: plan.url, lineNumber: plan.lineNumber, columnNumber: plan.columnNumber, condition,
    });
    assert.ok(breakpoint.locations.length > 0, 'the native gate location resolves');
    for (const reserve of [false, true]) for (const generic of [false, true]) {
      Object.assign(h.values, { reserve, generic });
      assert.equal(h.render().submitDisabled, false);
      assert.equal(h.render({ submitDisabled: true }).submitDisabled, true);
    }
    h.context.__cockpitEnhancements.apiService = false;
    assert.equal(h.render().submitDisabled, true, 'revoking the binding restores native gates');
    assert.equal(pauses, 0);
    await post('Debugger.removeBreakpoint', { breakpointId: breakpoint.breakpointId });
    h.context.__cockpitEnhancements.apiService = true;
    assert.equal(h.render().submitDisabled, true, 'removing the condition restores native behavior');
  } finally {
    await post('Debugger.disable');
    session.disconnect();
  }
});

test('live quota install rewrites only the main usage snapshot and refreshes the mounted composer', () => {
  const { api, context } = load();
  Object.assign(context.__cockpitEnhancements, { apiService: true });
  let dispatched = 0;
  const hook = { memoizedState: vm.runInNewContext('new Set()', context),
    queue: { dispatch: () => { dispatched++; } }, next: null };
  const type = function Composer() {};
  type.toString = () => 'function Composer(){let disabled=other||reserve||generic,next;return{submitDisabled:disabled}}';
  const composer = { __reactFiber$test: { type, memoizedState: hook, return: null } };
  const aside = {
    innerText: 'Codex 和工作使用额度已用完 工作空间额度已用完',
    style: { setProperty() {}, removeProperty() {} },
    setAttribute() {}, removeAttribute() {},
  };
  const usage = {
    user_id: 'user',
    rate_limit: { allowed: false, limit_reached: true, primary_window: { used_percent: 100 } },
    rate_limit_reached_type: { type: 'workspace_member_credits_depleted' },
    rate_limit_upsell: { banner_type: 'workspace_member_credits_depleted', model_slug: null },
    credits: { balance: 0 },
  };
  const query = { queryKey: ['rate-limit-status'], state: { data: usage } };
  const imageQuery = { queryKey: ['rate-limit-status', 'image-generation'], state: { data: { rate_limit: { allowed: false } } } };
  let subscribed = 0, unsubscribed = 0, published = usage, cacheListener;
  const client = {
    getQueryCache: () => ({ findAll: () => [query, imageQuery],
      subscribe: listener => { cacheListener = listener; subscribed++; return () => { unsubscribed++; }; } }),
    setQueryData(key, value) {
      const target = key[1] === 'image-generation' ? imageQuery : query;
      target.state.data = typeof value === 'function' ? value(target.state.data) : value;
      if (target === query) published = target.state.data;
    },
  };
  const originalSetter = client.setQueryData;
  Object.assign(context, {
    document: {
      body: { innerText: aside.innerText },
      documentElement: {},
      querySelectorAll(selector) {
        if (selector === '[data-codex-composer-root]') return [composer];
        if (selector === 'aside[role="status"]') return [aside];
        if (selector === '[data-cockpit-hidden-usage="true"]') return [aside];
        return [];
      },
    },
    MutationObserver: class { observe() {} disconnect() {} },
  });
  const finder = { __reactFiber$test: null };
  finder.__reactFiber$test = { child: null, sibling: null, memoizedProps: { client }, return: null };
  context.document.getElementById = () => finder;
  const result = api.installLiveQuotaGuard({
    hardVariable: 'reserve', guardVariable: 'generic', hostVariable: 'host',
  });
  assert.equal(result.ok, true);
  assert.equal(result.client, true);
  assert.equal(published.rate_limit.allowed, true);
  assert.equal(published.rate_limit.limit_reached, false);
  assert.equal(published.rate_limit_upsell, null);
  assert.equal(published.rate_limit_reached_type, null);
  assert.deepEqual(published.credits, { balance: 0 });
  assert.equal(imageQuery.state.data.rate_limit.allowed, false, 'image quota data is never changed');
  assert.equal(dispatched, 1);
  assert.equal(subscribed, 1);
  assert.equal(context.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(context.__cockpitEnhancements.nativePatchMode, 'live-quota');
  const updatedUsage = { ...usage, credits: { balance: 7 } };
  query.state.data = updatedUsage;
  cacheListener({ query });
  assert.equal(published.rate_limit.allowed, true, 'fresh cache updates retain compatibility');
  assert.equal(published.credits.balance, 7, 'real balances remain intact');
  client.setQueryData(['rate-limit-status'], previous => {
    assert.equal(previous.rate_limit.allowed, false, 'updaters receive the original snapshot');
    return { ...previous, credits: { balance: 8 } };
  });
  assert.equal(published.rate_limit.allowed, true);
  context.__cockpitQuotaCompat.disposeLiveQuotaGuard();
  assert.equal(unsubscribed, 1);
  assert.equal(published.rate_limit.allowed, false, 'cleanup restores the latest original quota');
  assert.equal(published.credits.balance, 8, 'cleanup preserves newer data');
  assert.equal(client.setQueryData, originalSetter);
});

function bundleBrowser() {
  const result = load(), { context } = result, listeners = new Map(), elements = [], blobs = new Map(), revoked = [];
  const nativeTest = fs.readFileSync(path.join(__dirname, 'native_dictation.test.cjs'), 'utf8');
  const fixtureMatch = /const fixture = (`[\s\S]*?`);\r?\n\r?\nfunction load/.exec(nativeTest);
  assert.ok(fixtureMatch, 'native capability fixture must remain available');
  const capabilityFixture = vm.runInNewContext(fixtureMatch[1]);
  let serial = 0;
  Object.assign(context, { Blob, AbortSignal, setTimeout, clearTimeout, name: 'existing-name',
    location: { protocol: 'app:', hostname: '-' },
    addEventListener(name, listener) { listeners.set(name, listener); },
    removeEventListener(name) { listeners.delete(name); },
    fetch: async url => ({ ok: true, text: async () => url.includes('index-') ? `import "./app-initial-fixture.js";`
      : url === initialUrl ? initial + capabilityFixture + '\nconst primaryPath="./app-primary-fixture.js";' : primary }),
  });
  context.__cockpitEnhancements.owner = 'bundle-owner';
  context.URL = class extends URL {
    static createObjectURL(blob) { const id = `blob:fixture-${++serial}`; blobs.set(id, blob); return id; }
    static revokeObjectURL(id) { revoked.push(id); }
  };
  context.HTMLScriptElement = { supports: () => true };
  context.document = {
    documentElement: null, scripts: [{ src: 'app://-/assets/index-fixture.js' }],
    createElement(tag) { return { tag, relList: { supports: () => true }, appendChild(node) { elements.push(node); } }; },
    appendChild(node) { node.parentNode = this; this.documentElement = node; },
    removeChild(node) { node.parentNode = null; this.documentElement = null; },
    addEventListener() {}, removeEventListener() {},
  };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8'), context);
  return { ...result, fast: context.__cockpitNativeFast, listeners, elements, blobs, revoked, capabilityFixture };
}

test('quota activation requires both real modules to evaluate, with one shared import map', async () => {
  const first = bundleBrowser();
  const prepared = await first.fast.prepareHandoff('quota-bundle');
  assert.equal(prepared.ok, true);
  assert.ok(prepared.sourceLength > initial.length + primary.length);
  first.listeners.get('beforeunload')();
  const next = bundleBrowser(); next.context.name = first.context.name;
  assert.equal(next.fast.consumeHandoff('quota-bundle').ok, true);
  assert.equal(next.context.name, 'existing-name');
  const maps = next.elements.filter(node => node.type === 'importmap');
  assert.equal(maps.length, 1);
  assert.deepEqual(Object.keys(JSON.parse(maps[0].textContent).imports), [initialUrl, primaryUrl]);
  assert.equal(next.blobs.size, 2);
  next.context.__cockpitNativeFastComplete();
  next.context.__cockpitNativeFastComplete();
  assert.equal(next.context.__cockpitEnhancements.nativePatchState, 'preloading');
  assert.equal(next.revoked.length, 0, 'do not revoke the composer blob before its evaluation');
  next.context.__cockpitNativeFastComplete('quota');
  assert.equal(next.context.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(next.revoked.length, 2);
  first.fast.cancelHandoff('quota-bundle'); next.fast.cancelHandoff('quota-bundle');
});

test('API Service binding alone prepares and activates both notice modules without mixed-mode compatibility', async () => {
  const first = bundleBrowser();
  Object.assign(first.context.__cockpitEnhancements, { quotaCompat: false, apiService: true });
  const prepared = await first.fast.prepareHandoff('api-service-only');
  assert.equal(prepared.ok, true);
  first.listeners.get('beforeunload')();
  const next = bundleBrowser();
  Object.assign(next.context.__cockpitEnhancements, { quotaCompat: false, apiService: true });
  next.context.name = first.context.name;
  assert.equal(next.fast.consumeHandoff('api-service-only').ok, true);
  assert.equal(next.blobs.size, 2);
  next.context.__cockpitNativeFastComplete();
  next.context.__cockpitNativeFastComplete('quota');
  assert.equal(next.context.__cockpitEnhancements.nativePatchState, 'active');
  assert.equal(next.context.__cockpitEnhancements.nativePatchFeatures, '0001');
  first.fast.cancelHandoff('api-service-only'); next.fast.cancelHandoff('api-service-only');
});

test('APIService unsupported Fast or quota contracts leave native dictation and all modules intact', async () => {
  for (const failure of ['fast', 'quota-initial', 'quota-primary']) {
    const first = bundleBrowser();
    Object.assign(first.context.__cockpitEnhancements, { apiService: true, nativeFast: failure === 'fast' });
    if (failure !== 'fast') {
      const originalFetch = first.context.fetch;
      first.context.fetch = async url => {
        const response = await originalFetch(url);
        const text = await response.text();
        return { ok: true, text: async () => failure === 'quota-initial' && url === initialUrl
          ? text.replace('modelProvider||null', 'unsupportedProvider||null')
          : failure === 'quota-primary' && url === primaryUrl
            ? text.replace('submitDisabled:disabled', 'otherDisabled:disabled') : text };
      };
    }
    const prepared = await first.fast.prepareHandoff('unsupported-' + failure);
    assert.equal(prepared.ok, false, JSON.stringify(prepared));
    assert.equal(prepared.mode, undefined, 'unsupported contracts cannot activate a partial mode');
    assert.equal(first.listeners.has('beforeunload'), false, 'failed preparation cannot stage a later partial activation');
    assert.equal(first.context.name, 'existing-name');
    assert.equal(first.elements.length, 0);
    assert.equal(first.blobs.size, 0);
    assert.equal(first.context.__cockpitNativeDictation, undefined);
    assert.equal(first.context.__cockpitApiDictation, undefined);
    const hookStart = first.capabilityFixture.indexOf('function l6u(');
    const hook = first.fast.functionRange(first.capabilityFixture, hookStart);
    assert.ok(hook);
    let native = { isLoading: false, isCapable: true };
    Object.assign(first.context, { u6u: { c: () => [] }, ty: {}, Eu: () => native });
    vm.runInNewContext(first.capabilityFixture.slice(hook.start, hook.end), first.context);
    for (const value of [true, false, null]) {
      native = { isLoading: value === null, isCapable: value === true };
      assert.equal(first.context.l6u(), value, 'native dictation entry follows its original capability');
      assert.equal(first.context.l6u(true), value, 'ChatGPT dictation entry follows its original capability');
    }
  }
});

test('quota handoffs reject retired capability mode, partial modules and instance-scope tampering', async () => {
  const first = bundleBrowser();
  Object.assign(first.context.__cockpitEnhancements, { apiService: true, nativeFast: false });
  const prepared = await first.fast.prepareHandoff('quota-validation');
  assert.equal(prepared.ok, true);
  assert.equal(prepared.mode, 'full');
  first.listeners.get('beforeunload')();
  const prefix = 'cockpit-native-handoff-v1:';
  const payload = JSON.parse(first.context.name.slice(prefix.length));
  assert.ok(payload.source.includes(first.capabilityFixture), 'the native dictation entry and transcription are preserved in the staged module');
  assert.equal(payload.source.includes('__cockpitNativeDictation'), false);
  assert.equal(payload.source.includes('__cockpitApiDictation'), false);
  for (const mutate of [
    value => { value.mode = 'capability-only'; value.features = '0001:capability-only'; },
    value => { value.mode = 'unknown'; }, value => { value.features = '1101'; },
    value => { value.requestedFeatures = '0001'; }, value => { value.modules = []; },
    value => { value.modules = [{ url: primaryUrl, source: 'unmodified native primary module' }]; },
    value => { value.modules[0].url = 'https://example.test/assets/app-primary-fixture.js'; },
    value => { value.source += ';globalThis.__cockpitNativeDictation.capability(true)'; },
    value => { value.source += ';globalThis.__cockpitApiDictation.transcribe(blob)'; },
  ]) {
    const changed = JSON.parse(JSON.stringify(payload)); mutate(changed);
    const next = bundleBrowser();
    Object.assign(next.context.__cockpitEnhancements, { apiService: true, nativeFast: false });
    next.context.name = prefix + JSON.stringify(changed);
    assert.equal(next.fast.consumeHandoff('quota-validation').ok, false);
    assert.equal(next.context.name, 'existing-name');
    assert.equal(next.blobs.size, 0);
    assert.equal(next.elements.length, 0);
    assert.equal(next.context.__cockpitEnhancements.nativePatchState, 'unsupported');
  }
  const official = bundleBrowser(); official.context.name = first.context.name;
  assert.equal(official.fast.consumeHandoff('quota-validation').ok, false);
  assert.equal(official.blobs.size, 0);
  first.fast.cancelHandoff('quota-validation');
});

test('missing, external or invalid composer modules never activate quota compatibility', async () => {
  const first = bundleBrowser(); await first.fast.prepareHandoff('invalid-bundle');
  first.listeners.get('beforeunload')();
  const prefix = 'cockpit-native-handoff-v1:';
  for (const mutate of [value => { value.modules = []; }, value => { value.modules[0].url = 'https://example.test/app-primary-a.js'; },
    value => { value.modules[0].source = 'unknown'; }, value => { value.modules.push(value.modules[0]); }]) {
    const value = JSON.parse(first.context.name.slice(prefix.length)); mutate(value);
    const next = bundleBrowser(); next.context.name = prefix + JSON.stringify(value);
    assert.equal(next.fast.consumeHandoff('invalid-bundle').ok, false);
    assert.equal(next.context.name, 'existing-name');
    assert.equal(next.blobs.size, 0);
  }
  first.fast.cancelHandoff('invalid-bundle');
});

test('a composer preload failure releases both blobs and does not report active', async () => {
  const first = bundleBrowser(); await first.fast.prepareHandoff('failed-bundle'); first.listeners.get('beforeunload')();
  const next = bundleBrowser(); next.context.name = first.context.name;
  assert.equal(next.fast.consumeHandoff('failed-bundle').ok, true);
  next.elements.filter(node => node.tag === 'link')[1].onerror();
  assert.equal(next.context.__cockpitEnhancements.nativePatchState, 'unsupported');
  assert.equal(next.revoked.length, 2);
  next.context.__cockpitNativeFastComplete(); next.context.__cockpitNativeFastComplete('quota');
  assert.equal(next.context.__cockpitEnhancements.nativePatchState, 'unsupported', 'late completions cannot activate a failed bundle');
  first.fast.cancelHandoff('failed-bundle');
});

test('an unsupported composer cancels preparation before a handoff or reload is installed', async () => {
  const h = bundleBrowser(), fetch = h.context.fetch;
  h.context.fetch = url => url === primaryUrl
    ? Promise.resolve({ ok: true, text: async () => 'unsupported composer version' }) : fetch(url);
  const result = await h.fast.prepareHandoff('unknown-composer');
  assert.equal(result.ok, false);
  assert.equal(h.context.name, 'existing-name');
  assert.equal(h.listeners.size, 0); assert.equal(h.blobs.size, 0);
});

test('the installed shared usage renderer hides the unified-path notice while preserving official and model/image notices', {
  skip: !process.env.COCKPIT_NATIVE_FAST_FIXTURE,
}, () => {
  const React = require('react'), { renderToStaticMarkup } = require('react-dom/server');
  const { api, context } = load();
  let source = fs.readFileSync(process.env.COCKPIT_NATIVE_FAST_FIXTURE, 'utf8');
  const initialUrl = `app://-/assets/${path.basename(process.env.COCKPIT_NATIVE_FAST_FIXTURE)}`;
  let result = api.transform(source, initialUrl);
  const current = result.diagnostics?.bindings?.layout === 'adapter-v1';
  if (current) {
    const names = [...new Set([...source.matchAll(/["'`]\.\/(app-primary-[A-Za-z0-9_-]+\.js)["'`]/g)].map(match => match[1]))];
    assert.equal(names.length, 1);
    source = fs.readFileSync(path.join(path.dirname(process.env.COCKPIT_NATIVE_FAST_FIXTURE), names[0]), 'utf8');
    result = api.transformComposer(source, `app://-/assets/${names[0]}`, initialUrl, result.diagnostics.bindings);
  }
  assert.equal(result.ok, true);
  const parse = require('@babel/parser').parse;
  const ast = parse(result.patchedSource, { sourceType: 'module' });
  const exported = current ? ast.program.body.find(node => node.type === 'FunctionDeclaration'
    && result.patchedSource.slice(node.start, node.end).includes('let __cockpitSharedScope=')).id.name
    : ast.program.body.find(node => node.type === 'ExportNamedDeclaration').specifiers
      .find(node => node.exported.name === 'Pjt').local.name;
  const native = ast.program.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === exported);
  const nativeSource = result.patchedSource.slice(native.start, native.end);
  let hooks = 0;
  Object.assign(context, {
    ALo: { c: () => [] }, jLo: { ...React, useRef(value) { hooks++; return React.useRef(value); } },
    MLo: { jsx: React.createElement }, $S: () => ({ data: false, status: 'success' }),
    KPo: () => ({}), kLo: () => 'usage_limit', OLo: () => false,
    oLo: () => React.createElement('aside', { 'data-native-notice': '' }, 'native usage notice'),
    m0t: { c: () => [] }, h0t: { ...React, useRef(value) { hooks++; return React.useRef(value); } },
    zR: { jsx: React.createElement }, KTe: () => ({ data: false, status: 'success' }), NQt: () => ({}),
    p0t: () => 'usage_limit', f0t: () => false,
    q1t: () => React.createElement('aside', { 'data-native-notice': '' }, 'native usage notice'),
  });
  vm.runInNewContext(nativeSource, context);
  context.__cockpitEnhancements.nativePatchState = 'active';
  const usage = { rate_limit: { allowed: false } };
  const banner = { banner_type: 'pro_rate_limit_reached', ctas: [] };
  const render = (settings = mixedSettings(), options = {}) => renderToStaticMarkup(
    api.wrapComposer(React, React.createElement, 'local', settings, options.provider, true,
      React.createElement(context[exported], { banner: { ...banner, ...options.banner }, rateLimitStatus: usage,
        fallbackContent: options.fallback }), options.imageLimit));
  assert.equal(render(), '');
  assert.equal(hooks, 1, 'original hooks still run on the hidden path');
  const originalNode = parse(source, { sourceType: 'module' }).program.body
    .find(node => node.type === 'FunctionDeclaration' && node.id.name === exported);
  vm.runInNewContext(source.slice(originalNode.start, originalNode.end), context);
  assert.match(render(), /data-native-notice/, 'the previously untouched unified renderer reproduces the reported banner');
  vm.runInNewContext(nativeSource, context);
  assert.equal(render(), '', 'the same native renderer and route must hide the notice after the fix');
  assert.match(render(undefined, { provider: 'openai' }), /data-native-notice/);
  for (const type of ['image_generation_limit_reached', 'selected_model_limit_reached', 'luna_reserve', 'unknown']) {
    assert.match(render(undefined, { banner: { banner_type: type } }), /data-native-notice/);
  }
  for (const key of ['model_slug', 'blocked_model_slug']) {
    if (current) {
      assert.equal(render(undefined, { banner: { [key]: 'limited-model' } }), '');
      assert.match(render(undefined, { provider: 'openai', banner: { [key]: 'limited-model' } }), /data-native-notice/);
    } else assert.match(render(undefined, { banner: { [key]: 'limited-model' } }), /data-native-notice/);
  }
  assert.match(render(undefined, { imageLimit: {} }), /data-native-notice/);
  assert.equal(render(undefined, { fallback: React.createElement('p', null, 'Safety notice') }), '<p>Safety notice</p>');
  usage.rate_limit.allowed = true;
  assert.match(render(), /data-native-notice/);
  usage.rate_limit.allowed = false;
  context.__cockpitEnhancements.nativePatchState = 'pending';
  assert.match(render(), /data-native-notice/);
  context.__cockpitEnhancements.nativePatchState = 'active';
  assert.equal(render(), '');
  assert.equal(JSON.stringify(banner), '{"banner_type":"pro_rate_limit_reached","ctas":[]}');
});

test('installed desktop initial and primary modules validate and parse together', {
  skip: !process.env.COCKPIT_NATIVE_FAST_FIXTURE || !process.env.COCKPIT_NATIVE_QUOTA_FIXTURE,
}, async () => {
  const { api, context } = load();
  const source = fs.readFileSync(process.env.COCKPIT_NATIVE_FAST_FIXTURE, 'utf8');
  const composer = fs.readFileSync(process.env.COCKPIT_NATIVE_QUOTA_FIXTURE, 'utf8');
  const initialName = path.basename(process.env.COCKPIT_NATIVE_FAST_FIXTURE);
  const primaryName = path.basename(process.env.COCKPIT_NATIVE_QUOTA_FIXTURE);
  const url = `app://-/assets/${initialName}`, primary = `app://-/assets/${primaryName}`;
  const contract = api.transform(source, url);
  assert.equal(contract.ok, true);
  const result = api.transformComposer(composer, primary, url, contract.diagnostics.bindings);
  assert.equal(result.ok, true, JSON.stringify(result.diagnostics));
  Object.assign(context, { Blob, setTimeout, clearTimeout, AbortSignal });
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'fast.js'), 'utf8'), context);
  const rewritten = await context.__cockpitNativeFast.rewriteModule(result.patchedSource, primary);
  const parsed = new vm.SourceTextModule(rewritten);
  assert.ok(parsed.dependencySpecifiers.every(specifier => !specifier.startsWith('.')));
  const current = contract.diagnostics.bindings.layout === 'adapter-v1';
  let nativeComposerSource = composer, patchedComposerSource = result.patchedSource, adapterUrl, adapter;
  if (current) {
    const adapterName = path.basename(contract.diagnostics.bindings.adapterPath);
    adapterUrl = new URL(contract.diagnostics.bindings.adapterPath, url).href;
    adapter = fs.readFileSync(path.join(path.dirname(process.env.COCKPIT_NATIVE_FAST_FIXTURE), adapterName), 'utf8');
    const adapted = api.transformAdapter(adapter, adapterUrl, url, primary, result.diagnostics.bindings);
    assert.equal(adapted.ok, true, JSON.stringify(adapted.diagnostics));
    nativeComposerSource = adapter; patchedComposerSource = adapted.patchedSource;
    assert.match(patchedComposerSource, /\|\|\(\(Rn\|\|Ur\)&&!globalThis\.__cockpitQuotaCompat\.enabledForComposer\(/);
    assert.ok(result.patchedSource.includes('shouldHideCurrentBanner(__cockpitBannerScope,'));
    assert.ok(result.patchedSource.includes('shouldHideCurrentBanner(__cockpitSharedScope,'));
  } else {
    assert.ok(result.patchedSource.includes('||((wt||vn)&&!globalThis.__cockpitQuotaCompat.enabledFor(bt,un,__cockpitQuotaProvider))'));
    assert.ok(result.patchedSource.includes('wrapComposer(PZ,FZ.jsx,bt,un,__cockpitQuotaProvider,'));
    assert.ok(result.patchedSource.includes('shouldHideBanner(__cockpitBannerScope,p,o,i)'));
    assert.ok(contract.patchedSource.includes('shouldHideBanner(__cockpitSharedScope,'));
  }
  const originalAst = require('@babel/parser').parse(nativeComposerSource, { sourceType: 'module' });
  const patchedAst = require('@babel/parser').parse(patchedComposerSource, { sourceType: 'module' });
  const wrappedName = current ? 'wrapCurrentComposer' : 'wrapComposer';
  const patchedComposer = patchedAst.program.body.find(node => node.type === 'FunctionDeclaration'
    && patchedComposerSource.slice(node.start, node.end).includes(`return globalThis.__cockpitQuotaCompat.${wrappedName}(`));
  const originalComposer = originalAst.program.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === patchedComposer.id.name);
  const returned = patchedComposer.body.body.findLast(node => node.type === 'ReturnStatement').argument;
  assert.equal(returned.callee.property.name, wrappedName, 'wrap the actual outer return, not a nested callback');
  for (const node of originalComposer.body.body.filter(node => node.type === 'FunctionDeclaration')) {
    const patched = patchedComposer.body.body.find(item => item.type === 'FunctionDeclaration' && item.id.name === node.id.name);
    assert.equal(patchedComposerSource.slice(patched.start, patched.end), nativeComposerSource.slice(node.start, node.end));
  }
  context.__cockpitEnhancements.apiService = true;
  context.document = { scripts: [{ src: 'app://-/assets/index-fixture.js' }] };
  context.fetch = async target => ({ ok: true, text: async () => target === url ? source : target === primary ? composer : target === adapterUrl ? adapter : `import "./${initialName}";` });
  const bundle = await context.__cockpitNativeFast.prepareDocument();
  assert.equal(bundle.ok, true, JSON.stringify({ reason: bundle.reason, diagnostics: bundle.diagnostics }));
  assert.equal(bundle.modules.length, current ? 2 : 1);
  if (current) assert.equal(bundle.modules[1].url, adapterUrl);
  assert.equal(bundle.modules[0].url, primary);
  assert.equal(bundle.patchedSource.includes('__cockpitNativeDictation'), false, 'quota activation never suppresses the native dictation entry');
  assert.equal(bundle.patchedSource.includes('__cockpitApiDictation'), false, 'quota activation never installs a transcription bridge');
  const originalAudio = require('@babel/parser').parse(source, { sourceType: 'module' });
  const patchedAudio = require('@babel/parser').parse(bundle.patchedSource, { sourceType: 'module' });
  const audioNames = current ? [
    ['dictationEnabled', 'whisper_killswitch'], ['fetch(`/transcribe`', 'multipart/form-data'],
    ['safePost(`/transcribe`', 'onRequestSuccess'],
  ].map(markers => {
    const found = originalAudio.program.body.filter(node => node.type === 'FunctionDeclaration'
      && markers.every(marker => source.slice(node.start, node.end).includes(marker)));
    assert.equal(found.length, 1); return found[0].id.name;
  }) : ['l6u', 'YLo', 'XLo'];
  for (const name of audioNames) {
    const before = originalAudio.program.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === name);
    const after = patchedAudio.program.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === name);
    assert.ok(before && after, `native dictation ${name} function remains available`);
    const expected = await context.__cockpitNativeFast.rewriteModule(source.slice(before.start, before.end), url);
    assert.equal(bundle.patchedSource.slice(after.start, after.end), expected, `native ${name} retains its original capability/transcription behavior`);
  }
  assert.equal(bundle.patchedSource.includes('cockpitDictationScope'), false);
  new vm.SourceTextModule(bundle.patchedSource);
  for (const module of bundle.modules) new vm.SourceTextModule(module.source);
});

test('a pending live-quota retry cannot overwrite newly activated Fast features', () => {
  for (const mode of ['full', 'fast-only']) {
    const { api, context } = load();
    const timers = new Map(); let serial = 0;
    Object.assign(context.__cockpitEnhancements, { apiService: true });
    Object.assign(context, { setTimeout(fn) { timers.set(++serial, fn); return serial; },
      clearTimeout(id) { timers.delete(id); }, document: { querySelectorAll: () => [] } });
    api.installLiveQuotaGuard({ hardVariable: 'h', guardVariable: 'g', hostVariable: 'host' });
    assert.equal(timers.size, 1);
    const pending = [...timers.values()][0];
    const state = context.__cockpitEnhancements;
    Object.assign(state, { nativePatchState: 'active', nativePatchMode: mode,
      nativePatchFeatures: mode === 'full' ? '1111' : '1010', nativePatchRequestedFeatures: '1111',
      nativeQuotaPatchState: mode === 'full' ? 'active' : 'unsupported',
      nativeQuotaPatchReason: mode === 'full' ? undefined : 'quota-shared-banner-contract-mismatch' });
    const before = JSON.stringify(state);
    pending();
    assert.equal(timers.size, 0);
    assert.equal(JSON.stringify(state), before);
    assert.equal(api.installLiveQuotaGuard({}).preservedNativeFast, true);
    assert.equal(JSON.stringify(state), before);
  }
});


test('a late live-quota retry preserves quota-only full modules without claiming Fast', () => {
  const { api, context } = load(), timers = new Map(); let serial = 0;
  Object.assign(context.__cockpitEnhancements, { apiService: true });
  Object.assign(context, { setTimeout(fn) { timers.set(++serial, fn); return serial; },
    clearTimeout(id) { timers.delete(id); }, document: { querySelectorAll: () => [] } });
  api.installLiveQuotaGuard({ hardVariable: 'h', guardVariable: 'g', hostVariable: 'host' });
  const pending = [...timers.values()][0];
  assert.equal(typeof pending, 'function');
  Object.assign(context.__cockpitEnhancements, { nativeFast: false, nativePatchState: 'active',
    nativePatchMode: 'full', nativePatchFeatures: '0101', nativePatchRequestedFeatures: '0101',
    nativePatchReason: 'module-evaluated' });
  const before = JSON.stringify(context.__cockpitEnhancements);
  pending();
  assert.equal(timers.size, 0);
  assert.equal(JSON.stringify(context.__cockpitEnhancements), before);
  const retained = api.installLiveQuotaGuard({});
  assert.equal(retained.preservedNativeModule, true);
  assert.equal(retained.preservedNativeFast, undefined);
  assert.equal(JSON.stringify(context.__cockpitEnhancements), before);
});
