export interface CodexModelSourceLike {
  label: string;
  kind: string;
}

export const modelSourceGroupKey = (
  modelId: string,
  resolveSource?: (modelId: string) => CodexModelSourceLike | undefined,
): string => {
  const source = resolveSource?.(modelId);
  if (source?.label.trim()) {
    return `${source.kind}:${source.label.trim().toLowerCase()}`;
  }
  const slash = modelId.trim().indexOf("/");
  if (slash > 0) {
    return `api:${modelId.trim().slice(0, slash).toLowerCase()}`;
  }
  return "subscription";
};

export const insertModelsBySource = <T extends { model_id: string }>(
  models: T[],
  toAdd: T[],
  resolveSource?: (modelId: string) => CodexModelSourceLike | undefined,
): T[] => {
  if (toAdd.length === 0) return models;
  const next = models.slice();
  for (const item of toAdd) {
    const key = modelSourceGroupKey(item.model_id, resolveSource);
    let insertAt = next.length;
    for (let index = next.length - 1; index >= 0; index -= 1) {
      if (modelSourceGroupKey(next[index].model_id, resolveSource) === key) {
        insertAt = index + 1;
        break;
      }
    }
    next.splice(insertAt, 0, item);
  }
  return next;
};

export const moveModel = <T>(models: T[], from: number, to: number): T[] => {
  if (
    from === to ||
    from < 0 ||
    to < 0 ||
    from >= models.length ||
    to >= models.length
  ) {
    return models;
  }
  const next = models.slice();
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
};
