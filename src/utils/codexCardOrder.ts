/** Reorder visible account slots without moving filtered-out accounts or other pages. */
export function reorderCodexCardIds(
  allIds: readonly string[],
  visibleIds: readonly string[],
  sourceId: string,
  targetId: string,
): string[] {
  const visible = new Set(visibleIds);
  // The global list is authoritative if visible IDs are stale or repeated.
  const reordered = allIds.filter((id) => visible.has(id));
  const sourceIndex = reordered.indexOf(sourceId);
  const targetIndex = reordered.indexOf(targetId);
  if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) {
    return [...allIds];
  }

  const [moved] = reordered.splice(sourceIndex, 1);
  reordered.splice(targetIndex, 0, moved);
  let visibleIndex = 0;
  return allIds.map((id) => visible.has(id) ? reordered[visibleIndex++] : id);
}

/** Retain saved IDs not currently loaded so a view change cannot erase their preference. */
export function mergeCodexCardOrder(
  savedIds: readonly string[],
  orderedIds: readonly string[],
): string[] {
  return [...new Set([...orderedIds, ...savedIds])];
}
