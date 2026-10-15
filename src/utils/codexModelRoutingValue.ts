import type { CodexInstanceApiRoute, CodexInstanceModelRouting } from "../types/instance.ts";

export const buildCodexModelRoutingValue = (
  enabled: boolean,
  routes: CodexInstanceApiRoute[],
): CodexInstanceModelRouting => ({
  // null means "no update" to Tauri. Retain routes while explicitly disabling.
  enabled,
  version: 1,
  routes: routes.map((route) => ({
    ...route,
    selectedModels: route.selectedModels?.slice(),
    extraModels: route.extraModels?.slice(),
  })),
});

export const areCodexModelRoutingsEqual = (
  previous: CodexInstanceModelRouting | null | undefined,
  next: CodexInstanceModelRouting | null | undefined,
): boolean =>
  JSON.stringify(previous ?? buildCodexModelRoutingValue(false, [])) ===
  JSON.stringify(next ?? buildCodexModelRoutingValue(false, []));

export const resolveRoutingCatalog = <T extends { model_id: string }>(
  models: T[],
  enabled: boolean,
  defaultModelId: string | null | undefined,
) => ({
  enabled: enabled && models.length > 0,
  models,
  defaultModelId: models.some((model) => model.model_id === defaultModelId)
    ? defaultModelId ?? null
    : null,
});
