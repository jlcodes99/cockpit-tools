(() => {
  function modelKey(value) {
    return typeof value === 'string' ? value.replace(/\s*·\s*\d+\s*[km]\b.*$/i, '').split(' · ').at(-1).toLowerCase()
      .replace(/[^a-z0-9]+/g, ' ').trim()
      .replace(/(?: (?:extra high|x high|xhigh|extra|minimal|low|medium|high|max|thinking|tiered|none))+$/, '')
      .replace(/^gemini (?=\d)/, '') : '';
  }

  function modelConfigs(status) {
    return status?.userStatus?.cascadeModelConfigData?.clientModelConfigs ??
      status?.cascadeModelConfigData?.clientModelConfigs ?? [];
  }

  function configNames(config) {
    return [config.label, config.modelDisplayName, config.modelId, config.model,
      config.modelOrAlias?.model, config.modelOrAlias?.alias].map(modelKey).filter(Boolean);
  }

  function aliases(label, status) {
    const key = modelKey(label);
    const configs = modelConfigs(status);
    const matching = configs.filter(config => configNames(config).includes(key));
    return new Set([key, ...matching.flatMap(configNames)].filter(Boolean));
  }

  function family(value) {
    const key = modelKey(value);
    if (/^(3p|claude|gpt)( |$)/.test(key)) return '3p';
    // Argon retired into the shared Gemini pool; Carbon keeps its own pool.
    if (/^(?:\d+ )?carbon( |$)/.test(key)) return 'carbon';
    if (/^gemini( |$)|^\d+(?: \d+)* (?:neon|argon|hydrogen|flash|pro|ultra)( |$)/.test(key)) return 'gemini';
    return null;
  }

  function retiredArgonBucket(id) {
    const raw = String(id ?? '').trim().toLowerCase();
    return raw === 'argon:weekly' || raw === 'argon:5h' ||
      (raw.startsWith('gemini-argon-') && (raw.endsWith('-weekly') || raw.endsWith('-5h')));
  }

  function sharedGeminiBucket(id) {
    const raw = String(id ?? '').trim().toLowerCase();
    return raw === 'gemini-5h' || raw === 'gemini:5h' || raw === 'gemini-weekly' || raw === 'gemini:weekly';
  }

  function retiredArgonGroup(group) {
    const buckets = group?.buckets ?? [];
    return buckets.length > 0 && buckets.every(bucket => retiredArgonBucket(bucket.bucketId));
  }

  function quotaWindows(summary, status, label) {
    const groups = summary?.response?.groups ?? summary?.groups ?? [];
    const selected = aliases(label, status);
    const groupFamily = [...selected].map(family).find(Boolean);
    const identities = group => [
      ...(group.modelIds ?? []),
      ...(group.description?.match(/^\s*Models within this group:\s*(.+)$/i)?.[1].split(',') ?? []),
    ].map(modelKey).filter(Boolean);
    // Retired Argon groups are filtered before matching so they neither supply
    // their stale buckets nor double up with the shared Gemini group.
    const activeGroups = groups.filter(group => !retiredArgonGroup(group));
    const explicit = activeGroups.filter(group => identities(group).some(id => selected.has(id)));
    const inferred = activeGroups.filter(group => groupFamily && !identities(group).length &&
      group.buckets?.some(bucket => family(bucket.bucketId) === groupFamily));
    let group = explicit.length === 1 ? explicit[0] :
      !explicit.length && inferred.length === 1 ? inferred[0] : null;
    if (!group && !explicit.length && !inferred.length && groupFamily === 'gemini' &&
        groups.some(retiredArgonGroup)) {
      // Argon retired into the shared Gemini pool; fall back to the shared bucket.
      group = activeGroups.find(item =>
        item.buckets?.some(bucket => sharedGeminiBucket(bucket.bucketId))) ?? null;
    }
    const byPeriod = new Map();
    for (const bucket of group?.buckets ?? []) {
      if (retiredArgonBucket(bucket.bucketId)) continue;
      const period = bucket.window ||
        (/5h$/i.test(bucket.bucketId) ? '5h' : /weekly$/i.test(bucket.bucketId) ? 'weekly' : null);
      const fraction = bucket.remainingFraction;
      if (!period || !Number.isFinite(fraction) || fraction < 0 || fraction > 1) continue;
      const window = { period, percent: Math.round(fraction * 100), reset: bucket.resetTime };
      if (!byPeriod.has(period) || window.percent < byPeriod.get(period).percent) byPeriod.set(period, window);
    }
    if (byPeriod.size) {
      return [...byPeriod.values()].sort((left, right) => {
        const order = period => period === '5h' ? 0 : period === 'weekly' ? 1 : 2;
        return order(left.period) - order(right.period);
      });
    }
    if (explicit.length || inferred.length) return [];
    const candidates = modelConfigs(status).filter(config => configNames(config).some(key => selected.has(key)));
    const own = candidates.length === 1 ? candidates[0].quotaInfo : null;
    return Number.isFinite(own?.remainingFraction) && own.remainingFraction >= 0 && own.remainingFraction <= 1
      ? [{ percent: Math.round(100 * own.remainingFraction), reset: own.resetTime }] : [];
  }

  function seconds(duration) {
    if (typeof duration === 'number') return Number.isFinite(duration) && duration >= 0 ? duration : null;
    if (typeof duration !== 'string') return null;
    const trimmed = duration.trim();
    if (/^\d+(?:\.\d+)?ms$/i.test(trimmed)) {
      const value = Number(trimmed.slice(0, -2)) / 1000;
      return Number.isFinite(value) ? value : null;
    }
    if (!/^\d+(?:\.\d+)?s$/i.test(trimmed)) return null;
    const value = Number(trimmed.slice(0, -1));
    return Number.isFinite(value) ? value : null;
  }

  function nonNegativeInteger(value) {
    const number = typeof value === 'string' && /^\d+$/.test(value) ? Number(value) : value;
    return Number.isSafeInteger(number) && number >= 0 ? number : null;
  }

  function computeSpeedSample(model, retry) {
    const stream = seconds(model.streamingDuration);
    const first = seconds(model.timeToFirstToken);
    if (stream === null || stream < 0.05) return null;
    const usage = retry?.usage ?? model.usage;
    const combined = nonNegativeInteger(usage?.outputTokens);
    const thinking = nonNegativeInteger(usage?.thinkingOutputTokens ?? usage?.thinkingTokens);
    const explicitOutput = nonNegativeInteger(usage?.responseOutputTokens);
    const output = explicitOutput ?? (combined !== null ? Math.max(0, combined - (thinking ?? 0)) : null);
    const total = combined ?? (output !== null && thinking !== null ? output + thinking : null);
    const effectiveOutput = output !== null && output > 0 ? output : total;
    if (effectiveOutput === null || effectiveOutput <= 0) return null;
    const elapsedSeconds = first !== null ? first + stream : stream;
    if (!Number.isFinite(elapsedSeconds) || elapsedSeconds <= 0) return null;
    return {
      rate: effectiveOutput / elapsedSeconds,
      overallRate: total === null ? null : total / elapsedSeconds,
      streamingRate: total === null ? null : total / stream,
      first,
      outputTokens: effectiveOutput,
      elapsedSeconds,
      hasTextOutput: output !== null && output > 0,
    };
  }

  function sessionMetrics(rows, status, label) {
    const selected = aliases(label, status);
    let context = null;
    let fallbackContext = null;
    let speed = null;
    let fallbackToolSpeed = null;
    let fallbackAnyModelSpeed = null;
    let hasCompletedSample = false;
    for (let i = rows.length - 1; i >= 0; i -= 1) {
      const row = rows[i];
      const model = row?.chatModel;
      if (!model) continue;
      const retry = model.retryInfos?.at(-1);
      if (model.error || retry?.error || row.error) continue;
      const window = model.chatStartMetadata?.contextWindowMetadata;
      const used = nonNegativeInteger(window?.estimatedTokensUsed);
      const max = nonNegativeInteger(window?.maxContextTokens);
      if (!fallbackContext && used !== null && max > 0) fallbackContext = { used, max };
      const isRowStreaming = Boolean(row.isStreaming || row.status === 'streaming');
      const stream = seconds(model.streamingDuration);
      const isRowCompleted = !isRowStreaming && (stream !== null || Boolean(model.finishReason) || row.done === true);
      if (isRowCompleted && !fallbackAnyModelSpeed) {
        const anySample = computeSpeedSample(model, retry);
        if (anySample) {
          const { hasTextOutput: _ignored, ...cleanAny } = anySample;
          fallbackAnyModelSpeed = cleanAny;
        }
      }
      const reported = [model.modelDisplayName, model.responseModel, model.chatStartMetadata?.model,
        model.model, model.modelOrAlias?.model, model.modelOrAlias?.alias].map(modelKey).filter(Boolean);
      if (reported.length && !reported.some(id => selected.has(id))) continue;
      if (!context && used !== null && max > 0) context = { used, max };
      if (!isRowCompleted) continue;
      hasCompletedSample = true;
      if (speed) continue;
      const sample = computeSpeedSample(model, retry);
      if (!sample) continue;
      const { hasTextOutput, ...cleanSample } = sample;
      if (hasTextOutput) {
        speed = cleanSample;
      } else if (!fallbackToolSpeed) {
        fallbackToolSpeed = cleanSample;
      }
    }
    return {
      context: context ?? fallbackContext,
      speed: speed ?? fallbackToolSpeed ?? (!hasCompletedSample ? fallbackAnyModelSpeed : null),
      hasCompletedSample: hasCompletedSample || Boolean(fallbackAnyModelSpeed),
    };
  }

  return { quotaWindows, sessionMetrics };
})()
