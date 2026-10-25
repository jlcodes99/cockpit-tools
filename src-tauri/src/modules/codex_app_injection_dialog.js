// Serialized with the statusline wrapper; all strings and data come from the host.
(() => {
  const esc = value => String(value ?? '').replace(/[&<>"']/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
  const label = key => usageLabels[key] || missingLabel;
  const finite = value => typeof value === 'number' && Number.isFinite(value) && value >= 0;
  const number = value => finite(value) ? value.toLocaleString(undefined, {maximumFractionDigits: value >= 1000 ? 1 : 0, notation: value >= 1000 ? 'compact' : 'standard'}) : missingLabel;
  const ms = value => !finite(value) ? missingLabel : value < 1000 ? Math.round(value) + 'ms' : (value / 1000).toFixed(2) + 's';
  const usd = value => !finite(value) ? missingLabel : value === 0 ? '$0.00' : value < 0.000001 ? '<$0.000001' : '$' + value.toFixed(value < 0.01 ? 6 : value < 1 ? 4 : 2);
  const percentage = value => finite(value) ? Math.round(value) + '%' : missingLabel;
  // Keep in parity with getCodexQuotaClass / getGrokQuotaClass in account overview.
  const quotaTone = (value, kind = 'codex') => !finite(value) ? null : kind === 'grok'
    ? value <= 10 ? 'critical' : value <= 30 ? 'low' : value <= 60 ? 'medium' : 'high'
    : value >= 80 ? 'high' : value >= 40 ? 'medium' : value >= 10 ? 'low' : 'critical';
  const quotaPercentage = (value, kind = 'codex') => '<span class="quota-percent"' + (quotaTone(value,kind) ? ' data-cockpit-quota-tone="' + quotaTone(value,kind) + '"' : '') + '>' + esc(percentage(value)) + '</span>';
  const badge = value => {
    if (typeof value !== 'string' || !value.trim()) return '';
    const key = value.trim().toLowerCase().replace(/^chatgpt[-_\s]?/, '').replace(/[-_\s]?plan$/, '');
    // Match API Service's known subscription categories. The user explicitly
    // requested these short labels; retain the raw source in the tooltip.
    const known = {plus:['PLUS','plus'],team:['TEAM','team'],teams:['TEAM','team'],business:['BUSINESS','team'],pro:['PRO','pro'],free:['FREE','free'],enterprise:['ENTERPRISE','enterprise'],edu:['EDU','team'],go:['GO','plus'],api_key:['API','unknown']};
    const [text,tone] = Object.hasOwn(known,key) ? known[key] : [value,'unknown'];
    return '<span class="badge ' + tone + '" title="' + esc(value) + '">' + esc(text) + '</span>';
  };
  const dot = tone => '<i class="dot ' + tone + '" aria-hidden="true"></i>';
  const metricValue = value => {
    const parts = String(value).match(/^(.*?)\s*(tok\/s|%|ms|s)$/);
    return parts ? esc(parts[1]) + '<small class="metric-unit">' + (parts[2] === '%' ? '' : ' ') + esc(parts[2]) + '</small>' : esc(value);
  };
  const metric = (name, value) => '<dl class="session-metric"><dt>' + esc(name) + '</dt><dd>' + metricValue(value) + '</dd></dl>';
  function apiKeyCount(root) {
    const pool = root.usageState?.__accountPool;
    if (typeof pool?.binding !== 'string' || !pool.binding) {
      delete root.poolState;
      return null;
    }
    if (root.poolState?.binding !== pool.binding) root.poolState = {binding:pool.binding,count:null};
    if (Number.isSafeInteger(pool.gptApiKeyCount) && pool.gptApiKeyCount >= 0) root.poolState.count = pool.gptApiKeyCount;
    else if (pool.pending !== true) root.poolState.count = null;
    // The scope cache survives chat changes and host disconnection, but never
    // a different service/key binding or an explicitly unavailable snapshot.
    return root.poolState.count;
  }
  const statuses = () => !finite(accountCount) || accountCount === 0 ? '' : '<div class="health"><span>ChatGPT</span><span>' + dot('green') + esc(label('available')) + ' <strong>' + number(availableAccountCount) + '/' + number(accountCount) + '</strong></span><span>' + dot('') + esc(label('issues')) + ' <strong>' + number(abnormalAccountCount) + '</strong></span><span>' + dot('') + esc(label('cooling')) + ' <strong>' + number(cooldownAccountCount) + '</strong></span></div>';
  const segmented = (name, choices, selected) => '<div class="segmented">' + choices.map(([value,key]) => '<button type="button" data-cockpit-dialog-action="' + name + '" data-value="' + value + '" aria-pressed="' + (value === selected) + '">' + esc(label(key)) + '</button>').join('') + '</div>';
  const binding = root => typeof root.usageState?.__binding?.binding === 'string' ? root.usageState.__binding.binding : null;
  const retryDelays = [2000, 5000, 10000];

  function state(root) {
    if (!root.dialogState || root.dialogState.threadId !== (root.threadId || null) || root.dialogState.binding !== binding(root)) {
      if (root.dialogState) cancel(root);
      root.dialogState = {threadId:root.threadId || null, binding:binding(root), tab:'overview', scope:'chat', period:'week', rows:[], nextCursor:null,
        summary:null, session:null, pending:null, handled:null, loaded:false, available:null, unavailableReason:null,
        error:false, requestedAt:0, paged:false, retryCount:0, retryAt:null, hostAvailable:root.hostAvailable !== false,
        scopeCaches:Object.create(null), summaryCaches:Object.create(null)};
    }
    return root.dialogState;
  }
  function issue(root, cursor = null, automaticRetry = false) {
    const s = state(root);
    if (!automaticRetry) s.retryCount = 0;
    s.retryAt = null;
    s.pending = {requestToken:Date.now().toString(36) + '-' + Math.random().toString(36).slice(2),
      threadId:s.threadId,binding:s.binding,scope:s.scope,period:s.period,cursor,view:s.tab};
    s.requestedAt = Date.now();
    s.error = false;
    if (s.available === false) s.available = null;
    s.unavailableReason = null;
    // The persistent CDP binding is optional. A disconnected binding leaves the
    // same token available to the bounded host poller, rather than losing it.
    if (typeof window.__cockpitUsageRequest === 'function') {
      try { window.__cockpitUsageRequest(JSON.stringify(s.pending)); } catch { /* Poll fallback. */ }
    }
  }
  function scheduleRetry(s) {
    s.retryAt = s.retryCount < retryDelays.length ? Date.now() + retryDelays[s.retryCount] : null;
  }
  function fail(s) {
    s.error = true;
    s.handled = s.pending.requestToken;
    scheduleRetry(s);
  }
  function accept(root) {
    const s = state(root), q = s.pending;
    if (!q) return;
    const response = root.usageState?.__dialog?.[q.requestToken];
    if (!response || response.loading || s.handled === q.requestToken) return;
    if (response.requestToken !== q.requestToken) return;
    // Never show a response from a former chat, scope, period or pagination request.
    if (['threadId','binding','scope','period','cursor','view'].some(key => (response[key] ?? null) !== (q[key] ?? null))) return;
    if (response.error) { fail(s); return; }
    s.handled = q.requestToken;
    s.available = response.available === true;
    s.unavailableReason = response.unavailableReason || null;
    // Unavailability is not an empty successful query. Keep only the already
    // verified snapshot in this exact thread/service binding and scope.
    if (!s.available) {
      if (s.unavailableReason === 'service_unavailable') scheduleRetry(s);
      return;
    }
    s.retryCount = 0;
    s.retryAt = null;
    const page = response.requests;
    if (q.view === 'logs' && page && Array.isArray(page.rows)) {
      if (q.cursor == null) s.rows = [];
      const known = new Set(s.rows.map(row => row.id));
      for (const row of page.rows) if (row && !known.has(row.id)) { s.rows.push(row); known.add(row.id); }
      s.nextCursor = typeof page.nextCursor === 'string' && page.nextCursor !== q.cursor ? page.nextCursor : null;
      s.loaded = true;
    }
    if (q.view === 'overview') {
      s.summary = response.summary || null;
      s.summaryCaches[s.period] = s.summary;
    }
    if (q.view === 'overview') s.session = response.session?.threadId === s.threadId && s.threadId ? response.session : null;
  }
  function query(root) {
    if (!root.quotaDetailsOpen) return null;
    const s = state(root);
    accept(root);
    const wasAvailable = s.hostAvailable;
    s.hostAvailable = root.hostAvailable !== false;
    if (!s.hostAvailable) return null;
    // A restored bridge must use a fresh token: a former failed token stays
    // settled in both the renderer and host cache. Keep the verified snapshot.
    if (!wasAvailable && s.pending && (s.error || s.available === false || loading(s))) {
      issue(root,s.pending.cursor);
    }
    if (loading(s) && Date.now() - s.requestedAt > 10000) {
      fail(s);
    }
    const completed = !s.pending || s.handled === s.pending.requestToken;
    if (completed && s.retryAt !== null && Date.now() >= s.retryAt) {
      s.retryCount += 1;
      issue(root,s.pending?.cursor || null,true);
    }
    // Do not refresh a paginated snapshot underneath the reader. A manual refresh starts a new one.
    else if (!s.pending || (completed && !s.error && s.available !== false && !s.paged && Date.now() - s.requestedAt > 5000)) issue(root);
    return s.pending;
  }
  function loading(s) { return Boolean(s.pending && s.handled !== s.pending.requestToken); }
  function cancel(root) {
    if (typeof window.__cockpitUsageRequest === 'function') {
      try { window.__cockpitUsageRequest('null'); } catch { /* Disconnected host. */ }
    }
    if (root.dialogState) {
      root.dialogState.pending = null;
      root.dialogState.retryCount = 0;
      root.dialogState.retryAt = null;
    }
  }
  function reset(root) { cancel(root); delete root.dialogState; }
  function refresh(root) {
    const s = state(root);
    s.paged = false;
    s.resetScroll = true;
    issue(root);
  }
  function rowSpeed(row) {
    return row.success === true && !row.errorCategory && Number.isSafeInteger(row.outputTokens) && row.outputTokens >= 100
      && Number.isSafeInteger(row.outputGenerationMs) && row.outputGenerationMs >= 100
      ? row.outputTokens / (row.outputGenerationMs / 1000) : null;
  }
  function rate(value) { return finite(value) && value > 0 ? (value >= 1 ? String(Math.round(value)) : value < 0.1 ? '< 0.1' : value.toFixed(1)) + ' tok/s' : missingLabel; }
  function metricList(items) { return '<dl class="cell-metrics">' + items.map(([key,value]) => '<div><dt>' + esc(label(key)) + '</dt><dd>' + esc(value) + '</dd></div>').join('') + '</dl>'; }
  function tiers(row) {
    // Keep parity with requestServiceTierPresentation and the explicit
    // recordedServiceTierAdaptation evidence used by API Service.
    const requested = String(row.requestedServiceTier || '').trim();
    const response = String(row.responseServiceTier || '').trim();
    const legacy = !requested && !response ? String(row.serviceTier || '').trim() : '';
    const value = row.serviceTierAdaptation;
    const adaptation = value?.reason === 'official_default_only' && value.sentTier === 'default'
      && typeof value.requestedTier === 'string' && value.requestedTier.trim() ? value : null;
    const tierLabel = tier => {
      const keys = {priority:'tierFast',fast:'tierFast',ultrafast:'tierUltrafast',standard:'tierStandard',default:'tierStandard',auto:'tierAuto'};
      return tier ? Object.hasOwn(keys,tier.toLowerCase()) ? label(keys[tier.toLowerCase()]) : tier : label('tierUnknown');
    };
    const entry = (key,tier) => '<span class="cell-tier" title="' + esc(label('tierHint')) + '">'
      + esc(label(key).replace(/\{\{\s*tier\s*\}\}/g, key === 'requestTier' ? tierLabel(tier) : tier || label('tierUnknown'))) + '</span>';
    return entry('requestTier',adaptation?.requestedTier || requested) + entry('responseTier',response)
      + (legacy ? entry('legacyTier',legacy) : '')
      + (adaptation ? '<span class="cell-tier adjusted" title="' + esc(label('tierHint')) + '">' + esc(label('tierAdjusted')) + '</span>' : '');
  }
  function requestRow(row) {
    const date = finite(row.timestamp) && row.timestamp > 0 ? new Date(row.timestamp) : null;
    const timestamp = date && Number.isFinite(date.getTime()) ? date : null;
    const category = String(row.errorCategory || '').trim().toLowerCase();
    const outcome = row.httpStatus === 499 || ['client_canceled','client_cancelled'].includes(category) ? 'cancelled' : category === 'stream_incomplete' ? 'incomplete' : row.success ? 'success' : 'failed';
    const authorization = row.authorizationRoute || (['official','local_gateway'].includes(row.transport) ? row.transport : null);
    const route = authorization === 'official' ? label('official') : authorization === 'local_gateway' ? label('local')
      : row.accountId ? label(String(row.accountId).startsWith('chatgpt_plan_') ? 'official' : 'local') : missingLabel;
    const address = typeof row.accountEmail === 'string' ? row.accountEmail.trim() : '';
    const visibleAddress = !/^(?:codex_|chatgpt_plan_|api-key-[a-f0-9]{8}$)/i.test(address) || address.includes('@') ? address : '';
    const email = row.accountDisplayName || visibleAddress || missingLabel;
    return '<tr class="request-row"><td><span class="cell-date">' + esc(timestamp ? timestamp.toLocaleDateString(undefined, {month:'2-digit',day:'2-digit'}) : missingLabel)
      + '</span><time class="cell-time">' + esc(timestamp ? timestamp.toLocaleTimeString(undefined, {hour12:false}) : missingLabel) + '</time></td>'
      + '<td><span class="cell-email" title="' + esc(email) + '">' + esc(email) + '</span>' + badge(row.plan) + '<span class="cell-route">' + esc(route) + '</span></td>'
      + '<td><span class="cell-model" title="' + esc(row.requestedModel || row.model || missingLabel) + '">' + esc(row.requestedModel || row.model || missingLabel) + '</span>'
      + (row.upstreamModel ? '<span class="cell-upstream" title="' + esc(row.upstreamModel) + '">↳ ' + esc(row.upstreamModel) + '</span>' : '') + '<span class="cell-muted">' + esc(row.reasoningEffort || missingLabel) + '</span>' + tiers(row) + '</td>'
      + '<td>' + metricList([['input',number(row.inputTokens)],['output',number(row.outputTokens)],['cached',number(row.cachedTokens)],['total',number(row.totalTokens)]]) + '</td>'
      + '<td>' + metricList([['firstResponse',ms(row.firstResponseMs)],['totalDuration',ms(row.latencyMs)],['speed',rate(rowSpeed(row))]]) + '</td>'
      + '<td><span class="cell-result ' + (outcome === 'success' ? '' : 'failed') + '">' + dot(outcome === 'success' ? 'green' : '') + esc(label(outcome)) + '</span>'
      + '<span class="cell-code">' + esc(row.httpStatus ? 'HTTP ' + row.httpStatus : missingLabel) + '</span>'
      + (row.recovered ? '<span class="cell-recovery">↳ ' + esc(label('recovered')) + '</span>' : '')
      + (row.errorCategory ? '<span class="cell-error" title="' + esc(row.errorMessage || row.errorCategory) + '">' + esc(row.errorCategory) + '</span>' : '') + '</td></tr>';
  }
  function quotaContent(root, input) {
    const {badgeKind:kind,formatMoney,shortProductLabel} = input;
    if (kind === 'deepseek') {
      const lines = balanceLines.length ? balanceLines : [{currency:null,totalBalance:null}];
      return resourceTable([label('provider'),accountPoolLabel,balanceLabel],lines.map(line => [
        'DeepSeek' + (line.currency ? ' · ' + line.currency : ''),number(line.accountCount),formatMoney(line.totalBalance,line.currency)
      ]));
    }
    if (kind === 'grok') {
      return grokLines.length ? resourceTable([label('provider'),accountPoolLabel,quotaOverviewLabel],grokLines.map(line => [
        'Grok · ' + shortProductLabel(line.product),number(line.count),percentage(line.remainingPercent)
      ]),grokLines.map(line => quotaTone(line.remainingPercent,'grok'))) : '<p class="empty-state">' + esc(grokEmptyLabel) + '</p>';
    }
    // API_KEY in the generic quota reply includes other providers. Only the
    // routing-aware backend count can establish how many actually supply GPT.
    const apiCount = apiKeyCount(root);
    const gptPlans = plans.filter(plan => String(plan.plan).toUpperCase() !== 'API_KEY' || apiCount !== 0)
      .map(plan => String(plan.plan).toUpperCase() === 'API_KEY' ? {...plan,count:apiCount} : plan);
    if (apiCount > 0 && !gptPlans.some(plan => String(plan.plan).toUpperCase() === 'API_KEY')) gptPlans.push({plan:'API_KEY',count:apiCount});
    const table = gptPlans.length ? '<table class="plan-table"><thead><tr>' + [planLabel,accountPoolLabel,fiveHourLabel,weeklyLabel].map(value => '<th scope="col">' + esc(value) + '</th>').join('') + '</tr></thead><tbody>'
      + gptPlans.map(plan => '<tr><td>' + badge(plan.plan) + (plan.official ? '<span class="cell-muted">' + esc(officialLabel) + '</span>' : '') + '</td><td>' + number(plan.count) + '</td><td>' + quotaPercentage(plan.fiveHourRemainingPercent) + '</td><td>' + quotaPercentage(plan.weeklyRemainingPercent) + '</td></tr>').join('') + '</tbody></table>' : '<p class="empty-state">' + esc(quotaEmptyLabel) + '</p>';
    return table + statuses();
  }
  function resourceTable(headers, rows, tones = []) {
    return '<table class="resource-table"><thead><tr>' + headers.map(value => '<th scope="col">' + esc(value) + '</th>').join('')
      + '</tr></thead><tbody>' + rows.map((row,index) => '<tr>' + row.map((value,column) => '<td>' + (column === 2 && tones[index] ? '<span class="quota-percent" data-cockpit-quota-tone="' + tones[index] + '">' + esc(value) + '</span>' : esc(value)) + '</td>').join('') + '</tr>').join('') + '</tbody></table>';
  }
  function overview(root, input) {
    const s = state(root), summary = s.summary, session = s.session;
    const sessionModel = session?.requestedModel || session?.model || missingLabel;
    const statsTitle = summary?.incomplete ? label('verifiedStatistics') : label('statistics');
    const statsHint = summary?.incomplete ? label('excludedHistory').replace(/\{\{\s*count\s*\}\}/g, String(summary.excludedUnknownRequests || 0)) : '';
    const successRate = summary && summary.requests > 0 && finite(summary.successRequests) ? percentage(summary.successRequests / summary.requests * 100) : missingLabel;
    const cost = usd(summary?.estimatedCostUsd);
    const range = summary && finite(summary.sinceMs) && finite(summary.untilMs) ? new Date(summary.sinceMs).toLocaleDateString() + ' — ' + new Date(Math.max(summary.sinceMs,summary.untilMs - 1)).toLocaleDateString() : missingLabel;
    return '<div class="overview-sections"><section class="business-section quota-block" data-resource-kind="' + esc(input.badgeKind) + '"><div class="business-label"><h3 class="section-title">' + dot('purple') + esc(input.badgeKind === 'deepseek' ? balanceLabel : label('quotaTitle')) + '</h3></div><div class="business-content">' + quotaContent(root,input) + '</div></section>'
      + '<section class="business-section session-block"><div class="business-label"><h3 class="section-title">' + dot('blue') + esc(label('session')) + '</h3></div><div class="business-content session-metrics"><dl class="session-metric model"><dt>' + esc(label('model')) + '</dt><dd title="' + esc(sessionModel) + '">' + esc(sessionModel) + '</dd>'
      + (session?.upstreamModel && session.upstreamModel !== sessionModel ? '<dd class="metric-sub" title="' + esc(session.upstreamModel) + '">↳ ' + esc(session.upstreamModel) + '</dd>' : '') + '<dd class="metric-sub">' + esc(session?.reasoningEffort || '') + '</dd></dl>'
      + metric(label('averageSpeed'),input.averageSpeed || missingLabel) + metric(label('averageCache'),input.averageCache || missingLabel) + metric(label('firstResponse'),ms(session?.firstResponseMs)) + '</div></section>'
      + '<section class="business-section stats-block"><div class="business-label"><h3 class="section-title" title="' + esc(statsHint) + '">' + dot('green') + esc(statsTitle) + '</h3>' + segmented('period',[['today','periodToday'],['week','periodWeek']],s.period) + '</div><div class="business-content"><div class="stats-grid">'
      + '<dl><dt>' + esc(label('requestsCount')) + '</dt><dd>' + number(summary?.requests) + '</dd><dd class="stats-sub">' + esc(label('successRate')) + ' ' + esc(successRate) + '</dd></dl>'
      + '<dl><dt>' + esc(label('totalTokens')) + '</dt><dd>' + number(summary?.totalTokens) + '</dd></dl><dl><dt>' + esc(label('estimatedValue')) + '</dt><dd class="value">' + esc(cost) + '</dd></dl><dl><dt>' + esc(label('avgLatency')) + '</dt><dd>' + metricValue(ms(summary?.averageLatencyMs)) + '</dd></dl></div><p class="stats-date">' + esc(range) + '</p></div></section></div>';
  }
  function logs(root) {
    const s = state(root), busy = loading(s);
    const empty = root.hostAvailable === false ? label('hostUnavailable') : s.error ? label('dataFailed') : busy ? label('loading')
      : !s.threadId && s.scope === 'chat' || s.unavailableReason === 'unlinked_session' ? label('unlinkedSession')
      : s.available === false ? label('serviceUnavailable') : label('noRequests');
    return '<section class="logs-view"><div class="log-toolbar">' + segmented('scope',[['chat','scopeChat'],['service','scopeService']],s.scope) + '<span class="log-caption">' + esc(label('newestFirst')) + '</span></div>'
      + '<div class="request-scroll" role="region" tabindex="0" aria-label="' + esc(label('requestsTitle')) + '"><table class="request-table"><colgroup>' + ['time','account','model','token','performance','result'].map(name => '<col class="' + name + '-col">').join('') + '</colgroup><thead><tr>'
      + ['requestTime','account','model','token','performance','result'].map(key => '<th scope="col">' + esc(label(key)) + '</th>').join('') + '</tr></thead><tbody>' + s.rows.map(requestRow).join('')
      + (!s.rows.length ? '<tr><td colspan="6" class="empty-state">' + esc(empty) + '</td></tr>' : '') + '</tbody></table></div><div class="load-more-row"><span class="records-count" role="status" aria-live="polite">'
      + esc(label('shownRows').replace(/\{\{\s*count\s*\}\}/g, String(s.rows.length))) + '</span><button class="load-more" type="button" data-cockpit-dialog-action="more"' + (busy || !s.nextCursor || root.hostAvailable === false ? ' disabled' : '') + '>'
      + esc(busy ? label('loading') : s.nextCursor || s.available === false ? label('more') : label('allLoaded')) + '</button></div></section>';
  }
  function fit(details) {
    const width = details.getBoundingClientRect().width;
    const chrome = ['.topbar','.tabs','.footer'].reduce((height, selector) => height + (details.querySelector(selector)?.getBoundingClientRect().height || 0), 2);
    const key = width + '|' + chrome + '|' + details.__cockpitOverviewHtml;
    if (details.__cockpitDialogSize?.key !== key) {
      // Measure the same overview at the actual dialog width, regardless of the
      // selected tab. The temporary probe never enters focus or scroll layout.
      const probe = document.createElement('div');
      probe.setAttribute('class','panel cockpit-usage-measure');
      probe.setAttribute('aria-hidden','true');
      probe.setAttribute('inert','');
      probe.innerHTML = details.__cockpitOverviewHtml;
      details.appendChild(probe);
      const height = probe.getBoundingClientRect().height;
      probe.remove();
      if (!Number.isFinite(height) || height <= 0) return;
      details.__cockpitDialogSize = {key,height:Math.ceil(chrome + height)};
    }
    details.style.height = Math.min(details.__cockpitDialogSize.height,700,window.innerHeight * 0.85,Math.max(0,window.innerHeight - 32)) + 'px';
  }
  function render(root, details, input) {
    query(root);
    const s = state(root), busy = loading(s);
    const disabled = root.refreshing || root.hostAvailable === false;
    const unavailable = s.available === false ? label(s.unavailableReason === 'unlinked_session' ? 'unlinkedSession' : 'serviceUnavailable') : '';
    const error = root.refreshError ? usageLabels[root.refreshError] || label('refreshFailed') : s.error ? label('dataFailed') : unavailable;
    details.__cockpitOverviewHtml = overview(root,input);
    const html = '<header class="topbar"><span>' + esc(label('title')) + '</span><button class="close" type="button" data-cockpit-quota-close aria-label="' + esc(closeLabel) + '">×</button></header>'
      + '<nav class="tabs" role="tablist" aria-label="' + esc(label('title')) + '">' + [['overview','overview'],['logs','requestsTitle']].map(([value,key]) => '<button class="tab" role="tab" id="cockpit-dialog-tab-' + value + '" aria-controls="cockpit-dialog-panel" aria-selected="' + (s.tab === value) + '" tabindex="' + (s.tab === value ? '0' : '-1') + '" type="button" data-cockpit-dialog-action="tab" data-value="' + value + '">' + esc(label(key)) + '</button>').join('') + '</nav>'
      + '<div id="cockpit-dialog-panel" role="tabpanel" aria-labelledby="cockpit-dialog-tab-' + s.tab + '" class="panel modal-body cockpit-usage-body" data-view="' + s.tab + '">' + (s.tab === 'overview' ? details.__cockpitOverviewHtml : logs(root)) + '</div>'
      + '<footer class="footer"><div class="cockpit-usage-feedback" role="status" aria-live="polite">' + (error ? '<span role="alert" data-cockpit-quota-error>' + esc(error) + '</span>' : '')
      + (s.error || s.available === false ? '<button type="button" class="retry" data-cockpit-dialog-action="retry">' + esc(label('retry')) + '</button>' : '')
      + (root.hostAvailable === false ? '<span>' + esc(label('hostUnavailable')) + '</span>' : busy ? '<span class="feedback">' + esc(label('loading')) + '</span>' : '')
      + '</div><button class="refresh" type="button" data-cockpit-quota-refresh' + (disabled ? ' disabled' : '') + '>' + esc(root.refreshing ? label('refreshing') : label('refreshData')) + '</button></footer>';
    if (details.__cockpitDialogHtml !== html) {
      const body = details.querySelector('.cockpit-usage-body'), scroll = details.querySelector('.request-scroll');
      const top = body?.scrollTop || 0, listTop = scroll?.scrollTop || 0;
      const active = document.activeElement;
      let selector = null;
      if (details.contains(active)) {
        if (active?.hasAttribute('data-cockpit-quota-refresh')) selector = '[data-cockpit-quota-refresh]';
        else if (active?.hasAttribute('data-cockpit-quota-close')) selector = '[data-cockpit-quota-close]';
        else if (active?.getAttribute('class')?.split(' ').includes('request-scroll')) selector = '.request-scroll';
        else if (active?.hasAttribute('data-cockpit-dialog-action')) {
          const action = active.getAttribute('data-cockpit-dialog-action'), value = active.getAttribute('data-value');
          if (['tab','scope','period','more','retry'].includes(action)) selector = '[data-cockpit-dialog-action="' + action + '"]' + (['overview','logs','chat','service','today','week'].includes(value) ? '[data-value="' + value + '"]' : '');
        }
      }
      details.innerHTML = html; details.__cockpitDialogHtml = html;
      if (details.querySelector('.cockpit-usage-body')) details.querySelector('.cockpit-usage-body').scrollTop = top;
      if (details.querySelector('.request-scroll')) details.querySelector('.request-scroll').scrollTop = listTop;
      if (selector) {
        const target = details.querySelector(selector);
        (target?.disabled ? details.querySelector('.request-scroll') : target)?.focus({preventScroll:true});
      }
    }
    if (s.resetScroll) {
      for (const selector of ['.cockpit-usage-body','.request-scroll']) { const node = details.querySelector(selector); if (node) node.scrollTop = 0; }
      s.resetScroll = false;
    }
    details.querySelectorAll('[data-cockpit-dialog-action]').forEach(button => {
      button.onclick = () => {
        if (button.disabled) return;
        const action = button.getAttribute('data-cockpit-dialog-action'), value = button.getAttribute('data-value');
        if (action === 'tab' && ['overview','logs'].includes(value)) { if (s.tab === value) return; s.tab = value; s.resetScroll = true; s.paged = false; issue(root); }
        else if (action === 'scope' && ['chat','service'].includes(value)) {
          if (s.scope === value) return;
          s.scopeCaches[s.scope] = {rows:s.rows,nextCursor:s.nextCursor,paged:s.paged,loaded:s.loaded,available:s.available,unavailableReason:s.unavailableReason};
          s.scope = value; s.resetScroll = true;
          Object.assign(s,s.scopeCaches[value] || {rows:[],nextCursor:null,paged:false,loaded:false,available:null,unavailableReason:null});
          issue(root);
        }
        else if (action === 'period' && ['today','week'].includes(value)) { if (s.period === value) return; s.period = value; s.summary = s.summaryCaches[value] || null; issue(root); }
        else if (action === 'more' && s.nextCursor) { s.paged = true; issue(root,s.nextCursor); }
        else if (action === 'retry') { issue(root,s.pending?.cursor || null); }
        else return;
        root.render();
      };
      if (button.getAttribute('data-cockpit-dialog-action') === 'tab') button.onkeydown = event => {
        let next;
        if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') next = s.tab === 'overview' ? 'logs' : 'overview';
        else if (event.key === 'Home') next = 'overview';
        else if (event.key === 'End') next = 'logs';
        else return;
        event.preventDefault(); if (next !== s.tab) { s.tab = next; s.resetScroll = true; s.paged = false; issue(root); root.render(); }
        details.querySelector('[data-cockpit-dialog-action="tab"][data-value="' + next + '"]')?.focus();
      };
    });
  }
  return {render, fit, query, reset, cancel, refresh, rowSpeed, quotaTone};
})()
