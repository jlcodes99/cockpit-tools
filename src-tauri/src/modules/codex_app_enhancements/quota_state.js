// API Service owns the real upstream quota. Adapt the desktop's incoming usage
// data, before its native query/selector/composer logic consumes it.
(() => {
  const root = typeof window === 'object' ? window : globalThis;
  if (root.__cockpitQuotaState) return;
  const enabled = () => root.__cockpitEnhancements?.apiService === true;
  const record = value => value != null && typeof value === 'object' && !Array.isArray(value);
  const snapshots = new WeakMap();
  const originals = new WeakMap();
  const cleanups = [];
  let pending, installed = false, generation = 0;
  const usageBanners = new Set(['workspace_member_usage_limit_reached', 'workspace_owner_usage_limit_reached',
    'workspace_member_credits_depleted', 'workspace_owner_credits_depleted', 'plus_rate_limit_reached',
    'plus_gpt6_rate_limit_reached', 'prolite_rate_limit_reached', 'pro_rate_limit_reached',
    'free_trial_rate_limit_reached', 'go_trial_rate_limit_reached', 'free_or_go_rate_limit_reached',
    'business_rate_limit_reached', 'business_gpt6_rate_limit_reached', 'cbp_rate_limit_reached',
    'legacy_rate_limit_reached', 'luna_reserve', 'selected_model_limit_reached']);
  function rate(value) {
    if (!record(value)) return value;
    const window = value => record(value) ? {...value, used_percent: 0} : value;
    return {...value, allowed: true, limit_reached: false,
      ...(value.primary_window ? {primary_window: window(value.primary_window)} : {}),
      ...(value.secondary_window ? {secondary_window: window(value.secondary_window)} : {})};
  }
  function usage(value) {
    if (!enabled() || !record(value) || typeof value.rate_limit?.allowed !== 'boolean') return value;
    const raw = originals.get(value) ?? value;
    if (snapshots.has(raw)) return snapshots.get(raw);
    const next = {...raw, rate_limit: rate(raw.rate_limit), rate_limit_reached_type: null};
    for (const key of ['code_review_rate_limit']) if (key in raw) next[key] = rate(raw[key]);
    if (Array.isArray(raw.additional_rate_limits)) next.additional_rate_limits = raw.additional_rate_limits.map(
      item => record(item) ? {...item, rate_limit: rate(item.rate_limit)} : item);
    if (record(raw.credits)) next.credits = {...raw.credits, has_credits: true, unlimited: true,
      balance: null, ...('overage_limit_reached' in raw.credits ? {overage_limit_reached: false} : {})};
    if (record(raw.spend_control)) {
      next.spend_control = {...raw.spend_control};
      if (record(raw.spend_control.individual_limit)) next.spend_control.individual_limit = {
        ...raw.spend_control.individual_limit, limit_reached: false, current_month_usage: 0};
    }
    if (usageBanners.has(raw.rate_limit_upsell?.banner_type)) next.rate_limit_upsell = null;
    for (const key of ['rate_limit_warning', 'model_picker_upsell', 'sidebar_usage_warnings', 'ambient_usage'])
      if (key in raw) next[key] = null;
    snapshots.set(raw, next); originals.set(next, raw);
    return next;
  }
  function frame(value) {
    if (!enabled()) return value;
    if (Array.isArray(value)) {
      let changed = false;
      const next = value.map(item => {
        if (item?.type !== 'message' || item.topic_id !== 'codex-work-usage-push-v1'
          || item.payload?.type !== 'usage.snapshot') return item;
        const payload = frame(item.payload);
        if (payload === item.payload) return item;
        changed = true; return {...item, payload};
      });
      return changed ? next : value;
    }
    if (value?.type !== 'usage.snapshot' || !record(value.payload?.usage_by_client)) return value;
    const clients = value.payload.usage_by_client, next = usage(clients.electron);
    return next === clients.electron ? value : {...value, payload: {...value.payload,
      usage_by_client: {...clients, electron: next}}};
  }
  function installMessages() {
    const proto = root.MessageEvent?.prototype, descriptor = proto && Object.getOwnPropertyDescriptor(proto, 'data');
    if (!descriptor?.get || !descriptor.configurable) throw Error('quota-message-boundary-unavailable');
    const events = new WeakMap();
    const get = function () {
      const raw = descriptor.get.call(this);
      if (!enabled()) return raw;
      if (events.has(this)) return events.get(this);
      let next = raw;
      if (typeof raw === 'string' && raw.includes('usage.snapshot')) {
        // Only the explicit usage protocol is decoded. Conversation payloads,
        // image data and ordinary WebSocket frames pass through unchanged.
        let decoded;
        try { decoded = JSON.parse(raw); } catch { return raw; }
        const adapted = frame(decoded);
        if (adapted !== decoded) next = JSON.stringify(adapted);
      } else if (record(raw) || Array.isArray(raw)) next = frame(raw);
      events.set(this, next); return next;
    };
    Object.defineProperty(proto, 'data', {...descriptor, get});
    cleanups.push(() => { if (Object.getOwnPropertyDescriptor(proto, 'data')?.get === get)
      Object.defineProperty(proto, 'data', descriptor); });
  }
  function usageRequest(url, options) {
    const path = typeof url === 'string' ? url.split('?')[0] : '';
    if (path !== '/wham/usage') return false;
    const headers = new root.Headers(options?.headers);
    return headers.get('OAI-Image-Generation-Limit-Reached') !== 'true';
  }
  function httpPrototype(require) {
    const namespaces = typeof require === 'function' && record(require.c)
      ? Object.values(require.c).map(module => module?.exports) : [require];
    for (const exports of namespaces) {
      if (!record(exports)) continue;
      for (const key of Object.keys(exports)) {
        let value;
        // A concurrently initializing module can still have a TDZ export.
        try { value = exports[key]; } catch { continue; }
        const proto = value?.prototype;
        if (typeof value === 'function' && typeof value.getInstance === 'function'
          && typeof proto?.get === 'function' && typeof proto.cancelStream === 'function'
          && typeof proto.sendRequest === 'function') return proto;
      }
    }
    return null;
  }
  function attachRuntime(require) {
    if (!enabled()) return {ok: false, reason: 'disabled'};
    if (installed) return {ok: true};
    const proto = httpPrototype(require);
    if (!proto) return {ok: false, reason: 'quota-http-boundary-unavailable'};
    const descriptor = Object.getOwnPropertyDescriptor(proto, 'fetch');
    const sendDescriptor = Object.getOwnPropertyDescriptor(proto, 'sendRequest'), send = proto.sendRequest;
    if (!sendDescriptor || !sendDescriptor.configurable && !sendDescriptor.writable
      || descriptor && !descriptor.configurable && !descriptor.writable)
      return {ok: false, reason: 'quota-http-boundary-readonly'};
    const original = proto.fetch;
    const fetch = async function (url, options) {
      const response = await original.call(this, url, options);
      if (!enabled() || !usageRequest(url, options) || !response.ok || response.status === 204) return response;
      const text = await response.text(), raw = JSON.parse(text), next = usage(raw);
      const headers = new root.Headers(response.headers);
      if (next !== raw) { headers.delete('content-length'); headers.delete('content-encoding'); }
      const result = new root.Response(next === raw ? text : JSON.stringify(next), {
        status: response.status, statusText: response.statusText, headers});
      for (const key of ['url', 'redirected', 'type']) Object.defineProperty(result, key, {value: response[key]});
      return result;
    };
    try { installMessages(); }
    catch (error) { return {ok: false, reason: error.message}; }
    // Current desktop uses Response streams; the shipped named-module Windows
    // build returns parsed IPC responses directly through sendRequest.
    if (descriptor) {
      Object.defineProperty(proto, 'fetch', {...descriptor, value: fetch});
      cleanups.push(() => { if (proto.fetch === fetch) Object.defineProperty(proto, 'fetch', descriptor); });
    }
    const sendRequest = async function (method, url, options) {
      const response = await send.call(this, method, url, options);
      if (!enabled() || !usageRequest(url, options) || !(response?.status >= 200 && response.status < 300)) return response;
      const next = usage(response?.body);
      return next === response?.body ? response : {...response, body: next};
    };
    Object.defineProperty(proto, 'sendRequest', {...sendDescriptor, value: sendRequest});
    cleanups.push(() => { if (proto.sendRequest === sendRequest) Object.defineProperty(proto, 'sendRequest', sendDescriptor); });
    installed = true;
    return {ok: true};
  }
  const mainQuery = query => {
    const key = query.queryKey;
    return Array.isArray(key) && ['rate-limit-status', 'rate-limit-status-pending', 'usage-settings-refresh'].includes(key[0])
      && key[1] !== 'image-generation';
  };
  const mainQueries = client => client.getQueryCache().findAll({predicate: mainQuery}).filter(mainQuery);
  function attachCache(client) {
    if (!enabled() || !client?.getQueryCache || !client?.setQueryData) return false;
    if (cleanups.some(cleanup => cleanup.client === client)) return true;
    const cache = client.getQueryCache();
    let writing = false;
    const rewrite = query => {
      if (writing || !enabled() || !mainQuery(query)) return;
      const raw = query.state?.data, next = usage(raw);
      if (next === raw) return;
      writing = true;
      try { client.setQueryData(query.queryKey, next, {updatedAt: query.state.dataUpdatedAt}); }
      finally { writing = false; }
    };
    const unsubscribe = cache.subscribe(event => {
      if (event?.query && (event.type === 'updated' || event.type === 'added')) rewrite(event.query);
    });
    for (const query of mainQueries(client)) rewrite(query);
    const cleanup = () => {
      unsubscribe();
      for (const query of mainQueries(client)) {
        const raw = originals.get(query.state?.data);
        if (raw) client.setQueryData(query.queryKey, raw, {updatedAt: query.state.dataUpdatedAt});
      }
    };
    cleanup.client = client; cleanups.push(cleanup); return true;
  }
  function findCache() {
    const element = root.document?.getElementById?.('root');
    const key = element && Object.keys(element).find(key => key.startsWith('__reactContainer$') || key.startsWith('__reactFiber$'));
    let fiber = element?.[key];
    if (fiber?.stateNode?.current) fiber = fiber.stateNode.current;
    const queue = fiber ? [fiber] : [], seen = new Set();
    for (let count = 0; queue.length && count < 10000; count++) {
      const node = queue.pop();
      if (!node || seen.has(node)) continue;
      seen.add(node);
      for (const value of [node.memoizedProps?.client, node.memoizedProps?.value, node.stateNode])
        if (value?.getQueryCache && value?.setQueryData) return value;
      if (node.child) queue.push(node.child);
      if (node.sibling) queue.push(node.sibling);
    }
    return null;
  }
  async function install() {
    if (!enabled()) return {ok: false, reason: 'disabled'};
    if (pending) return pending;
    const epoch = generation;
    let cancelled = false, timer;
    const work = (async () => {
      if (root.document?.readyState === 'loading') await new Promise(resolve => {
        root.document.addEventListener('DOMContentLoaded', resolve, {once: true});
      });
      const entries = [...(root.document?.scripts ?? [])].map(script => script.src)
        .filter(url => /^app:\/\/-\/assets\/[\w.-]+\.js$/.test(url));
      let require;
      for (const url of entries) {
        const module = await import(url);
        if (typeof module.__webpack_require__ === 'function') { require = module.__webpack_require__; break; }
        if (httpPrototype(module)) { require = module; break; }
      }
      if (!require) {
        const loaded = [...(root.performance?.getEntriesByType?.('resource') ?? [])].map(entry => entry.name);
        for (const url of loaded.filter(url => /^app:\/\/-\/assets\/(?:setting-storage|app-initial)-[\w-]+\.js$/.test(url))) {
          const module = await import(url);
          if (httpPrototype(module)) { require = module; break; }
        }
      }
      if (!require) return {ok: false, reason: 'quota-runtime-unavailable'};
      // App-service modules finish loading asynchronously after the entry.
      // Poll a bounded number of times without holding a renderer/backend lock.
      for (let attempt = 0; attempt < 20; attempt++) {
        if (cancelled || !enabled() || generation !== epoch) return {ok: false, reason: 'disabled'};
        const result = attachRuntime(require);
        if (result.ok && attachCache(findCache())) return result;
        if (result.ok && attempt === 19) return result;
        if (!result.ok && result.reason !== 'quota-http-boundary-unavailable') return result;
        await new Promise(resolve => root.setTimeout(resolve, 250));
      }
      return {ok: false, reason: 'quota-http-boundary-unavailable'};
    })();
    pending = Promise.race([work.catch(() => ({ok: false, reason: 'quota-runtime-unavailable'})), new Promise(resolve => {
      timer = root.setTimeout(() => { cancelled = true; resolve({ok: false, reason: 'quota-data-timeout'}); }, 10000);
    })]).then(result => {
      if (enabled() && generation === epoch) {
        const state = root.__cockpitEnhancements;
        state.nativeQuotaPatchState = result.ok ? 'active' : 'unsupported';
        state.nativeQuotaPatchReason = result.ok ? 'quota-data-adapted' : result.reason;
      }
      return result;
    }).finally(() => { root.clearTimeout(timer); pending = null; });
    return pending;
  }
  function dispose() {
    generation++;
    const clients = cleanups.map(cleanup => cleanup.client).filter(Boolean);
    for (const cleanup of cleanups.splice(0).reverse()) cleanup();
    installed = false;
    // Stream/IPC data may already have been decoded into new objects by the
    // native client. Once the binding is revoked, re-read these active queries
    // from the original transport instead of retaining their adapted view.
    root.queueMicrotask(() => {
      if (enabled()) return;
      for (const client of clients) if (typeof client.invalidateQueries === 'function') {
        for (const prefix of ['rate-limit-status', 'rate-limit-status-pending', 'usage-settings-refresh'])
          client.invalidateQueries({queryKey: [prefix], refetchType: 'active'}).catch(() => {
            root.console?.warn('[Cockpit quota] quota-source-refresh-failed');
          });
      }
    });
  }
  root.__cockpitQuotaState = {usage, frame, attachRuntime, attachCache, install, dispose};
})();
