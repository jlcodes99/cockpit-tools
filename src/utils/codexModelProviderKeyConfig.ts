import type { CodexModelProvider } from "../services/codexModelProviderService";

/** Optional per-credential model settings. Missing fields inherit legacy provider defaults. */
export interface CodexProviderModelConfig {
  modelCatalog?: string[];
  modelContextWindows?: Record<string, number>;
  supportsVision?: boolean;
  modelCapabilities?: Record<string, { supportsVision?: boolean }>;
  visionRoutingModel?: string | null;
}

export function cloneCodexProviderModelConfig(config: CodexProviderModelConfig): CodexProviderModelConfig {
  return {
    ...(config.modelCatalog !== undefined ? { modelCatalog: [...config.modelCatalog] } : {}),
    ...(config.modelContextWindows !== undefined ? { modelContextWindows: { ...config.modelContextWindows } } : {}),
    ...(config.supportsVision !== undefined ? { supportsVision: config.supportsVision } : {}),
    ...(config.modelCapabilities !== undefined ? {
      modelCapabilities: Object.fromEntries(Object.entries(config.modelCapabilities).map(([model, capability]) => [model, { ...capability }])),
    } : {}),
    ...(config.visionRoutingModel !== undefined ? { visionRoutingModel: config.visionRoutingModel } : {}),
  };
}

/** Resolve by credential, never by display name: names are optional and need not be unique. */
export function resolveCodexModelProviderForApiKey(
  provider: CodexModelProvider,
  apiKey?: string | null,
): CodexModelProvider {
  const key = provider.apiKeys.find((item) => apiKey?.trim() && item.apiKey.trim() === apiKey.trim());
  if (!key) return provider;
  const config = cloneCodexProviderModelConfig(key);
  // Once a key owns a catalog, inherited metadata can only refer to models in that catalog.
  const modelIds = config.modelCatalog === undefined ? null : new Set(config.modelCatalog.map((model) => model.toLowerCase()));
  const contexts = modelIds && config.modelContextWindows === undefined
    ? Object.fromEntries(Object.entries(provider.modelContextWindows ?? {}).filter(([model]) => modelIds.has(model.toLowerCase())))
    : config.modelContextWindows ?? provider.modelContextWindows;
  const capabilities = modelIds && config.modelCapabilities === undefined
    ? Object.fromEntries(Object.entries(provider.modelCapabilities ?? {}).filter(([model]) => modelIds.has(model.toLowerCase())))
    : config.modelCapabilities ?? provider.modelCapabilities;
  const inheritedVisionModel = provider.visionRoutingModel && (!modelIds || modelIds.has(provider.visionRoutingModel.toLowerCase()))
    ? provider.visionRoutingModel : undefined;
  // Empty arrays/maps and an explicit null routing model are preserved opt-outs.
  return { ...provider, ...config, modelContextWindows: contexts, modelCapabilities: capabilities,
    visionRoutingModel: config.visionRoutingModel === null ? undefined : config.visionRoutingModel ?? inheritedVisionModel };
}
