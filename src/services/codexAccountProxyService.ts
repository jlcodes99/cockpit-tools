import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';
import { proxyEnginePrerequisiteKey, withProxyEnginePrerequisite } from '../utils/codexProxyEnginePrerequisite';

export interface CodexProxyProbeResult {
  ip: string;
  latencyMs: number;
  checkedAt: number;
  protocol: string;
}

export interface CodexProxyRecentRequest {
  timestamp: number;
  modelId: string;
  success: boolean;
  httpStatus: number | null;
  latencyMs: number;
}

export function getCodexAccountProxyRecentRequests(accountId: string): Promise<CodexProxyRecentRequest[]> {
  return invoke('codex_account_proxy_recent_requests', { accountId });
}

export type CodexProxyRuntimeState = 'unbound' | 'direct' | 'idle' | 'starting' | 'running' | 'stopped' | 'missing';
export interface CodexProxyDesktopEntryStatus {
  state: 'listening' | 'failed' | 'stopped';
  port: number | null;
  requestCount: number;
  lastRequestState: 'none' | 'connecting' | 'forwarded' | 'failed';
  lastError: string | null;
}
export interface CodexProxyRuntimeStatus {
  account: CodexProxyRuntimeState;
  desktop: CodexProxyRuntimeState;
  sidecar?: CodexProxyRuntimeState;
  accountPort: number | null;
  desktopPort: number | null;
  sidecarPort?: number | null;
  accountNode?: string | null;
  desktopNode?: string | null;
  sidecarNode?: string | null;
  accountSelection?: CodexProxySelection | null;
  desktopSelection?: CodexProxySelection | null;
  sidecarSelection?: CodexProxySelection | null;
  desktopEntry?: CodexProxyDesktopEntryStatus | null;
  proxySource?: 'account' | 'unified' | 'none';
  effectiveProxy?: CodexAccount['egress_proxy'];
}
export interface CodexProxySelection { name: string; delayMs: number | null; checkedAt: number | null }
export function getCodexProxyRuntimeStatus(accountId: string): Promise<CodexProxyRuntimeStatus> {
  return invoke('get_codex_account_proxy_status', { accountId });
}

export function testCodexAccountProxy(accountId: string, requestId: string, proxyUrl: string | null): Promise<CodexProxyProbeResult> {
  return withProxyEnginePrerequisite(invoke('test_codex_account_egress_proxy', { accountId, requestId, proxyUrl }));
}

export function cancelCodexAccountProxy(accountId: string, requestId: string): Promise<void> {
  return invoke('cancel_codex_account_egress_proxy', { accountId, requestId });
}

export function proxyErrorKey(error: unknown, fallback: 'saveFailed' | 'probeFailed' = 'saveFailed'): string {
  const prerequisite = proxyEnginePrerequisiteKey(error);
  if (prerequisite) return prerequisite;
  const code = String(error).replace(/^Error:\s*/, '');
  if (code === 'UNIFIED_PROXY_STORAGE' || code === 'UNIFIED_PROXY_LOADING') return 'codex.proxy.unified.errorRead';
  if (code === 'UNIFIED_PROXY_TIMEOUT') return 'codex.proxy.unified.errorTimeout';
  const compatibility: Record<string,string> = {PROXY_ECH_DNS:'errorEchDns',PROXY_DNS_FAILED:'errorDns',PROXY_TLS_FAILED:'errorTls',PROXY_HANDSHAKE_FAILED:'errorHandshake',PROXY_CONNECTION_REFUSED:'errorRefused',PROXY_CONNECT_TIMEOUT:'errorConnectTimeout',PROXY_TARGET_FAILED:'errorTarget',PROXY_INTERFACE_FAILED:'errorInterface',PROXY_NETWORK_INVALID:'errorInterface'};
  if (compatibility[code]) return 'codex.proxy.catalog.' + compatibility[code];
  const keys: Record<string, string> = {
    PROXY_INVALID_URL: 'invalidUrl', PROXY_UNSUPPORTED_PROTOCOL: 'unsupportedProtocol',
    PROXY_ACCOUNT_UNSUPPORTED: 'unsupportedAccount', PROXY_SAVE_FAILED: 'saveFailed',
    PROXY_PROBE_TIMEOUT: 'probeTimeout', PROXY_PROBE_BUSY: 'probeBusy',
    PROXY_PROBE_CANCELLED: 'probeCancelled',
    PROXY_PROBE_FAILED: 'probeFailed', PROXY_PROBE_RESPONSE: 'probeFailed',
    PROXY_ENGINE_MISSING: 'engineMissing', PROXY_ENGINE_TIMEOUT: 'probeTimeout',
    PROXY_ENGINE_START_FAILED: 'probeFailed', PROXY_ENGINE_STOP_FAILED: 'probeFailed',
    PROXY_ENGINE_VERSION: 'probeFailed', PROXY_UNSUPPORTED_OPTION: 'unsupportedProtocol',
    PROXY_ENGINE_STOPPED: 'probeFailed', PROXY_RUNTIME_NOT_READY: 'probeFailed',
    PROXY_RUNTIME_FAILED: 'probeFailed', PROXY_RUNTIME_LIMIT: 'probeBusy',
    PROXY_RUNTIME_STARTING: 'probeBusy',
    PROXY_BINDING_CHANGED: 'saveFailed', PROXY_CLIENT_FAILED: 'probeFailed',
  };
  return `codex.proxy.${keys[code] || fallback}`;
}
