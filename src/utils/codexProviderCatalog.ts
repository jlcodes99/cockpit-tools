export type CodexCatalogToolProfile = "proxy_chat" | "native_responses";

export type CodexCatalogInputModality = "text" | "image";

export interface CodexCatalogModelEntry {
  model: string;
  displayName?: string;
  contextWindow?: number;
  supportsParallelToolCalls?: boolean;
  inputModalities?: CodexCatalogInputModality[];
  baseInstructions?: string;
}

export interface CodexStructuredModelCatalog {
  profile?: CodexCatalogToolProfile;
  models: CodexCatalogModelEntry[];
}

export type CodexModelCatalogInput =
  | string[]
  | CodexCatalogModelEntry[]
  | CodexStructuredModelCatalog
  | null
  | undefined;

function normalizeModelEntry(entry: string | CodexCatalogModelEntry): CodexCatalogModelEntry | null {
  if (typeof entry === "string") {
    const model = entry.trim();
    return model ? { model } : null;
  }

  const model = entry.model.trim();
  if (!model) {
    return null;
  }

  return {
    ...entry,
    model,
    displayName: entry.displayName?.trim() || undefined,
    inputModalities: entry.inputModalities?.filter(
      (value): value is CodexCatalogInputModality => value === "text" || value === "image",
    ),
    baseInstructions: entry.baseInstructions?.trim() || undefined,
  };
}

export function normalizeCodexModelCatalog(
  input: CodexModelCatalogInput,
): CodexStructuredModelCatalog {
  const profile =
    input && !Array.isArray(input) && "models" in input ? input.profile : undefined;
  const values = input && !Array.isArray(input) && "models" in input ? input.models : input ?? [];

  return {
    profile,
    models: values.map(normalizeModelEntry).filter((entry): entry is CodexCatalogModelEntry => !!entry),
  };
}

export function codexModelCatalogHasStructuredMetadata(input: CodexModelCatalogInput): boolean {
  return normalizeCodexModelCatalog(input).models.some(
    (entry) =>
      !!entry.displayName ||
      typeof entry.contextWindow === "number" ||
      typeof entry.supportsParallelToolCalls === "boolean" ||
      !!entry.inputModalities?.length ||
      !!entry.baseInstructions,
  );
}
