import { invoke } from '@tauri-apps/api/core';
import { WorkbuddyAccount } from '../types/workbuddy';

/**
 * CodeBuddy CLI 平台服务。
 * 账号数据完全复用 WorkBuddy 账号库（同一 SSO 账号体系），
 * 唯一差异是切号目标：写入 CodeBuddy CLI 官方认证文件
 * （%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth/Tencent-Cloud.coding-copilot.info）。
 */

export async function injectWorkbuddyToCodebuddyCli(accountId: string): Promise<string> {
  return await invoke('inject_workbuddy_to_codebuddy_cli', { accountId });
}

export async function importCodebuddyCliFromLocal(): Promise<WorkbuddyAccount[]> {
  return await invoke('import_codebuddy_cli_from_local');
}

