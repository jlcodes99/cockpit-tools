(settings, data, styles, formatContextUsage) => {
  if (window.top !== window || location.protocol !== 'https:' || location.hostname !== '127.0.0.1') return false;
  if (window.__cockpitAntigravityComposer) return window.__cockpitAntigravityComposer.refresh(settings);
  const footerSelector = '[data-agent-composer-root].agy-composer-footer';
  if (!document.body || !document.querySelector(footerSelector)) return false;
  const pickerSelector = '[data-testid="model-selector-trigger"]';
  const sidePanel = '[data-side-panel-focus-scope="true"]';
  const controlSelector = 'button, a[href], input, [role="button"], [role="combobox"]';
  const viewSelector = '[data-testid="conversation-view"][data-cascade-id]';
  const root = { settings };
  window.__cockpitAntigravityComposer = root;

  const style = document.createElement('style');
  style.textContent = styles.join('\n');
  document.head.append(style);
  const capsule = document.createElement('button');
  capsule.type = 'button';
  capsule.hidden = true;
  capsule.setAttribute('data-cockpit-quota-footer', '');
  capsule.setAttribute('data-cockpit-ag-capsule', '');
  capsule.setAttribute('aria-haspopup', 'dialog');
  capsule.setAttribute('aria-controls', 'cockpit-ag-composer-details');
  capsule.setAttribute('aria-expanded', 'false');
  const details = document.createElement('div');
  details.id = 'cockpit-ag-composer-details';
  details.hidden = true;
  details.setAttribute('data-cockpit-ag-details', '');
  details.setAttribute('role', 'dialog');
  const header = document.createElement('header');
  const title = document.createElement('strong');
  const close = document.createElement('button');
  close.type = 'button';
  const rows = document.createElement('dl');
  header.append(title, close);
  details.append(header, rows);
  document.body.append(capsule, details);

  const visible = node => node && node.getClientRects().length > 0 &&
    !node.closest('[inert], [aria-hidden="true"]') && getComputedStyle(node).visibility === 'visible';

  function chatScope() {
    const footers = [...document.querySelectorAll(footerSelector)].filter(footer =>
      !footer.closest(sidePanel) && visible(footer) && visible(footer.querySelector(pickerSelector)));
    if (footers.length !== 1) return null;
    const footer = footers[0];
    const picker = footer.querySelector(pickerSelector);
    const model = picker.getAttribute('data-model-label')?.trim() ||
      picker.innerText?.replace(/\s+/g, ' ').trim() ||
      picker.textContent?.replace(/\s+/g, ' ').trim() || null;
    for (let pane = footer.parentElement; pane && pane !== document.body; pane = pane.parentElement) {
      const views = [...pane.querySelectorAll(viewSelector)].filter(view => !view.closest(sidePanel) && visible(view));
      if (!views.length) continue;
      const id = views.length === 1 ? views[0].getAttribute('data-cascade-id') : null;
      return { footer, model, id: id === 'conversation' ? null : id };
    }
    return { footer, model, id: null };
  }

  function authToken() {
    try {
      return window.__APP_CONFIG__?.csrfToken || localStorage.getItem('agy_session_csrf_token');
    } catch {
      return null;
    }
  }

  async function request(method, args, token) {
    const response = await fetch('/exa.language_server_pb.LanguageServerService/' + method, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Connect-Protocol-Version': '1',
        'x-codeium-csrf-token': token,
      },
      body: JSON.stringify(args),
      signal: AbortSignal.timeout(4000),
    });
    if (!response.ok) throw new Error(String(response.status));
    return response.json();
  }

  const number = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
  const duration = new Intl.NumberFormat(undefined, { maximumSignificantDigits: 4 });
  let quota = { summary: null, status: null };
  let token = null;
  let active = null;
  let quotaDue = 0;
  let quotaBusy = false;
  let nextMetadata = 0;
  let lastText = '';
  let scheduled = false;
  let observedFooter = null;
  const layout = new ResizeObserver(scheduleRender);

  function refreshQuota(currentToken) {
    if (quotaBusy || Date.now() < quotaDue) return;
    quotaBusy = true;
    quotaDue = Date.now() + 30000;
    void Promise.allSettled([
      request('RetrieveUserQuotaSummary', {}, currentToken),
      request('GetUserStatus', {}, currentToken),
    ]).then(([summary, status]) => {
      if (currentToken !== token) return;
      if (summary.status === 'fulfilled') quota.summary = summary.value;
      if (status.status === 'fulfilled') quota.status = status.value;
      if (!quota.summary) quotaDue = Date.now() + 5000;
      scheduleRender();
    }).finally(() => { quotaBusy = false; });
  }

  async function refreshMetrics(session, currentToken) {
    if (session.busy || Date.now() < nextMetadata) return;
    session.busy = true;
    nextMetadata = Date.now() + 1500;
    try {
      let offset = session.offset;
      if (offset === null) {
        const result = await request('GetCascadeTrajectory', {
          cascadeId: session.id,
          trajectoryVerbosity: 'CLIENT_TRAJECTORY_VERBOSITY_PROD_UI',
          disableRehydration: true,
        }, currentToken);
        const count = Number(result.numTotalGeneratorMetadata ?? result.trajectory?.generatorMetadata?.length);
        if (!Number.isSafeInteger(count) || count < 0) return;
        offset = Math.max(0, count - 12);
      }
      const result = await request('GetCascadeTrajectoryGeneratorMetadata', {
        cascadeId: session.id, generatorMetadataOffset: offset, includeMessages: false,
      }, currentToken);
      if (session !== active || currentToken !== token) return;
      const metadata = result.generatorMetadata ?? [];
      if (!Array.isArray(metadata)) return;
      session.offset = !metadata.length && offset > 0 ? null : Math.max(0, offset + metadata.length - 12);
      const { context, speed, hasCompletedSample } = data.sessionMetrics(metadata, quota.status, session.model);
      if (context) session.metrics.context = context;
      if (speed) session.metrics.speed = speed;
      else if (hasCompletedSample && !session.metrics.speed) session.metrics.speed = null;
      scheduleRender();
    } catch {
      // The page can briefly restart or rotate authentication. Keep this chat's last sample.
    } finally {
      session.busy = false;
    }
  }

  function setOpen(open, focus = true) {
    details.hidden = !open;
    capsule.setAttribute('aria-expanded', String(open));
    if (open) render();
    if (focus) (open ? close : capsule).focus({ preventScroll: true });
  }

  function setScope(scope, currentToken) {
    if (currentToken !== token) {
      token = currentToken;
      quota = { summary: null, status: null };
      quotaDue = 0;
      active = null;
    }
    const session = scope?.id && scope.model ? scope : null;
    if (session?.id === active?.id && session?.model === active?.model) return;
    if (!details.hidden) setOpen(false, false);
    const prevMetrics = session && session.id === active?.id ? active.metrics : {};
    active = session ? { id: session.id, model: session.model, offset: null, metrics: prevMetrics } : null;
    nextMetadata = 0;
  }

  function periodLabel(period) {
    if (period === '5h') return root.settings.fiveHour;
    if (period === 'weekly') return root.settings.weekly;
    return period ?? '';
  }

  function quotaTone(value) {
    return value >= 80 ? 'high' : value >= 40 ? 'medium' : value >= 10 ? 'low' : 'critical';
  }

  function metric(key, label, value, tone, tooltip) {
    const item = document.createElement('span');
    item.setAttribute('data-cockpit-quota-metric', '');
    item.setAttribute('data-cockpit-metric-key', key);
    if (key === 'context') {
      item.setAttribute('data-cockpit-context-ring', '');
      if (value.percent === null) item.setAttribute('data-cockpit-metric-unknown', '');
      else item.style.setProperty('--cockpit-context-fill', value.percent);
      item.title = value.tooltip;
      const reading = document.createElement('span');
      reading.setAttribute('data-cockpit-context-reading', '');
      reading.textContent = value.label;
      item.append(reading);
      return item;
    }
    if (tooltip) item.title = tooltip;
    if (value === null) item.setAttribute('data-cockpit-metric-unknown', '');
    if (tone) item.setAttribute('data-cockpit-quota-tone', tone);
    if (label) {
      const name = document.createElement('span');
      name.setAttribute('data-cockpit-metric-label', '');
      name.textContent = label;
      item.append(name);
    }
    const measurement = document.createElement('span');
    measurement.setAttribute('data-cockpit-metric-value', '');
    measurement.textContent = value ?? `${root.settings.missing} tok/s`;
    item.append(measurement);
    return item;
  }

  function position(footer) {
    const [left, right] = footer.children;
    if (!left || !right) return false;
    const bounds = footer.getBoundingClientRect();
    const occupied = [...left.querySelectorAll(controlSelector)].filter(visible);
    const lo = Math.max(bounds.left, ...occupied.map(node => node.getBoundingClientRect().right)) + 6;
    const hi = right.getBoundingClientRect().left - 6;
    const width = capsule.getBoundingClientRect().width;
    if (hi - lo < width) return false;
    const center = Math.max(lo + width / 2, Math.min((bounds.left + bounds.right) / 2, hi - width / 2));
    capsule.style.left = center + 'px';
    capsule.style.top = (bounds.top + bounds.height / 2) + 'px';
    return true;
  }

  function renderDetails(windows, contextUsage, speed) {
    const { title: text, quota, unknown, noData, resetsAt, context: contextLabel,
      responseSpeed, overallSpeed, streamingSpeed, firstResponse } = root.settings;
    details.setAttribute('aria-label', text);
    title.textContent = text;
    close.textContent = root.settings.close;
    const pairs = windows.length ? windows.map(window => {
      const reset = window.reset && new Date(window.reset);
      const time = reset && Number.isFinite(reset.valueOf()) ? ' · ' + resetsAt + ' ' + reset.toLocaleString() : '';
      return [periodLabel(window.period) || quota, window.percent + '%' + time];
    }) : [[quota, unknown]];
    const formatSpeed = rate => Number.isFinite(rate) ? number.format(rate) + ' tok/s' : noData;
    pairs.push([contextLabel, contextUsage.amount ?? noData],
      [responseSpeed, formatSpeed(speed?.rate)],
      [overallSpeed, formatSpeed(speed?.overallRate)],
      [streamingSpeed, formatSpeed(speed?.streamingRate)]);
    if (speed?.first != null) pairs.push([firstResponse, number.format(speed.first) + ' s']);
    rows.replaceChildren(...pairs.flatMap(([key, value]) => {
      const label = document.createElement('dt');
      const reading = document.createElement('dd');
      label.textContent = key;
      label.setAttribute('title', key);
      reading.textContent = value;
      return [label, reading];
    }));
    const rect = capsule.getBoundingClientRect();
    const width = details.getBoundingClientRect().width;
    details.style.left = Math.max(12, Math.min((rect.left + rect.right - width) / 2, innerWidth - width - 12)) + 'px';
    details.style.bottom = (innerHeight - rect.top + 8) + 'px';
    details.style.maxHeight = Math.max(0, rect.top - 20) + 'px';
  }

  function render() {
    const scope = chatScope();
    setScope(scope, authToken());
    if (observedFooter !== scope?.footer) {
      layout.disconnect();
      observedFooter = scope?.footer;
      if (observedFooter) {
        layout.observe(observedFooter);
        for (const child of observedFooter.children) layout.observe(child);
      }
    }
    if (!scope) {
      capsule.hidden = true;
      if (!details.hidden) setOpen(false, false);
      return;
    }
    const windows = data.quotaWindows(quota.summary, quota.status, scope.model).slice(0, 2);
    const { context, speed } = active?.metrics ?? {};
    const contextUsage = formatContextUsage(context?.used, context?.max, root.settings, true);
    const speedTooltip = speed
      ? `${root.settings.responseSpeed}\n${number.format(speed.outputTokens)} ${root.settings.token} / ${duration.format(speed.elapsedSeconds)} s`
      : `${root.settings.responseSpeed} · ${root.settings.noData}`;
    const values = windows.map(window => ['quota', periodLabel(window.period), window.percent + '%', quotaTone(window.percent)]);
    values.push(['context', '', contextUsage],
      ['speed', '', speed ? number.format(speed.rate) + ' tok/s' : null, null, speedTooltip]);
    const text = JSON.stringify(values);
    if (lastText !== text) {
      capsule.replaceChildren(...values.map(args => metric(...args)));
      capsule.setAttribute('aria-label', [root.settings.title, ...Array.from(capsule.children, item => item.textContent)].join(' · '));
      lastText = text;
    }
    capsule.hidden = false;
    const optional = [...capsule.querySelectorAll('[data-cockpit-metric-key="context"], [data-cockpit-metric-key="speed"]')];
    optional.forEach(item => { item.hidden = false; });
    let placed = position(scope.footer);
    while (!placed && optional.length) {
      optional.pop().hidden = true;
      placed = position(scope.footer);
    }
    capsule.hidden = !placed || ![...capsule.children].some(item => !item.hidden);
    if (capsule.hidden && !details.hidden) setOpen(false, false);
    if (!details.hidden) renderDetails(windows, contextUsage, speed);
  }

  function scheduleRender() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => {
      scheduled = false;
      render();
    });
  }

  function tick() {
    const scope = chatScope();
    setScope(scope, authToken());
    if (scope && token) {
      refreshQuota(token);
      if (active) void refreshMetrics(active, token);
    }
    scheduleRender();
  }

  capsule.addEventListener('click', () => setOpen(details.hidden));
  close.addEventListener('click', () => setOpen(false));
  window.addEventListener('pointerdown', event => {
    if (details.hidden) return;
    const path = event.composedPath();
    if (!path.includes(details) && !path.includes(capsule)) setOpen(false, false);
  }, true);
  details.addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      event.preventDefault();
      setOpen(false);
    }
  });
  window.addEventListener('resize', scheduleRender);
  window.addEventListener('scroll', scheduleRender, true);
  new MutationObserver(records => {
    const watched = footerSelector + ', ' + viewSelector;
    if (records.some(record => {
      if (record.type === 'attributes') return record.target.matches?.(watched + ', ' + pickerSelector);
      if (record.target.closest?.(footerSelector)) return true;
      return [...record.addedNodes, ...record.removedNodes].some(node =>
        node.nodeType === Node.ELEMENT_NODE && (node.matches(watched) || node.querySelector(watched)));
    })) scheduleRender();
  }).observe(document.body, { childList: true, subtree: true, attributes: true,
    attributeFilter: ['data-model-label', 'data-cascade-id', 'aria-hidden'] });
  root.refresh = updatedSettings => {
    root.settings = updatedSettings;
    tick();
    return true;
  };
  return root.refresh(settings);
}
