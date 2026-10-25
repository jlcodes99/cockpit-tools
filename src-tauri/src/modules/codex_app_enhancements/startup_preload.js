// Windows Codex treats a renderer reload as a new cold start. Prepare the
// transformed modules before the first application script is requested so the
// normal window can activate without navigating a second time.
(() => {
  const root = typeof window === 'object' ? window : globalThis;
  if (root.__cockpitStartupPreloadStarted) return;
  root.__cockpitStartupPreloadStarted = true;
  const record = value => { root.__cockpitStartupPreload = value; return value; };
  const fail = reason => record({ ok: false, reason });
  const local = () => root.location?.protocol === 'app:'
    && root.location?.hostname === '-'
    && root.location?.pathname === '/index.html';
  if (!local()) return;
  if (root.document?.documentElement) return fail('document-already-started');
  const read = url => {
    const request = new root.XMLHttpRequest();
    request.open('GET', url, false);
    request.send();
    if (request.status && (request.status < 200 || request.status >= 300))
      throw new Error('asset-unavailable');
    if (typeof request.responseText !== 'string' || !request.responseText)
      throw new Error('asset-unavailable');
    return request.responseText;
  };
  try {
    const fast = root.__cockpitNativeFast, quota = root.__cockpitQuotaCompat;
    if (!fast || !quota) return fail('runtime-unavailable');
    const indexUrl = 'app://-/index.html';
    const html = read(indexUrl);
    const entries = [...html.matchAll(/<script\b[^>]*\bsrc=(["'])(.*?)\1/g)]
      .map(match => new URL(match[2], indexUrl).href)
      .filter(url => fast.localAsset(url, true));
    if (entries.length < 1 || entries.length > 4) return fail('unsupported-entry');
    const candidates = new Set();
    for (const entry of entries) {
      for (const match of read(entry).matchAll(/["'`]\.\/app-initial-[A-Za-z0-9_-]+\.js["'`]/g))
        candidates.add(new URL(match[0].slice(1, -1), entry).href);
    }
    if (candidates.size !== 1) return fail('ambiguous-module');
    const initialUrl = [...candidates][0], initialSource = read(initialUrl);
    let initial = fast.prepareLive(initialSource, initialUrl);
    if (!initial.ok) return fail(initial.diagnostics?.reason || 'initial-transform-failed');
    const modules = [];
    if ((initial.mode || 'full') === 'full' && (root.__cockpitEnhancements?.quotaCompat === true
      || root.__cockpitEnhancements?.apiService === true)) {
      try {
        const contract = quota.transform(initialSource, initialUrl);
        if (!contract.ok) throw Error(contract.diagnostics?.reason || 'quota-contract-failed');
        const primaryUrls = new Set(
          [...initialSource.matchAll(/["'`]\.\/app-primary-[A-Za-z0-9_-]+\.js["'`]/g)]
            .map(match => new URL(match[0].slice(1, -1), initialUrl).href),
        );
        if (primaryUrls.size !== 1) throw Error('primary-module-not-unique');
        const primaryUrl = [...primaryUrls][0];
        const primary = quota.transformComposer(
          read(primaryUrl), primaryUrl, initialUrl, contract.diagnostics?.bindings,
        );
        if (!primary.ok) throw Error(primary.diagnostics?.reason || 'composer-transform-failed');
        const currentLayout = contract.diagnostics?.bindings?.layout === 'adapter-v1';
        const primaryKind = currentLayout ? 'quota-primary' : 'quota';
        const prepared = `${fast.rewriteModuleSync(primary.patchedSource, primaryUrl)}
;window.__cockpitNativeFastComplete?.(${JSON.stringify(primaryKind)});
//# sourceURL=${primaryUrl}
`;
        modules.push({ url: primaryUrl, source: prepared });
        if (currentLayout) {
          const adapterUrl = new URL(contract.diagnostics.bindings.adapterPath, initialUrl).href;
          if (!fast.localAdapter(adapterUrl, initialUrl)) throw Error('quota-adapter-url-invalid');
          const adapter = quota.transformAdapter(read(adapterUrl), adapterUrl, initialUrl,
            primaryUrl, primary.diagnostics?.bindings);
          if (!adapter.ok) throw Error(adapter.diagnostics?.reason || 'quota-adapter-contract-mismatch');
          const preparedAdapter = `${fast.rewriteModuleSync(adapter.patchedSource, adapterUrl)}
;window.__cockpitNativeFastComplete?.("quota");
//# sourceURL=${adapterUrl}
`;
          modules.push({ url: adapterUrl, source: preparedAdapter });
        }
      } catch (error) {
        const reason = typeof error?.message === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(error.message)
          ? error.message : 'quota-primary-unavailable';
        // Restart from the untouched initial module, never publish a partial
        // quota patch after the primary module failed its own contract.
        initial = fast.prepareFastOnly(initialSource, initialUrl, reason);
        if (!initial.ok) return fail(initial.diagnostics?.reason || 'initial-transform-failed');
        modules.length = 0;
      }
    }
    const initialPrepared = `${fast.rewriteModuleSync(initial.patchedSource, initialUrl)}
;window.__cockpitNativeFastComplete?.();
//# sourceURL=${initialUrl}
`;
    const mode = initial.mode || 'full';
    const installed = fast.installModule(initialPrepared, initialUrl, modules, mode, initial.quotaReason ?? null);
    return record({
      ok: installed.ok,
      reason: installed.reason || 'preloaded-before-document',
      initialUrl,
      primaryUrls: modules.map(module => module.url),
      ...(mode === 'fast-only' ? { mode, quotaReason: initial.quotaReason } : {}),
    });
  } catch {
    return fail('startup-preload-failed');
  }
})();
