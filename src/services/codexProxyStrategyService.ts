import { invoke } from '@tauri-apps/api/core';
import { withProxyEnginePrerequisite } from '../utils/codexProxyEnginePrerequisite';
import { catalogErrorKey, type ProxyCatalog, type ProxyCatalogNode, type ProxyCatalogSource } from './codexProxyCatalogService';

/** Group strategies a self-built policy may use; they map to the engine's own group kinds. */
export type ProxyStrategyKind = 'select' | 'fallback' | 'url-test' | 'load-balance';
export const PROXY_STRATEGY_KINDS: ProxyStrategyKind[] = ['select', 'fallback', 'url-test', 'load-balance'];

/** One ordered member: stable source and item ids, never a display name. */
export interface ProxyStrategyMember { sourceId: string; itemId: string }
export interface ProxyStrategyOptions { url?: string; interval?: number; timeout?: number; tolerance?: number; lazy?: boolean }
export interface ProxyStrategyDraft {
  id?: string;
  name: string;
  kind: ProxyStrategyKind;
  members: ProxyStrategyMember[];
  options: ProxyStrategyOptions;
}

/** Ranges the backend accepts; the dialog rejects anything outside before the call. */
export const PROXY_STRATEGY_LIMITS = {
  name: 80,
  members: 64,
  url: 2048,
  interval: { min: 30, max: 3600 },
  timeout: { min: 1, max: 30 },
  tolerance: { min: 0, max: 1000 },
} as const;

/** Presets the backend falls back to; a blank field keeps them. */
export const PROXY_STRATEGY_DEFAULTS = {
  url: 'https://www.gstatic.com/generate_204',
  interval: 180,
  timeout: 5,
  tolerance: 50,
  lazy: true,
} as const;

/** Advanced options exactly as typed; blanks are omitted from the payload. */
export interface ProxyStrategyOptionsForm { url: string; interval: string; timeout: string; tolerance: string; lazy: boolean }
export function emptyStrategyOptionsForm(): ProxyStrategyOptionsForm {
  return { url: '', interval: '', timeout: '', tolerance: '', lazy: PROXY_STRATEGY_DEFAULTS.lazy };
}

/** Restore a saved strategy's parameters; a blank field keeps the backend default. */
export function strategyOptionsForm(source?: ProxyCatalogSource | null): ProxyStrategyOptionsForm {
  const stored = source?.strategyOptions;
  if (!stored) return emptyStrategyOptionsForm();
  return {
    url: stored.url ?? '',
    interval: stored.interval == null ? '' : String(stored.interval),
    timeout: stored.timeout == null ? '' : String(stored.timeout),
    tolerance: stored.tolerance == null ? '' : String(stored.tolerance),
    lazy: stored.lazy ?? PROXY_STRATEGY_DEFAULTS.lazy,
  };
}

export type ProxyStrategyOptionField = 'url' | 'interval' | 'timeout' | 'tolerance';
/** Fields the engine actually applies per kind; the encoder silently ignores the rest. */
export const PROXY_STRATEGY_OPTION_FIELDS: Record<ProxyStrategyKind, ProxyStrategyOptionField[]> = {
  select: [],
  fallback: ['url', 'interval', 'timeout'],
  'url-test': ['url', 'interval', 'timeout', 'tolerance'],
  'load-balance': ['url', 'interval'],
};
const OPTION_ERROR_KEYS: Record<ProxyStrategyOptionField, string> = {
  url: 'codex.proxy.catalog.strategyErrorUrl',
  interval: 'codex.proxy.catalog.strategyErrorRange',
  timeout: 'codex.proxy.catalog.strategyErrorRange',
  tolerance: 'codex.proxy.catalog.strategyErrorRange',
};

function outOfRange(raw: string, min: number, max: number): boolean {
  const value = raw.trim();
  if (!value) return false;
  if (!/^\d+$/.test(value)) return true;
  const parsed = Number(value);
  return parsed < min || parsed > max;
}

/** A kind only ever reads its own options; the others must not reach the payload. */
function applicableForm(kind: ProxyStrategyKind, form: ProxyStrategyOptionsForm): ProxyStrategyOptionsForm {
  const fields = PROXY_STRATEGY_OPTION_FIELDS[kind];
  const keep = (field: ProxyStrategyOptionField, value: string) => fields.includes(field) ? value : '';
  return { ...form, url: keep('url', form.url), interval: keep('interval', form.interval), timeout: keep('timeout', form.timeout), tolerance: keep('tolerance', form.tolerance) };
}

/** The health check address must be a plain http(s) URL without credentials or fragment. */
function invalidTestUrl(raw: string): boolean {
  const value = raw.trim();
  if (value.length > PROXY_STRATEGY_LIMITS.url) return true;
  try {
    const url = new URL(value);
    return !['http:', 'https:'].includes(url.protocol)
      || !url.hostname || !!url.username || !!url.password || !!url.hash;
  } catch {
    return true;
  }
}

/** Field errors stay beside their input; only translation keys leave this module. */
export function strategyOptionErrors(form: ProxyStrategyOptionsForm, kind: ProxyStrategyKind): Partial<Record<ProxyStrategyOptionField, string>> {
  const applicable = applicableForm(kind, form);
  const errors: Partial<Record<ProxyStrategyOptionField, string>> = {};
  const url = applicable.url.trim();
  if (url && invalidTestUrl(url)) errors.url = OPTION_ERROR_KEYS.url;
  if (outOfRange(applicable.interval, PROXY_STRATEGY_LIMITS.interval.min, PROXY_STRATEGY_LIMITS.interval.max)) errors.interval = OPTION_ERROR_KEYS.interval;
  if (outOfRange(applicable.timeout, PROXY_STRATEGY_LIMITS.timeout.min, PROXY_STRATEGY_LIMITS.timeout.max)) errors.timeout = OPTION_ERROR_KEYS.timeout;
  if (outOfRange(applicable.tolerance, PROXY_STRATEGY_LIMITS.tolerance.min, PROXY_STRATEGY_LIMITS.tolerance.max)) errors.tolerance = OPTION_ERROR_KEYS.tolerance;
  return errors;
}

/** Only the values this kind applies travel; a blank field keeps the engine default. */
export function strategyOptions(form: ProxyStrategyOptionsForm, kind: ProxyStrategyKind): ProxyStrategyOptions {
  if (!PROXY_STRATEGY_OPTION_FIELDS[kind].length) return {};
  const applicable = applicableForm(kind, form);
  const errors = strategyOptionErrors(applicable, kind);
  const options: ProxyStrategyOptions = { lazy: form.lazy };
  const url = applicable.url.trim();
  if (url && !errors.url) options.url = url;
  const number = (field: 'interval' | 'timeout' | 'tolerance') => {
    const raw = applicable[field].trim();
    return !raw || errors[field] ? undefined : Number(raw);
  };
  const interval = number('interval');
  const timeout = number('timeout');
  const tolerance = number('tolerance');
  if (interval !== undefined) options.interval = interval;
  if (timeout !== undefined) options.timeout = timeout;
  if (tolerance !== undefined) options.tolerance = tolerance;
  return options;
}

export function strategyMemberId(member: ProxyStrategyMember): string {
  return `${member.sourceId}:${member.itemId}`;
}

/** Ordered, de-duplicated members: the first entry is the primary, blanks and extras are dropped. */
export function strategyOrderedMembers(members: ProxyStrategyMember[]): ProxyStrategyMember[] {
  const seen = new Set<string>();
  const ordered: ProxyStrategyMember[] = [];
  for (const member of members) {
    if (!member?.sourceId || !member?.itemId) continue;
    const key = strategyMemberId(member);
    if (seen.has(key)) continue;
    seen.add(key);
    ordered.push({ sourceId: member.sourceId, itemId: member.itemId });
    if (ordered.length >= PROXY_STRATEGY_LIMITS.members) break;
  }
  return ordered;
}

const STRATEGY_KIND_LABEL_KEYS: Record<ProxyStrategyKind, string> = {
  select: 'manualGroup', fallback: 'fallbackGroup', 'url-test': 'latencyGroup', 'load-balance': 'loadBalanceGroup',
};
const STRATEGY_KIND_HINT_KEYS: Record<ProxyStrategyKind, string> = {
  select: 'codex.proxy.catalog.strategyHintSelect',
  fallback: 'codex.proxy.catalog.strategyHintFallback',
  'url-test': 'codex.proxy.catalog.strategyHintUrlTest',
  'load-balance': 'codex.proxy.catalog.strategyHintLoadBalance',
};

/** Reuse the catalog's group labels; unknown kinds stay a plain strategy badge. */
export function strategyKindKey(kind: string): string {
  const label = STRATEGY_KIND_LABEL_KEYS[kind as ProxyStrategyKind];
  return `codex.proxy.catalog.${label ?? 'strategyTag'}`;
}

export function strategyHintKey(kind: ProxyStrategyKind): string {
  return STRATEGY_KIND_HINT_KEYS[kind];
}

/** A one-member policy is a fixed node: never describe it as failover-capable. */
export function strategyNoticeKey(kind: ProxyStrategyKind, memberCount: number): string {
  return memberCount <= 1 ? 'codex.proxy.catalog.strategySingleNotice' : strategyHintKey(kind);
}

/** The engine resolves group members by name, so a strategy may not shadow one of its nodes. */
export function strategyNameTaken(name: string, memberNames: string[]): boolean {
  const trimmed = name.trim();
  return !!trimmed && memberNames.includes(trimmed);
}

export interface ProxyStrategyCandidate { sourceId: string; sourceName: string; itemId: string; name: string; protocol: string; server?: string | null; port?: number | null }

/** Source selection is independent: a source name must never make all its nodes match. */
export function filterStrategyCandidates(candidates: ProxyStrategyCandidate[], query: string, sourceId: string) {
  const keywords = query.trim().toLocaleLowerCase().split(/\s+/u).filter(Boolean);
  return candidates.filter((entry) => (!sourceId || entry.sourceId === sourceId)
    && keywords.every((keyword) => [entry.name, entry.server ?? '', String(entry.port ?? ''),
      entry.server ? `${entry.server}:${entry.port ?? ''}` : '']
      .some((value) => value.toLocaleLowerCase().includes(keyword))));
}

/** A name is only a hint, not proof that the node is unusable. Never remove it. */
export function isPossibleProxyNotice(name: string): boolean {
  return /^(剩余流量|距离下次重置|套餐到期|到期时间|官网地址|.*官网地址\s*[:：]|remaining traffic|traffic remaining|expires?\s*[:：])/i.test(name.trim());
}

/** Selectable members: supported nodes of every non-strategy source, in catalog order. */
export function strategyCandidates(sources: ProxyCatalogSource[]): ProxyStrategyCandidate[] {
  return sources.flatMap((source) => source.kind === 'strategy' ? [] : source.nodes
    .filter((node) => node.supported)
    .map((node) => ({ sourceId: source.id, sourceName: source.name, itemId: node.id, name: node.name, protocol: node.protocol, server: node.server, port: node.port })));
}

function strategyGroup(source: ProxyCatalogSource) {
  return source.groups.find((entry) => PROXY_STRATEGY_KINDS.includes(entry.kind as ProxyStrategyKind)) ?? source.groups[0];
}

/** The stored strategy type, or null when the view carries no known group kind. */
export function strategyKindOf(source: ProxyCatalogSource): ProxyStrategyKind | null {
  const kind = strategyGroup(source)?.kind ?? '';
  return PROXY_STRATEGY_KINDS.includes(kind as ProxyStrategyKind) ? kind as ProxyStrategyKind : null;
}

function matchCandidate(node: ProxyCatalogNode, candidates: ProxyStrategyCandidate[]): ProxyStrategyCandidate | undefined {
  const exact = candidates.find((entry) => entry.itemId === node.id && entry.name === node.name);
  if (exact) return exact;
  const byId = candidates.filter((entry) => entry.itemId === node.id);
  if (byId.length === 1) return byId[0];
  const byName = candidates.filter((entry) => entry.name === node.name);
  return byName.length === 1 ? byName[0] : undefined;
}

export interface ProxyStrategyMemberView {
  key: string; name: string; sourceName: string; sourceId: string; itemId: string; matched: boolean;
  /** The original source is gone, but the saved copy inside the strategy still works. */
  originRemoved: boolean;
}

/** Read the ordered members back from the catalog view. A saved identity record is matched by its
 * exact (sourceId, itemId); only a strategy without records falls back to the legacy name match.
 * Members whose node is gone stay visible as unmatched instead of being silently replaced. */
export function strategyMemberViews(source: ProxyCatalogSource, sources: ProxyCatalogSource[]): ProxyStrategyMemberView[] {
  const group = strategyGroup(source);
  const order = group?.members.length ? group.members : source.nodes.map((node) => node.name);
  const candidates = strategyCandidates(sources);
  const records = source.strategyMembers;
  if (records?.length) {
    return records.map((record, index) => {
      const match = candidates.find((entry) => entry.sourceId === record.sourceId && entry.itemId === record.itemId);
      // 原来源已删除但策略内的副本仍可用：继续使用已保存副本，编辑保存时原样保留。
      const copy = source.nodes.find((entry) => entry.name === record.name);
      const originRemoved = !match && !sources.some((entry) => entry.id === record.sourceId) && !!copy?.supported;
      return {
        key: `${record.sourceId}:${record.itemId}#${index}`,
        name: record.name,
        sourceName: match?.sourceName ?? (originRemoved ? record.sourceName : ''),
        sourceId: record.sourceId,
        itemId: record.itemId,
        matched: !!match || originRemoved,
        originRemoved,
      };
    });
  }
  return order.map((name, index) => {
    const node = source.nodes.find((entry) => entry.name === name);
    const match = node ? matchCandidate(node, candidates) : undefined;
    return {
      key: `${node?.id ?? name}#${index}`,
      name: node?.name ?? name,
      sourceName: match?.sourceName ?? '',
      sourceId: match?.sourceId ?? '',
      itemId: match?.itemId ?? '',
      matched: !!match,
      originRemoved: false,
    };
  });
}

export interface ProxyStrategyEditorMember {
  sourceId: string; itemId: string; name: string; sourceName: string;
  /** The original source was deleted: this member is kept as the strategy's saved copy. */
  savedCopy: boolean;
}

export interface ProxyStrategyEditorMembers {
  /** Everything the dialog may submit, including members that only survive as saved copies. */
  members: ProxyStrategyEditorMember[];
  kept: ProxyStrategyEditorMember[];
  unmatched: string[];
}

/** Split the saved members for the editor: matched members stay submittable as they are, while a
 * deleted original source keeps using its saved copy instead of asking for a new selection. Only
 * members that no longer resolve stay out, exactly as before. */
export function strategyEditorMembers(views: ProxyStrategyMemberView[]): ProxyStrategyEditorMembers {
  const members = views.filter((view) => view.matched).map((view) => ({
    sourceId: view.sourceId, itemId: view.itemId, name: view.name, sourceName: view.sourceName, savedCopy: view.originRemoved,
  }));
  return { members, kept: members.filter((member) => member.savedCopy), unmatched: views.filter((view) => !view.matched).map((view) => view.name) };
}

export interface ProxyStrategyView {
  id: string; name: string; kind: ProxyStrategyKind | null; updatedAt: number; error: string | null; members: ProxyStrategyMemberView[];
}

/** Every saved strategy, newest first; only catalog identifiers are exposed. */
export function strategyViews(sources: ProxyCatalogSource[]): ProxyStrategyView[] {
  return sources.filter((source) => source.kind === 'strategy')
    .map((source) => ({
      id: source.id, name: source.name, kind: strategyKindOf(source), updatedAt: source.updatedAt,
      error: source.error, members: strategyMemberViews(source, sources),
    }))
    .sort((a, b) => b.updatedAt - a.updatedAt);
}

/** Wire args for one save: identifiers and explicit options only, never credentials. */
export function strategySaveArgs(draft: ProxyStrategyDraft): Record<string, unknown> {
  return {
    ...(draft.id ? { id: draft.id } : {}),
    name: draft.name.trim(),
    kind: draft.kind,
    members: strategyOrderedMembers(draft.members),
    options: draft.options,
  };
}

/** Strategy rejects reuse the catalog allowlist; only its own input copy differs. */
export function strategyErrorKey(error: unknown): string {
  const key = catalogErrorKey(error);
  return key === 'codex.proxy.catalog.invalidInput' ? 'codex.proxy.catalog.strategyErrorInvalid' : key;
}

function timed<T>(pending: Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('CATALOG_TIMEOUT')), 60_000);
    pending.then(resolve, reject).finally(() => clearTimeout(timer));
  });
}

/** Saves one strategy and returns the full catalog view the panels render from. */
export function saveProxyStrategy(draft: ProxyStrategyDraft): Promise<ProxyCatalog> {
  return withProxyEnginePrerequisite(timed(invoke<ProxyCatalog>('codex_proxy_strategy_save', strategySaveArgs(draft))));
}

export function removeProxyStrategy(id: string): Promise<ProxyCatalog> {
  return timed(invoke<ProxyCatalog>('codex_proxy_strategy_remove', { id }));
}
