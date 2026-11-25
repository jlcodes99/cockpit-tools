(() => {
  function normalizeLoopbackOrigin(raw) {
    if (typeof raw !== 'string') return '';
    const match = raw.trim().match(/^(https?):\/\/(\[[^\]]+\]|[^/:?#]+)(?::(\d+))?/i);
    if (!match) return '';
    const protocol = match[1].toLowerCase();
    const host = match[2].toLowerCase();
    const normalizedHost = host === 'localhost' || host === '::1' || host === '[::1]' ? '127.0.0.1' : host;
    const port = match[3] || (protocol === 'https' ? '443' : '80');
    return `${protocol}://${normalizedHost}:${port}`;
  }

  function readSelection(picker, state) {
    if (!picker) return null;
    const fiberKey = Object.keys(picker).find(key =>
      key.startsWith('__reactFiber$') || key.startsWith('__reactInternalInstance$'));
    for (let fiber = fiberKey ? picker[fiberKey] : null; fiber; fiber = fiber.return) {
      const props = fiber.memoizedProps;
      if (!props?.isComposerOwned || typeof props.activeInstanceId !== 'string' ||
          typeof props.model !== 'string' || !Array.isArray(props.instanceEntries)) continue;

      const entry = props.instanceEntries.find(item => item.instanceId === props.activeInstanceId);
      if (!entry) return null;
      // ACP shares one native driver across providers. Resolve only the instance
      // in Cockpit's local workspace so quotas and switches use the same account.
      const managedAcp = entry.driverKind === 'acpRegistry' && state?.environmentId &&
        props.environmentId === state.environmentId
        ? state.workspace?.providerInstances?.find(row => row.instanceId === entry.instanceId)
        : null;
      const instanceOptions = props.modelOptionsByInstance?.get?.(entry.instanceId);
      const effectiveModel = props.model === 'antigravity-default'
        ? (instanceOptions?.find(item => item.isDefault)?.slug ||
           entry.snapshot?.models?.find(item => item.isDefault)?.slug ||
           props.model)
        : props.model;
      const option = instanceOptions?.find(item => item.slug === effectiveModel);
      const nativeOption = entry.snapshot?.models?.find(item => item.slug === effectiveModel);
      const thread = readActiveThread(fiber, props.environmentId);
      const threadModel = thread?.modelSelection;
      const sameModel = threadModel?.instanceId === entry.instanceId &&
        (threadModel.model === props.model || threadModel.model === effectiveModel);
      return {
        environmentId: props.environmentId,
        threadId: thread?.id ?? null,
        contextProviderThreadId: sameModel ? thread.activeProviderThreadId ?? null : null,
        instanceId: entry.instanceId,
        driver: managedAcp?.driver ?? entry.driverKind,
        supportsQuota: Boolean(managedAcp) || ['codex', 'antigravity', 'claudeAgent', 'grok'].includes(entry.driverKind),
        managedAcp: Boolean(managedAcp),
        name: entry.label || entry.displayName || entry.snapshot?.displayName || entry.driverKind,
        model: effectiveModel,
        modelLabel: option?.name || option?.label || option?.displayName || effectiveModel,
        effort: entry.driverKind === 'antigravity'
          ? readAntigravityEffort(fiber, entry.instanceId, effectiveModel,
              option?.capabilities?.optionDescriptors ?? nativeOption?.capabilities?.optionDescriptors)
          : null,
        snapshot: entry.snapshot,
        instanceEntries: props.instanceEntries,
        multiple: Array.isArray(props.selectedModels ?? props.multipleModelSelections) &&
          (props.selectedModels ?? props.multipleModelSelections).length > 1,
      };
    }
    return null;
  }

  function providerStatus(entries, gateways) {
    return entries.filter(entry => entry.snapshot && typeof entry.instanceId === 'string').slice(0, 256)
      .map(({ instanceId, snapshot }) => {
        const endpointOrigin = normalizeLoopbackOrigin(snapshot.auth?.endpoint);
        return {
          instanceId,
          accountEmail: snapshot.auth?.email,
          workspaceId: snapshot.auth?.workspaceId,
          authStatus: snapshot.auth?.status,
          installed: snapshot.installed,
          status: snapshot.status,
          checkedAt: snapshot.checkedAt,
          message: snapshot.message,
          modelCount: !snapshot.modelsRetained && Array.isArray(snapshot.models) ? snapshot.models.length : 0,
          availability: snapshot.availability ?? (snapshot.status === 'ready' ? 'available' : undefined),
          matchesGateway: Boolean(endpointOrigin && gateways?.some(row =>
            row.instanceId === instanceId && normalizeLoopbackOrigin(row.usage?.origin) === endpointOrigin)),
        };
      });
  }

  function readActiveThread(fiber, environmentId) {
    for (let node = fiber; node; node = node.return) {
      const props = node.memoizedProps;
      if (props?.environmentId === environmentId && props.routeKind === 'server' &&
          typeof props.activeThreadId === 'string' && props.activeThread?.id === props.activeThreadId) {
        return props.activeThread;
      }
    }
    return null;
  }

  const slug = value => String(value ?? '').trim().toLowerCase().replaceAll(/[^a-z0-9]+/g, '-');

  function readAntigravityEffort(fiber, instanceId, model, descriptors) {
    const reasoning = descriptors?.find(option => option.type === 'select' && option.role === 'reasoning');
    if (!reasoning) return null;
    const optionValue = selection => selection?.instanceId === instanceId && selection.model === model
      ? selection.options?.find(option => option.id === reasoning.id)?.value
      : undefined;
    let reported;
    let selected;
    for (let node = fiber; node; node = node.return) {
      const props = node.memoizedProps;
      selected = optionValue(props?.selectedModelSelection);
      if (typeof selected === 'string') break;
      reported ??= optionValue(props?.reportedModelSelection);
    }
    const value = selected ?? reported ?? reasoning.currentValue;
    if (typeof value !== 'string') return null;
    const level = slug(value).match(/(?:^|-)(minimal|low|medium|high|xhigh|max)$/);
    return level?.[1] ?? null;
  }

  function antigravityFamily(value) {
    const id = slug(value);
    if (/^(3p|claude|gpt)(-|$)/.test(id)) return '3p';
    if (/^argon(-|$)|^gemini-.*\bargon\b/.test(id)) return 'argon';
    if (/^carbon(-|$)|^gemini-.*\bcarbon\b/.test(id)) return 'carbon';
    if (/^gemini(-|$)/.test(id)) return 'gemini';
    return null;
  }

  function bucket(id) {
    const match = slug(id).match(/^(3p|claude|argon|carbon|gemini(?:-(?:argon|carbon)(?:-wtu)?)?)-(5h|weekly)$/);
    return match ? { family: antigravityFamily(match[1]), period: match[2] } : null;
  }

  function quotaModel(value) {
    return slug(value).replace(/-(minimal|low|medium|high|xhigh|max|thinking)$/, '');
  }

  function reportedGroupWindows(groups, availableWindows, models, family) {
    const group = groups?.find(item => item.models?.some(model => models.has(quotaModel(model)))) ??
      groups?.find(item => !item.models?.length && antigravityFamily(item.id) === family);
    if (!group) return [];
    return group.windows.map(entry => {
      const source = availableWindows.find(window => window.id === entry.id);
      return {
        id: entry.id, label: source?.label || group.name, resetTime: source?.resetTime,
        period: entry.period, remainingPercent: entry.percentage,
      };
    }).filter(window => Number.isFinite(window.remainingPercent));
  }

  function displayGroupWindows(windows) {
    const byPeriod = new Map();
    for (const window of windows) {
      const period = window.period || bucket(window.id)?.period;
      const key = period || window.id;
      if (!byPeriod.has(key) || window.remainingPercent < byPeriod.get(key).remainingPercent) {
        const shortLabel = period === '5h' ? '5h' : period === 'weekly' ? '7d' : undefined;
        byPeriod.set(key, { ...window, shortLabel });
      }
    }
    const main = ['5h', 'weekly'].flatMap(period => {
      const window = byPeriod.get(period);
      byPeriod.delete(period);
      return window ? [window] : [];
    });
    return [...main, ...byPeriod.values()];
  }

  function relevantWindows(windows, selection, groups) {
    const valid = (windows || []).filter(window =>
      (Number.isFinite(window.remainingPercent) || Number.isFinite(window.remainingAmount)) &&
      (!window.modelIds?.length || window.modelIds.includes(selection.model)));
    if (selection.driver === 'claudeAgent') {
      const sonnet = /^claude-sonnet(-|$)/.test(slug(selection.model));
      return valid.filter(window => window.id !== 'sonnet-weekly' || sonnet);
    }
    if (selection.driver !== 'antigravity') return valid;

    const modelIds = [selection.model, selection.modelLabel];
    const family = modelIds.map(antigravityFamily).find(Boolean);
    const models = new Set(modelIds.map(quotaModel).filter(Boolean));
    const reported = reportedGroupWindows(groups, valid, models, family);
    if (reported.length) return displayGroupWindows(reported);
    if (!family) return [];

    const isGpt = modelIds.some(id => /^gpt(-|$)/.test(slug(id)));
    const groupMatch = window => bucket(window.id)?.family === family &&
      !(isGpt && /^claude-/.test(window.id));
    // Explicit membership can displace the legacy bucket/model naming.
    const permittedLegacy = valid.filter(window => !groups?.some(group =>
      group.models?.length && group.windows.some(entry => entry.id === window.id)));
    const matching = permittedLegacy.filter(groupMatch);
    if (matching.length) return displayGroupWindows(matching);

    // Historical cache entries can predate group members. Google's shared
    // Gemini allowance covers these documented models when the account only
    // reports the shared bucket. An independent bucket always takes precedence.
    if ([...models].some(model => /^gemini-(4-(neon|carbon|argon|hydrogen)|3-8-flash)$/.test(model))) {
      const shared = permittedLegacy.filter(window => bucket(window.id)?.family === 'gemini');
      if (shared.length) return displayGroupWindows(shared);
    }
    const exactModels = new Set(modelIds.map(slug).filter(Boolean));
    if (selection.effort) {
      // The ACP picker sends a base model and a separate reasoning level. Older
      // Antigravity quota records may include that level in the model ID.
      const effortModels = new Set(Array.from(exactModels)
        .filter(id => antigravityFamily(id) === family && !/-(minimal|low|medium|high|xhigh|max)$/.test(id))
        .map(id => `${id}-${selection.effort}`));
      const effortWindows = valid.filter(window => effortModels.has(slug(window.id)));
      if (effortWindows.length) return effortWindows;
    }
    return valid.filter(window => exactModels.has(slug(window.id)) ||
      window.modelIds?.includes(selection.model) ||
      // Older upstream snapshots supply per-family reported limits without model IDs.
      (family !== 'argon' && family !== 'carbon' &&
        window.id === (family === 'gemini' ? 'gemini-quota' : isGpt ? 'gpt-quota' : 'claude-quota')));
  }

  function windowLabel(window) {
    if (window.id === 'credits') return labels.credits;
    const label = window.shortLabel || window.label;
    if (window.kind === 'weekly' || /^(7d|weekly)$/i.test(label)) return labels.weekly;
    if (window.windowDurationMins === 300) return '5h';
    if (/^[0-9]+[mhdw]$/i.test(label)) return label;
    if (Number.isFinite(window.remainingAmount)) return label;
    // Model-level records may report a percentage without specifying a quota period.
    return '';
  }

  function remainingAmountText(window) {
    return Number.isFinite(window.remainingAmount)
      ? window.remainingAmount.toLocaleString(undefined, { maximumFractionDigits: 6 }) : null;
  }

  function capsuleWindows(source) {
    return source.windows.slice(0, 2).map(window => {
      const label = windowLabel(window);
      const value = remainingAmountText(window) ?? `${Math.round(window.remainingPercent)}%`;
      return {
        label,
        value,
        text: [label, value].filter(Boolean).join(' '),
        percent: window.remainingPercent,
      };
    });
  }

  function readNativeQuota(snapshot) {
    const auth = snapshot?.auth;
    return {
      account: auth?.email || '',
      plan: snapshot?.badgeLabel || auth?.label || '',
      windows: auth?.status === 'unauthenticated' ? [] : (snapshot?.usageLimits?.windows || []).map(window => ({
        id: window.id,
        label: window.label,
        modelIds: window.modelIds,
        kind: window.kind,
        windowDurationMins: window.windowDurationMins,
        remainingPercent: Number.isFinite(window.usedPercent) ? 100 - window.usedPercent : null,
        resetTime: window.resetsAt,
      })),
    };
  }

  function displayPlan(driver, value) {
    const raw = slug(value);
    const plans = {
      free: 'Free', pro: 'Pro', ultra: 'Ultra', plus: 'Plus', max: 'Max',
      team: 'Team', business: 'Business', enterprise: 'Enterprise', edu: 'Edu',
      'api-key': 'API Key', 'max-5x': 'max-5x', 'max-20x': 'max-20x',
    };
    const key = raw.replace(/^(g1|google-one|google-ai|chatgpt|(?:default-)?claude)-/, '').replace(/-tier$/, '');
    if (driver === 'antigravity' && key === 'standard') return 'Free';
    if (Object.hasOwn(plans, key)) return plans[key];
    if (driver === 'antigravity' || raw === 'unknown') return labels.unknownPlan;
    return value || '';
  }

  function accountBadge(source, selection, badge, codexStyle) {
    const instance = source.instance;
    if (source.identityProblem || instance?.boundIsApiService) return '';
    if (!instance?.boundAccountId) return undefined;
    const plan = instance.boundAccountPlan?.trim();
    const known = plan && plan.toLowerCase() !== 'unknown';
    const kind = instance.boundAccountPlanBadgeKind;
    const appearance = kind ?? (known ? displayPlan(selection.driver, plan) : undefined);
    const normalizedAntigravity = selection.driver === 'antigravity' &&
      /^(?:g1-)?(?:pro|ultra|free|standard)(?:-tier)?$/i.test(plan || '');
    const showRaw = !normalizedAntigravity && (!kind || kind === 'cockpit-api');
    return badge(known ? plan : '—', showRaw, selection.driver === 'codex' ? codexStyle : '', appearance);
  }

  function quotaSource(state, selection) {
    const auth = selection.snapshot?.auth;
    const local = Boolean(state.environmentId && state.environmentId === selection.environmentId);
    const instance = local ? state.workspace?.providerInstances?.find(row =>
      row.instanceId === selection.instanceId && row.driver === selection.driver) : null;
    const status = readNativeQuota(selection.snapshot);
    const native = {
      account: status.account,
      plan: displayPlan(selection.driver, status.plan),
      windows: relevantWindows(status.windows, selection),
      providerQuotas: [],
      instance,
    };
    if (!instance) return native;

    if (instance.boundIsApiService) {
      const usage = state.gateways?.find(item => item.instanceId === instance.instanceId)?.usage;
      const usageOrigin = normalizeLoopbackOrigin(usage?.origin);
      const endpointOrigin = normalizeLoopbackOrigin(auth?.endpoint);
      // A saved binding alone cannot show which source the active Codex server uses.
      if (!usageOrigin || !endpointOrigin || endpointOrigin !== usageOrigin) return native;
      return {
        account: labels.gatewayName,
        plan: '',
        windows: relevantWindows(usage.windows, selection),
        providerQuotas: usage.providerQuotas || [],
        gateway: usage,
        isApiService: true,
        instance,
      };
    }

    if (!instance.boundAccountEmail && !instance.boundAccountId) return native;
    // A previously opened provider session may continue reporting the old account.
    const email = auth?.email || (selection.driver === 'antigravity' ? instance.runtime?.accountEmail : null);
    const sameAccount = Boolean(email && instance.boundAccountEmail &&
      email.toLowerCase() === instance.boundAccountEmail.toLowerCase() &&
      (!instance.boundAccountWorkspaceId || auth?.workspaceId === instance.boundAccountWorkspaceId ||
        (selection.driver === 'grok' && !auth?.workspaceId)) &&
      auth?.status !== 'unauthenticated');
    const mismatch = instance.runtime?.authUsesKeyring || (email && instance.boundAccountEmail && !sameAccount);
    if (mismatch || (selection.driver === 'antigravity' && !sameAccount)) {
      return { instance, account: email || '', plan: '', windows: [], providerQuotas: [],
        identityProblem: mismatch ? labels.accountMismatch : labels.identityPending };
    }
    const windows = relevantWindows(instance.windows, selection, instance.quotaSummaryGroups);
    return {
      account: instance.boundAccountEmail || native.account,
      refreshAccountId: selection.driver !== 'claudeAgent' || instance.boundAccountPlan !== 'API_KEY'
        ? instance.boundAccountId : null,
      plan: instance.boundAccountPlan ? displayPlan(selection.driver, instance.boundAccountPlan) : sameAccount ? native.plan : '',
      windows: windows.length ? windows : sameAccount ? native.windows : [],
      providerQuotas: instance.providerQuota ? [instance.providerQuota] : [],
      instance,
    };
  }

  function centeredCapsulePosition(bounds, picker, controls, capsule, viewport) {
    const gap = 6;
    const top = (picker.top + picker.bottom - capsule.height) / 2;
    const bottom = top + capsule.height;
    const left = Math.max(bounds.left, 0) + gap;
    const right = Math.min(bounds.right, viewport.width) - gap;
    if (capsule.width <= 0 || capsule.height <= 0 || right - left < capsule.width ||
        top < Math.max(bounds.top, 0) || bottom > Math.min(bounds.bottom, viewport.height)) return null;

    const occupied = controls
      .filter(rect => rect.width > 0 && rect.height > 0 && top < rect.bottom && bottom > rect.top &&
        rect.right + gap > left && rect.left - gap < right)
      .map(rect => [Math.max(left, rect.left - gap), Math.min(right, rect.right + gap)])
      .sort((a, b) => a[0] - b[0]);
    let cursor = left;
    let widest = 0;
    let position = null;
    for (const [start, end] of [...occupied, [right, right]]) {
      const width = start - cursor;
      if (width >= capsule.width && width > widest) {
        widest = width;
        position = { left: cursor + (width - capsule.width) / 2, top };
      }
      cursor = Math.max(cursor, end);
    }
    return position;
  }

  return { readSelection, relevantWindows, quotaSource, capsuleWindows, remainingAmountText, centeredCapsulePosition, displayPlan, accountBadge, providerStatus };
})()
