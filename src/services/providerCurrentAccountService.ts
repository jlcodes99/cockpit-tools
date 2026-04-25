import { invoke } from '@tauri-apps/api/core';

export type ProviderCurrentPlatform =
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'claude'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'workbuddy'
  | 'github_copilot'
  | 'zed';

export async function getProviderCurrentAccountId(
  platform: ProviderCurrentPlatform,
): Promise<string | null> {
  return await invoke('get_provider_current_account_id', { platform });
}
