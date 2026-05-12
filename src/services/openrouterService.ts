import { invoke } from '@tauri-apps/api/core';
import { OpenRouterAccount, OpenRouterModel } from '../types/openrouter';

export async function listOpenRouterAccounts(): Promise<OpenRouterAccount[]> {
  return await invoke('list_openrouter_accounts');
}

export async function addOpenRouterAccount(apiKey: string): Promise<OpenRouterAccount> {
  return await invoke('add_openrouter_account', { apiKey });
}

export async function deleteOpenRouterAccount(accountId: string): Promise<void> {
  return await invoke('delete_openrouter_account', { accountId });
}

export async function deleteOpenRouterAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_openrouter_accounts', { accountIds });
}

export async function importOpenRouterFromJson(jsonContent: string): Promise<OpenRouterAccount[]> {
  return await invoke('import_openrouter_from_json', { jsonContent });
}

export async function exportOpenRouterAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_openrouter_accounts', { accountIds });
}

export async function refreshOpenRouterAccount(accountId: string): Promise<OpenRouterAccount> {
  return await invoke('refresh_openrouter_token', { accountId });
}

export async function refreshAllOpenRouterTokens(): Promise<number> {
  return await invoke('refresh_all_openrouter_tokens');
}

export async function updateOpenRouterAccountTags(
  accountId: string,
  tags: string[],
): Promise<OpenRouterAccount> {
  return await invoke('update_openrouter_account_tags', { accountId, tags });
}

export async function injectOpenRouterAccount(accountId: string): Promise<string> {
  return await invoke('inject_openrouter_account', { accountId });
}

export async function fetchOpenRouterCredits(
  accountId: string,
): Promise<{ total_credits: number | null; total_usage: number | null }> {
  return await invoke('fetch_openrouter_credits', { accountId });
}

export async function fetchOpenRouterModels(): Promise<OpenRouterModel[]> {
  return await invoke('fetch_openrouter_models');
}

export async function fetchOpenRouterActivity(
  accountId: string,
  days?: number,
): Promise<unknown> {
  return await invoke('fetch_openrouter_activity', { accountId, days });
}
