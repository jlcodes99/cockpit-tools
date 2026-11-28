import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';
import type { ProxyCatalogSelections } from './codexProxyCatalogService';
import { withProxyEnginePrerequisite } from '../utils/codexProxyEnginePrerequisite';

/** Only public references and display names cross IPC, never an engine snapshot. */
export interface AntigravityProxyView {
  accountId: string;
  binding: CodexAccount['egress_proxy'] | null;
  error: string | null;
  legacyBound: boolean;
}
export const antigravityProxyApi = {
  list: () => invoke<AntigravityProxyView[]>('antigravity_proxy_list'),
  get: (accountId: string) => invoke<AntigravityProxyView>('antigravity_proxy_get', { accountId }),
  bind: (accountId: string, sourceId: string, itemId: string, selections: ProxyCatalogSelections, groupId?: string) =>
    withProxyEnginePrerequisite(invoke<AntigravityProxyView>('antigravity_proxy_bind', { accountId, sourceId, itemId, selections, groupId })),
  clear: (accountId: string) => invoke<AntigravityProxyView>('antigravity_proxy_clear', { accountId }),
};
