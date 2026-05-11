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

export async function listDevinCliAccounts(): Promise<DevinCliAccount[]> {
  return await invoke('read_devin_cli_accounts');
}

export async function isDevinCliSwitcherInstalled(): Promise<boolean> {
  return await invoke('devin_cli_is_installed');
}

export async function executeDevinCliCommand(
  args: string[],
  terminal?: string,
): Promise<string> {
  return await invoke('execute_devin_cli_command', {
    args,
    terminal: terminal ?? null,
  });
}
