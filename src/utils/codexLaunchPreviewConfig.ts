import type { CodexQuickConfig } from '../types/codex';
import type { InstanceProfile } from '../types/instance';
import { buildCodexModelRoutingValue } from './codexModelRoutingValue';

// Only compare the configuration that this editor can overwrite. Runtime fields
// (PID, running state, timestamps, speed) must not invalidate an editing session.
export function codexLaunchPreviewInstanceConfigKey(instance?: InstanceProfile): string {
  return JSON.stringify({
    userDataDir: instance?.userDataDir ?? null,
    bindAccountId: instance?.bindAccountId ?? null,
    followLocalAccount: instance?.followLocalAccount ?? false,
    modelRouting: instance?.modelRouting ?? buildCodexModelRoutingValue(false, []),
  });
}

export function codexLaunchPreviewQuickConfigKey(config: CodexQuickConfig): string {
  return JSON.stringify({
    context: config.detected_model_context_window ?? null,
    compact: config.detected_auto_compact_token_limit ?? null,
    enabled: config.experimental_model_catalog_enabled,
    available: config.experimental_model_catalog_available,
    conflict: config.experimental_model_catalog_conflict ?? null,
    defaultModel: config.experimental_model_catalog_default_model_id ?? null,
    models: config.experimental_model_catalog_models.map((model) => ({
      id: model.model_id,
      name: model.display_name,
      reasoning: model.reasoning_efforts ?? null,
      context: model.context_window ?? null,
      compact: model.auto_compact_token_limit ?? null,
    })),
  });
}
