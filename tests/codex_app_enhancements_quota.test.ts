import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../src-tauri/src/modules/codex_app_enhancements/quota.js', import.meta.url), 'utf8');
const fixture = 'var gate=atom(U,(host,{get:read})=>{let account=read(auth);if(account.authMethod!==`chatgpt`)return {hardBlocked:false};return {hardBlocked:read(usage).rate_limit?.allowed===!1}}),blocked=family(U,(host,{get:read})=>read(gate,host).hardBlocked);';
const exports = 'var provider=atom(({summary:item})=>item.modelProvider||null);function Banner(props){let memo=0,{banner:b,fallbackContent:f,lastImpressionKeyRef:r,onDismiss:d,onShown:s,onOwnerRequestComplete:o,presentationKey:k,rateLimitStatus:u,renderContent:render}=props,ref=(0,React.useRef)(null);let image=b.banner_type===`luna_reserve`||b.banner_type===`image_generation_limit_reached`;return JSX.jsx("banner",{fallbackContent:f,image})}export{blocked as quotaExport,provider as providerExport};';
const url = 'app://-/assets/app-initial-fixture.js';
function runtime() {
  const context = vm.createContext({ URL });
  vm.runInContext(readFileSync(new URL('../src-tauri/src/modules/codex_app_enhancements/fast.js', import.meta.url), 'utf8'), context);
  vm.runInContext(source, context);
  return { context, api: context.__cockpitQuotaCompat };
}

test('quota compatibility preserves native exhaustion while admitting only verified independent API composers', () => {
  const { context, api } = runtime();
  const result = api.transform(fixture + exports, url);
  assert.equal(result.ok, true);
  assert.ok(result.patchedSource.includes('rate_limit?.allowed===!1'));
  assert.ok(result.patchedSource.includes('hardBlocked:read(usage).rate_limit?.allowed===!1'));
  const state = { rate_limit: { allowed: false, limit_reached: true }, credits: { balance: 0 } };
  context.U = {};
  context.auth = {};
  context.usage = {};
  context.atom = (scope: unknown, read: unknown) => read ?? scope;
  context.family = (_scope: unknown, read: unknown) => read;
  context.__cockpitEnhancements = { quotaCompat: true };
  vm.runInContext(result.patchedSource.replace(/export\{[^}]+\};/, ''), context);
  const get = (target: unknown) => target === context.auth
    ? { authMethod: 'chatgpt' }
    : target === context.usage ? state : context.gate('local', { get });
  assert.equal(context.blocked('local', { get }), true, 'initial quota atoms must retain real exhaustion');
  const settings = { isConfigDataPending: false, resolvedConfig: { model_provider: 'relay', model_providers: { relay: { requires_openai_auth: true, experimental_bearer_token: 'fixture-key', base_url: 'https://example.invalid/v1' } } } };
  assert.equal(api.enabledFor('local', settings), true);
  assert.equal(api.enabledFor('remote', settings), false);
  assert.equal(context.blocked('remote', { get }), true);
  context.__cockpitEnhancements.quotaCompat = false;
  assert.equal(context.blocked('local', { get }), true);
  assert.deepEqual(state, { rate_limit: { allowed: false, limit_reached: true }, credits: { balance: 0 } });
});

test('unknown, ambiguous or previously transformed contracts retain the original source', () => {
  const { api } = runtime();
  for (const input of [fixture.replace('`chatgpt`', '`other`'), fixture + fixture, fixture.replace('rate_limit?.allowed', 'unrelated'), 'unrelated']) {
    const result = api.transform(input, url);
    assert.equal(result.ok, false);
    assert.equal(result.patchedSource, input);
  }
  assert.equal(api.transform(fixture + exports, 'https://example.test/app.js').ok, false);
  const once = api.transform(fixture + exports, url).patchedSource;
  assert.equal(api.transform(once, url).ok, false);
});

test('current account-verified quota family remains supported without modifying its snapshot', () => {
  const { api } = runtime();
  const verified = 'var quota=family(U,(host,{get:read})=>{let identity=read(auth),snapshot=read(usage);if(snapshot.user_id!==identity.userId||snapshot.account_id!==identity.authenticatedAccountId)return empty;return {hardBlocked:snapshot.rate_limit?.allowed===!1}}),blocked=family(U,(host,{get:read})=>read(quota,host).hardBlocked);';
  const result = api.transform(verified + exports, url);
  assert.equal(result.ok, true);
  assert.ok(result.patchedSource.includes('snapshot.account_id!==identity.authenticatedAccountId'));
  assert.ok(result.patchedSource.includes('hardBlocked:snapshot.rate_limit?.allowed===!1'));
});
