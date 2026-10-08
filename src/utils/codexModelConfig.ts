import type { CodexExperimentalModelDefinition, CodexReasoningEffort } from '../types/codex';

const ERROR_KEYS: Record<string, string> = {
  MODEL_CONFIG_JSON_INVALID: 'json', MODEL_CONFIG_VERSION_UNSUPPORTED: 'version',
  MODEL_CONFIG_FIELDS_UNSUPPORTED: 'fields', MODEL_CONFIG_TOO_LARGE: 'tooLarge',
  MODEL_CONFIG_TOO_MANY_ITEMS: 'tooMany', MODEL_CONFIG_DUPLICATE: 'duplicate',
  MODEL_CONFIG_STATE_CHANGED: 'stateChanged', MODEL_CONFIG_ACCOUNT_MISSING: 'accountMissing',
  MODEL_CONFIG_MODEL_MISSING: 'modelMissing', MODEL_CONFIG_SERVICE_MISSING: 'serviceMissing',
  MODEL_CONFIG_PRICE_INVALID: 'price', MODEL_CONFIG_ALIAS_INVALID: 'alias', MODEL_CONFIG_RULE_INVALID: 'rule',
  MODEL_CONFIG_BUSY: 'busy', MODEL_CONFIG_INSTANCE_MISSING: 'instanceMissing',
  MODEL_CONFIG_READ_FAILED: 'read', MODEL_CONFIG_WRITE_FAILED: 'write', MODEL_CONFIG_SERIALIZE_FAILED: 'write',
  MODEL_CONFIG_VALIDATION_FAILED: 'validation', MODEL_CONFIG_RECOVERY_INVALID: 'recovery',
  MODEL_CONFIG_RECOVERY_CONFLICT: 'recovery', MODEL_CONFIG_EXISTING_INVALID: 'recovery',
  MODEL_CONFIG_STRATEGY_INVALID: 'invalid',
};
const MODEL_ERROR_KEYS: Record<string, string> = {
  EXPERIMENTAL_MODEL_CATALOG_DEFAULT_REASONING_INVALID: 'codex.experimentalModels.defaultReasoningInvalid',
  EXPERIMENTAL_MODEL_CATALOG_MODELS_REQUIRED: 'codex.experimentalModelCatalog.models.validation.required',
  EXPERIMENTAL_MODEL_CATALOG_MODEL_ID_INVALID: 'codex.experimentalModelCatalog.models.validation.modelId',
  EXPERIMENTAL_MODEL_CATALOG_DISPLAY_NAME_INVALID: 'codex.experimentalModelCatalog.models.validation.displayName',
  EXPERIMENTAL_MODEL_CATALOG_MODEL_ID_DUPLICATE: 'codex.experimentalModelCatalog.models.validation.duplicate',
  EXPERIMENTAL_MODEL_CATALOG_CONTEXT_WINDOW_INVALID: 'codex.experimentalModelCatalog.models.validation.contextWindow',
  EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_INVALID: 'codex.experimentalModelCatalog.models.validation.autoCompact',
  EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_RANGE_INVALID: 'codex.experimentalModelCatalog.models.validation.autoCompactRange',
};

export function codexModelConfigErrorKey(error: unknown, fallback = 'invalid'): string {
  const text = String(error);
  const modelCode = Object.keys(MODEL_ERROR_KEYS).find((code) => text.includes(code));
  if (modelCode) return MODEL_ERROR_KEYS[modelCode];
  const code = Object.keys(ERROR_KEYS).find((code) => text.includes(code));
  return `codex.modelConfig.errors.${code ? ERROR_KEYS[code] : fallback}`;
}

/** Capability updates clear an invalid override while preserving inherited behavior. */
export function withSupportedReasoningEfforts(
  model: CodexExperimentalModelDefinition,
  reasoningEfforts: CodexReasoningEffort[] | undefined,
  inheritedEfforts?: readonly CodexReasoningEffort[],
): CodexExperimentalModelDefinition {
  const supported = reasoningEfforts?.length ? reasoningEfforts : inheritedEfforts;
  return {
    ...model,
    reasoning_efforts: reasoningEfforts?.length ? reasoningEfforts : undefined,
    default_reasoning_effort: model.default_reasoning_effort && supported
      && !supported.includes(model.default_reasoning_effort) ? undefined : model.default_reasoning_effort,
  };
}
