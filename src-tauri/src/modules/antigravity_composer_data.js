(() => {
  function modelKey(value) {
    return typeof value === 'string' ? value.split(' · ', 1)[0].toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim()
      .replace(/(?: (?:minimal|low|medium|high|xhigh|max|thinking|none))+$/, '')
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
    if (/^(?:\d+ )?argon( |$)/.test(key)) return 'argon';
    if (/^(?:\d+ )?carbon( |$)/.test(key)) return 'carbon';
    if (/^gemini( |$)|^\d+(?: \d+)* (?:neon|hydrogen|flash|pro|ultra)( |$)/.test(key)) return 'gemini';
    return null;
  }

  function quotaWindows(summary, status, label) {
    const groups = summary?.response?.groups ?? summary?.groups ?? [];
    const selected = aliases(label, status);
    const groupFamily = [...selected].map(family).find(Boolean);
    const identities = group => [
      ...(group.modelIds ?? []),
      ...(group.description?.match(/^\s*Models within this group:\s*(.+)$/i)?.[1].split(',') ?? []),
    ].map(modelKey).filter(Boolean);
    const explicit = groups.filter(group => identities(group).some(id => selected.has(id)));
    const inferred = groups.filter(group => groupFamily && !identities(group).length &&
      group.buckets?.some(bucket => family(bucket.bucketId) === groupFamily));
    const group = explicit.length === 1 ? explicit[0] :
      !explicit.length && inferred.length === 1 ? inferred[0] : null;
    const byPeriod = new Map();
    for (const bucket of group?.buckets ?? []) {
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
    if (typeof duration !== 'string' || !/^\d+(?:\.\d+)?s$/.test(duration)) return null;
    const value = Number(duration.slice(0, -1));
    return Number.isFinite(value) ? value : null;
  }

  function nonNegativeInteger(value) {
    const number = typeof value === 'string' && /^\d+$/.test(value) ? Number(value) : value;
    return Number.isSafeInteger(number) && number >= 0 ? number : null;
  }

  function sessionMetrics(rows, status, label) {
    const selected = aliases(label, status);
    let context = null;
    let speed = null;
    let hasCompletedSample = false;
    for (let i = rows.length - 1; i >= 0; i -= 1) {
      const row = rows[i];
      const model = row?.chatModel;
      if (!model) continue;
      const reported = [model.modelDisplayName, model.model, model.modelOrAlias?.model,
        model.modelOrAlias?.alias].map(modelKey).filter(Boolean);
      if (reported.length && !reported.some(id => selected.has(id))) continue;
      const retry = model.retryInfos?.at(-1);
      if (model.error || retry?.error || row.error) continue;
      const window = model.chatStartMetadata?.contextWindowMetadata;
      const used = nonNegativeInteger(window?.estimatedTokensUsed);
      const max = nonNegativeInteger(window?.maxContextTokens);
      if (!context && used !== null && max > 0) context = { used, max };
      if (hasCompletedSample || row.isStreaming || row.status === 'streaming') continue;
      const stream = seconds(model.streamingDuration);
      const first = seconds(model.timeToFirstToken);
      if (stream === null && !model.finishReason && row.done !== true) continue;
      hasCompletedSample = true;
      const usage = retry?.usage ?? model.usage;
      const combined = nonNegativeInteger(usage?.outputTokens);
      const thinking = nonNegativeInteger(usage?.thinkingOutputTokens ?? usage?.thinkingTokens);
      const output = nonNegativeInteger(usage?.responseOutputTokens) ?? Math.max(0, (combined ?? 0) - (thinking ?? 0));
      const elapsedSeconds = first === null || stream === null ? null : first + stream;
      if (output >= 100 && stream >= 0.1 && Number.isFinite(elapsedSeconds)) {
        const total = combined ?? (thinking === null ? null : output + thinking);
        speed = {
          rate: output / elapsedSeconds,
          overallRate: total === null ? null : total / elapsedSeconds,
          streamingRate: total === null ? null : total / stream,
          first, outputTokens: output, elapsedSeconds,
        };
      }
    }
    return { context, speed, hasCompletedSample };
  }

  return { quotaWindows, sessionMetrics };
})()
