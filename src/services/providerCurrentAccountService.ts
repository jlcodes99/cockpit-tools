import { invoke } from '@tauri-apps/api/core';

export type ProviderCurrentPlatform =
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'claude_desktop_account'
  | 'claude_code_account'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'workbuddy'
  | 'github_copilot'
  | 'zed';

export async function getProviderCurrentAccountId(
  platform: ProviderCurrentPlatform,
): Promise<string | null> {
  return await invoke('get_provider_current_account_id', { platform });
}
