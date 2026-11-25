() => {
  const root = window.__cockpitT3CodeInjection || (window.__cockpitT3CodeInjection = {});
  const capsuleSelector = '[data-cockpit-t3-capsule]';
  const detailsSelector = '[data-cockpit-t3-details]';
  const styleSelector = '[data-cockpit-t3-style]';
  const ownSelector = `${capsuleSelector}, ${detailsSelector}, ${styleSelector}, [data-cockpit-t3-popover]`;
  const controlsSelector = '[data-chat-composer-controls="footer"], [data-chat-composer-resting-controls="true"]';

  if (root.version !== 20) {
    root.observer?.disconnect();
    root.layoutObserver?.disconnect();
    if (root.resizeHandler) {
      window.removeEventListener('scroll', root.resizeHandler, true);
      window.removeEventListener('resize', root.resizeHandler);
    }
    if (root.outsideHandler) window.removeEventListener('pointerdown', root.outsideHandler, true);
    if (root.keyHandler) window.removeEventListener('keydown', root.keyHandler);
    document.querySelectorAll(ownSelector).forEach(element => element.remove());
    Object.assign(root, { version: 20, observer: null, layoutObserver: null, surface: null, quotaDetailsOpen: false });
  }

  const esc = value => String(value ?? '').replace(/[&<>"']/g, ch => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[ch]);
  const missing = labels.missing;
  const decimal = new Intl.NumberFormat('en-US', {
    useGrouping: false, minimumFractionDigits: 1, maximumFractionDigits: 1, roundingMode: 'trunc',
  });
  const speedNumber = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
  function formatMoney(value, currency) {
    if (!Number.isFinite(value)) return missing;
    const symbol = { CNY: '¥', RMB: '¥', USD: '$', EUR: '€' }[String(currency || '').toUpperCase()];
    return symbol ? symbol + value.toFixed(2) : value.toFixed(2) + (currency ? ' ' + currency : '');
  }
  const shortProductLabel = value => String(value || '').trim().replace(/^grok[\s_-]*/i, '') || value || 'Grok';
  function balanceLabel(quota) {
    if (quota.unlimited) return labels.unlimited;
    if (!Number.isFinite(quota.remaining)) return '';
    const value = quota.remaining.toLocaleString(undefined, { maximumFractionDigits: 6 });
    return value + (quota.unit === '%' ? '%' : quota.unit ? ' ' + quota.unit : '');
  }
  function resetLabel(value) {
    const reset = new Date(value);
    return value && Number.isFinite(reset.valueOf()) ? reset.toLocaleString(undefined, {
      month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
    }) : '';
  }
  function cachePercent(usage) {
    const input = usage?.totalInputTokens;
    const cached = usage?.totalCachedTokens;
    if (!Number.isSafeInteger(input) || input <= 0 || !Number.isSafeInteger(cached) || cached < 0 || cached > input) return null;
    const tenths = BigInt(cached) * 1000n / BigInt(input);
    return `${tenths / 10n}.${tenths % 10n}%`;
  }
  function metric({ key, label, value, title, unit, tone }) {
    return `<span data-cockpit-quota-metric data-cockpit-metric-key="${key}" title="${esc(title)}"${tone ? ` data-cockpit-quota-tone="${tone}"` : ''}${value == null ? ' data-cockpit-metric-unknown' : ''}>
      ${key === 'speed' ? '<span data-cockpit-metric-icon aria-hidden="true">↗</span>' : ''}
      ${label ? `<span data-cockpit-metric-label>${esc(label)}</span>` : ''}
      <span data-cockpit-metric-value>${esc(value ?? missing)}</span>
      ${unit ? `<span data-cockpit-metric-unit>${esc(unit)}</span>` : ''}
    </span>`;
  }

  if (!document.querySelector(styleSelector)) {
    const style = document.createElement('style');
    style.setAttribute('data-cockpit-t3-style', '');
    style.textContent = sharedStyles.join('\n') + `
      [data-cockpit-t3-capsule],[data-cockpit-t3-details]{
        --color-token-text-primary:var(--foreground);
        --color-token-text-secondary:var(--muted-foreground);
        --color-token-main-surface-primary:var(--popover);
        --color-token-main-surface-secondary:var(--accent)
      }
      [data-cockpit-t3-capsule][data-cockpit-quota-footer]{transform:none;z-index:30;border-color:var(--border)}
      [data-cockpit-t3-capsule][hidden],[data-cockpit-t3-capsule] [hidden],[data-cockpit-t3-details][hidden]{display:none}
      [data-cockpit-t3-details][data-cockpit-quota-details]{max-width:calc(100vw - 24px);z-index:99999}
      [data-cockpit-t3-details] .direct-account{display:flex;align-items:center;gap:7px;min-width:0;padding:0 0 6px}
      [data-cockpit-t3-details] .direct-account > span:first-child{flex:0 1 auto}
      [data-cockpit-t3-details] .direct-account span,[data-cockpit-t3-details] .direct-windows td{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
      [data-cockpit-t3-details] .direct-account .badge{flex:none;max-width:50%}
    `;
    document.head.appendChild(style);
  }

  function placeCapsule(capsule, picker, surface, footer) {
    const toolbar = footer ?? surface;
    const controls = [...toolbar.querySelectorAll('button,a[href],input,select,textarea,[role="button"],[role="menuitem"],[role="combobox"]')]
      .filter(control => control !== capsule && !control.closest('[inert], [aria-hidden="true"]') &&
        getComputedStyle(control).visibility === 'visible' &&
        (footer || control.closest(controlsSelector)));
    const { left, right } = toolbar.getBoundingClientRect();
    // Compact native controls can be shorter than the capsule.
    const { top, bottom } = surface.getBoundingClientRect();
    const position = quotaSelection.centeredCapsulePosition(
      { left, right, top, bottom }, picker.getBoundingClientRect(),
      controls.map(control => control.getBoundingClientRect()), capsule.getBoundingClientRect(),
      { width: window.innerWidth, height: window.innerHeight });
    if (!position) return false;
    capsule.style.left = position.left + 'px';
    capsule.style.top = position.top + 'px';
    return true;
  }

  function setOpen(open, focusTarget = true) {
    root.quotaDetailsOpen = open;
    if (!open) root.dialog?.cancel(root);
    root.render();
    if (focusTarget) {
      document.querySelector(open ? `${detailsSelector} [data-cockpit-quota-close]` : capsuleSelector)?.focus();
    }
  }

  function render() {
    const picker = [...document.querySelectorAll('[data-chat-provider-model-picker="true"]')]
      .find(node => node.closest(controlsSelector) && !node.closest('[inert], [aria-hidden="true"]') &&
        node.getClientRects().length > 0 && getComputedStyle(node).visibility === 'visible');
    const selection = quotaSelection.readSelection(picker, state);
    const surface = picker?.closest('[data-chat-composer-surface="true"]');
    let capsule = document.querySelector(capsuleSelector);
    let details = document.querySelector(detailsSelector);
    const local = Boolean(selection && state.environmentId === selection.environmentId && state.environmentId);
    const clerkAccount = clerkSync.read(root, state);
    const providerStatus = local && !selection.multiple
      ? quotaSelection.providerStatus(selection.instanceEntries, state.gateways) : [];
    root.evaluation = { activeInstanceId: null, gatewayActive: false, providerStatus, clerkAccount };
    if (!surface || !selection || selection.multiple || !selection.supportsQuota) {
      if (capsule) capsule.hidden = true;
      if (details) details.hidden = true;
      root.dialog?.reset(root);
      root.quotaDetailsOpen = false;
      root.selectionKey = null;
      root.refreshToken = null;
      root.refreshError = null;
      return;
    }

    const conversation = local && selection.threadId ? {
      environmentId: selection.environmentId,
      threadId: selection.threadId,
      providerInstanceId: selection.instanceId,
      contextProviderThreadId: selection.contextProviderThreadId,
      driver: selection.driver,
    } : null;
    const thread = conversation && Object.keys(conversation).every(key =>
      conversation[key] === state.thread?.conversation?.[key]) ? state.thread : null;
    const source = quotaSelection.quotaSource(state, selection);
    const gateway = source.gateway;
    const sourceKey = JSON.stringify([selection.environmentId, selection.instanceId, selection.threadId,
      gateway?.origin, gateway?.binding, source.account, source.instance?.boundAccountId,
      source.instance?.boundAccountWorkspaceId]);
    if (root.selectionKey !== sourceKey) {
      root.selectionKey = sourceKey;
      root.dialog?.reset(root);
      delete root.poolState;
      root.refreshToken = null;
      root.refreshError = null;
    }
    if (root.refreshToken && state.refresh?.token === root.refreshToken) {
      root.refreshToken = null;
      root.refreshError = state.refresh.success ? null : 'refreshFailed';
    }

    root.threadId = gateway ? thread?.usage?.nativeThreadId || null : selection.threadId;
    const scopedSpeed = gateway?.binding && thread?.speed?.binding === gateway.binding &&
      thread.speed.threadId === root.threadId && thread.model === selection.model ? thread.speed : null;
    const isAntigravity = selection.driver === 'antigravity';
    const showSessionMetrics = isAntigravity || Boolean(gateway);
    const antigravitySpeed = isAntigravity && selection.contextProviderThreadId &&
      thread?.model === selection.model ? thread.usage?.antigravitySpeed : null;
    const responseSpeed = antigravitySpeed?.rate > 0 ? speedNumber.format(antigravitySpeed.rate) : null;
    const latestSpeed = isAntigravity ? responseSpeed :
      scopedSpeed?.latestTokensPerSecond > 0 ? decimal.format(scopedSpeed.latestTokensPerSecond) : null;
    const speedLabel = isAntigravity ? labels.responseSpeed : labels.speed;
    const averageSpeed = scopedSpeed?.samples > 0 && scopedSpeed.tokensPerSecond > 0
      ? decimal.format(scopedSpeed.tokensPerSecond) : null;
    const cacheValue = cachePercent(gateway ? scopedSpeed : thread?.usage);
    const context = thread?.usage?.context;
    const contextUsage = formatContextUsage(context?.usedTokens, context?.maxTokens, labels);
    root.usageState = {
      __binding: gateway?.binding ? { binding: gateway.binding } : null,
      __accountPool: gateway ? { binding: gateway.binding || gateway.origin, gptApiKeyCount: gateway.gptApiKeyCount } : null,
      __dialog: gateway ? state.dialog : null,
    };
    root.evaluation = {
      activeInstanceId: local ? selection.instanceId : null,
      clerkAccount,
      model: selection.model,
      conversation: conversation && (conversation.contextProviderThreadId || gateway || selection.driver === 'antigravity' || selection.driver === 'codex')
        ? conversation : null,
      gatewayActive: Boolean(gateway),
      refreshToken: root.refreshToken || null,
      refreshAccountId: local ? source.refreshAccountId : null,
      providerStatus,
    };
    root.refreshing = Boolean(root.refreshToken);
    root.hostAvailable = true;

    if (!capsule) {
      capsule = document.createElement('button');
      capsule.type = 'button';
      capsule.setAttribute('data-cockpit-t3-capsule', '');
      capsule.setAttribute('data-cockpit-quota-footer', '');
      capsule.setAttribute('aria-haspopup', 'dialog');
      capsule.setAttribute('aria-controls', 'cockpit-t3-quota-details');
      document.body.appendChild(capsule);
    }
    const footer = picker.closest('[data-chat-composer-footer="true"]');
    if (root.surface !== surface || root.layoutPicker !== picker || root.layoutFooter !== footer) {
      root.layoutObserver?.disconnect();
      root.layoutObserver = new ResizeObserver(() => root.scheduleRender());
      root.layoutObserver.observe(surface);
      root.layoutObserver.observe(picker);
      if (footer) root.layoutObserver.observe(footer);
      root.surface = surface;
      root.layoutPicker = picker;
      root.layoutFooter = footer;
    }

    root.dialog = createUsageDialog(gateway);
    const badgeKind = gateway?.model === selection.model ? gateway.badgeKind : 'codex';
    const windows = quotaSelection.capsuleWindows(source);
    let quotaFields = [];
    if (badgeKind === 'deepseek') {
      const balance = gateway.balanceLines[0];
      quotaFields = [{ key: 'deepseek', label: labels.balance,
        value: balance ? formatMoney(balance.totalBalance, balance.currency) : null }];
    } else if (badgeKind === 'grok') {
      const line = gateway.grokLines.reduce((best, row) => !best || row.remainingPercent > best.remainingPercent ? row : best, null);
      quotaFields = [{ key: 'grok',
        value: line ? `${Math.round(line.remainingPercent)}%` : null, tone: root.dialog.quotaTone(line?.remainingPercent, 'grok') }];
    } else if (gateway) {
      quotaFields = windows.map((window, index) => ({
        key: 'quota-' + index,
        label: window.label,
        value: `${Math.round(window.percent)}%`,
        tone: root.dialog.quotaTone(window.percent),
        title: usageLabels.aggregateHint,
      }));
      if (!quotaFields.length) quotaFields = [
        { key: 'five-hour', label: labels.fiveHour, title: labels.noQuota },
        { key: 'weekly', label: labels.weekly, title: labels.noQuota },
      ];
    } else {
      const quota = windows.map(window => {
        const tone = window.percent != null ? ` data-cockpit-quota-tone="${root.dialog.quotaTone(window.percent)}"` : '';
        return `<span${tone}><span data-cockpit-metric-value>${esc(window.text)}</span></span>`;
      })
        .join(' · ') || esc(source.providerQuotas.map(balanceLabel).find(Boolean) || (selection.managedAcp ? labels.noQuota : missing));
      const summary = `<span data-cockpit-quota-metric>${quota}</span>`;
      quotaFields = [{ html: summary }];
    }
    const quotaHtml = quotaFields.map(field => field.html || metric(field)).join('');
    const contextFill = contextUsage.percent === null ? 'data-cockpit-metric-unknown' :
      `style="--cockpit-context-fill:${contextUsage.percent}"`;
    const contextHtml = showSessionMetrics ? `<span data-cockpit-quota-metric data-cockpit-metric-key="context"
      data-cockpit-context-ring title="${esc(contextUsage.tooltip)}" aria-label="${esc(contextUsage.label)}" ${contextFill}>
      <span data-cockpit-metric-value>${esc(contextUsage.percent ?? missing)}</span>
    </span>` : '';
    const speedHtml = showSessionMetrics ? metric({
      key: 'speed', value: latestSpeed, unit: 'tok/s',
      title: antigravitySpeed
        ? `${speedLabel}\n${antigravitySpeed.outputTokens.toLocaleString()} ${labels.token} / ${antigravitySpeed.elapsedSeconds.toLocaleString(undefined, { maximumFractionDigits: 3 })} s`
        : latestSpeed === null ? `${speedLabel} · ${labels.noData}` : speedLabel,
    }) : '';
    capsule.hidden = false;
    capsule.onclick = () => setOpen(!root.quotaDetailsOpen);
    const fullHtml = quotaHtml + contextHtml + speedHtml;
    if (capsule.__cockpitT3Html !== fullHtml) {
      capsule.innerHTML = fullHtml;
      const summary = Array.from(capsule.children, item => item.getAttribute('aria-label') ?? item.textContent)
        .join(' ').replace(/\s+/g, ' ').trim();
      capsule.setAttribute('aria-label', `${labels.title} · ${summary}`);
      capsule.__cockpitT3Html = fullHtml;
    }
    // Yield speed, then context before hiding the quota capsule.
    const metrics = [...capsule.querySelectorAll('[data-cockpit-metric-key="context"],[data-cockpit-metric-key="speed"]')];
    metrics.forEach(element => { element.hidden = false; });
    let placed = placeCapsule(capsule, picker, surface, footer);
    while (!placed && metrics.length) {
      metrics.pop().hidden = true;
      placed = placeCapsule(capsule, picker, surface, footer);
    }
    placed = placed && [...capsule.children].some(item => !item.hidden);
    capsule.hidden = !placed;
    if (!placed && root.quotaDetailsOpen) {
      root.quotaDetailsOpen = false;
      root.dialog.cancel(root);
    }
    capsule.setAttribute('aria-expanded', String(root.quotaDetailsOpen));

    if (!root.quotaDetailsOpen) {
      if (details) details.hidden = true;
      return;
    }
    if (!details) {
      details = document.createElement('section');
      details.id = 'cockpit-t3-quota-details';
      details.setAttribute('data-cockpit-t3-details', '');
      details.setAttribute('data-cockpit-quota-details', '');
      details.setAttribute('role', 'dialog');
      details.setAttribute('aria-label', labels.title);
      document.body.appendChild(details);
    }
    details.hidden = false;
    details.dataset.cockpitT3Driver = selection.driver;
    const speedDetails = isAntigravity ? [
      [labels.responseSpeed, antigravitySpeed?.rate],
      [labels.overallSpeed, antigravitySpeed?.overallRate],
      [labels.streamingSpeed, antigravitySpeed?.streamingRate],
    ].map(([label, rate]) => [label, Number.isFinite(rate) ? speedNumber.format(rate) + ' tok/s' : labels.noData]) : null;
    root.dialog.render(root, details, {
      contextDetail: contextUsage.amount ?? labels.noData, contextLabel: labels.context,
      accountBadge: quotaSelection.accountBadge(source, selection, root.dialog.badge, state.codexPlanBadgeStyle),
      badgeKind, formatMoney, shortProductLabel,
      canRefresh: Boolean(local && (gateway || source.refreshAccountId)),
      latestSpeed: latestSpeed === null ? labels.noData : latestSpeed + ' tok/s',
      speedLabel, speedDetails, showSessionMetrics,
      averageSpeed: averageSpeed === null ? missing : averageSpeed + ' tok/s',
      averageCache: cacheValue ?? missing,
      direct: gateway ? null : {
        account: source.account || selection.name,
        plan: source.plan,
        model: selection.modelLabel,
        firstResponseMs: antigravitySpeed?.firstResponseMs,
        windows: source.windows.map(window => ({
          ...window, label: window.id === 'credits' ? labels.credits : window.label,
          amount: quotaSelection.remainingAmountText(window), reset: resetLabel(window.resetTime),
        })),
        balances: source.providerQuotas.map(quota => ({ name: quota.name, value: balanceLabel(quota) || missing })),
        resetLabel: labels.resetsAt,
      },
    });
    details.querySelector('[data-cockpit-quota-close]').onclick = () => setOpen(false);
    const refreshButton = details.querySelector('[data-cockpit-quota-refresh]');
    if (refreshButton) refreshButton.onclick = () => {
      if (root.refreshing) return;
      root.refreshError = null;
      root.refreshToken = Date.now().toString(36) + '-' + Math.random().toString(36).slice(2);
      if (gateway) root.dialog.refresh(root);
      root.render();
    };
    if (placed) root.dialog.place(details, capsule, true);
    if (gateway) root.evaluation.dialogQuery = root.dialog.query(root);
  }

  root.render = render;
  root.scheduleRender = () => {
    if (root.renderScheduled) return;
    root.renderScheduled = true;
    window.requestAnimationFrame(() => {
      root.renderScheduled = false;
      root.render();
    });
  };
  if (!root.observer) {
    root.observer = new MutationObserver(records => {
      if (records.some(record => !record.target.closest?.(ownSelector))) root.scheduleRender();
    });
    root.observer.observe(document.documentElement, {
      childList: true, subtree: true, attributes: true,
      attributeFilter: ['class', 'aria-expanded'],
    });
    root.keyHandler = event => {
      if (event.key === 'Escape' && root.quotaDetailsOpen) {
        event.preventDefault();
        setOpen(false);
      }
    };
    root.outsideHandler = event => {
      if (!root.quotaDetailsOpen) return;
      const path = event.composedPath();
      if (path.includes(document.querySelector(detailsSelector)) ||
          path.includes(document.querySelector(capsuleSelector))) return;
      setOpen(false, false);
    };
    root.resizeHandler = () => root.scheduleRender();
    window.addEventListener('keydown', root.keyHandler);
    window.addEventListener('pointerdown', root.outsideHandler, true);
    window.addEventListener('resize', root.resizeHandler);
    window.addEventListener('scroll', root.resizeHandler, true);
  }
  render();
  return root.evaluation;
}
