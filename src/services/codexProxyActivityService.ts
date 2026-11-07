import { invoke } from '@tauri-apps/api/core';

export type ProxyActivityChannel = 'account' | 'desktop' | 'sidecar';

export interface ProxyActivityLog {
  id: number;
  timestamp: number;
  channel: ProxyActivityChannel;
  level: 'info' | 'warning' | 'error';
  network: 'TCP' | 'UDP' | null;
  target: string | null;
  rule: string | null;
  outbound: string | null;
  errorCode: string | null;
}

export interface ProxyActiveConnection {
  id: string;
  channel: ProxyActivityChannel;
  startedAt: string;
  network: string | null;
  target: string;
  chains: string[];
  rule: string | null;
  upload: number;
  download: number;
}

export interface ProxyActivitySnapshot {
  supported: boolean;
  enabled: boolean;
  captureError: boolean;
  connectionsError: boolean;
  logs: ProxyActivityLog[];
  connections: ProxyActiveConnection[];
}

/** Aggregate counters only: no targets, chains, rules or credentials. */
export interface ProxyActivitySummaryEntry {
  accountId: string;
  supported: boolean;
  enabled: boolean;
  connectionCount: number;
  upload: number;
  download: number;
}

export const getProxyActivity = (accountId: string): Promise<ProxyActivitySnapshot> =>
  invoke('codex_proxy_activity_snapshot', { accountId });

export const getProxyActivitySummary = (): Promise<ProxyActivitySummaryEntry[]> =>
  invoke('codex_proxy_activity_summary');

export const setProxyActivityEnabled = (accountId: string, enabled: boolean): Promise<void> =>
  invoke('codex_proxy_activity_set_enabled', { accountId, enabled });

export const clearProxyActivity = (accountId: string): Promise<void> =>
  invoke('codex_proxy_activity_clear', { accountId });
