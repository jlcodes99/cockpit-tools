import type { ProxyCatalogSource, ProxyCatalogSelections } from '../services/codexProxyCatalogService';

/** Resolve explicit manual choices only. Automatic policies never invent a current node. */
export function proxyPickerTarget(source: ProxyCatalogSource, itemId: string, selections: ProxyCatalogSelections) {
  let target = [...source.nodes, ...source.groups].find((item) => item.id === itemId);
  const visited = new Set<string>();
  while (target && 'kind' in target && target.kind === 'select' && !visited.has(target.id)) {
    visited.add(target.id);
    const name = selections[target.id];
    const next = [...source.nodes, ...source.groups].find((item) => item.name === name);
    if (!next || visited.has(next.id)) break;
    target = next;
  }
  return target;
}

/** A whole source remains inspectable when its nodes only need permission or a refresh. */
export function proxySourceInspectable(source: ProxyCatalogSource): boolean {
  return source.needsRefresh === true || source.nodes.length > 0 || source.groups.length > 0;
}
