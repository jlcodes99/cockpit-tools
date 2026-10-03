import { invoke } from '@tauri-apps/api/core';

export interface AntigravityCliStatus {
  executable: string | null;
  version: string | null;
  version_error: string | null;
  config_dir: string;
  api_key_mode: boolean;
  credential_present: boolean;
  credential_error: string | null;
  current_account_id: string | null;
}

export function getAntigravityCliStatus(): Promise<AntigravityCliStatus> {
  return invoke('antigravity_cli_status');
}

export function launchAntigravityCli(workingDirectory?: string): Promise<void> {
  return invoke('antigravity_cli_launch', { workingDirectory });
}
