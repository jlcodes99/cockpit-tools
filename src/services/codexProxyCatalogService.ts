import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';
import type { CodexProxyProbeResult } from './codexAccountProxyService';
import { proxyEnginePrerequisiteKey, withProxyEnginePrerequisite } from '../utils/codexProxyEnginePrerequisite';

export interface ProxyCatalogNode { id: string; name: string; protocol: string; supported: boolean; error: string | null; insecure?: boolean; udp?: boolean | null; server?: string | null; port?: number | null }
export interface ProxyCatalogIssue { name: string; error: string }
export interface ProxyCatalogGroup { id: string; name: string; kind: string; members: string[]; supported: boolean; error: string | null; issues?: ProxyCatalogIssue[]; insecureNodeIds?: string[]; testUrl?: string | null }
/** A saved default is draft-only prefill: stable ids plus the manual member choices. */
export interface ProxyCatalogDefault { itemId: string; groupId: string | null; selections: ProxyCatalogSelections }
/** Strategy sources only: the original source and node each copied member came from. */
export interface ProxyCatalogStrategyMember { sourceId: string; itemId: string; name: string; sourceName: string }
export interface ProxyCatalogSource {
  id: string; name: string; kind: 'subscription' | 'manual' | 'strategy'; updatedAt: number; lastAttemptAt: number | null;
  revision: string; autoUpdate: boolean; error: string | null; nodes: ProxyCatalogNode[]; groups: ProxyCatalogGroup[]; needsRefresh?: boolean;
  default: ProxyCatalogDefault | null; defaultInvalidated: boolean;
  /** Subscription sources only: usage from the `subscription-userinfo` response header, never the raw header. */
  usage?: { upload: number; download: number; total: number; expireAt: number | null; at: number } | null;
  /** Strategy sources only: the saved health-check parameters, so editing never resets them. */
  strategyOptions?: { kind?: string; url?: string | null; interval?: number | null; timeout?: number | null; tolerance?: number | null; lazy?: boolean | null } | null;
  /** Strategy sources only: ordered original identities, so editing never guesses by name. */
  strategyMembers?: ProxyCatalogStrategyMember[] | null;
}
export interface ProxyCatalog { sources: ProxyCatalogSource[] }
export type ProxyCatalogSelections = Record<string, string>;
export interface ProxyCatalogDependencyAccount { id: string; name: string; email: string }
/** Read-only removal preview: identifiers and display names only, never node credentials. */
export interface ProxyCatalogDependencies {
  sourceId: string; revision: string; accounts: ProxyCatalogDependencyAccount[]; accountCount: number; unifiedProxy: boolean;
  strategies: string[]; strategyCount: number;
}
function timed<T>(pending: Promise<T>, command: string, args?: Record<string, unknown>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      if (typeof args?.requestId === 'string' && command !== 'codex_proxy_catalog_cancel') {
        void invoke('codex_proxy_catalog_cancel', { requestId: args.requestId }).catch(() => {});
      }
      reject(new Error('CATALOG_TIMEOUT'));
    }, 60_000);
    pending.then(resolve, reject).finally(() => clearTimeout(timer));
  });
}
function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const request = timed(invoke<T>(command, args), command, args);
  // Reads, cancellation and removal remain available without an engine.
  const guarded = ['codex_proxy_catalog_import', 'codex_proxy_catalog_bind', 'codex_proxy_catalog_probe', 'codex_proxy_catalog_latency'];
  return guarded.includes(command) ? withProxyEnginePrerequisite(request) : request;
}
let listing: Promise<ProxyCatalog> | null = null;
export function getProxyCatalog(): Promise<ProxyCatalog> {
  if (!listing) {
    listing = invoke<ProxyCatalog>('codex_proxy_catalog_list');
    void listing.then(() => { listing = null; }, () => { listing = null; });
  }
  return timed(listing, 'codex_proxy_catalog_list');
}
export function importProxyCatalog(requestId: string, name: string, input: string, kind: 'subscription' | 'manual', options?: ProxyImportOptions): Promise<ProxyCatalog> {
  return call('codex_proxy_catalog_import', { requestId, name, input, kind, options });
}
export function refreshProxyCatalog(requestId: string, sourceId: string): Promise<ProxyCatalog> {
  return call('codex_proxy_catalog_refresh', { requestId, sourceId });
}
export function cancelProxyCatalog(requestId: string): Promise<void> { return call('codex_proxy_catalog_cancel', { requestId }); }
export function removeProxyCatalog(sourceId: string): Promise<ProxyCatalog> { return call('codex_proxy_catalog_remove', { sourceId }); }
export function getProxyCatalogDependencies(sourceId: string): Promise<ProxyCatalogDependencies> {
  return call('codex_proxy_catalog_dependencies', { sourceId });
}
export function setProxyCatalogAutoUpdate(sourceId: string, enabled: boolean): Promise<ProxyCatalog> {
  return call('codex_proxy_catalog_set_auto_update', { sourceId, enabled });
}
export function setProxyCatalogDefault(sourceId: string, itemId: string, selections: ProxyCatalogSelections, groupId?: string): Promise<ProxyCatalog> {
  return call('codex_proxy_catalog_set_default', { sourceId, itemId, selections, ...(groupId === undefined ? {} : { groupId }) });
}
export function clearProxyCatalogDefault(sourceId: string): Promise<ProxyCatalog> {
  return call('codex_proxy_catalog_clear_default', { sourceId });
}
export function bindProxyCatalog(accountId: string, sourceId: string, itemId: string, selections: ProxyCatalogSelections, groupId?: string): Promise<CodexAccount> {
  return call('codex_proxy_catalog_bind', { accountId, sourceId, itemId, selections, ...(groupId === undefined ? {} : { groupId }) });
}
export function probeProxyCatalog(requestId: string, sourceId: string, itemId: string, selections: ProxyCatalogSelections): Promise<CodexProxyProbeResult> {
  return call('codex_proxy_catalog_probe', { requestId, sourceId, itemId, selections });
}
/** Only reachable selectors matter: unrelated groups never block binding a node. */
export function catalogSelectors(source: ProxyCatalogSource, itemId: string, selections: ProxyCatalogSelections): ProxyCatalogGroup[] {
  const required: ProxyCatalogGroup[] = [];
  const visited = new Set<string>();
  const visit = (group: ProxyCatalogGroup | undefined) => {
    if (!group || visited.has(group.id)) return;
    visited.add(group.id);
    if (group.kind === 'select') required.push(group);
    const members = group.kind === 'select' ? [selections[group.id]].filter(Boolean) : group.members;
    for (const name of members) visit(source.groups.find((entry) => entry.name === name));
  };
  visit(source.groups.find((entry) => entry.id === itemId));
  return required;
}
const catalogGroupLabels: Record<string, string> = {
  select: 'manualGroup', 'url-test': 'latencyGroup', fallback: 'fallbackGroup', 'load-balance': 'loadBalanceGroup',
};
/** Resolve only known group strategies to localized labels. */
export function catalogGroupKindKey(kind: string): string {
  return `codex.proxy.catalog.${Object.prototype.hasOwnProperty.call(catalogGroupLabels, kind) ? catalogGroupLabels[kind] : 'reasonStrategy'}`;
}
/** Only these built-ins preserve fail-closed group semantics for account proxies. */
export function isCatalogBlockingMember(name: string): boolean {
  return name === 'REJECT' || name === 'REJECT-DROP';
}
/** Explain unavailable resources using fixed copy only, never backend details. */
export function catalogUnsupportedKey(item?: { error?: string | null; kind?: string; protocol?: string }, memberName?: string): string {
  const prefix = 'codex.proxy.catalog.';
  if (!item) {
    if (isCatalogBlockingMember(memberName ?? '')) return prefix + 'blockingMemberHint';
    return prefix + (['DIRECT', 'PASS', 'PASS-RULE', 'COMPATIBLE'].includes(memberName ?? '') ? 'reasonBuiltin' : 'reasonMissing');
  }
  const reasons: Record<string, string> = {
    SUBSCRIPTION_GROUP_STRATEGY: 'reasonStrategy',
    SUBSCRIPTION_GROUP_OPTIONS: 'reasonOptions',
    SUBSCRIPTION_GROUP_TIMEOUT: 'reasonGroupTimeout',
    SUBSCRIPTION_PROVIDER_UNSUPPORTED: 'reasonProvider',
    SUBSCRIPTION_GROUP_UNAVAILABLE: 'reasonMembers',
    SUBSCRIPTION_GROUP_CYCLE: 'reasonCycle',
    SUBSCRIPTION_GROUP_MEMBER_MISSING: 'reasonMissing',
    SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED: 'reasonMemberUnsupported',
    PROXY_UNSUPPORTED_OPTION: 'reasonOptions',
    PROXY_TLS_INSECURE: 'reasonTlsInsecure',
    PROXY_TRANSPORT_UNSUPPORTED: 'reasonTransport',
    PROXY_ECH_UNSUPPORTED: 'reasonEch',
    SUBSCRIPTION_INVALID: 'reasonInvalid',
  };
  if (item.error && reasons[item.error]) return prefix + reasons[item.error];
  if (item.kind && !Object.prototype.hasOwnProperty.call(catalogGroupLabels, item.kind)) return prefix + 'reasonStrategy';
  if (item.error === 'SUBSCRIPTION_UNSUPPORTED') return prefix + 'reasonOptions';
  return prefix + 'reasonUnknown';
}
/** Never show an untrusted backend message or a subscription URL in an error. */
export function catalogErrorKey(error: unknown): string {
  const prerequisite = proxyEnginePrerequisiteKey(error);
  if (prerequisite) return prerequisite;
  const code = String(error).replace(/^Error:\s*/, '');
  if (code === 'UNIFIED_PROXY_STORAGE' || code === 'UNIFIED_PROXY_LOADING') return 'codex.proxy.unified.errorRead';
  if (code === 'UNIFIED_PROXY_TIMEOUT') return 'codex.proxy.unified.errorTimeout';
  const keys: Record<string, string> = {
    IMPORT_INVALID: 'invalidInput', IMPORT_AMBIGUOUS: 'ambiguous', IMPORT_EMPTY: 'noNewNodes', CATALOG_URL: 'invalidInput', CATALOG_NAME: 'invalidInput', CATALOG_FORMAT: 'invalidInput',
    CATALOG_INVALID: 'invalidInput', CATALOG_DOWNLOAD: 'downloadFailed', CATALOG_REDIRECT: 'downloadFailed',
    CATALOG_TIMEOUT: 'timeout', CATALOG_LIMIT: 'tooLarge', CATALOG_BUSY: 'busy', CATALOG_FINISHING: 'busy',
    CATALOG_CHANGED: 'changed', CATALOG_NOT_FOUND: 'changed', CATALOG_EXISTS: 'exists',
    CATALOG_PARTIAL_UNBIND: 'partialDelete',
    PROXY_ECH_DNS: 'errorEchDns', PROXY_DNS_FAILED: 'errorDns', PROXY_TLS_FAILED: 'errorTls', PROXY_HANDSHAKE_FAILED: 'errorHandshake',
    PROXY_CONNECTION_REFUSED: 'errorRefused', PROXY_CONNECT_TIMEOUT: 'errorConnectTimeout', PROXY_TARGET_FAILED: 'errorTarget',
    PROXY_INTERFACE_FAILED: 'errorInterface', PROXY_NETWORK_INVALID: 'errorInterface', PROXY_INTERFACES_FAILED: 'errorInterface',
    PROXY_PROBE_BUSY: 'busy', PROXY_RESOURCE_INVALID: 'unsupported', PROXY_UNSUPPORTED_OPTION: 'unsupported',
    SUBSCRIPTION_UNSUPPORTED: 'unsupported', SUBSCRIPTION_INVALID: 'invalidInput', SUBSCRIPTION_EMPTY: 'invalidInput',
    SUBSCRIPTION_TOO_LARGE: 'tooLarge', SUBSCRIPTION_DUPLICATE_NAME: 'invalidInput', SUBSCRIPTION_GROUP_UNAVAILABLE: 'unsupported',
    SUBSCRIPTION_GROUP_STRATEGY: 'reasonStrategy', SUBSCRIPTION_GROUP_OPTIONS: 'reasonOptions',
    SUBSCRIPTION_GROUP_CYCLE: 'reasonCycle', SUBSCRIPTION_GROUP_MEMBER_MISSING: 'reasonMissing',
    SUBSCRIPTION_GROUP_MEMBER_UNSUPPORTED: 'reasonMemberUnsupported',
    SUBSCRIPTION_GROUP_TIMEOUT: 'reasonGroupTimeout',
    SUBSCRIPTION_PROVIDER_UNSUPPORTED: 'reasonProvider', PROXY_TLS_INSECURE: 'reasonTlsInsecure',
    PROXY_TRANSPORT_UNSUPPORTED: 'reasonTransport', PROXY_ECH_UNSUPPORTED: 'reasonEch',
  };
  if (code === 'CATALOG_CANCELLED' || code === 'PROXY_PROBE_CANCELLED') return 'common.cancelled';
  if (code === 'PROXY_ENGINE_MISSING') return 'codex.proxy.engineMissing';
  if (code === 'PROXY_PROBE_TIMEOUT' || code === 'PROXY_ENGINE_TIMEOUT') return 'codex.proxy.probeTimeout';
  if (code === 'PROXY_PROBE_FAILED') return 'codex.proxy.probeFailed';
  return `codex.proxy.catalog.${keys[code] ?? 'failed'}`;
}

export interface ProxyImportOptions { protocol: string; format: string; skipInvalid: boolean; skipDuplicates: boolean; fallbackName: string }
export interface ProxyImportRow { line: number; name: string | null; protocol: string | null; server: string | null; port: number | null; authenticated: boolean; duplicate: boolean; error: string | null }
export interface ProxyImportPreview { structured: boolean; rows: ProxyImportRow[]; valid: number; invalid: number; duplicates: number }
export interface ProxyLatency { latencyMs: number; checkedAt: number }
export function previewProxyImport(input: string, options: ProxyImportOptions): Promise<ProxyImportPreview> {
  return call('codex_proxy_catalog_preview', { input, options });
}
export function renameProxySource(sourceId: string, name: string): Promise<ProxyCatalog> {
  return call('codex_proxy_catalog_rename', { sourceId, name });
}
export function measureProxyLatency(requestId: string, sourceId: string, nodeId: string, revision: string, groupId?: string): Promise<ProxyLatency> {
  return call('codex_proxy_catalog_latency', { requestId, sourceId, nodeId, revision, ...(groupId ? { groupId } : {}) });
}
/** Browsing keeps immediate leaf members and child groups as separate choices. */
export function catalogGroupNodes(source: ProxyCatalogSource, groupId: string, includeUnsupported = false): string[] {
  const group = source.groups.find((g) => g.id === groupId);
  const nodes = new Map(source.nodes.map((node) => [node.name, node]));
  return [...new Set((group?.members ?? []).flatMap((name) => { const node = nodes.get(name); return node && (node.supported || includeUnsupported) ? [node.id] : []; }))];
}
/** Test only the clicked item and its descendants, never its parent or other saved choices. */
export function catalogLatencyCandidates(source: ProxyCatalogSource, itemId: string): string[] {
  const ids = new Set<string>();
  const visited = new Set<string>();
  const node = (id: string) => { const value = source.nodes.find((entry) => entry.id === id); if (value?.supported) ids.add(value.id); };
  node(itemId);
  const visit = (id: string) => {
    if (visited.has(id)) return;
    visited.add(id);
    const group = source.groups.find((entry) => entry.id === id);
    for (const name of group?.members ?? []) {
      const leaf = source.nodes.find((entry) => entry.name === name);
      if (leaf) node(leaf.id);
      const child = source.groups.find((entry) => entry.name === name);
      if (child) visit(child.id);
    }
  };
  visit(itemId);
  return [...ids];
}
export function setProxyNodeInsecure(sourceId: string, nodeId: string, revision: string, enabled: boolean): Promise<ProxyCatalog> { return call('codex_proxy_catalog_insecure', { sourceId, nodeId, revision, enabled }); }
export function setProxyGroupInsecure(sourceId: string, groupId: string, revision: string, enabled: boolean): Promise<ProxyCatalog> { return call('codex_proxy_catalog_group_insecure', { sourceId, groupId, revision, enabled }); }
