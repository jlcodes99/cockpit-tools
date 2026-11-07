const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const { parse } = require('@babel/parser');
const generate = require('@babel/generator').default;
const fixtureDir = process.env.COCKPIT_NATIVE_QUOTA_CURRENT_DIR;
function load() {
  const context = { URL, module: { exports: {} }, __cockpitEnhancements: { quotaCompat: true, nativePatchState: 'active' } };
  context.window = context; context.globalThis = context;
  for (const name of ['fast', 'quota', 'quota_current']) vm.runInNewContext(fs.readFileSync(path.join(__dirname, `${name}.js`), 'utf8'), context);
  return { context, api: context.__cockpitQuotaCompat };
}
const settings = () => ({ isConfigDataPending: false, resolvedConfig: {
  model_provider: 'relay', model_providers: { relay: { requires_openai_auth: true,
    experimental_bearer_token: 'test-api-key', base_url: 'http://127.0.0.1:58554/v1' } },
} });
const scope = () => ({ value: { kind: 'local', conversationId: 'test-thread' } });
test('current composer eligibility follows the actual thread provider and local execution scope', () => {
  const { api } = load(), config = settings(), local = scope();
  assert.equal(api.enabledForComposer('local', config, 'relay', {}, local), true);
  for (const [host, cfg, provider, props, route] of [
    ['remote', config, 'relay', {}, local], ['local', config, 'openai', {}, local],
    ['local', config, null, {}, local], ['local', { ...config, isConfigDataPending: true }, 'relay', {}, local],
    ['local', config, 'relay', { useAsbDirect: true }, local],
    ['local', config, 'relay', { selectedProject: { type: 'remote' } }, local],
    ['local', config, 'relay', { homeRunLocationRemoteProject: {} }, local],
    ['local', config, 'relay', {}, { value: { kind: 'cloud' } }],
    ['local', config, 'relay', {}, { value: { kind: 'new', cloudThreadPrototype: {} } }],
  ]) assert.equal(api.enabledForComposer(host, cfg, provider, props, route), false);
  assert.equal(api.enabledForComposer('local', config, null, {}, { value: { kind: 'new' } }), true);
  assert.equal(api.enabledForComposer('local', config, null, {}, { value: { kind: 'local', conversationId: null } }), true);
});
test('current model quota notices are scoped to API routes and preserve images, official routes and raw usage', () => {
  const { api, context } = load(), usage = { rate_limit: { allowed: true }, additional_rate_limits: [{ limit_name: 'gpt-test', rate_limit: { allowed: false } }],
    rate_limit_upsell: { banner_type: 'pro_rate_limit_reached', blocked_model_slug: 'gpt-test' } };
  const raw = JSON.stringify(usage), compatible = { host: 'local', compatible: true };
  assert.equal(api.shouldHideCurrentBanner(compatible, 'local', usage, null), true);
  for (const [route, host, image] of [[null, 'local', null], [{ ...compatible, compatible: false }, 'local', null],
    [compatible, 'remote', null], [compatible, 'local', {}]]) assert.equal(api.shouldHideCurrentBanner(route, host, usage, image), false);
  for (const type of ['image_generation_limit_reached', 'luna_reserve', 'unknown'])
    assert.equal(api.shouldHideCurrentBanner(compatible, 'local', { ...usage, rate_limit_upsell: { ...usage.rate_limit_upsell, banner_type: type } }, null), false);
  context.__cockpitEnhancements.nativePatchState = 'pending';
  assert.equal(api.shouldHideCurrentBanner(compatible, 'local', usage, null), false);
  assert.equal(JSON.stringify(usage), raw);
});
function artifacts() {
  const initialName = process.env.COCKPIT_NATIVE_QUOTA_CURRENT_INITIAL || 'app-initial-feb30b196d11.js';
  const initial = fs.readFileSync(path.join(fixtureDir, initialName), 'utf8');
  const primaryName = [...new Set([...initial.matchAll(/["'`]\.\/(app-primary-[a-z0-9]+\.js)["'`]/g)].map(match => match[1]))];
  const adapterName = [...new Set([...initial.matchAll(/["'`]\.\/(codex-composer-adapter-[a-z0-9]+\.js)["'`]/g)].map(match => match[1]))];
  assert.equal(primaryName.length, 1); assert.equal(adapterName.length, 1);
  return { initial, primary: fs.readFileSync(path.join(fixtureDir, primaryName[0]), 'utf8'), adapter: fs.readFileSync(path.join(fixtureDir, adapterName[0]), 'utf8'),
    initialUrl: `app://-/assets/${initialName}`, primaryUrl: `app://-/assets/${primaryName[0]}`, adapterUrl: `app://-/assets/${adapterName[0]}` };
}
function transformAll() {
  const h = load(), raw = artifacts(), i = h.api.transform(raw.initial, raw.initialUrl);
  assert.equal(i.ok, true, JSON.stringify(i.diagnostics));
  const p = h.api.transformComposer(raw.primary, raw.primaryUrl, raw.initialUrl, i.diagnostics.bindings);
  assert.equal(p.ok, true, JSON.stringify(p.diagnostics));
  const a = h.api.transformAdapter(raw.adapter, raw.adapterUrl, raw.initialUrl, raw.primaryUrl, p.diagnostics.bindings);
  assert.equal(a.ok, true, JSON.stringify(a.diagnostics));
  return { ...h, raw, i, p, a };
}
test('installed current modules validate together and change only two presenters and the adapter', { skip: !fixtureDir }, () => {
  const h = transformAll();
  assert.equal(h.i.diagnostics.bindings.layout, 'adapter-v1');
  assert.ok(h.i.patchedSource.startsWith(h.raw.initial), 'quota atoms, quota snapshots and model routing stay byte-identical');
  for (const [source, result, expectedChanged] of [[h.raw.primary, h.p, 2], [h.raw.adapter, h.a, 1]]) {
    const before = parse(source, { sourceType: 'module' }), after = parse(result.patchedSource, { sourceType: 'module' });
    const functions = ast => ast.program.body.filter(node => node.type === 'FunctionDeclaration');
    assert.equal(functions(before).length, functions(after).length);
    let changed = 0;
    for (const fn of functions(before)) {
      const patched = functions(after).find(node => node.id.name === fn.id.name);
      if (source.slice(fn.start, fn.end) !== result.patchedSource.slice(patched.start, patched.end)) changed++;
    }
    assert.equal(changed, expectedChanged);
  }
  const adapterAst = parse(h.a.patchedSource, { sourceType: 'module' });
  const fn = adapterAst.program.body.find(node => node.type === 'FunctionDeclaration' && h.a.patchedSource.slice(node.start, node.end).includes('__cockpitQuotaProvider='));
  const returned = fn.body.body.findLast(node => node.type === 'ReturnStatement');
  assert.equal(returned.argument.callee.property.name, 'wrapCurrentComposer');
  const provider = fn.body.body.flatMap(node => node.declarations || []).find(node => node.id.name === '__cockpitQuotaProvider');
  assert.equal(provider.init.type, 'CallExpression', 'provider hook stays unconditional at function top level');
});
test('actual current submit expression removes only verified OAuth quota guards for all other disabled combinations', { skip: !fixtureDir }, () => {
  const h = transformAll(), location = h.a.diagnostics.location;
  const expressions = [h.raw.adapter, h.a.patchedSource].map(source => {
    const ast = parse(source, { sourceType: 'module' });
    const fn = ast.program.body.find(node => node.type === 'FunctionDeclaration' && source.slice(node.start, node.end).includes(`submitDisabled:Si`));
    return fn.body.body.flatMap(node => node.declarations || []).find(node => node.id.name === 'Si').init;
  });
  const names = ['yt', 'Vr', 'Pn', 'ln', 'pn', 'Wn', 'kr', 'cr', 'jr', 'Rn', 'Ur', 'U', 'Fr', '__cockpitQuotaProvider', 'e', 'R', 'globalThis'];
  const funcs = expressions.map(expression => Function(...names, `return ${generate(expression).code}`));
  for (let bits = 0; bits < 2048; bits++) {
    const values = Array.from({ length: 11 }, (_, index) => Boolean(bits & (1 << index)));
    values[8] = { isLoading: values[8] };
    const evaluate = (fn, route, props = {}, provider = 'relay', cfg = settings()) => fn(...values, 'local', cfg, provider, props, route, h.context);
    const original = evaluate(funcs[0], scope()), patched = evaluate(funcs[1], scope());
    const withoutQuota = funcs[0](...values.slice(0, 9), false, false, 'local', settings(), 'relay', {}, scope(), h.context);
    assert.equal(patched, withoutQuota, `unrelated disabled guards: ${bits}`);
    assert.equal(evaluate(funcs[1], scope(), {}, 'openai'), original);
    assert.equal(evaluate(funcs[1], scope(), { useAsbDirect: true }), original);
    assert.equal(evaluate(funcs[1], { value: { kind: 'cloud' } }), original);
  }
  Object.assign(h.context.__cockpitEnhancements, { apiService: true, apiKeyAuthVerified: true, quotaCompat: false });
  const pure = settings(); pure.resolvedConfig.model_providers.relay.requires_openai_auth = false;
  const blocked = [false, false, false, false, false, false, false, false, { isLoading: false }, true, true];
  assert.equal(funcs[0](...blocked, 'local', pure, 'relay', {}, scope(), h.context), true);
  assert.equal(funcs[1](...blocked, 'local', pure, 'relay', {}, scope(), h.context), false);
  assert.equal(funcs[1](...blocked, 'local', pure, 'openai', {}, scope(), h.context), true);
  blocked[0] = true;
  assert.equal(funcs[1](...blocked, 'local', pure, 'relay', {}, scope(), h.context), true);
  assert.equal(location.layout, 'adapter-v1');
});
test('current contracts reject duplicated or changed gates without publishing partial source', { skip: !fixtureDir }, () => {
  const h = transformAll();
  for (const source of [h.raw.adapter.replace('||Rn||Ur,', '||Rn||Ur||unexpected,'),
    h.raw.adapter.replace('K3 as Ge', 'K3 as Wrong'), h.raw.adapter + h.raw.adapter]) {
    const result = h.api.transformAdapter(source, h.raw.adapterUrl, h.raw.initialUrl, h.raw.primaryUrl, h.p.diagnostics.bindings);
    assert.equal(result.ok, false); assert.equal(result.patchedSource, source);
  }
  const primary = h.raw.primary.replace('authMethod!==`chatgpt`', 'authMethod!==`different`');
  assert.equal(h.api.transformComposer(primary, h.raw.primaryUrl, h.raw.initialUrl, h.i.diagnostics.bindings).ok, false);
  assert.equal(h.api.transformAdapter(h.raw.adapter, 'https://example.test/assets/codex-composer-adapter.js', h.raw.initialUrl, h.raw.primaryUrl, h.p.diagnostics.bindings).ok, false);
});

test('actual current shared renderer keeps hooks and safety fallback while isolating official and API model notices', { skip: !fixtureDir }, () => {
  const React = require('react'), { renderToStaticMarkup } = require('react-dom/server');
  const h = transformAll(), parseFunction = source => {
    const ast = parse(source, { sourceType: 'module' });
    const node = ast.program.body.find(node => node.type === 'FunctionDeclaration' && node.id.name === 'RR');
    assert.ok(node); return source.slice(node.start, node.end);
  };
  let hooks = 0;
  Object.assign(h.context, { m0t: { c: () => [] }, h0t: { ...React, useRef(value) { hooks++; return React.useRef(value); } },
    KTe: () => ({ data: false, status: 'success' }), NQt: () => ({}), p0t: () => 'usage_limit', f0t: () => false,
    zR: { jsx: React.createElement }, q1t: () => React.createElement('aside', null, 'native usage notice') });
  const usage = { rate_limit: { allowed: false } }, banner = { banner_type: 'pro_rate_limit_reached', ctas: [] };
  const render = ({ provider = 'relay', config = settings(), props = {}, route = scope(), model = null, type = banner.banner_type } = {}) =>
    renderToStaticMarkup(h.api.wrapCurrentComposer(React, React.createElement, 'local', config, provider,
      h.api.enabledForComposer('local', config, provider, props, route),
      React.createElement(h.context.RR, { banner: { ...banner, banner_type: type, blocked_model_slug: model }, rateLimitStatus: usage,
        fallbackContent: React.createElement('p', null, 'Safety notice') })));
  vm.runInNewContext(parseFunction(h.raw.primary), h.context);
  assert.match(render(), /native usage notice/, 'actual unpatched renderer reproduces stale OAuth notice');
  vm.runInNewContext(parseFunction(h.p.patchedSource), h.context);
  hooks = 0;
  assert.equal(render(), '<p>Safety notice</p>'); assert.equal(hooks, 1);
  assert.equal(render({ model: 'gpt-test' }), '<p>Safety notice</p>');
  for (const options of [{ provider: 'openai' }, { provider: null }, { model: 'gpt-test', provider: 'openai' },
    { props: { useAsbDirect: true } }, { route: { value: { kind: 'cloud' } } },
    { type: 'image_generation_limit_reached' }, { type: 'luna_reserve' }, { type: 'unknown' }])
    assert.match(render(options), /native usage notice/);
  const pure = settings(); pure.resolvedConfig.model_providers.relay.requires_openai_auth = false;
  Object.assign(h.context.__cockpitEnhancements, { apiService: true, apiKeyAuthVerified: true, quotaCompat: false });
  assert.equal(render({ config: pure }), '<p>Safety notice</p>');
  assert.equal(render({ config: pure, model: 'gpt-test' }), '<p>Safety notice</p>');
  assert.match(render({ config: pure, provider: 'openai' }), /native usage notice/);
  assert.equal(JSON.stringify(usage), '{"rate_limit":{"allowed":false}}');
});


test('new layout live activation requires a complete reload without touching DOM or real usage data', { skip: !fixtureDir }, async () => {
  const h = transformAll();
  h.context.__cockpitEnhancements.apiService = true;
  h.context.document = { scripts: [{ src: 'app://-/assets/index-current.js' }] };
  const files = new Map([[h.raw.initialUrl, h.raw.initial], [h.raw.primaryUrl, h.raw.primary], [h.raw.adapterUrl, h.raw.adapter],
    ['app://-/assets/index-current.js', `import './${path.basename(h.raw.initialUrl)}';`]]);
  h.context.fetch = async url => ({ ok: files.has(url), text: async () => files.get(url) });
  const before = JSON.stringify(h.context.__cockpitEnhancements);
  const result = await h.api.prepareLiveQuotaGate();
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'quota-restart-required');
  assert.equal(JSON.stringify(h.context.__cockpitEnhancements), before);
});

test('API service binding does not override an explicit official composer scope in the current layout', () => {
  const { api, context } = load();
  context.__cockpitEnhancements.apiService = true;
  const usage = { rate_limit: { allowed: false }, rate_limit_upsell: { banner_type: 'pro_rate_limit_reached' } };
  assert.equal(api.shouldHideCurrentBanner({ host: 'local', compatible: false }, 'local', usage, null), false);
  assert.equal(api.shouldHideCurrentBanner(null, undefined, usage, null), false);
  context.__cockpitEnhancements.nativePatchMode = 'full';
  context.__cockpitEnhancements.nativePatchFeatures = '0101';
  assert.equal(api.installLiveQuotaGuard({}).preservedNativeModule, true, 'quota-only full module survives reconnect too');
});


test('verified API-only routes with no OAuth bypass stale subscription gates and notices only in their own composer', () => {
  const React = require('react'), { renderToStaticMarkup } = require('react-dom/server');
  const { api, context } = load(), config = settings(), local = scope();
  config.resolvedConfig.model_providers.relay.requires_openai_auth = false;
  Object.assign(context.__cockpitEnhancements, { apiService: true, apiKeyAuthVerified: true, quotaCompat: false });
  const usage = { rate_limit: { allowed: false }, rate_limit_upsell: { banner_type: 'pro_rate_limit_reached', blocked_model_slug: 'gpt-test' } };
  const render = (provider = 'relay', cfg = config, props = {}, route = local) => {
    function Notice() { const value = api.useBannerScope(React); return api.shouldHideCurrentBanner(value, 'local', usage, null) ? null : React.createElement('aside', null, 'native notice'); }
    return renderToStaticMarkup(api.wrapCurrentComposer(React, React.createElement, 'local', cfg, provider,
      api.enabledForComposer('local', cfg, provider, props, route), React.createElement(Notice)));
  };
  assert.equal(api.enabledForComposer('local', config, 'relay', {}, local), true);
  assert.equal(render(), '');
  assert.match(render('openai'), /native notice/);
  assert.match(render(null), /native notice/);
  assert.match(render('relay', config, { useAsbDirect: true }), /native notice/);
  assert.match(render('relay', config, {}, { value: { kind: 'cloud' } }), /native notice/);
  assert.match(render('relay', { ...config, isConfigDataPending: true }), /native notice/);
  context.__cockpitEnhancements.apiKeyAuthVerified = false;
  assert.match(render(), /native notice/);
  context.__cockpitEnhancements.apiKeyAuthVerified = true;
  config.resolvedConfig.model_providers.relay.experimental_bearer_token = '';
  assert.match(render(), /native notice/);
});
