import type { CodexModelProvider } from '../../../services/codexModelProviderService';
import type { CodexPelicanProviderTarget } from '../../../types/codexPelican';
import { CODEX_PELICAN_DEFAULT_MODEL } from '../../../types/codexPelican';
import { resolveCodexModelProviderForApiKey } from "../../../utils/codexModelProviderKeyConfig";
import { isImageGenerationModelId } from '../../../utils/codexTestModel';

export function pelicanProviderModels(provider: CodexModelProvider, apiKeyId?: string): string[] {
  provider = resolveCodexModelProviderForApiKey(provider, provider.apiKeys.find((key) => key.id === apiKeyId)?.apiKey);
  return [...new Set((provider.modelCatalog ?? []).map((model) => model.trim()).filter((model) => model && !isImageGenerationModelId(model)))];
}

export function pelicanProviderTarget(provider: CodexModelProvider, apiKeyId?: string): CodexPelicanProviderTarget | null {
  const key = provider.apiKeys.find((key) => key.id === apiKeyId && key.apiKey.trim())
    ?? provider.apiKeys.find((key) => key.apiKey.trim());
  if (!key) return null;
  const models = pelicanProviderModels(provider, key.id);
  return { providerId: provider.id, apiKeyId: key.id,
    model: models.find((model) => model.toLowerCase() === CODEX_PELICAN_DEFAULT_MODEL) ?? models[0] ?? CODEX_PELICAN_DEFAULT_MODEL };
}

export function validatePelicanProviderTargets(providers: CodexModelProvider[], targets: CodexPelicanProviderTarget[]): boolean {
  return targets.length > 0 && targets.length <= 200 && targets.every((target) => target.model.trim() && target.model.length <= 128
    && providers.some((provider) => provider.id === target.providerId && provider.apiKeys.some((key) => key.id === target.apiKeyId && key.apiKey.trim())));
}
