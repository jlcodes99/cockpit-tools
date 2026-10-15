import type { CodexAccount } from '../types/codex';
import type { ProxyCatalog, ProxyCatalogSource, ProxyCatalogSelections } from '../services/codexProxyCatalogService';
import { isCatalogBlockingMember } from '../services/codexProxyCatalogService';
import { proxySourceInspectable } from './codexProxyPickerModel';

/** Restore identity only. Missing resources must never silently select a replacement. */
export function restoreProxySelection(catalog: ProxyCatalog, saved: CodexAccount['egress_proxy']) {
  const source = saved?.sourceId
    ? catalog.sources.find((entry) => entry.id === saved.sourceId)
    : catalog.sources.find(proxySourceInspectable);
  const itemId = source && saved?.sourceId === source.id
    && [...source.nodes, ...source.groups].some((entry) => entry.id === saved.itemId) ? saved.itemId! : '';
  return { sourceId: source?.id ?? '', itemId, groupId: source ? proxySelectionGroup(source, itemId, saved?.groupId) : '' };
}

export function proxySelectionGroup(source: ProxyCatalogSource, itemId: string, savedGroupId?: string | null): string {
  const selectedGroup = source.groups.find((group) => group.id === itemId);
  const item = selectedGroup ?? source.nodes.find((entry) => entry.id === itemId);
  // A node or subgroup can belong to multiple groups. Restore only an explicit parent.
  if (item && source.groups.some((group) => group.id === savedGroupId && group.members.includes(item.name))) return savedGroupId!;
  return selectedGroup?.id ?? '';
}

/** A saved source default only pre-fills the draft. Unreachable defaults return null
 * so the caller keeps its own behaviour instead of guessing a node or member.
 */
export function sourceDefaultDraft(source?: ProxyCatalogSource | null): { itemId: string; groupId: string; selections: ProxyCatalogSelections } | null {
  const saved = source?.default;
  if (!source || !saved?.itemId) return null;
  const selections = defaultProxySelections(source, saved.itemId, saved.selections ?? {});
  return selections === null ? null
    : { itemId: saved.itemId, groupId: proxySelectionGroup(source, saved.itemId, saved.groupId), selections };
}

/** A saved root selector exposes its member name without revealing proxy credentials. */
export function savedRootProxySelections(source: ProxyCatalogSource, saved: CodexAccount['egress_proxy']): ProxyCatalogSelections {
  const group = source.groups.find((entry) => entry.id === saved?.itemId && entry.kind === 'select');
  const member = saved?.selectedName;
  return group && member && group.members.includes(member) ? { [group.id]: member } : {};
}

/** Resolve only explicitly chosen manual members; automatic groups retain their policy.
 * Return null for missing choices, cycles, unavailable members or excessive nesting.
 */
export function defaultProxySelections(source: ProxyCatalogSource, itemId: string, chosen: ProxyCatalogSelections = {}): ProxyCatalogSelections | null {
  const nodes = new Map(source.nodes.map((node) => [node.name, node]));
  const groups = new Map(source.groups.map((group) => [group.name, group]));
  let remaining = 512;
  const visit = (name: string, path: Set<string>): ProxyCatalogSelections | null => {
    if (--remaining < 0) return null;
    // Blocking rules are real group members; they must never turn into a direct fallback.
    if (isCatalogBlockingMember(name)) return {};
    if (['DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE'].includes(name)) return null;
    const node = nodes.get(name);
    if (node) return node.supported ? {} : null;
    const group = groups.get(name);
    if (!group?.supported || path.has(name) || path.size >= 16) return null;
    const nextPath = new Set(path).add(name);
    if (group.kind === 'select') {
      const member = chosen[group.id];
      if (!member || !group.members.includes(member)) return null;
      const nested = visit(member, nextPath);
      return nested ? { ...nested, [group.id]: member } : null;
    }
    if (!['url-test', 'fallback', 'load-balance'].includes(group.kind) || !group.members.length) return null;
    let result: ProxyCatalogSelections = {};
    for (const member of group.members) {
      const nested = visit(member, nextPath);
      if (!nested) return null;
      result = { ...result, ...nested };
    }
    return result;
  };
  const item = [...source.nodes, ...source.groups].find((entry) => entry.id === itemId);
  // Built-ins may only be reached through a selected group, never bound as a node.
  return item && !isCatalogBlockingMember(item.name) ? visit(item.name, new Set()) : null;
}
