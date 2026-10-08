import type { CodexQuotaErrorInfo } from '../types/codex';

// Keep in sync with codex_quota::is_local_proxy_error and the engine preflight.
// Match complete local messages only: an upstream body/code may contain these words.
const LOCAL_PROXY_ERROR_CODES = new Set([
  'PROXY_RUNTIME_LIMIT',
  'PROXY_RUNTIME_BUSY',
  'PROXY_RUNTIME_READ_TIMEOUT',
  'PROXY_RUNTIME_CAPACITY',
  'PROXY_RUNTIME_FAILED',
  'PROXY_RUNTIME_NOT_READY',
  'PROXY_ENGINE_STOPPED',
  'PROXY_ENGINE_STOP_FAILED',
  'PROXY_STATUS_FAILED',
  'PROXY_CLIENT_FAILED',
  'PROXY_INVALID_URL',
  'PROXY_UNSUPPORTED_PROTOCOL',
  'PROXY_RESOURCE_INVALID',
  'PROXY_UNSUPPORTED_OPTION',
  'PROXY_RESOURCE_SELECTION_REQUIRED',
  'PROXY_BINDING_CHANGED',
  'PROXY_ACCOUNT_UNSUPPORTED',
  'UNIFIED_PROXY_LOADING',
  'UNIFIED_PROXY_TIMEOUT',
  'UNIFIED_PROXY_STORAGE',
  'UNIFIED_PROXY_INVALID',
  'PROXY_ENGINE_MISSING',
  'PROXY_ENGINE_TIMEOUT',
  'PROXY_ENGINE_VERSION',
  'PROXY_ENGINE_START_FAILED',
  'ENGINE_INSTALL_VERIFY',
  'ENGINE_INSTALL_IO',
  'ENGINE_INSTALL_TIMEOUT',
  'ENGINE_INSTALL_BUSY',
  'ENGINE_INSTALL_UNSUPPORTED',
  'ENGINE_INSTALL_ARCHIVE',
  'ENGINE_INSTALL_TOO_LARGE',
]);

export function getLocalCodexProxyErrorCode(message?: string | null): string | null {
  let localMessage = message?.trim() ?? '';
  for (const prefix of [
    'Token 已过期，刷新 Token 失败: ',
    'Token 已过期且刷新失败: ',
  ]) {
    if (localMessage.startsWith(prefix)) localMessage = localMessage.slice(prefix.length);
  }
  return LOCAL_PROXY_ERROR_CODES.has(localMessage) ? localMessage : null;
}

export function isLocalCodexProxyError(message?: string | null): boolean {
  return getLocalCodexProxyErrorCode(message) !== null;
}

/** Historical local read failures must not appear as account/quota problems. */
export function getCodexAccountQuotaError(
  error?: CodexQuotaErrorInfo | null,
): CodexQuotaErrorInfo | undefined {
  return error && !isLocalCodexProxyError(error.message) ? error : undefined;
}
