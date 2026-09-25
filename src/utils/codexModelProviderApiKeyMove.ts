import type { CodexExperimentalModelDefinition } from '../types/codex';

export interface MovableCodexProviderApiKey {
  id: string;
  name: string;
  apiKey: string;
  modelCatalog?: string[];
  modelContextWindows?: Record<string, number>;
  modelAutoCompactTokenLimits?: Record<string, number>;
  compactionMode?: 'auto' | 'remote' | 'local';
  modelDefinitions?: CodexExperimentalModelDefinition[];
  defaultModelId?: string;
  createdAt: number;
  updatedAt: number;
}

export interface CodexProviderApiKeyOwner {
  id: string;
  apiKeys: MovableCodexProviderApiKey[];
  updatedAt: number;
}

export type CodexProviderApiKeyMoveResult =
  | "not_moved"
  | "moved"
  | "deduplicated"
  | "name_conflict";

export function moveCodexProviderApiKey(
  providers: CodexProviderApiKeyOwner[],
  previousProviderId: string | null | undefined,
  targetProviderId: string,
  apiKey: string,
  now = Date.now(),
): CodexProviderApiKeyMoveResult {
  const sourceId = previousProviderId?.trim();
  const targetId = targetProviderId.trim();
  const normalizedApiKey = apiKey.trim();
  if (!sourceId || !targetId || sourceId === targetId || !normalizedApiKey) {
    return "not_moved";
  }

  const source = providers.find((provider) => provider.id === sourceId);
  const target = providers.find((provider) => provider.id === targetId);
  if (!source || !target) return "not_moved";

  const sourceIndex = source.apiKeys.findIndex(
    (item) => item.apiKey.trim() === normalizedApiKey,
  );
  if (sourceIndex < 0) return "not_moved";

  const sourceApiKey = source.apiKeys[sourceIndex];
  const targetApiKey = target.apiKeys.find(
    (item) => item.apiKey.trim() === normalizedApiKey,
  );
  if (targetApiKey) {
    const sourceName = sourceApiKey.name.trim();
    const targetName = targetApiKey.name.trim();
    if (sourceName && targetName && sourceName !== targetName) {
      return "name_conflict";
    }
    if (!targetName && sourceName) {
      targetApiKey.name = sourceName;
    }
    if (targetApiKey.modelCatalog === undefined && sourceApiKey.modelCatalog !== undefined) {
      targetApiKey.modelCatalog = [...sourceApiKey.modelCatalog];
      targetApiKey.modelContextWindows = { ...sourceApiKey.modelContextWindows };
      targetApiKey.modelAutoCompactTokenLimits = { ...sourceApiKey.modelAutoCompactTokenLimits };
      targetApiKey.compactionMode = sourceApiKey.compactionMode;
      targetApiKey.modelDefinitions = sourceApiKey.modelDefinitions?.map((model) => ({ ...model }));
      targetApiKey.defaultModelId = sourceApiKey.defaultModelId;
    }
    targetApiKey.updatedAt = now;
    source.apiKeys.splice(sourceIndex, 1);
    source.updatedAt = now;
    target.updatedAt = now;
    return "deduplicated";
  }

  source.apiKeys.splice(sourceIndex, 1);
  target.apiKeys.push({
    ...sourceApiKey,
    apiKey: normalizedApiKey,
    updatedAt: now,
  });
  source.updatedAt = now;
  target.updatedAt = now;
  return "moved";
}
