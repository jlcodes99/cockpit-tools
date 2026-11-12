import type { CodexLocalAccessModelAlias } from "../types/codexLocalAccess";

export function parseModelAliasText(value: string): CodexLocalAccessModelAlias[] {
  const seen = new Set<string>();
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const fork = /\s(\+|fork)$/i.test(line);
      const cleaned = line.replace(/\s(\+|fork)$/i, "").trim();
      const parts = cleaned.includes("=>")
        ? cleaned.split("=>")
        : cleaned.split(/\s+as\s+/i);
      const sourceModel = parts[0]?.trim() ?? "";
      const alias = parts[1]?.trim() ?? "";
      if (!sourceModel || !alias) return null;
      const key = alias.toLowerCase();
      if (seen.has(key)) return null;
      seen.add(key);
      return { sourceModel, alias, fork };
    })
    .filter((item): item is CodexLocalAccessModelAlias => Boolean(item));
}

export function serializeModelAliases(
  values: CodexLocalAccessModelAlias[] | null | undefined,
): string {
  return (values ?? [])
    .map(
      (item) => `${item.sourceModel} => ${item.alias}${item.fork ? " +" : ""}`,
    )
    .join("\n");
}

const claudeCodePreset = [
  { alias: "claude-fable-5", sourceModel: "gpt-5.6-sol" },
  { alias: "claude-opus-4-8", sourceModel: "gpt-5.6-terra" },
  { alias: "claude-sonnet-5", sourceModel: "gpt-5.6-luna" },
  { alias: "claude-haiku-4-5-20251001", sourceModel: "gpt-5.6-luna" },
];

export function appendClaudeCodeModelAliases(draft: string, visibleModelIds: readonly string[]): string {
  const existingRules = parseModelAliasText(draft);
  const existingAliases = new Set(existingRules.map(({ alias }) => alias.toLowerCase()));
  const keepSourceModel = new Map<string, boolean>();
  for (const { sourceModel, fork } of existingRules) {
    const key = sourceModel.toLowerCase();
    keepSourceModel.set(key, fork || keepSourceModel.get(key) === true);
  }
  const visibleModels = new Set(visibleModelIds.map((model) => model.toLowerCase()));
  const additions = claudeCodePreset.filter(({ alias, sourceModel }) =>
    visibleModels.has(sourceModel) &&
    !visibleModels.has(alias) &&
    !existingAliases.has(alias) &&
    !existingAliases.has(sourceModel),
  );
  if (additions.length === 0) return draft;
  const separator = !draft || draft.endsWith("\n") ? "" : "\n";
  return draft + separator + serializeModelAliases(additions.map((item) => ({
    ...item,
    fork: keepSourceModel.get(item.sourceModel) ?? true,
  })));
}
