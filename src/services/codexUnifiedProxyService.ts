import { invoke } from '@tauri-apps/api/core';
import type { ProxyCatalogSelections } from './codexProxyCatalogService';
import { catalogErrorKey } from './codexProxyCatalogService';
import { proxyEnginePrerequisiteKey, withProxyEnginePrerequisite } from '../utils/codexProxyEnginePrerequisite';

export type CodexUnifiedProxyMode = 'off' | 'all_accounts';

/** A unified exit keeps only the catalog reference; the proxy snapshot stays on the backend. */
export interface CodexUnifiedProxyBinding {
  protocol: string;
  name: string;
  sourceName: string;
  sourceId: string;
  itemId: string;
  groupId: string | null;
  selectedName: string | null;
}

export interface CodexUnifiedProxyView {
  mode: CodexUnifiedProxyMode;
  binding: CodexUnifiedProxyBinding | null;
  /** Account kinds that can use an egress proxy. */
  eligibleAccountIds: string[];
  /** Eligible accounts whose own binding takes precedence over the unified exit. */
  independentAccountIds: string[];
  /** `CATALOG_NOT_FOUND` means the referenced source is gone and the backend switched itself off. */
  staleError: string | null;
}

export interface CodexUnifiedProxyPreview {
  binding: CodexUnifiedProxyBinding;
  eligibleAccountIds: string[];
  independentAccountIds: string[];
}

function selectionArgs(sourceId: string, itemId: string, selections: ProxyCatalogSelections, groupId?: string) {
  return { sourceId, itemId, selections, ...(groupId ? { groupId } : {}) };
}

export function getCodexUnifiedProxy(): Promise<CodexUnifiedProxyView> {
  return invoke('codex_unified_proxy_get');
}

/** Read-only count of what enabling or changing the exit would affect. */
export function previewCodexUnifiedProxy(
  sourceId: string,
  itemId: string,
  selections: ProxyCatalogSelections,
  groupId?: string,
): Promise<CodexUnifiedProxyPreview> {
  return invoke('codex_unified_proxy_preview', selectionArgs(sourceId, itemId, selections, groupId));
}

/** Applying always means `all_accounts`; there is no partial mode. */
export function applyCodexUnifiedProxy(
  sourceId: string,
  itemId: string,
  selections: ProxyCatalogSelections,
  groupId?: string,
): Promise<CodexUnifiedProxyView> {
  return withProxyEnginePrerequisite(invoke('codex_unified_proxy_apply', selectionArgs(sourceId, itemId, selections, groupId)));
}

export function disableCodexUnifiedProxy(): Promise<CodexUnifiedProxyView> {
  return invoke('codex_unified_proxy_disable');
}

/** Map backend codes to copy keys only; a raw backend message is never shown to the user. */
export function unifiedProxyErrorKey(error: unknown): string {
  const prerequisite = proxyEnginePrerequisiteKey(error);
  if (prerequisite) return prerequisite;
  const code = String(error).replace(/^Error:\s*/, '');
  const prefix = 'codex.proxy.unified.';
  if (code === 'UNIFIED_PROXY_STORAGE' || code === 'UNIFIED_PROXY_LOADING') return prefix + 'errorRead';
  if (code === 'UNIFIED_PROXY_BUSY') return prefix + 'errorBusy';
  if (code === 'UNIFIED_PROXY_TIMEOUT' || code === 'CATALOG_TIMEOUT') return prefix + 'errorTimeout';
  if (code === 'UNIFIED_PROXY_STALE' || code === 'CATALOG_NOT_FOUND' || code === 'CATALOG_CHANGED') return prefix + 'errorStale';
  if (code.startsWith('PROXY_')) return catalogErrorKey(error);
  return prefix + 'errorFailed';
}
