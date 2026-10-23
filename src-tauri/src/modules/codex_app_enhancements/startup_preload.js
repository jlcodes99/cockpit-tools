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
    const initial = fast.prepareLive(initialSource, initialUrl);
    if (!initial.ok) return fail(initial.diagnostics?.reason || 'initial-transform-failed');
    const initialPrepared = `${fast.rewriteModuleSync(initial.patchedSource, initialUrl)}
;window.__cockpitNativeFastComplete?.();
//# sourceURL=${initialUrl}
`;
    const modules = [];
    if (root.__cockpitEnhancements?.quotaCompat === true
      || root.__cockpitEnhancements?.apiService === true) {
      const contract = quota.transform(initialSource, initialUrl);
      if (!contract.ok) return fail(contract.diagnostics?.reason || 'quota-contract-failed');
      const primaryUrls = new Set(
        [...initialSource.matchAll(/["'`]\.\/app-primary-[A-Za-z0-9_-]+\.js["'`]/g)]
          .map(match => new URL(match[0].slice(1, -1), initialUrl).href),
      );
      if (primaryUrls.size !== 1) return fail('primary-module-not-unique');
      const primaryUrl = [...primaryUrls][0];
      const primary = quota.transformComposer(
        read(primaryUrl), primaryUrl, initialUrl, contract.diagnostics?.bindings,
      );
      if (!primary.ok) return fail(primary.diagnostics?.reason || 'composer-transform-failed');
      const prepared = `${fast.rewriteModuleSync(primary.patchedSource, primaryUrl)}
;window.__cockpitNativeFastComplete?.("quota");
//# sourceURL=${primaryUrl}
`;
      modules.push({ url: primaryUrl, source: prepared });
    }
    const installed = fast.installModule(initialPrepared, initialUrl, modules, 'full');
    return record({
      ok: installed.ok,
      reason: installed.reason || 'preloaded-before-document',
      initialUrl,
      primaryUrls: modules.map(module => module.url),
    });
  } catch {
    return fail('startup-preload-failed');
  }
})();
