import { invoke } from '@tauri-apps/api/core';
export interface CodexProxyCountryPolicy { expectedCountry: string | null }
export function getCodexProxyCountryPolicy(accountId: string): Promise<CodexProxyCountryPolicy> {
  return invoke('get_codex_proxy_country_policy', { accountId });
}
export function setCodexProxyCountryPolicy(accountId: string, expectedCountry: string | null): Promise<CodexProxyCountryPolicy> {
  return invoke('set_codex_proxy_country_policy', { accountId, expectedCountry });
}

export interface CodexProxyEnv {
  document: string; source: string; revision: string; recognitionFailed: boolean;
}
export interface CodexCountryCheck {
  checkId: string; accountId: string; scope: 'launch' | 'manual';
  expectedCountry: string | null; actualCountry: string | null;
  status: 'checking' | 'passed' | 'blocked'; checkedAt: number; elapsedMs: number;
  source: string; reason: string | null; stale: boolean;
}
export const getCodexProxyEnv = (accountId: string): Promise<CodexProxyEnv> =>
  invoke('get_codex_proxy_env', { accountId });
export const setCodexProxyEnv = (accountId: string, document: string, revision: string): Promise<CodexProxyEnv> =>
  invoke('set_codex_proxy_env', { accountId, document, revision });
export const getCodexCountryChecks = (accountId: string): Promise<CodexCountryCheck[]> =>
  invoke('get_codex_proxy_country_checks', { accountId });
export const checkCodexCountry = (accountId: string): Promise<CodexCountryCheck> =>
  invoke('check_codex_proxy_country', { accountId });
