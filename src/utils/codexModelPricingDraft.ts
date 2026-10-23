export interface ModelPricingDraft {
  modelId: string;
  longContextThresholdTokens: string;
  inputUsdPerMillion: string;
  cachedInputUsdPerMillion: string;
  outputUsdPerMillion: string;
  standardLongInputUsdPerMillion: string;
  standardLongCachedInputUsdPerMillion: string;
  standardLongOutputUsdPerMillion: string;
  standardLongPriceOverride: boolean;
  priorityInputUsdPerMillion: string;
  priorityCachedInputUsdPerMillion: string;
  priorityOutputUsdPerMillion: string;
  hasPreset: boolean;
  custom: boolean;
}

export type ModelPricingDraftField = keyof Omit<ModelPricingDraft,
  'modelId' | 'hasPreset' | 'custom' | 'standardLongPriceOverride'>;

const longFields = [
  'standardLongInputUsdPerMillion',
  'standardLongCachedInputUsdPerMillion',
  'standardLongOutputUsdPerMillion',
] as const;

function longContextOutputMultiplier(modelId: string): number {
  const segments = modelId.trim().split('/');
  const model = segments[segments.length - 1].replace(/\([^()]*\)$/, '').toLowerCase();
  return model === 'grok-4.7' || model === 'grok-4.7-latest' ? 2 : 1.5;
}

export function updateModelPricingDraft(
  draft: ModelPricingDraft, field: ModelPricingDraftField, value: string,
): ModelPricingDraft {
  const next = { ...draft, [field]: value };
  if (longFields.some((key) => key === field)) {
    next.standardLongPriceOverride = longFields.some((key) => next[key].trim() !== '');
  } else if (!next.standardLongPriceOverride && (
    field === 'inputUsdPerMillion' || field === 'cachedInputUsdPerMillion'
    || field === 'outputUsdPerMillion'
  )) {
    const scaled = (raw: string, multiplier: number) => {
      const parsed = Number(raw);
      return raw.trim() && Number.isFinite(parsed) && parsed >= 0
        ? String(parsed * multiplier) : '';
    };
    // Keep unedited display prices derived from the base, including cache fallback.
    // Models without a long-context tier retain empty fields.
    if (next.longContextThresholdTokens.trim()) {
      next.standardLongInputUsdPerMillion = scaled(next.inputUsdPerMillion, 2);
      next.standardLongCachedInputUsdPerMillion = scaled(
        next.cachedInputUsdPerMillion.trim() ? next.cachedInputUsdPerMillion : next.inputUsdPerMillion, 2,
      );
      next.standardLongOutputUsdPerMillion = scaled(
        next.outputUsdPerMillion, longContextOutputMultiplier(next.modelId),
      );
    }
  }
  return next;
}
