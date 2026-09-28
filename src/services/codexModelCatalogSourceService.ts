import { invoke } from "@tauri-apps/api/core";
import type { CodexExperimentalModelDefinition } from "../types/codex";

export type CodexModelCatalogSource = "codex" | "upstream" | "cockpit";

export type CodexModelSourceMetadata = {
  contextWindow?: number;
  autoCompactTokenLimit?: number;
};

export type CodexModelCatalogSourceInfo = {
  source: CodexModelCatalogSource;
  fetchedAt: number;
  manuallyAdjusted?: boolean;
  cacheInfo?: string;
  modelMetadata?: Record<string, CodexModelSourceMetadata>;
};

export function normalizeCodexModelCatalogSourceInfo(value: unknown): CodexModelCatalogSourceInfo | undefined {
  if (!value || typeof value !== "object") return undefined;
  const source = (value as { source?: unknown }).source;
  const fetchedAt = (value as { fetchedAt?: unknown }).fetchedAt;
  if (source !== "codex" && source !== "upstream" && source !== "cockpit") return undefined;
  if (typeof fetchedAt !== "number" || !Number.isFinite(fetchedAt) || fetchedAt <= 0) return undefined;
  return {
    source,
    fetchedAt,
    manuallyAdjusted: (value as { manuallyAdjusted?: unknown }).manuallyAdjusted === true,
    cacheInfo: typeof (value as { cacheInfo?: unknown }).cacheInfo === "string"
      ? (value as { cacheInfo: string }).cacheInfo
      : undefined,
    modelMetadata: Object.fromEntries(Object.entries(
      (value as { modelMetadata?: unknown }).modelMetadata &&
        typeof (value as { modelMetadata?: unknown }).modelMetadata === "object"
        ? (value as { modelMetadata: Record<string, unknown> }).modelMetadata : {},
    ).flatMap(([id, metadata]) => {
      if (!metadata || typeof metadata !== "object") return [];
      const window = (metadata as { contextWindow?: unknown }).contextWindow;
      const limit = (metadata as { autoCompactTokenLimit?: unknown }).autoCompactTokenLimit;
      const contextWindow = typeof window === "number" && Number.isSafeInteger(window) && window > 0 ? window : undefined;
      const autoCompactTokenLimit = typeof limit === "number" && Number.isSafeInteger(limit) && limit > 0 ? limit : undefined;
      return contextWindow || autoCompactTokenLimit ? [[id, { contextWindow, autoCompactTokenLimit }]] : [];
    })),
  };
}

export type CodexModelCatalogSourceResult = {
  source: CodexModelCatalogSource;
  models: CodexExperimentalModelDefinition[];
};

/** Fetch the official Codex catalog through the bound OAuth app-server profile. */
export async function listCodexModelCatalogSourceModels(input: {
  source: "codex" | "cockpit";
  accountId?: string | null;
  instanceId?: string | null;
}): Promise<CodexExperimentalModelDefinition[]> {
  return await invoke<CodexExperimentalModelDefinition[]>(
    "codex_list_model_catalog_source_models",
    {
      source: input.source,
      accountId: input.accountId ?? null,
      instanceId: input.instanceId ?? null,
    },
  );
}
