import type { CodexModelProvider } from "../services/codexModelProviderService";
import { resolveCodexModelProviderForApiKey } from "./codexModelProviderKeyConfig";
import {
  expandLegacyProviderVisionCapabilities,
  materializeProviderModelVisionCapabilities,
} from "./codexModelProviderVision";

/** Preserve credential defaults before filling the provider's inherited capability table. */
export function migrateCodexModelProviderVisionCapabilities(
  provider: CodexModelProvider,
): boolean {
  let changed = false;
  for (const apiKey of provider.apiKeys) {
    // Only a credential default without its own map can inherit newly generated
    // provider values. Preserve explicit maps, including sparse and empty maps.
    if (apiKey.supportsVision === undefined || apiKey.modelCapabilities !== undefined) {
      continue;
    }
    const effective = resolveCodexModelProviderForApiKey(provider, apiKey.apiKey);
    if (materializeProviderModelVisionCapabilities(effective)) {
      apiKey.modelCapabilities = effective.modelCapabilities;
      apiKey.updatedAt = Date.now();
      changed = true;
    }
  }
  changed = expandLegacyProviderVisionCapabilities(provider) || changed;
  return materializeProviderModelVisionCapabilities(provider) || changed;
}
