import { invoke } from '@tauri-apps/api/core';
import type { ClaudeAccount } from '../types/claude';

export interface ClaudeCommandInfo {
  accountId: string;
  configDir: string;
  command: string;
}

export async function listClaudeAccounts(): Promise<ClaudeAccount[]> {
  return await invoke('list_claude_accounts');
}

export async function createClaudeAccount(payload: {
  name?: string | null;
  loginMode?: string | null;
  loginHintEmail?: string | null;
  anthropicBaseUrl?: string | null;
  anthropicAuthToken?: string | null;
  disableNonessentialTraffic?: boolean | null;
}): Promise<ClaudeAccount> {
  return await invoke('create_claude_account', {
    name: payload.name ?? null,
    loginMode: payload.loginMode ?? null,
    loginHintEmail: payload.loginHintEmail ?? null,
    anthropicBaseUrl: payload.anthropicBaseUrl ?? null,
    anthropicAuthToken: payload.anthropicAuthToken ?? null,
    disableNonessentialTraffic: payload.disableNonessentialTraffic ?? null,
  });
}

export async function deleteClaudeAccount(accountId: string): Promise<void> {
  return await invoke('delete_claude_account', { accountId });
}

export async function deleteClaudeAccounts(accountIds: string[]): Promise<void> {
  return await invoke('delete_claude_accounts', { accountIds });
}

export async function refreshClaudeAccount(accountId: string): Promise<ClaudeAccount> {
  return await invoke('refresh_claude_account', { accountId });
}

export async function refreshAllClaudeAccounts(): Promise<number> {
  return await invoke('refresh_all_claude_accounts');
}

export async function injectClaudeAccount(accountId: string): Promise<string> {
  return await invoke('inject_claude_account', { accountId });
}

export async function updateClaudeAccountTags(
  accountId: string,
  tags: string[],
): Promise<ClaudeAccount> {
  return await invoke('update_claude_account_tags', { accountId, tags });
}

export async function exportClaudeAccounts(accountIds: string[]): Promise<string> {
  return await invoke('export_claude_accounts', { accountIds });
}

export async function importClaudeFromJson(jsonContent: string): Promise<ClaudeAccount[]> {
  return await invoke('import_claude_from_json', { jsonContent });
}

export async function getClaudeAccountsIndexPath(): Promise<string> {
  return await invoke('get_claude_accounts_index_path');
}

export async function getClaudeLoginCommand(accountId: string): Promise<ClaudeCommandInfo> {
  return await invoke('get_claude_login_command', { accountId });
}

export async function getClaudeLaunchCommand(accountId: string): Promise<ClaudeCommandInfo> {
  return await invoke('get_claude_launch_command', { accountId });
}

export async function executeClaudeLoginCommand(
  accountId: string,
  terminal?: string,
): Promise<string> {
  return await invoke('execute_claude_login_command', {
    accountId,
    terminal: terminal ?? null,
  });
}

export async function executeClaudeLaunchCommand(
  accountId: string,
  terminal?: string,
): Promise<string> {
  return await invoke('execute_claude_launch_command', {
    accountId,
    terminal: terminal ?? null,
  });
}
