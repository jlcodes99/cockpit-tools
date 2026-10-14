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
  return JSON.stringify([config.context_config_key, config.model_catalog_config_key]);
}
