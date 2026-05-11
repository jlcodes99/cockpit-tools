import { invoke } from '@tauri-apps/api/core';

export interface DevinCliAccount {
  id: string;
  name: string;
  email?: string | null;
  tier?: string | null;
  plan?: string | null;
  orgId?: string | null;
  createdAt: number;
  lastUsedAt?: number | null;
  needsLogin: boolean;
}

export interface DevinAuthStatus {
  loggedIn: boolean;
  email?: string | null;
  name?: string | null;
  tier?: string | null;
  plan?: string | null;
}

export async function listDevinCliAccounts(): Promise<DevinCliAccount[]> {
  return await invoke('devin_cli_list_accounts');
}

export async function isDevinCliInstalled(): Promise<boolean> {
  return await invoke('devin_cli_is_devin_installed');
}

export async function addDevinCliAccount(name: string): Promise<DevinCliAccount> {
  return await invoke('devin_cli_add_account', { name });
}

export async function removeDevinCliAccount(id: string): Promise<DevinCliAccount> {
  return await invoke('devin_cli_remove_account', { id });
}

export async function renameDevinCliAccount(id: string, newName: string): Promise<DevinCliAccount> {
  return await invoke('devin_cli_rename_account', { id, newName });
}

export async function loginDevinCliAccount(id: string, terminal?: string): Promise<string> {
  return await invoke('devin_cli_login_account', { id, terminal: terminal ?? null });
}

export async function useDevinCliAccount(id: string, args?: string[], terminal?: string): Promise<string> {
  return await invoke('devin_cli_use_account', { id, args: args ?? [], terminal: terminal ?? null });
}

export async function checkDevinCliAuthStatus(id: string): Promise<DevinAuthStatus> {
  return await invoke('devin_cli_check_auth_status', { id });
}

export async function syncAllDevinCliAccounts(): Promise<DevinCliAccount[]> {
  return await invoke('devin_cli_sync_all_accounts');
}
